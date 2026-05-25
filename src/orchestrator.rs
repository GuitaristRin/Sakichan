use crate::backend::{create_backend, LlmBackend, ModelOptions, UsageStats};
use crate::config::SakichanConfig;
use crate::display::*;
use crate::executor::Executor;
use crate::handoff::{
    build_fix_prompt, build_generation_prompt, parse_sakichan_tags, sakichan_tag_to_module,
    Constraint, Issue, IssueSeverity, MajorRestartReport, Module, ReviewResult, ReviewStatus,
};
use crate::logger::Logger;
use crate::rules::RulesManager;
use crate::slots::SlotRole;
use crate::state::{AppState, DetectedTool};
use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

// ── Constants ─────────────────────────────────────────────────────────────────

const REVIEW_MAX_ATTEMPTS: usize = 3;
const CARGO_FIX_MAX: usize = 5;
const MAX_RESTART: usize = 3;

// ── Model call helpers ────────────────────────────────────────────────────────

fn record_usage(state: &Arc<Mutex<AppState>>, usage: &UsageStats) {
    if let Ok(mut st) = state.lock() {
        st.usage.add(usage);
        let _ = st.save_usage();
    }
}

fn add_tokens(global_tokens: &Arc<Mutex<u64>>, usage: &UsageStats) {
    if let Ok(mut t) = global_tokens.lock() {
        *t += usage.input_tokens + usage.output_tokens;
    }
}

fn call_model(
    backend: &dyn LlmBackend,
    model: &str,
    prompt: &str,
    opts: &ModelOptions,
    work_dir: &Path,
    state: &Arc<Mutex<AppState>>,
    global_tokens: &Arc<Mutex<u64>>,
    global_start: &Arc<Instant>,
    spin: SpinnerState,
) -> Result<String> {
    let spinner = Spinner::new(spin, Arc::clone(global_tokens), Arc::clone(global_start));
    spinner.set_hint(model);
    let (raw, usage) = backend.generate_complete(model, prompt, opts)?;
    add_tokens(global_tokens, &usage);
    spinner.stop();
    record_usage(state, &usage);

    let (clean, tool_results) = process_system_calls(&raw, work_dir);
    if tool_results.is_empty() {
        return Ok(raw);
    }

    println!("{GRAY}  🔍 解析工具调用结果，追加上下文重新生成...{RESET}");
    let followup = format!(
        "{prompt}\n\n## Tool call results:\n{tool_results}\n\nNow generate the complete output:"
    );
    let sp2 = Spinner::new(SpinnerState::Crafting, Arc::clone(global_tokens), Arc::clone(global_start));
    let (raw2, usage2) = match backend.generate_complete(model, &followup, opts) {
        Ok(r) => r,
        Err(_) => { sp2.stop(); return Ok(clean); }
    };
    add_tokens(global_tokens, &usage2);
    sp2.stop();
    record_usage(state, &usage2);
    Ok(raw2)
}

// ── Slot model/options resolution ─────────────────────────────────────────────

fn slot_model(role: SlotRole, state: &Arc<Mutex<AppState>>) -> String {
    state.lock().unwrap()
        .slot_assignments
        .get(role.name())
        .cloned()
        .unwrap_or_else(|| match role {
            SlotRole::Architect | SlotRole::SeniorEngineer => "deepseek-r1:8b".to_string(),
            _ => "qwen2.5-coder:7b".to_string(),
        })
}

fn slot_opts(role: SlotRole, config: &SakichanConfig) -> ModelOptions {
    ModelOptions::from_preset(&config.get_preset_for_slot(role.name()))
}

// ── Module parsing ────────────────────────────────────────────────────────────

fn parse_modules(plan_text: &str) -> Vec<Module> {
    // Try SAKICHAN markup first
    let sakichan = parse_sakichan_tags(plan_text);
    if !sakichan.is_empty() {
        return sakichan.into_iter().map(sakichan_tag_to_module).collect();
    }

    // Fall back to legacy [MODULE:] format
    let text = plan_text
        .find("[MODULE_PLAN]")
        .map(|p| &plan_text[p + "[MODULE_PLAN]".len()..])
        .unwrap_or(plan_text);

    let mut modules: Vec<Module> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_block = String::new();

    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("[MODULE:") {
            if let Some(name) = current_name.take() {
                if !current_block.trim().is_empty() {
                    modules.push(build_module(name, &current_block));
                }
            }
            current_block.clear();
            let name = t.trim_start_matches("[MODULE:").trim_end_matches(']').trim().to_string();
            if !name.is_empty() { current_name = Some(name); }
        } else if current_name.is_some() {
            current_block.push_str(line);
            current_block.push('\n');
        }
    }
    if let Some(name) = current_name {
        if !current_block.trim().is_empty() {
            modules.push(build_module(name, &current_block));
        }
    }

    if modules.is_empty() {
        let any_compile = plan_text.to_lowercase().contains("needs_compile: true")
            || plan_text.to_lowercase().contains("needs_compile:true");
        let slot = detect_slot_from_text(plan_text);
        modules.push(Module {
            name: "主模块".to_string(),
            full_spec: plan_text.chars().take(2000).collect(),
            needs_compile: any_compile,
            assigned_slot: slot,
            ..Default::default()
        });
    }
    modules
}

fn build_module(name: String, block: &str) -> Module {
    let mut needs_compile = false;
    let mut assigned_slot = SlotRole::Programmer;
    let inputs = Vec::new();
    let outputs = Vec::new();
    let mut constraints = Vec::new();
    let mut verification = Vec::new();
    let mut in_constraints = false;
    let mut in_verification = false;

    for line in block.lines() {
        let t = line.trim();

        if let Some(v) = t.strip_prefix("needs_compile:") {
            needs_compile = v.trim() == "true";
            in_constraints = false; in_verification = false;
        } else if let Some(v) = t.strip_prefix("assigned_slot:") {
            assigned_slot = SlotRole::from_str(v.trim()).unwrap_or(SlotRole::Programmer);
            in_constraints = false; in_verification = false;
        } else if let Some(v) = t.strip_prefix("assigned_model:") {
            // backward compat
            assigned_slot = SlotRole::from_str(v.trim()).unwrap_or(SlotRole::Programmer);
            in_constraints = false; in_verification = false;
        } else if t.starts_with("inputs:") || t.starts_with("input:") {
            in_constraints = false; in_verification = false;
        } else if t.starts_with("outputs:") || t.starts_with("output:") {
            in_constraints = false; in_verification = false;
        } else if t.starts_with("constraints:") {
            in_constraints = true; in_verification = false;
        } else if t.starts_with("verification:") || t.starts_with("verify:") {
            in_constraints = false; in_verification = true;
        } else if in_constraints && (t.starts_with("- ") || t.starts_with("* ")) {
            let content = t.trim_start_matches("- ").trim_start_matches("* ");
            if let Some(c) = Constraint::parse_line(content) {
                constraints.push(c);
            }
        } else if in_verification && (t.starts_with("- ") || t.starts_with("* ")) {
            let content = t.trim_start_matches("- ").trim_start_matches("* ");
            if !content.is_empty() { verification.push(content.to_string()); }
        }
    }

    Module {
        name,
        inputs,
        outputs,
        implementation: String::new(),
        constraints,
        verification,
        assigned_slot,
        needs_compile,
        full_spec: block.to_string(),
    }
}

fn detect_slot_from_text(text: &str) -> SlotRole {
    let up = text.to_uppercase();
    if up.contains("DSR1") || up.contains("DEEPSEEK") || up.contains("ARCHITECT") {
        SlotRole::Architect
    } else {
        SlotRole::Programmer
    }
}

// ── Question parsing ──────────────────────────────────────────────────────────

fn parse_questions(response: &str) -> Vec<String> {
    let mut questions = Vec::new();
    for line in response.lines() {
        let t = line.trim();
        if t.len() < 6 { continue; }
        let is_numbered = t.chars().next().map_or(false, |c| c.is_ascii_digit())
            && (t.contains(". ") || t.contains("、") || t.contains("）"));
        let has_q = t.contains('❓') || t.ends_with('?') || t.ends_with('？');
        if is_numbered || has_q {
            let q = if let Some(pos) = t.find(". ") {
                t[pos + 2..].trim().to_string()
            } else if let Some(pos) = t.find("、") {
                t[pos + 3..].trim().to_string()
            } else if t.starts_with('❓') {
                t[3..].trim().to_string()
            } else {
                t.to_string()
            };
            if !q.is_empty() && !questions.contains(&q) { questions.push(q); }
        }
    }
    questions.truncate(3);
    questions
}

// ── File utilities ────────────────────────────────────────────────────────────

fn list_files_in(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().into_string().unwrap_or_default();
            if name.starts_with('.') || name == "target" { continue; }
            if path.is_file() {
                files.push(name);
            } else if path.is_dir() {
                for sub in list_files_in(&path) {
                    files.push(format!("{name}/{sub}"));
                }
            }
        }
    }
    files.sort();
    files
}

fn generate_project_tree(work_dir: &Path) -> String {
    let name = work_dir.file_name().unwrap_or_default().to_string_lossy();
    let mut lines = vec![format!("{}/", name)];
    build_tree(work_dir, "", &mut lines);
    lines.join("\n")
}

fn build_tree(dir: &Path, prefix: &str, lines: &mut Vec<String>) {
    const SKIP: &[&str] = &["target", ".git"];
    let Ok(read) = fs::read_dir(dir) else { return };
    let parent = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
    let mut items: Vec<_> = read.filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            if SKIP.contains(&n.as_str()) { return false; }
            if parent == ".sakichan" && (n == "sandboxes" || n == "sessions") { return false; }
            true
        })
        .collect();
    items.sort_by_key(|e| (e.path().is_file() as u8, e.file_name()));
    for (i, entry) in items.iter().enumerate() {
        let is_last = i == items.len() - 1;
        let conn = if is_last { "└── " } else { "├── " };
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.path().is_dir();
        lines.push(format!("{}{}{}{}", prefix, conn, name, if is_dir { "/" } else { "" }));
        if is_dir {
            let child = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
            build_tree(&entry.path(), &child, lines);
        }
    }
}

fn build_code_context(written_files: &HashMap<String, String>) -> String {
    if written_files.is_empty() { return "(no files generated yet)".to_string(); }
    let mut sorted: Vec<_> = written_files.iter().collect();
    sorted.sort_by_key(|(k, _)| k.as_str());
    sorted.iter().map(|(f, c)| format!("=== {} ===\n{}\n\n", f, c)).collect()
}

fn solution_section(design: &str) -> String {
    if design.is_empty() { String::new() }
    else { format!("## 解决方案设计（Phase C，请仔细阅读）\n{design}\n\n") }
}

// ── Grep helpers ──────────────────────────────────────────────────────────────

const GREP_TEXT_EXTS: &[&str] = &[
    "rs", "toml", "md", "txt", "json", "yaml", "yml",
    "py", "js", "ts", "tsx", "cpp", "c", "h", "go", "java",
];
const GREP_SKIP_DIRS: &[&str] = &["target", ".git", ".sakichan"];

fn grep_file(root: &Path, path: &Path, pattern: &str, ctx: usize, results: &mut Vec<String>) {
    let Ok(content) = fs::read_to_string(path) else { return };
    let rel = path.strip_prefix(root).unwrap_or(path);
    let lines: Vec<&str> = content.lines().collect();
    let matches: Vec<usize> = lines.iter().enumerate()
        .filter(|(_, l)| l.to_lowercase().contains(pattern))
        .map(|(i, _)| i)
        .collect();
    if matches.is_empty() { return; }
    let mut shown = std::collections::BTreeSet::<usize>::new();
    for &idx in &matches {
        for j in idx.saturating_sub(ctx)..(idx + ctx + 1).min(lines.len()) { shown.insert(j); }
    }
    let mut prev: Option<usize> = None;
    for j in shown {
        if let Some(p) = prev { if j > p + 1 { results.push("  ---".to_string()); } }
        let marker = if matches.contains(&j) { "▶" } else { " " };
        results.push(format!("{}:{} {} {}", rel.display(), j + 1, marker, lines[j]));
        prev = Some(j);
    }
}

fn grep_walk(root: &Path, dir: &Path, pattern: &str, ctx: usize, results: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if GREP_SKIP_DIRS.contains(&name.as_str()) { continue; }
        let path = entry.path();
        if path.is_dir() {
            grep_walk(root, &path, pattern, ctx, results);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if GREP_TEXT_EXTS.contains(&ext) { grep_file(root, &path, pattern, ctx, results); }
        }
    }
}

fn grep_in_dir(work_dir: &Path, pattern: &str, search_path: &str, ctx: usize) -> String {
    let pat = pattern.to_lowercase();
    let dir = if search_path.is_empty() || search_path == "." {
        work_dir.to_path_buf()
    } else {
        work_dir.join(search_path)
    };
    let mut results: Vec<String> = Vec::new();
    grep_walk(&dir, &dir, &pat, ctx, &mut results);
    if results.is_empty() {
        format!("[GREP: no matches for \"{pattern}\" in {search_path}]")
    } else {
        let count = results.iter().filter(|l| l.contains('▶')).count();
        format!("[GREP \"{pattern}\" — {count} matches]\n{}\n[/GREP]", results.join("\n"))
    }
}

// ── Web search ────────────────────────────────────────────────────────────────

fn web_search(query: &str) -> String {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Saki-chan/0.4.0")
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("[WEB SEARCH ERROR: {e}]"),
    };
    let resp = match client
        .get("https://api.duckduckgo.com/")
        .query(&[("q", query), ("format", "json"), ("no_html", "1"), ("skip_disambig", "1")])
        .send()
    {
        Ok(r) => r,
        Err(e) => return format!("[WEB SEARCH ERROR: {e}]"),
    };
    let json: serde_json::Value = match resp.json() {
        Ok(v) => v,
        Err(e) => return format!("[WEB SEARCH ERROR: parse: {e}]"),
    };
    let mut result = String::new();
    if let Some(t) = json["AbstractText"].as_str() {
        if !t.is_empty() { result.push_str(t); result.push('\n'); }
    }
    if let Some(topics) = json["RelatedTopics"].as_array() {
        for topic in topics.iter().take(5) {
            if let Some(t) = topic["Text"].as_str() {
                if !t.is_empty() { result.push_str("- "); result.push_str(t); result.push('\n'); }
            }
        }
    }
    if result.is_empty() {
        format!("[WEB SEARCH: no results for \"{query}\"]")
    } else {
        format!("[WEB SEARCH: \"{query}\"]\n{}\n[/WEB SEARCH]", result.chars().take(1000).collect::<String>())
    }
}

// ── System call processing ────────────────────────────────────────────────────

fn process_system_calls(response: &str, work_dir: &Path) -> (String, String) {
    let mut tool_results = String::new();
    let mut clean = response.to_string();

    while let Some(start) = clean.find("[SYSTEM:read_file") {
        if let Some(end) = clean[start..].find(']') {
            let tag = clean[start..start + end + 1].to_string();
            let result = tag.find("path=\"").map(|p| {
                let r = &tag[p + 6..];
                let path = &r[..r.find('"').unwrap_or(r.len())];
                match fs::read_to_string(work_dir.join(path)) {
                    Ok(c) => format!("\n[FILE:{path}]\n{c}\n[/FILE]"),
                    Err(e) => format!("\n[ERROR reading {path}: {e}]"),
                }
            }).unwrap_or_default();
            tool_results.push_str(&result);
            clean = format!("{}{}", &clean[..start], &clean[start + end + 1..]);
        } else { break; }
    }

    while let Some(start) = clean.find("[SYSTEM:list_files") {
        if let Some(end) = clean[start..].find(']') {
            let tag = clean[start..start + end + 1].to_string();
            let dir = tag.find("path=\"").map(|p| {
                let r = &tag[p + 6..];
                work_dir.join(&r[..r.find('"').unwrap_or(r.len())])
            }).unwrap_or_else(|| work_dir.to_path_buf());
            let files = list_files_in(&dir);
            tool_results.push_str(&format!("\n[FILES in {}]\n{}\n[/FILES]", dir.display(), files.join("\n")));
            clean = format!("{}{}", &clean[..start], &clean[start + end + 1..]);
        } else { break; }
    }

    while let Some(start) = clean.find("[SYSTEM:grep") {
        if let Some(end) = clean[start..].find(']') {
            let tag = clean[start..start + end + 1].to_string();
            let pattern = tag.find("pattern=\"").map(|p| {
                let r = &tag[p + 9..]; r[..r.find('"').unwrap_or(r.len())].to_string()
            }).unwrap_or_default();
            let path = tag.find("path=\"").map(|p| {
                let r = &tag[p + 6..]; r[..r.find('"').unwrap_or(r.len())].to_string()
            }).unwrap_or_else(|| ".".to_string());
            let ctx = tag.find("context=\"").and_then(|p| {
                let r = &tag[p + 9..]; r[..r.find('"').unwrap_or(r.len())].parse::<usize>().ok()
            }).unwrap_or(2);
            if !pattern.is_empty() {
                tool_results.push_str(&format!("\n{}\n", grep_in_dir(work_dir, &pattern, &path, ctx)));
            }
            clean = format!("{}{}", &clean[..start], &clean[start + end + 1..]);
        } else { break; }
    }

    while let Some(start) = clean.find("[SYSTEM:web_search") {
        if let Some(end) = clean[start..].find(']') {
            let tag = clean[start..start + end + 1].to_string();
            let query = tag.find("query=\"").map(|p| {
                let r = &tag[p + 7..]; r[..r.find('"').unwrap_or(r.len())].to_string()
            }).unwrap_or_default();
            if !query.is_empty() {
                println!("{CYAN}  🌐 搜索: {query}{RESET}");
                tool_results.push_str(&format!("\n{}\n", web_search(&query)));
            }
            clean = format!("{}{}", &clean[..start], &clean[start + end + 1..]);
        } else { break; }
    }

    (clean, tool_results)
}

// ── File write helpers ────────────────────────────────────────────────────────

fn ensure_cargo_toml(work_dir: &Path) {
    let cargo = work_dir.join("Cargo.toml");
    if !cargo.exists() {
        let _ = fs::write(
            &cargo,
            "[package]\nname = \"project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
        );
        println!("{GREEN}📦 已自动创建 Cargo.toml{RESET}");
    }
}

fn write_or_patch_files(
    blocks: &[(String, String, String)],
    work_dir: &Path,
    written_files: &mut HashMap<String, String>,
    all_completed: &mut Vec<String>,
) {
    for (lang, filename, code) in blocks {
        if filename.is_empty() { continue; }
        if lang == "patch" {
            let old_snap = written_files.get(filename).cloned()
                .or_else(|| fs::read_to_string(work_dir.join(filename)).ok())
                .unwrap_or_default();
            match apply_patch(filename, code, work_dir) {
                Ok(true) => {
                    let updated = fs::read_to_string(work_dir.join(filename)).unwrap_or_default();
                    print_code_diff(filename, &old_snap, &updated);
                    written_files.insert(filename.clone(), updated);
                    if !all_completed.contains(filename) { all_completed.push(filename.clone()); }
                }
                Ok(false) => {}
                Err(e) => {
                    println!("{YELLOW}  ⚠ Patch failed ({e}), full write{RESET}");
                    let fpath = work_dir.join(filename);
                    if let Some(p) = fpath.parent() { let _ = fs::create_dir_all(p); }
                    let _ = fs::write(&fpath, code);
                    print_code_diff(filename, &old_snap, code);
                    written_files.insert(filename.clone(), code.clone());
                    if !all_completed.contains(filename) { all_completed.push(filename.clone()); }
                }
            }
        } else {
            let fpath = work_dir.join(filename);
            // Read existing content once for both guards and diff
            let old_content = fs::read_to_string(&fpath).unwrap_or_default();
            if !old_content.is_empty() {
                // Guard 1: markdown prose overwrite prevention
                let looks_prose = code.lines().take(10).any(|l| {
                    let t = l.trim();
                    t.starts_with("# ") || t.starts_with("## ") || t.starts_with("---")
                        || t.starts_with("**") || (t.starts_with("- ") && !t.contains("//"))
                });
                if looks_prose {
                    println!("{YELLOW}  ⚠ 文档内容试图覆盖 {filename}，已跳过{RESET}");
                    continue;
                }
                // Guard 2: shrink protection (< 30% of original)
                let old_lines = old_content.lines().count();
                let new_lines = code.lines().count();
                if old_lines > 10 && new_lines < old_lines * 30 / 100 {
                    println!("{YELLOW}  ⚠ 拒绝写入 {filename}：新内容({new_lines}行)不足原来({old_lines}行)的30%{RESET}");
                    continue;
                }
            }
            if let Some(p) = fpath.parent() { let _ = fs::create_dir_all(p); }
            let _ = fs::write(&fpath, code);
            print_code_diff(filename, &old_content, code);
            written_files.insert(filename.clone(), code.clone());
            if !all_completed.contains(filename) { all_completed.push(filename.clone()); }
        }
    }
}

fn apply_patch(filename: &str, patch_content: &str, work_dir: &Path) -> Result<bool, String> {
    const OLD_M: &str = "---OLD---";
    const NEW_M: &str = "---NEW---";
    const END_M: &str = "---END---";

    let old_pos = patch_content.find(OLD_M).ok_or("missing ---OLD---")?;
    let after_old = patch_content[old_pos + OLD_M.len()..].trim_start_matches('\n');
    let new_pos = after_old.find(NEW_M).ok_or("missing ---NEW---")?;
    let old_content = after_old[..new_pos].trim_end_matches('\n');
    let after_new = after_old[new_pos + NEW_M.len()..].trim_start_matches('\n');
    let end_pos = after_new.find(END_M).unwrap_or(after_new.len());
    let new_content = after_new[..end_pos].trim_end_matches('\n');

    if old_content.is_empty() { return Err("---OLD--- is empty".to_string()); }

    let fpath = work_dir.join(filename);
    if !fpath.exists() { return Err(format!("file not found: {filename}")); }
    let current = fs::read_to_string(&fpath).map_err(|e| e.to_string())?;
    if !current.contains(old_content) {
        return Err(format!("OLD section not found verbatim in {filename}"));
    }
    fs::write(&fpath, current.replacen(old_content, new_content, 1)).map_err(|e| e.to_string())?;
    Ok(true)
}

fn extract_code_blocks(response: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let re1 = regex::Regex::new(
        r"(?s)```(\w+)\s+filename\s*=\s*[\x22\x27]?([^\x22\x27\n\r]+?)[\x22\x27]?\s*\n(.*?)```",
    ).unwrap();
    for cap in re1.captures_iter(response) {
        let lang = cap[1].to_string();
        let filename = cap[2].trim().to_string();
        let code = cap[3].to_string();
        if !filename.is_empty() && !code.trim().is_empty() {
            results.push((lang, filename, code));
        }
    }
    if results.is_empty() {
        let re2 = regex::Regex::new(r"(?s)```(\w+)\s*\n(.*?)```").unwrap();
        for cap in re2.captures_iter(response) {
            let lang = cap[1].to_string();
            let code = cap[2].to_string();
            if code.trim().is_empty() { continue; }
            let filename = if code.contains("[package]") || code.contains("[dependencies]") {
                "Cargo.toml".to_string()
            } else if code.contains("fn main") {
                "src/main.rs".to_string()
            } else if let Some(line) = code.lines().next() {
                if let Some(pos) = line.find(".rs") {
                    let start = line[..pos].rfind(' ').map_or(0, |i| i + 1);
                    line[start..pos + 3].to_string()
                } else { "src/lib.rs".to_string() }
            } else { "src/lib.rs".to_string() };
            results.push((lang, filename, code));
        }
    }
    results
}

// ── Extract helpers ───────────────────────────────────────────────────────────

fn extract_solution_design(response: &str) -> String {
    response.find("[SOLUTION_DESIGN]")
        .map(|pos| response[pos + "[SOLUTION_DESIGN]".len()..].trim().to_string())
        .unwrap_or_else(|| response.trim().to_string())
}

fn extract_think_block(response: &str) -> Option<String> {
    response.find("[THINK]").map(|start| {
        let after = &response[start + "[THINK]".len()..];
        let end = after.find("```").unwrap_or(after.len());
        after[..end].trim().to_string()
    }).filter(|s| !s.is_empty())
}

fn extract_rules_update(response: &str) -> Option<String> {
    response.find("[RULES_UPDATE]")
        .map(|pos| response[pos + "[RULES_UPDATE]".len()..].trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_review_result(module_name: &str, raw: &str) -> ReviewResult {
    let mut issues = Vec::new();
    for line in raw.lines() {
        let t = line.trim();
        let (severity, rest) = if t.contains("[MAJOR]") {
            (IssueSeverity::Major, t.replacen("[MAJOR]", "", 1))
        } else if t.contains("[MINOR]") {
            (IssueSeverity::Minor, t.replacen("[MINOR]", "", 1))
        } else if t.contains("[INFO]") {
            (IssueSeverity::Info, t.replacen("[INFO]", "", 1))
        } else if t.contains('⚠') {
            (IssueSeverity::Minor, t.replace('⚠', ""))
        } else {
            continue;
        };
        if !rest.trim().is_empty() {
            issues.push(Issue { severity, location: None, description: rest.trim().to_string() });
        }
    }
    let status = if issues.iter().any(|i| matches!(i.severity, IssueSeverity::Major))
        || raw.contains('⚠')
    {
        ReviewStatus::RevisionRequired
    } else {
        ReviewStatus::Approved
    };
    ReviewResult { module_name: module_name.to_string(), issues, status }
}

// ── Git helpers ───────────────────────────────────────────────────────────────

fn git_checkpoint(work_dir: &Path, description: &str) -> String {
    let _ = Command::new("git").args(["add", "-A"]).current_dir(work_dir).output();
    let safe: String = description.chars()
        .map(|c| if c == '"' || c == '\'' || c == '\\' { '-' } else { c })
        .collect();
    let msg = format!("sakichan: checkpoint - {}", safe.chars().take(60).collect::<String>());
    match Command::new("git")
        .args(["commit", "-m", &msg, "--allow-empty"])
        .current_dir(work_dir)
        .output()
    {
        Ok(o) if o.status.success() => format!("Created: {msg}"),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            format!("Git: {}", err.lines().next().unwrap_or("error").trim())
        }
        Err(e) => format!("Error: {e}"),
    }
}

// ── Phase I: verification strategy ───────────────────────────────────────────

fn detect_verification_command(
    written_files: &HashMap<String, String>,
    toolchain: &[DetectedTool],
) -> Option<String> {
    let has_tool = |name: &str| toolchain.iter().any(|t| t.name == name);
    let has_ext  = |ext: &str| written_files.keys().any(|f| f.ends_with(ext));

    if has_ext(".rs") && has_tool("cargo") {
        return Some("cargo check 2>&1".to_string());
    }
    if has_ext(".py") {
        if has_tool("python3") {
            let files: Vec<_> = written_files.keys().filter(|f| f.ends_with(".py")).cloned().collect();
            return Some(format!("python3 -m py_compile {} 2>&1", files.join(" ")));
        } else if has_tool("python") {
            let files: Vec<_> = written_files.keys().filter(|f| f.ends_with(".py")).cloned().collect();
            return Some(format!("python -m py_compile {} 2>&1", files.join(" ")));
        }
    }
    if (has_ext(".ts") || has_ext(".tsx")) && has_tool("tsc") {
        return Some("tsc --noEmit 2>&1".to_string());
    }
    if has_ext(".go") && has_tool("go") {
        return Some("go build ./... 2>&1".to_string());
    }
    if has_ext(".zig") && has_tool("zig") {
        let files: Vec<_> = written_files.keys().filter(|f| f.ends_with(".zig")).cloned().collect();
        return Some(format!("zig ast-check {} 2>&1", files.join(" ")));
    }
    None
}

fn run_verification(
    config: &SakichanConfig,
    work_dir: &Path,
    executor: &Executor,
    written_files: &HashMap<String, String>,
    any_compile: bool,
    toolchain: &[DetectedTool],
) -> (bool, String) {
    if !any_compile || written_files.is_empty() {
        return (true, String::new());
    }

    // Config [[verification]] entries are custom overrides; auto-detect otherwise
    let custom = config.verification.iter()
        .find(|v| v.detector.matches(work_dir) && v.command.is_some());

    let full_cmd = if let Some(strat) = custom {
        let cmd = strat.command.as_deref().unwrap();
        println!("{CYAN}  🔍 验证策略: {} (自定义){RESET}", strat.name);
        if strat.args.is_empty() {
            cmd.to_string()
        } else {
            format!("{} {} 2>&1", cmd, strat.args.join(" "))
        }
    } else if let Some(cmd) = detect_verification_command(written_files, toolchain) {
        println!("{CYAN}  🔍 自动检测验证命令: {cmd}{RESET}");
        cmd
    } else {
        println!("{GRAY}  ℹ️  无匹配验证工具，跳过自动化验证{RESET}");
        return (true, String::new());
    };

    let (ok, output, _) = match executor.run(&full_cmd) {
        Ok(r) => r,
        Err(e) => {
            println!("{RED}  ✗ 验证命令失败: {e}{RESET}");
            return (false, e.to_string());
        }
    };

    if ok {
        println!("{GREEN}  ✓ 验证通过{RESET}");
        (true, String::new())
    } else {
        let errors: String = output.chars().take(1500).collect();
        println!("{RED}  ✗ 验证失败{RESET}");
        for line in output.lines().take(8) {
            if line.contains("error") { println!("    {GRAY}{line}{RESET}"); }
        }
        (false, errors)
    }
}

// ── Main orchestrator ─────────────────────────────────────────────────────────

pub fn run_orchestrator(
    state: &Arc<Mutex<AppState>>,
    user_request: &str,
    context: &mut Vec<String>,
) -> Result<()> {
    let global_tokens = Arc::new(Mutex::new(0u64));
    let global_start = Arc::new(Instant::now());

    let (config, work_dir, output_lang, toolchain_section, toolchain_info) = {
        let st = state.lock().unwrap();
        let lang_str = if st.lang == "zh_TW" { "Traditional Chinese (繁體中文)" } else { "English" };
        let tc_sec = st.toolchain_prompt_section();
        let tc_info = st.toolchain_info.clone();
        (st.config.clone(), st.work_dir.clone(), lang_str.to_string(), tc_sec, tc_info)
    };

    let backend = create_backend(&config)?;
    let executor = Executor::new(work_dir.clone());
    let rules_mgr = RulesManager::new(work_dir.join(".sakichan.md"));
    let logger = Logger::from_work_dir(&work_dir);
    let _ = logger.init();

    let rules = rules_mgr.load();
    let project_tree = generate_project_tree(&work_dir);

    // ══════════════════════════════════════════════════════════════════════
    // Phase A: Summarize — ProductOwner
    // ══════════════════════════════════════════════════════════════════════
    println!("{CYAN}【A】整理需求...{RESET}");

    let model_po = slot_model(SlotRole::ProductOwner, state);
    let opts_po = slot_opts(SlotRole::ProductOwner, &config);

    let prompt_a = format!(
        "{role}{toolchain}请整理以下用户需求，输出自然语言摘要，包含：\n\
        1. 用户期望的输出类型（代码/文档/分析/其他）\n\
        2. 用户已提供的信息\n\
        3. 可能需要补充的信息\n\n\
        不调用工具，只整理。\n\n\
        用户需求：{request}\n\n\
        项目目录：\n{tree}\n\n\
        项目规则：\n{rules}",
        role = SlotRole::ProductOwner.role_prompt(),
        toolchain = toolchain_section,
        request = user_request,
        tree = project_tree,
        rules = rules,
    );

    let summary_a = call_model(
        backend.as_ref(), &model_po, &prompt_a, &opts_po,
        &work_dir, state, &global_tokens, &global_start,
        SpinnerState::Thinking,
    )?;
    println!("{GRAY}  📋 需求摘要已生成 ({} chars){RESET}", summary_a.len());

    // ══════════════════════════════════════════════════════════════════════
    // Phase B: Clarification — ProductOwner
    // ══════════════════════════════════════════════════════════════════════
    println!("{CYAN}【B】初步澄清...{RESET}");

    let prompt_b = format!(
        "{role}## 用户需求摘要\n{summary}\n\n\
        ## 项目规则\n{rules}\n\n\
        请列出最多 3 个真正影响实现方向的问题（用数字序号），\
        或者如果需求已经明确，输出 \"✅ 需求明确，无需澄清\"。\n\
        不问能从项目文件推断的问题。每个问题后附推荐答案（用方括号）。\n\
        所有输出用 {lang}。",
        role = SlotRole::ProductOwner.role_prompt(),
        summary = summary_a,
        rules = rules,
        lang = output_lang,
    );

    let b_raw = call_model(
        backend.as_ref(), &model_po, &prompt_b, &opts_po,
        &work_dir, state, &global_tokens, &global_start,
        SpinnerState::Thinking,
    )?;

    let mut decisions: Vec<String> = Vec::new();

    if !b_raw.contains('✅') {
        let questions = parse_questions(&b_raw);
        if !questions.is_empty() {
            println!("{CYAN}📋 已了解的信息:{RESET}");
            println!("  {GRAY}{}{RESET}", summary_a.lines().take(3).collect::<Vec<_>>().join(" "));
            println!();
            for (i, q) in questions.iter().enumerate() {
                print!("{YELLOW}❓ [{}/{}] {}: {RESET}", i + 1, questions.len(), q);
                let _ = io::stdout().flush();
                let mut ans = String::new();
                io::stdin().read_line(&mut ans)?;
                let ans = ans.trim();
                decisions.push(format!(
                    "Q: {} → A: {}",
                    q,
                    if ans.is_empty() { "(用户接受默认)" } else { ans }
                ));
            }
        } else {
            println!("{GREEN}✅ 需求明确，进入分析{RESET}");
        }
    } else {
        println!("{GREEN}✅ 需求明确，进入分析{RESET}");
    }

    let decisions_str = if decisions.is_empty() { "无需额外澄清".to_string() } else { decisions.join("\n") };

    // ══════════════════════════════════════════════════════════════════════
    // C→J main loop
    // ══════════════════════════════════════════════════════════════════════
    let mut written_files: HashMap<String, String> = HashMap::new();
    let mut all_completed: Vec<String> = Vec::new();
    let mut compile_ok = true;
    let mut solution_design = String::new();
    let mut restart_count = 0usize;
    let mut restart_report: Option<MajorRestartReport> = None;

    'main_loop: loop {
        if restart_count >= MAX_RESTART {
            println!("{YELLOW}⚠ 已达最大重启次数，继续执行{RESET}");
            break 'main_loop;
        }
        if restart_count > 0 {
            println!("{CYAN}🔄 重新分析（第 {restart_count} 次）{RESET}");
            written_files.clear();
        }
        restart_count += 1;

        let model_arch = slot_model(SlotRole::Architect, state);
        let opts_arch = slot_opts(SlotRole::Architect, &config);

        // ══════════════════════════════════════════════════════════════════
        // Phase C: Solution Design — Architect
        // ══════════════════════════════════════════════════════════════════
        println!("{CYAN}【C】方案设计...{RESET}");

        let restart_ctx = restart_report.as_ref()
            .map(|r| r.format_for_prompt())
            .unwrap_or_default();

        let prompt_c = format!(
            "{role}{restart_ctx}\
            ## 用户完整需求\n{request}\n\n\
            ## 澄清决策\n{decisions}\n\n\
            ## 项目目录\n{tree}\n\n\
            ## 项目规则\n{rules}\n\n\
            可用工具（需要时使用）：\n\
            - [SYSTEM:read_file path=\"path\"]\n\
            - [SYSTEM:list_files path=\"dir\"]\n\
            - [SYSTEM:grep pattern=\"symbol\" path=\"src\"]\n\
            - [SYSTEM:web_search query=\"关键词\"]\n\n\
            请设计完整的解决方案。输出以 [SOLUTION_DESIGN] 开头，自然语言，无字数限制。\n\
            必须包含：1. 问题理解  2. 技术方案  3. 关键技术决策及理由  4. 成功标准  5. 可能的风险点\n\
            注意：此时不拆分模块，不生成代码。只输出方案。\n\
            所有输出用 {lang}。",
            role = SlotRole::Architect.role_prompt(),
            request = user_request,
            decisions = decisions_str,
            tree = project_tree,
            rules = rules,
            lang = output_lang,
        );

        let c_raw = call_model(
            backend.as_ref(), &model_arch, &prompt_c, &opts_arch,
            &work_dir, state, &global_tokens, &global_start,
            SpinnerState::Architecting,
        )?;

        solution_design = extract_solution_design(&c_raw);
        println!("{GRAY}  📐 方案设计已生成 ({} chars){RESET}", solution_design.len());
        for line in solution_design.lines().take(6) {
            if !line.trim().is_empty() { println!("  {GRAY}{}{RESET}", line.trim()); }
        }
        println!();

        // ══════════════════════════════════════════════════════════════════
        // Phase D: Direction Check — Architect
        // ══════════════════════════════════════════════════════════════════
        println!("{CYAN}【D】方向确认...{RESET}");

        let prompt_d = format!(
            "{role}{sol}请检查上述方案，列出最多 2 个关键方向性问题（技术选型、架构风格等），\n\
            或者如果方向已明确，输出 \"✅ 方案方向明确，无需确认\"。\n\
            只问方案中存在真正选择分叉的地方。所有输出用 {lang}。",
            role = SlotRole::Architect.role_prompt(),
            sol = solution_section(&solution_design),
            lang = output_lang,
        );

        let d_raw = call_model(
            backend.as_ref(), &model_arch, &prompt_d, &opts_arch,
            &work_dir, state, &global_tokens, &global_start,
            SpinnerState::Thinking,
        )?;

        if !d_raw.contains('✅') {
            let questions = parse_questions(&d_raw);
            if !questions.is_empty() {
                println!("{CYAN}📐 方案摘要:{RESET}");
                for line in solution_design.lines().take(4) {
                    if !line.trim().is_empty() { println!("  {GRAY}{}{RESET}", line.trim()); }
                }
                println!();

                let mut rejected = false;
                for (i, q) in questions.iter().take(2).enumerate() {
                    print!("{YELLOW}🔀 [{}/{}] {} {GRAY}[回车接受 / 输入 'n' 重新分析]: {RESET}",
                        i + 1, questions.len().min(2), q);
                    let _ = io::stdout().flush();
                    let mut ans = String::new();
                    io::stdin().read_line(&mut ans)?;
                    let ans = ans.trim().to_lowercase();
                    if ans == "n" || ans == "no" {
                        rejected = true;
                        break;
                    }
                    decisions.push(format!("方向确认 Q: {} → A: {}", q, if ans.is_empty() { "接受默认" } else { &ans }));
                }

                if rejected {
                    println!("{YELLOW}↩ 回到方案设计阶段...{RESET}");
                    continue 'main_loop;
                }
            }
        }
        println!("{GREEN}✅ 方案方向确认，进入模块规划{RESET}");

        // ══════════════════════════════════════════════════════════════════
        // Phase E: Module Plan — Architect
        // ══════════════════════════════════════════════════════════════════
        println!("{CYAN}【E】模块规划...{RESET}");

        let prompt_e = format!(
            "{role}{sol}{toolchain}## 澄清决策\n{decisions}\n\n\
            请将解决方案拆分为可执行的模块，使用以下标记语言输出每个模块：\n\n\
            <SAKICHAN:MODULE name=\"模块名\" slot=\"Programmer|Architect|SeniorEngineer|QA|ProductOwner\" compile=\"true|false\">\n\
              <SAKICHAN:INPUT>输入规范</SAKICHAN:INPUT>\n\
              <SAKICHAN:OUTPUT>输出规范</SAKICHAN:OUTPUT>\n\
              <SAKICHAN:CONSTRAINT type=\"HARD|SOFT|INFO\">约束内容</SAKICHAN:CONSTRAINT>\n\
              <SAKICHAN:VERIFY>验收条件</SAKICHAN:VERIFY>\n\
            </SAKICHAN:MODULE>\n\n\
            属性说明：\n\
            - slot=Architect: 文档/分析任务，compile=\"false\"\n\
            - slot=Programmer（默认）或 SeniorEngineer（复杂逻辑）: 代码任务，compile=\"true\"\n\
            - 规范要足够详细，执行模型直接可用\n\
            所有输出用 {lang}。",
            role = SlotRole::Architect.role_prompt(),
            sol = solution_section(&solution_design),
            toolchain = toolchain_section,
            decisions = decisions_str,
            lang = output_lang,
        );

        let e_raw = call_model(
            backend.as_ref(), &model_arch, &prompt_e, &opts_arch,
            &work_dir, state, &global_tokens, &global_start,
            SpinnerState::Architecting,
        )?;

        let modules = parse_modules(&e_raw);
        let any_compile = modules.iter().any(|m| m.needs_compile);

        println!("{CYAN}模块列表:{RESET}");
        for (i, m) in modules.iter().enumerate() {
            let kind = if m.needs_compile { "🔨" } else { "📝" };
            println!("  {GREEN}{}. {}{RESET} {kind}  {GRAY}[{}]{RESET}", i + 1, m.name, m.assigned_slot.name());
        }
        println!();

        // Confirmation gate
        let current_edit_mode = state.lock().unwrap().edit_mode;
        if !current_edit_mode {
            print!("{CYAN}▶ 执行 {} 个模块? {GRAY}[y=执行 / n=取消 / a=始终执行]: {RESET}", modules.len());
            let _ = io::stdout().flush();
            let mut ans = String::new();
            io::stdin().read_line(&mut ans)?;
            match ans.trim().to_lowercase().as_str() {
                "a" | "always" => { state.lock().unwrap().edit_mode = true; }
                "y" | "yes" | "" => {}
                _ => { println!("{GRAY}已取消。{RESET}"); return Ok(()); }
            }
        }

        if any_compile { ensure_cargo_toml(&work_dir); }
        let checkpoint_info = git_checkpoint(&work_dir, user_request);
        state.lock().unwrap().checkpoint_count += 1;
        println!("{CYAN}  ● Git(checkpoint){RESET}");
        println!("  {GRAY}⎿  {checkpoint_info}{RESET}");
        println!();

        // ══════════════════════════════════════════════════════════════════
        // Phase F + G: Generate + Quick Review (per module)
        // ══════════════════════════════════════════════════════════════════
        let module_count = modules.len();
        for (mod_idx, module) in modules.iter().enumerate() {
            println!("{PINK}● 模块 {}/{}: {}  [{}]{RESET}",
                mod_idx + 1, module_count, module.name, module.assigned_slot.name());

            let exec_role = module.assigned_slot;
            let exec_model = slot_model(exec_role, state);
            let exec_opts = slot_opts(exec_role, &config);

            for attempt in 1..=REVIEW_MAX_ATTEMPTS {
                // ── Phase F: Generate ──────────────────────────────────────
                let gen_prompt = build_generation_prompt(
                    module,
                    &solution_design,
                    user_request,
                    &project_tree,
                    &output_lang,
                    restart_report.as_ref(),
                    &toolchain_section,
                );

                let f_raw = call_model(
                    backend.as_ref(), &exec_model, &gen_prompt, &exec_opts,
                    &work_dir, state, &global_tokens, &global_start,
                    SpinnerState::Crafting,
                )?;

                if let Some(think) = extract_think_block(&f_raw) {
                    println!("{GRAY}  [THINK]{RESET}");
                    for line in think.lines().take(6) {
                        if !line.trim().is_empty() { println!("    {GRAY}{}{RESET}", line.trim()); }
                    }
                }

                let blocks = extract_code_blocks(&f_raw);
                if blocks.is_empty() {
                    println!("{YELLOW}  ⚠ 模块 {} 未生成代码块（尝试 {attempt}）{RESET}", module.name);
                } else {
                    write_or_patch_files(&blocks, &work_dir, &mut written_files, &mut all_completed);
                }

                // ── Phase G: Quick Review — QA ─────────────────────────────
                let module_code = {
                    let mut s = String::new();
                    for (fname, content) in &written_files {
                        s.push_str(&format!("=== {} ===\n{}\n\n", fname, content));
                    }
                    if s.is_empty() { "(无生成文件)".to_string() } else { s }
                };

                let review_prompt = format!(
                    "{role}请检查以下代码是否符合模块规范。\n\n\
                    ## 模块规范（Phase E 输出）\n{spec}\n\n\
                    ## 当前代码\n{code}\n\n\
                    检查清单：\n\
                    1. 是否实现了规范中的所有输入输出\n\
                    2. 拼写错误、语法错误\n\
                    3. 与规范的一致性\n\
                    4. 文件内容是否大幅缩减（行数 < 原来的 30%）\n\n\
                    用 [REVIEW] 开头。通过输出 ✓，问题用 [MAJOR]/[MINOR] 标注。\n\
                    所有输出用 {lang}。",
                    role = SlotRole::QA.role_prompt(),
                    spec = module.full_spec,
                    code = module_code,
                    lang = output_lang,
                );

                let model_qa = slot_model(SlotRole::QA, state);
                let opts_qa = slot_opts(SlotRole::QA, &config);

                let sp = Spinner::new(SpinnerState::Reviewing, Arc::clone(&global_tokens), Arc::clone(&global_start));
                let (g_raw, g_usage) = backend.generate_complete(&model_qa, &review_prompt, &opts_qa)?;
                add_tokens(&global_tokens, &g_usage);
                sp.stop();
                record_usage(state, &g_usage);

                let review = parse_review_result(&module.name, &g_raw);

                print!("{CYAN}  [G 评估 {attempt}/{REVIEW_MAX_ATTEMPTS}]{RESET} ");
                if matches!(review.status, ReviewStatus::Approved) {
                    println!("{GREEN}✓ 通过{RESET}");
                    break;
                }

                println!("{YELLOW}⚠ 发现问题{RESET}");
                for issue in review.issues.iter().take(3) {
                    let sev = match issue.severity {
                        IssueSeverity::Major => format!("{RED}[MAJOR]{RESET}"),
                        IssueSeverity::Minor => format!("{YELLOW}[MINOR]{RESET}"),
                        IssueSeverity::Info => format!("{GRAY}[INFO]{RESET}"),
                    };
                    println!("    {} {}", sev, issue.description);
                }

                if attempt == REVIEW_MAX_ATTEMPTS {
                    println!("  {YELLOW}⚠ 达到最大评估次数，继续{RESET}");
                    break;
                }

                // Build structured fix prompt for next F iteration
                let fix_prompt = build_fix_prompt(
                    &review,
                    &module_code,
                    module,
                    &solution_design,
                    &output_lang,
                );
                let fix_raw = call_model(
                    backend.as_ref(), &exec_model, &fix_prompt, &exec_opts,
                    &work_dir, state, &global_tokens, &global_start,
                    SpinnerState::Fixing,
                )?;
                let fix_blocks = extract_code_blocks(&fix_raw);
                if !fix_blocks.is_empty() {
                    write_or_patch_files(&fix_blocks, &work_dir, &mut written_files, &mut all_completed);
                }
            }
        }

        // ══════════════════════════════════════════════════════════════════
        // Phase H: Merge Check (static, no model)
        // ══════════════════════════════════════════════════════════════════
        println!("{CYAN}【H】合并检查...{RESET}");
        let mut h_issues = Vec::new();

        if written_files.is_empty() {
            h_issues.push("⚠ 未生成任何文件".to_string());
        }

        if any_compile {
            for (fname, content) in &written_files {
                if !fname.ends_with(".rs") { continue; }
                for line in content.lines() {
                    let t = line.trim();
                    if t.starts_with("use crate::") {
                        let module_name = t
                            .trim_start_matches("use crate::")
                            .split("::")
                            .next()
                            .unwrap_or("")
                            .trim_end_matches(';');
                        let mod_file = format!("src/{module_name}.rs");
                        if !written_files.contains_key(&mod_file) && !work_dir.join(&mod_file).exists() {
                            h_issues.push(format!(
                                "⚠ {fname} 引用了 crate::{module_name}，但 {mod_file} 未生成也不存在"
                            ));
                        }
                    }
                }
            }
        }

        if h_issues.is_empty() {
            println!("{GREEN}  ✅ 合并检查通过{RESET}");
        } else {
            for issue in &h_issues { println!("  {YELLOW}{issue}{RESET}"); }
        }
        let h_issues_str = h_issues.join("\n");

        // ══════════════════════════════════════════════════════════════════
        // Phase I: Overall Eval — SeniorEngineer (compile) + Architect (review)
        // ══════════════════════════════════════════════════════════════════
        println!("{CYAN}【I】总体评估...{RESET}");

        // Compile/verification loop (SeniorEngineer fixes)
        let model_se = slot_model(SlotRole::SeniorEngineer, state);
        let opts_se = slot_opts(SlotRole::SeniorEngineer, &config);

        let mut cargo_errors = String::new();
        if any_compile && !written_files.is_empty() {
            let (mut ok, mut last_err) = run_verification(&config, &work_dir, &executor, &written_files, any_compile, &toolchain_info);
            compile_ok = ok;

            // SeniorEngineer fix loop
            if !ok {
                for fix_attempt in 1..=CARGO_FIX_MAX {
                    let all_code = build_code_context(&written_files);
                    let fix_prompt = format!(
                        "{role}{sol}## 编译错误（第 {fix_attempt} 次）\n```\n{err}\n```\n\n\
                        ## 当前代码\n{all_code}\n\n\
                        可搜索相关符号：[SYSTEM:grep pattern=\"符号名\" path=\"src\"]\n\n\
                        ## 输出格式（二选一）\n\
                        ```patch filename=\"path\"\n---OLD---\n原始\n---NEW---\n修复\n---END---\n```\n\
                        ```rust filename=\"path\"\n// 完整代码\n```\n\
                        所有说明用 {lang}。",
                        role = SlotRole::SeniorEngineer.role_prompt(),
                        sol = solution_section(&solution_design),
                        err = last_err,
                        lang = output_lang,
                    );

                    let fix_raw = call_model(
                        backend.as_ref(), &model_se, &fix_prompt, &opts_se,
                        &work_dir, state, &global_tokens, &global_start,
                        SpinnerState::FixingDsr1,
                    )?;

                    let fix_blocks = extract_code_blocks(&fix_raw);
                    if !fix_blocks.is_empty() {
                        write_or_patch_files(&fix_blocks, &work_dir, &mut written_files, &mut all_completed);
                    }

                    let (ok2, err2) = run_verification(&config, &work_dir, &executor, &written_files, any_compile, &toolchain_info);
                    ok = ok2; last_err = err2;
                    if ok { compile_ok = true; break; }
                    if fix_attempt == CARGO_FIX_MAX {
                        cargo_errors = last_err.clone();
                        println!("{RED}  ❌ 编译失败 after {CARGO_FIX_MAX} attempts{RESET}");
                    }
                }
            }
        } else if !written_files.is_empty() && !any_compile {
            println!("{CYAN}  📝 文档任务，跳过编译{RESET}");
        }

        // Architect review
        let all_code_ctx = build_code_context(&written_files);
        let i_prompt = format!(
            "{role}## 解决方案设计（Phase C）\n{sol}\n\n\
            ## 合并检查问题（Phase H）\n{h_issues}\n\n\
            ## 编译状态\n{compile_status}\n\n\
            ## 完整项目代码\n{code}\n\n\
            请对照 Phase C 的「成功标准」，检查：\n\
            1. 是否实现了方案中的所有行为\n\
            2. 各模块是否正确集成\n\
            3. 接口是否一致\n\n\
            用 [ARCHITECT] 开头，逐条列出。如有问题：\n\
            - [MINOR] 可追加修正指令修复\n\
            - [MAJOR] 需要重新生成（可能需要回到 Phase C）\n\
            全部通过输出：[ARCHITECT] ✓ 整体评估通过\n\
            所有输出用 {lang}。",
            role = SlotRole::Architect.role_prompt(),
            sol = solution_design,
            h_issues = if h_issues_str.is_empty() { "无".to_string() } else { h_issues_str.clone() },
            compile_status = if compile_ok { "✓ 编译通过".to_string() }
                             else { format!("✗ 编译失败\n{cargo_errors}") },
            code = all_code_ctx,
            lang = output_lang,
        );

        let i_raw = call_model(
            backend.as_ref(), &model_arch, &i_prompt, &opts_arch,
            &work_dir, state, &global_tokens, &global_start,
            SpinnerState::Evaluating,
        )?;

        for line in i_raw.lines().take(20) {
            let t = line.trim();
            if t.is_empty() { continue; }
            if t.contains("[MAJOR]") { println!("  {RED}{t}{RESET}"); }
            else if t.contains("[MINOR]") { println!("  {YELLOW}{t}{RESET}"); }
            else if t.starts_with("[ARCHITECT]") { println!("  {CYAN}{t}{RESET}"); }
            else { println!("  {GRAY}{t}{RESET}"); }
        }

        if i_raw.contains("[MAJOR]") {
            print!("{YELLOW}⚠ 发现重大问题。是否回到 Phase C 重新分析？{GRAY}[y=重新分析/n=继续]: {RESET}");
            let _ = io::stdout().flush();
            let mut ans = String::new();
            io::stdin().read_line(&mut ans)?;
            if ans.trim().to_lowercase() == "y" || ans.trim().to_lowercase() == "yes" {
                // Build restart report for next C iteration
                let major_issues: Vec<String> = i_raw.lines()
                    .filter(|l| l.contains("[MAJOR]"))
                    .map(|l| l.trim().to_string())
                    .collect();
                restart_report = Some(MajorRestartReport {
                    triggered_by: Some(SlotRole::Architect),
                    reason: major_issues.first().cloned().unwrap_or_else(|| "架构评审发现重大问题".to_string()),
                    attempted_fixes: vec![],
                    constraints_for_redesign: vec![],
                    preserved_artifacts: all_completed.clone(),
                });
                println!("{CYAN}↩ 回到 Phase C...{RESET}");
                continue 'main_loop;
            }
        }

        // Handle MINOR issues: one rework pass
        if i_raw.contains("[MINOR]") && !i_raw.contains("[MAJOR]") {
            println!("{YELLOW}🔧 修复小问题...{RESET}");
            let rework_prompt = format!(
                "{role}{sol}## 评估意见\n{feedback}\n\n\
                ## 当前代码\n{code}\n\n\
                请修复所有 [MINOR] 问题，输出修改后的文件。所有说明用 {lang}。",
                role = SlotRole::SeniorEngineer.role_prompt(),
                sol = solution_section(&solution_design),
                feedback = i_raw,
                code = build_code_context(&written_files),
                lang = output_lang,
            );

            let rework_raw = call_model(
                backend.as_ref(), &model_se, &rework_prompt, &opts_se,
                &work_dir, state, &global_tokens, &global_start,
                SpinnerState::FixingDsr1,
            )?;

            let rework_blocks = extract_code_blocks(&rework_raw);
            if !rework_blocks.is_empty() {
                write_or_patch_files(&rework_blocks, &work_dir, &mut written_files, &mut all_completed);
            }
        }

        if !i_raw.contains("[MAJOR]") {
            println!("{GREEN}✓ 总体评估通过{RESET}");
        }

        // ══════════════════════════════════════════════════════════════════
        // Phase J: Wrap-up — ProductOwner
        // ══════════════════════════════════════════════════════════════════
        println!();
        print_change_summary(&all_completed);

        let modules_summary = modules.iter().enumerate()
            .map(|(i, m)| format!("{}. {} [{}]", i + 1, m.name, m.assigned_slot.name()))
            .collect::<Vec<_>>()
            .join("\n");

        let current_rules = rules_mgr.load();
        let rules_prompt = format!(
            "## 本次完成的工作\n{modules_summary}\n\n\
            ## 当前 .sakichan.md\n{current_rules}\n\n\
            请更新项目规则文件，保留原有信息，追加本次发现。\n\
            输出用 [RULES_UPDATE] 标记。"
        );

        let sp = Spinner::new(SpinnerState::Thinking, Arc::clone(&global_tokens), Arc::clone(&global_start));
        let rules_updated = match backend.generate_complete(&model_po, &rules_prompt, &opts_po) {
            Ok((rules_raw, usage_ru)) => {
                add_tokens(&global_tokens, &usage_ru);
                sp.stop();
                record_usage(state, &usage_ru);
                if let Some(new_rules) = extract_rules_update(&rules_raw) {
                    let ok = fs::write(work_dir.join(".sakichan.md"), &new_rules).is_ok();
                    if ok { println!("{GRAY}📄 规则文件已更新{RESET}"); }
                    ok
                } else { false }
            }
            Err(_) => { sp.stop(); false }
        };

        if !rules_updated {
            let structure = list_files_in(&work_dir).join("\n");
            let _ = rules_mgr.update(&all_completed, &structure);
            println!("{GRAY}📄 规则文件已更新{RESET}");
        }

        let elapsed_secs = global_start.elapsed().as_secs();
        let elapsed_str = if elapsed_secs >= 60 {
            format!("{}m {}s", elapsed_secs / 60, elapsed_secs % 60)
        } else {
            format!("{}s", elapsed_secs)
        };

        let _ = logger.log_task(
            user_request,
            &solution_design.lines().next().unwrap_or("").chars().take(80).collect::<String>(),
            &all_completed,
            compile_ok,
            &[],
            &model_arch,
            global_start.elapsed().as_secs_f64(),
        );

        let log_name = format!(
            "{}_log.md",
            work_dir.file_name().unwrap_or_default().to_string_lossy()
        );
        println!("{GRAY}📝 日志已更新 → {log_name}{RESET}");

        // Git commit
        let summary: String = user_request.lines().next().unwrap_or("").chars().take(50).collect();
        let commit_msg = format!("sakichan: {}", summary.trim());
        let _ = Command::new("git").args(["add", "-A"]).current_dir(&work_dir).output();
        match Command::new("git")
            .args(["commit", "-m", &commit_msg, "--allow-empty"])
            .current_dir(&work_dir)
            .output()
        {
            Ok(o) if o.status.success() => {
                println!("{CYAN}● Git(commit){RESET}");
                println!("  {GRAY}⎿  Committed: {commit_msg}{RESET}");
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                let first = err.lines().next().unwrap_or("").trim();
                if !first.is_empty() { println!("{GRAY}● Git(commit): {first}{RESET}"); }
            }
            Err(e) => println!("{YELLOW}● Git(commit) error: {e}{RESET}"),
        }

        println!("{PINK}✻ Baked for {elapsed_str}{RESET}");

        context.push(format!("User: {user_request}"));
        context.push(format!(
            "Assistant: Completed {} modules, compile_ok={compile_ok}",
            modules.len()
        ));

        let session_id = uuid::Uuid::new_v4().to_string();
        let _ = state.lock().unwrap().save_session(&session_id, context);

        break 'main_loop;
    }

    Ok(())
}
