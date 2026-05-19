use crate::display::*;
use crate::executor::Executor;
use crate::logger::Logger;
use crate::ollama::{ModelOptions, OllamaClient, UsageStats};
use crate::rules::RulesManager;
use crate::state::AppState;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const QWEN: &str = "qwen2.5-coder:7b";
const DSR1: &str = "deepseek-r1:8b";
const REVIEW_MAX_ATTEMPTS: usize = 3;
const ARCH_MAX_ITERATIONS: usize = 2;
const CARGO_FIX_MAX: usize = 5;

// Fix 3: Generic AI persona prepended to main prompts.
const ROLE_HEADER: &str = "你是 Saki-chan，一个通用 AI 助手。你的任务是理解用户需求并给出完整的分析和方案。\n你可以处理代码构建、代码分析、文档撰写、学术写作等各种类型的任务。\n请根据需求的性质自行判断如何处理。\n\n";

// ── Model option presets ──────────────────────────────────────────────────────

fn dsr1_opts() -> ModelOptions {
    ModelOptions {
        temperature: Some(0.3),
        top_p: Some(0.85),
        top_k: Some(40),
        num_predict: Some(-1),
        num_ctx: Some(8192),
        ..Default::default()
    }
}

fn qwen_ctx_opts() -> ModelOptions {
    ModelOptions {
        temperature: Some(0.1),
        top_p: Some(0.8),
        top_k: Some(20),
        num_predict: Some(2048),
        repeat_penalty: Some(1.05),
        seed: Some(42),
        num_ctx: Some(8192),
    }
}

fn qwen_gen_opts() -> ModelOptions {
    ModelOptions {
        temperature: Some(0.2),
        top_p: Some(0.8),
        top_k: Some(20),
        num_predict: Some(2048),
        repeat_penalty: Some(1.05),
        seed: Some(42),
        num_ctx: Some(8192),
    }
}

// ── Data structures ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
struct AnalysisResponse {
    #[serde(default)]
    understanding: String,
    #[serde(default)]
    task_type: String,
    #[serde(default)]
    complexity: u8,
    #[serde(default)]
    gathered_info: Vec<GatheredInfo>,
    #[serde(default)]
    clarifications: Vec<Clarification>,
}

#[derive(Debug, Deserialize, Default)]
struct GatheredInfo {
    #[serde(default)]
    label: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    source: String,
}

#[derive(Debug, Deserialize)]
struct Clarification {
    question: String,
    #[serde(default)]
    recommendation: String,
}

#[derive(Debug, Deserialize)]
struct PlanResponse {
    steps: Vec<PlanStep>,
}

#[derive(Debug, Deserialize)]
struct PlanStep {
    id: u32,
    name: String,
    #[serde(default)]
    submodule_prompt: String,
    #[serde(default = "default_model")]
    assigned_model: String,
    #[serde(default)]
    files_to_create: Vec<String>,
    #[serde(default)]
    verification: String,
    #[serde(default)]
    needs_compile: bool,
}

fn default_model() -> String { "QWEN".to_string() }

// ── Utility helpers ───────────────────────────────────────────────────────────

fn resolve_model(assigned: &str) -> &'static str {
    if assigned.to_uppercase().contains("DSR") || assigned.to_uppercase().contains("DEEP") {
        DSR1
    } else {
        QWEN
    }
}

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

// Fix 2: Recursive directory tree builder. Skips target/, .git/,
// and .sakichan/sandboxes + .sakichan/sessions.
fn build_tree(dir: &Path, prefix: &str, lines: &mut Vec<String>) {
    const SKIP: &[&str] = &["target", ".git"];
    let Ok(read) = fs::read_dir(dir) else { return };

    let parent_name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();

    let mut items: Vec<_> = read
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if SKIP.contains(&name.as_str()) { return false; }
            if parent_name == ".sakichan" && (name == "sandboxes" || name == "sessions") {
                return false;
            }
            true
        })
        .collect();

    items.sort_by_key(|e| {
        let is_file = e.path().is_file() as u8;
        (is_file, e.file_name())
    });

    for (i, entry) in items.iter().enumerate() {
        let is_last = i == items.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let is_dir = path.is_dir();
        lines.push(format!("{}{}{}{}", prefix, connector, name, if is_dir { "/" } else { "" }));
        if is_dir {
            let child_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
            build_tree(&path, &child_prefix, lines);
        }
    }
}

fn generate_project_tree(work_dir: &Path) -> String {
    let name = work_dir.file_name().unwrap_or_default().to_string_lossy();
    let mut lines = vec![format!("{}/", name)];
    build_tree(work_dir, "", &mut lines);
    lines.join("\n")
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

    let match_indices: Vec<usize> = lines.iter().enumerate()
        .filter(|(_, l)| l.to_lowercase().contains(pattern))
        .map(|(i, _)| i)
        .collect();
    if match_indices.is_empty() { return; }

    let mut shown = std::collections::BTreeSet::<usize>::new();
    for &idx in &match_indices {
        for j in idx.saturating_sub(ctx)..(idx + ctx + 1).min(lines.len()) {
            shown.insert(j);
        }
    }

    let shown_vec: Vec<usize> = shown.into_iter().collect();
    let mut prev: Option<usize> = None;
    for &j in &shown_vec {
        if let Some(p) = prev { if j > p + 1 { results.push("  ---".to_string()); } }
        let marker = if match_indices.contains(&j) { "▶" } else { " " };
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
            if GREP_TEXT_EXTS.contains(&ext) {
                grep_file(root, &path, pattern, ctx, results);
            }
        }
    }
}

fn grep_in_dir(work_dir: &Path, pattern: &str, search_path: &str, ctx: usize) -> String {
    let pattern_lower = pattern.to_lowercase();
    let search_dir = if search_path.is_empty() || search_path == "." {
        work_dir.to_path_buf()
    } else {
        work_dir.join(search_path)
    };
    let mut results: Vec<String> = Vec::new();
    grep_walk(&search_dir, &search_dir, &pattern_lower, ctx, &mut results);
    if results.is_empty() {
        format!("[GREP: no matches for \"{pattern}\" in {search_path}]")
    } else {
        let count = results.iter().filter(|l| l.contains('▶')).count();
        format!(
            "[GREP \"{pattern}\" — {count} matches]\n{}\n[/GREP]",
            results.join("\n")
        )
    }
}

fn extract_code_blocks(response: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    let re1 = regex::Regex::new(
        r"(?s)```(\w+)\s+filename\s*=\s*[\x22\x27]?([^\x22\x27\n\r]+?)[\x22\x27]?\s*\n(.*?)```"
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
            } else {
                if let Some(line) = code.lines().next() {
                    if let Some(pos) = line.find(".rs") {
                        let start = line[..pos].rfind(' ').map_or(0, |i| i + 1);
                        line[start..pos + 3].to_string()
                    } else { "src/lib.rs".to_string() }
                } else { "src/lib.rs".to_string() }
            };
            results.push((lang, filename, code));
        }
    }
    results
}

// ── Patch application ─────────────────────────────────────────────────────────

fn apply_patch(filename: &str, patch_content: &str, work_dir: &Path) -> Result<bool, String> {
    const OLD_M: &str = "---OLD---";
    const NEW_M: &str = "---NEW---";
    const END_M: &str = "---END---";

    let Some(old_pos) = patch_content.find(OLD_M) else {
        return Err("missing ---OLD--- marker".to_string());
    };
    let after_old = patch_content[old_pos + OLD_M.len()..].trim_start_matches('\n');

    let Some(new_pos) = after_old.find(NEW_M) else {
        return Err("missing ---NEW--- marker".to_string());
    };
    let old_content = after_old[..new_pos].trim_end_matches('\n');

    let after_new = after_old[new_pos + NEW_M.len()..].trim_start_matches('\n');
    let end_pos = after_new.find(END_M).unwrap_or(after_new.len());
    let new_content = after_new[..end_pos].trim_end_matches('\n');

    if old_content.is_empty() {
        return Err("---OLD--- section is empty".to_string());
    }

    let fpath = work_dir.join(filename);
    if !fpath.exists() {
        return Err(format!("file not found: {filename}"));
    }
    let current = fs::read_to_string(&fpath).map_err(|e| e.to_string())?;
    if !current.contains(old_content) {
        return Err(format!("OLD section not found verbatim in {filename} — use full-file format instead"));
    }
    let updated = current.replacen(old_content, new_content, 1);
    fs::write(&fpath, &updated).map_err(|e| e.to_string())?;
    Ok(true)
}

fn parse_json_from_response(response: &str) -> Option<String> {
    if let Some(start) = response.find('{') {
        let sub = &response[start..];
        let mut depth = 0i32;
        let mut end = 0;
        for (i, c) in sub.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => { depth -= 1; if depth == 0 { end = i + 1; break; } }
                _ => {}
            }
        }
        if end > 0 { return Some(sub[..end].to_string()); }
    }
    None
}

fn process_system_calls(response: &str, work_dir: &Path) -> (String, String) {
    let mut tool_results = String::new();
    let mut clean = response.to_string();

    while let Some(start) = clean.find("[SYSTEM:read_file") {
        if let Some(end) = clean[start..].find(']') {
            let tag = &clean[start..start + end + 1];
            let mut result = String::new();
            if let Some(path_start) = tag.find("path=\"") {
                let rest = &tag[path_start + 6..];
                if let Some(path_end) = rest.find('"') {
                    let path = &rest[..path_end];
                    let full_path = work_dir.join(path);
                    match fs::read_to_string(&full_path) {
                        Ok(content) => result = format!("\n[FILE:{path}]\n{content}\n[/FILE]"),
                        Err(e) => result = format!("\n[ERROR reading {path}: {e}]"),
                    }
                }
            }
            tool_results.push_str(&result);
            clean = format!("{}{}", &clean[..start], &clean[start + end + 1..]);
        } else { break; }
    }

    while let Some(start) = clean.find("[SYSTEM:list_files") {
        if let Some(end) = clean[start..].find(']') {
            let tag = &clean[start..start + end + 1];
            let dir = if let Some(path_start) = tag.find("path=\"") {
                let rest = &tag[path_start + 6..];
                if let Some(path_end) = rest.find('"') {
                    work_dir.join(&rest[..path_end])
                } else { work_dir.to_path_buf() }
            } else { work_dir.to_path_buf() };
            let files = list_files_in(&dir);
            tool_results.push_str(&format!("\n[FILES in {}]\n{}\n[/FILES]", dir.display(), files.join("\n")));
            clean = format!("{}{}", &clean[..start], &clean[start + end + 1..]);
        } else { break; }
    }

    while let Some(start) = clean.find("[SYSTEM:grep") {
        if let Some(end) = clean[start..].find(']') {
            let tag = &clean[start..start + end + 1];
            let pattern = tag.find("pattern=\"")
                .map(|p| { let r = &tag[p + 9..]; r[..r.find('"').unwrap_or(r.len())].to_string() })
                .unwrap_or_default();
            let path = tag.find("path=\"")
                .map(|p| { let r = &tag[p + 6..]; r[..r.find('"').unwrap_or(r.len())].to_string() })
                .unwrap_or_else(|| ".".to_string());
            let ctx = tag.find("context=\"")
                .and_then(|p| { let r = &tag[p + 9..]; r[..r.find('"').unwrap_or(r.len())].parse::<usize>().ok() })
                .unwrap_or(2);
            if !pattern.is_empty() {
                tool_results.push_str(&format!("\n{}\n", grep_in_dir(work_dir, &pattern, &path, ctx)));
            }
            clean = format!("{}{}", &clean[..start], &clean[start + end + 1..]);
        } else { break; }
    }

    (clean, tool_results)
}

fn record_usage(state: &Arc<Mutex<AppState>>, usage: &UsageStats) {
    if let Ok(mut st) = state.lock() {
        st.usage.add(usage);
        let _ = st.save_usage();
    }
}

// Fix 7: Accumulate tokens into the global counter after each chat call.
fn add_global_tokens(global_tokens: &Arc<Mutex<u64>>, usage: &UsageStats) {
    if let Ok(mut t) = global_tokens.lock() {
        *t += usage.input_tokens + usage.output_tokens;
    }
}

fn gather_existing_code(work_dir: &Path, files: &[String]) -> String {
    let mut result = String::new();
    for f in files {
        let fpath = work_dir.join(f);
        if fpath.exists() {
            if let Ok(content) = fs::read_to_string(&fpath) {
                result.push_str(&format!("=== {} ===\n{}\n\n", f, content));
            }
        }
    }
    if result.is_empty() { result = "(no existing files)".to_string(); }
    result
}

fn ensure_cargo_toml(work_dir: &Path) {
    let cargo = work_dir.join("Cargo.toml");
    if !cargo.exists() {
        let _ = fs::write(&cargo, "[package]\nname = \"project\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n");
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
            let old_snapshot = written_files.get(filename).cloned()
                .or_else(|| fs::read_to_string(work_dir.join(filename)).ok())
                .unwrap_or_default();
            match apply_patch(filename, code, work_dir) {
                Ok(true) => {
                    let updated = fs::read_to_string(work_dir.join(filename)).unwrap_or_default();
                    print_code_diff(filename, &old_snapshot, &updated);
                    written_files.insert(filename.clone(), updated);
                    if !all_completed.contains(filename) { all_completed.push(filename.clone()); }
                }
                Ok(false) => {}
                Err(e) => {
                    println!("{YELLOW}  ⚠ Patch failed ({e}), falling back to full write{RESET}");
                    let fpath = work_dir.join(filename);
                    if let Some(p) = fpath.parent() { let _ = fs::create_dir_all(p); }
                    let _ = fs::write(&fpath, code);
                    print_code_diff(filename, &old_snapshot, code);
                    written_files.insert(filename.clone(), code.clone());
                    if !all_completed.contains(filename) { all_completed.push(filename.clone()); }
                }
            }
        } else {
            let fpath = work_dir.join(filename);
            // Guard: never overwrite an existing source file with non-code content.
            // Detects markdown/plain-text masquerading as a .rs/.py/.cpp file.
            let is_source_ext = ["rs", "py", "cpp", "c", "go", "js", "ts", "java"].iter()
                .any(|e| filename.ends_with(&format!(".{e}")));
            if is_source_ext && fpath.exists() {
                let looks_like_prose = code.lines().take(10).any(|l| {
                    let t = l.trim();
                    t.starts_with("# ") || t.starts_with("## ") || t.starts_with("---")
                    || t.starts_with("**") || t.starts_with("- ") && !t.contains("//")
                });
                if looks_like_prose {
                    println!("{YELLOW}  ⚠ 文档步骤尝试覆盖源代码文件 {filename}，已跳过{RESET}");
                    continue;
                }
            }
            let old_content = fs::read_to_string(&fpath).unwrap_or_default();
            if let Some(p) = fpath.parent() { let _ = fs::create_dir_all(p); }
            let _ = fs::write(&fpath, code);
            print_code_diff(filename, &old_content, code);
            written_files.insert(filename.clone(), code.clone());
            if !all_completed.contains(filename) { all_completed.push(filename.clone()); }
        }
    }
}

fn build_code_context(written_files: &HashMap<String, String>) -> String {
    if written_files.is_empty() { return "(no files generated yet)".to_string(); }
    let mut result = String::new();
    let mut sorted: Vec<_> = written_files.iter().collect();
    sorted.sort_by_key(|(k, _)| k.as_str());
    for (filename, content) in sorted {
        result.push_str(&format!("=== {} ===\n{}\n\n", filename, content));
    }
    result
}

fn git_checkpoint(work_dir: &Path, description: &str) -> String {
    let _ = Command::new("git")
        .args(["add", "-A"])
        .current_dir(work_dir)
        .output();

    let safe: String = description.chars()
        .map(|c| if c == '"' || c == '\'' || c == '\\' { '-' } else { c })
        .collect();
    let msg = format!("sakichan: checkpoint - {safe}");

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

// Fix 1b: Extract the [ANALYSIS_DOC] section from Phase 1 response.
fn extract_analysis_doc(response: &str) -> String {
    response.find("[ANALYSIS_DOC]")
        .map(|pos| response[pos + "[ANALYSIS_DOC]".len()..].trim().to_string())
        .unwrap_or_default()
}

// Fix 5b: Extract [THINK] block content (everything before the first code fence).
fn extract_think_block(response: &str) -> Option<String> {
    response.find("[THINK]").map(|start| {
        let after = &response[start + "[THINK]".len()..];
        let end = after.find("```").unwrap_or(after.len());
        after[..end].trim().to_string()
    }).filter(|s| !s.is_empty())
}

// Fix 6: Extract content after [RULES_UPDATE] marker.
fn extract_rules_update(response: &str) -> Option<String> {
    response.find("[RULES_UPDATE]")
        .map(|pos| response[pos + "[RULES_UPDATE]".len()..].trim().to_string())
        .filter(|s| !s.is_empty())
}

// Fix 1c: Build the mental model section to inject into downstream prompts.
fn mental_model_section(analysis_doc: &str) -> String {
    if analysis_doc.is_empty() {
        String::new()
    } else {
        format!("## 项目心智模型（由 Phase 1 生成，请仔细阅读）\n{}\n\n", analysis_doc)
    }
}

// ── Phase 0: Context gathering ────────────────────────────────────────────────

fn gather_context(
    ollama: &OllamaClient,
    user_request: &str,
    files_str: &str,
    work_dir: &Path,
) -> String {
    const FILE_EXTS: &[&str] = &[
        ".md", ".rs", ".toml", ".txt", ".json", ".yaml", ".yml",
        ".py", ".js", ".ts", ".cpp", ".c", ".h", ".lock",
    ];
    if !FILE_EXTS.iter().any(|ext| user_request.contains(ext)) {
        return user_request.to_string();
    }

    let prompt = format!(
        "Extract file names/paths explicitly mentioned in this user request. \
Return ONLY a JSON array of strings. If none, return [].\n\n\
User request: {user_request}\nAvailable files: {files_str}\n\n\
Return ONLY like: [\"src/main.rs\", \"Cargo.toml\"]"
    );

    let Ok((response, _)) = ollama.chat(QWEN, &prompt, Some(&qwen_ctx_opts())) else {
        return user_request.to_string();
    };

    let files: Vec<String> = response.find('[')
        .and_then(|s| response[s..].find(']').map(|e| &response[s..s + e + 1]))
        .and_then(|slice| serde_json::from_str(slice).ok())
        .unwrap_or_default();

    if files.is_empty() { return user_request.to_string(); }

    let mut file_contents = String::new();
    for filename in &files {
        let fpath = work_dir.join(filename);
        if fpath.exists() {
            if let Ok(content) = fs::read_to_string(&fpath) {
                file_contents.push_str(&format!("\n=== {filename} ===\n{content}\n"));
            }
        }
    }

    if file_contents.is_empty() { return user_request.to_string(); }

    format!("用户需求: {user_request}\n\n相关文件内容:{file_contents}")
}

// ── Review helpers ────────────────────────────────────────────────────────────

fn review_has_warnings(review: &str) -> bool {
    review.contains('⚠')
}

fn arch_has_issues(arch: &str) -> bool {
    arch.contains("[MAJOR]")
}

// Fix 1c: review/architect prompts now receive the analysis_doc mental model.
fn build_review_prompt(step: &PlanStep, written_files: &HashMap<String, String>, analysis_doc: &str) -> String {
    let code_ctx = {
        let mut s = String::new();
        for f in &step.files_to_create {
            if let Some(content) = written_files.get(f) {
                s.push_str(&format!("=== {} ===\n{}\n\n", f, content));
            }
        }
        if s.is_empty() { build_code_context(written_files) } else { s }
    };

    format!(
        "{mental_model}你是代码审查员。请检查以下代码是否存在问题。\n\n\
检查清单：\n\
1. 拼写错误、语法错误、缺少分号/括号等会导致编译失败的问题\n\
2. 函数签名、类型、模块引用是否正确\n\
3. 是否实现了子模块 prompt 中规定的所有输入输出\n\
4. 与接口规范是否一致\n\
5. 是否有明显的逻辑漏洞（如未处理的 None/Error）\n\n\
子模块要求：\n{submodule_prompt}\n\n\
代码：\n{code_ctx}\n\n\
用 [REVIEW] 开头，逐条列出检查结果。格式：\n\
[REVIEW] filename\n✓ 检查项描述\n⚠ 发现问题：具体描述",
        mental_model = mental_model_section(analysis_doc),
        submodule_prompt = step.submodule_prompt,
        code_ctx = code_ctx,
    )
}

fn build_fix_prompt(
    step: &PlanStep,
    written_files: &HashMap<String, String>,
    review: &str,
    output_lang: &str,
    analysis_doc: &str,
) -> String {
    let code_ctx = build_code_context(written_files);
    format!(
        "{mental_model}## Environment\n- OS: Windows 11\n- Shell: PowerShell\n\n\
审查意见如下，请修复所有标记了 ⚠ 的问题。\n\n\
可以先搜索相关代码再修复：[SYSTEM:grep pattern=\"符号名\" path=\"src\"]\n\n\
审查结果：\n{review}\n\n\
子模块要求：\n{submodule_prompt}\n\n\
当前代码：\n{code_ctx}\n\n\
## 输出格式（二选一）\n\
小范围修复（推荐）：\n\
```patch filename=\"path/to/file.rs\"\n---OLD---\n待替换的原始代码（必须与文件完全一致）\n---NEW---\n修复后的代码\n---END---\n```\n\
大范围重写：\n\
```rust filename=\"path/to/file.rs\"\n// 完整文件内容\n```\n\
所有注释和说明用 {output_lang}。",
        mental_model = mental_model_section(analysis_doc),
        submodule_prompt = step.submodule_prompt,
    )
}

fn build_architect_prompt(user_request: &str, plan_summary: &str, all_code: &str, analysis_doc: &str) -> String {
    format!(
        "{mental_model}你是架构师。请检查以下模块是否组装正确。\n\n\
原始需求：{user_request}\n\n\
规划方案：\n{plan_summary}\n\n\
各模块代码：\n{all_code}\n\n\
检查：\n\
1. 各模块间的接口是否匹配（A 的输出类型 = B 的输入类型）\n\
2. 数据流是否完整（从入口到出口）\n\
3. 是否有多余模块或缺失模块\n\
4. 整体是否符合规划\n\n\
用 [ARCHITECT] 开头，逐条列出。如有问题，标注严重程度：\n\
- [MINOR] 可追加修正指令修复\n\
- [MAJOR] 需要重新生成该模块\n\n\
如果一切正常，输出：[ARCHITECT] ✓ 所有模块接口匹配，架构检查通过。",
        mental_model = mental_model_section(analysis_doc),
    )
}

fn build_rework_prompt(
    user_request: &str,
    arch_feedback: &str,
    all_code: &str,
    output_lang: &str,
    analysis_doc: &str,
) -> String {
    format!(
        "{mental_model}## Environment\n- OS: Windows 11\n- Shell: PowerShell\n\n\
架构师发现以下问题，请修复所有受影响的代码。\n\n\
原始需求：{user_request}\n\n\
架构师意见：\n{arch_feedback}\n\n\
当前代码：\n{all_code}\n\n\
## 输出格式（每个受影响的文件二选一）\n\
小范围修复（推荐）：\n\
```patch filename=\"path/to/file.rs\"\n---OLD---\n待替换的原始代码\n---NEW---\n修复后的代码\n---END---\n```\n\
大范围重写：\n\
```rust filename=\"path/to/file.rs\"\n// 完整文件内容\n```\n\
所有注释和说明用 {output_lang}。",
        mental_model = mental_model_section(analysis_doc),
    )
}

// ── Main orchestrator ─────────────────────────────────────────────────────────

pub fn run_orchestrator(
    state: &Arc<Mutex<AppState>>,
    user_request: &str,
    context: &mut Vec<String>,
) -> Result<()> {
    // Fix 7: Global timer and token counter shared across all spinner instances.
    let global_tokens = Arc::new(Mutex::new(0u64));
    let global_start = Arc::new(Instant::now());

    let (host, work_dir, edit_mode, output_lang) = {
        let st = state.lock().unwrap();
        (
            st.ollama_host.clone(),
            st.work_dir.clone(),
            st.edit_mode,
            if st.lang == "zh_TW" {
                "Traditional Chinese (繁體中文)".to_string()
            } else {
                "English".to_string()
            },
        )
    };

    let ollama = OllamaClient::new(&host);
    let executor = Executor::new(work_dir.clone());
    let rules_mgr = RulesManager::new(work_dir.join(".sakichan.md"));
    let logger = Logger::from_work_dir(&work_dir);
    let _ = logger.init();

    let rules = rules_mgr.load();
    let existing_files = list_files_in(&work_dir);
    let files_str = existing_files.join(", ");

    // Fix 2: Generate project directory tree for Phase 1 prompt.
    let project_tree = generate_project_tree(&work_dir);

    // ════════════════════════════════════════════════════════════════════
    // Phase 0: Context Gathering
    // ════════════════════════════════════════════════════════════════════
    let enriched_request = gather_context(&ollama, user_request, &files_str, &work_dir);

    // ════════════════════════════════════════════════════════════════════
    // Phase 1: Analysis  (Fixes 1a, 2b, 3a)
    // ════════════════════════════════════════════════════════════════════
    println!("{CYAN}🔍 分析需求中... / Analyzing...{RESET}");

    let prompt_a = format!(
        "{role}## Environment\n\
- OS: Windows 11\n\
- Shell: PowerShell\n\
- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)\n\
- Path separator: \\\n\n\
## 项目目录结构\n\
{project_tree}\n\n\
Work directory: {work_dir}\n\
Existing files: {files_str}\n\
Project rules:\n\
{rules}\n\n\
User request:\n\
{request}\n\n\
Available tools (include in response if you need to inspect files):\n\
- [SYSTEM:read_file path=\"relative/path\"]\n\
- [SYSTEM:list_files path=\"dir\"]\n\n\
Based on the request, output ONLY valid JSON:\n\
{{\"understanding\":\"brief understanding in {lang}\",\"task_type\":\"natural language description of task type\",\"complexity\":5,\"gathered_info\":[{{\"label\":\"目标\",\"value\":\"...\",\"source\":\"需求推断\"}}],\"clarifications\":[{{\"question\":\"q\",\"recommendation\":\"suggestion\"}}]}}\n\n\
Rules:\n\
- task_type is free-form natural language, NOT an enum\n\
- complexity is 1-10\n\
- clarifications must be empty [] if everything is clear; max 3 items\n\
- Only ask clarifications that GENUINELY change the implementation direction\n\
- All text in {lang}\n\n\
完成 JSON 输出后，请生成一份完整的项目分析文档（Markdown 格式），用 [ANALYSIS_DOC] 作为起始标记。\n\
这份文档将作为共享心智模型，注入到后续所有规划和执行步骤中。\n\n\
文档结构：\n\
## 项目心智模型\n\
### 项目概述\n\
### 目录结构与模块职责\n\
### 关键依赖与数据流\n\
### 重要接口与约定\n\
### 注意事项\n\n\
请基于已读取的所有文件内容来撰写，确保信息准确完整。",
        role = ROLE_HEADER,
        project_tree = project_tree,
        work_dir = work_dir.display(),
        files_str = files_str,
        rules = rules,
        request = enriched_request,
        lang = output_lang,
    );

    let spinner = Spinner::new(SpinnerState::Thinking, Arc::clone(&global_tokens), Arc::clone(&global_start));
    let (analysis_raw, usage_a) = ollama.chat(DSR1, &prompt_a, Some(&dsr1_opts()))?;
    add_global_tokens(&global_tokens, &usage_a);
    spinner.stop();
    record_usage(state, &usage_a);

    let (analysis_clean, tool_results_a) = process_system_calls(&analysis_raw, &work_dir);
    let analysis_text = if tool_results_a.is_empty() {
        analysis_clean.clone()
    } else {
        format!("{analysis_clean}\n{tool_results_a}")
    };

    let analysis: AnalysisResponse = parse_json_from_response(&analysis_text)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();

    // Fix 1b: Extract the analysis document generated by Phase 1.
    let analysis_doc = extract_analysis_doc(&analysis_clean);

    println!("{CYAN}📊 复杂度 / Complexity: {}/10{RESET}", analysis.complexity);
    if !analysis.task_type.is_empty() {
        println!("{GRAY}任务类型 / Task type: {}{RESET}", analysis.task_type);
    }
    if !analysis.understanding.is_empty() {
        println!("{GRAY}理解: {}{RESET}", analysis.understanding);
    }
    if !analysis_doc.is_empty() {
        println!("{GRAY}📝 分析文档已生成 / Analysis doc generated ({} chars){RESET}", analysis_doc.len());
    }

    // ════════════════════════════════════════════════════════════════════
    // Phase 2: Clarification
    // ════════════════════════════════════════════════════════════════════
    if !analysis.gathered_info.is_empty() {
        println!("{CYAN}📋 已获取的信息:{RESET}");
        for info in &analysis.gathered_info {
            println!(
                "  {GREEN}✓{RESET} {}: {} {GRAY}({}){RESET}",
                info.label, info.value, info.source
            );
        }
        println!();
    }

    const MAX_INLINE_QUESTIONS: usize = 3;
    let mut decisions = Vec::new();

    if !analysis.clarifications.is_empty() {
        let total = analysis.clarifications.len();
        for (i, c) in analysis.clarifications.iter().enumerate() {
            if i >= MAX_INLINE_QUESTIONS {
                decisions.push(format!("Q: {} → A: {} (auto)", c.question, c.recommendation));
                continue;
            }
            println!();
            print!(
                "{YELLOW}❓ [{}/{}] {} {GRAY}[推荐: {}]{YELLOW}: {RESET}",
                i + 1,
                total.min(MAX_INLINE_QUESTIONS),
                c.question,
                c.recommendation
            );
            let _ = io::stdout().flush();
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();
            let decision = if input.is_empty() {
                c.recommendation.clone()
            } else {
                input.to_string()
            };
            decisions.push(format!("Q: {} → A: {}", c.question, decision));
        }
        println!("{CYAN}🔒 进入自动模式{RESET}");
    } else {
        println!("{GREEN}✅ 信息充分，进入规划{RESET}");
    }

    // ════════════════════════════════════════════════════════════════════
    // Phase 3: Planning  (Fixes 1c, 3b, 4a)
    // ════════════════════════════════════════════════════════════════════
    println!();
    println!("{CYAN}📋 规划步骤中... / Planning...{RESET}");

    let decisions_str = if decisions.is_empty() {
        "None".to_string()
    } else {
        decisions.join("\n")
    };

    let prompt_p = format!(
        "{role}{mental_model}## Environment\n\
- OS: Windows 11\n\
- Shell: PowerShell\n\n\
## 任务类型\n\
{task_type}\n\n\
注意：如果任务类型不是代码构建（如'撰写报告'、'分析代码'、'学术写作'），\n\
规划时应生成对应的产物（.md 文档、分析报告等），而非 .rs 代码文件。\n\n\
You are planning implementation steps. Output in {lang}.\n\n\
User request: {request}\n\
Understanding: {understanding}\n\
Decisions made: {decisions}\n\
Project rules: {rules}\n\
Existing files: {files_str}\n\n\
Output ONLY valid JSON with this schema:\n\
{{\"steps\":[{{\"id\":1,\"name\":\"动词开头的步骤名\",\"submodule_prompt\":\"完整独立prompt，包含：职责、输入接口、输出规范、不负责的范围。执行模型无需其他上下文即可完成工作。\",\"assigned_model\":\"QWEN\",\"files_to_create\":[\"path/to/file.rs\"],\"needs_compile\":false,\"verification\":\"如何验证\"}}]}}\n\n\
Rules:\n\
- submodule_prompt MUST be self-contained; orchestrator will prepend project background\n\
- assigned_model: \"QWEN\" ONLY for straightforward source-code generation (.rs/.py/.js etc.); \"DSR1\" for everything else — including documentation, reports, analysis, markdown writing, architecture design, and any task where the primary output is text/markdown\n\
- When output file extension is .md, .txt, or involves no compilation, assigned_model MUST be \"DSR1\"\n\
- step names MUST start with a verb (e.g. \"实现 Fibonacci 函数\", not \"Fibonacci 函数\")\n\
- For non-code tasks (docs, reports, analysis): files_to_create MUST use .md or .txt extension and MUST be NEW files — NEVER list existing .rs/.py source files as output targets\n\
- If the user says \"save to root directory\", root means the project root (where Cargo.toml lives), NOT src/\n\
- needs_compile: true ONLY if this step produces source code intended for compilation (e.g. .rs, .cpp, .c)\n\
- needs_compile: false for documentation, reports, analysis, markdown, config files, plain text, etc.\n\
- When in doubt, set needs_compile to false\n\
- All text in {lang}",
        role = ROLE_HEADER,
        mental_model = mental_model_section(&analysis_doc),
        task_type = analysis.task_type,
        lang = output_lang,
        request = user_request,
        understanding = analysis.understanding,
        decisions = decisions_str,
        rules = rules,
        files_str = files_str,
    );

    let spinner = Spinner::new(SpinnerState::Thinking, Arc::clone(&global_tokens), Arc::clone(&global_start));
    let (plan_raw, usage_p) = ollama.chat(DSR1, &prompt_p, Some(&dsr1_opts()))?;
    add_global_tokens(&global_tokens, &usage_p);
    spinner.stop();
    record_usage(state, &usage_p);

    let plan: PlanResponse = parse_json_from_response(&plan_raw)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or(PlanResponse {
            steps: vec![PlanStep {
                id: 1,
                name: "生成代码".to_string(),
                submodule_prompt: user_request.to_string(),
                assigned_model: "QWEN".to_string(),
                files_to_create: vec!["src/main.rs".to_string()],
                verification: "检查文件".to_string(),
                needs_compile: false,
            }],
        });

    println!("{CYAN}步骤 / Steps:{RESET}");
    for step in &plan.steps {
        let kind = if step.needs_compile { "🔨" } else { "📝" };
        println!(
            "  {GREEN}{}. {} {GRAY}[{}] {}{RESET}",
            step.id, step.name, step.assigned_model, kind
        );
    }
    println!();

    if !edit_mode {
        println!("{YELLOW}⚠️  只读模式，跳过代码生成 / Read-only mode — use /edit to enable{RESET}");
        return Ok(());
    }

    // ════════════════════════════════════════════════════════════════════
    // Phase 4: Execution
    // ════════════════════════════════════════════════════════════════════

    let checkpoint_info = git_checkpoint(&work_dir, user_request);
    state.lock().unwrap().checkpoint_count += 1;
    println!("{CYAN}  ● Git(checkpoint){RESET}");
    println!("  {GRAY}⎿  {checkpoint_info}{RESET}");
    println!();

    let any_needs_compile = plan.steps.iter().any(|s| s.needs_compile);
    if any_needs_compile { ensure_cargo_toml(&work_dir); }

    let mut written_files: HashMap<String, String> = HashMap::new();
    let mut all_completed: Vec<String> = Vec::new();
    let step_count = plan.steps.len();

    // ── 4a + 4b: Generate each step, then review ─────────────────────

    for (step_idx, step) in plan.steps.iter().enumerate() {
        println!("{PINK}● {}/{}: {}{RESET}", step_idx + 1, step_count, step.name);

        let exec_model = resolve_model(&step.assigned_model);
        let exec_opts = if exec_model == QWEN { qwen_gen_opts() } else { dsr1_opts() };

        let prior_code = gather_existing_code(&work_dir, &step.files_to_create);

        let gen_prompt = format!(
            "{role}{mental_model}## Environment\n\
- OS: Windows 11\n\
- Shell: PowerShell\n\
- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)\n\
- Path separator: \\\n\n\
Project: {request}\n\
Task type: {task_type}\n\n\
Existing relevant code:\n\
{prior_code}\n\n\
Your submodule task:\n\
{submodule_prompt}\n\n\
Target files: {files}\n\n\
## 可用上下文工具（需要时优先调用，然后再生成代码）\n\
- 搜索符号/函数定义：[SYSTEM:grep pattern=\"symbol_name\" path=\"src\"]\n\
- 读取文件：[SYSTEM:read_file path=\"src/file.rs\"]\n\n\
在生成代码之前，请先用 [THINK] 标记输出你的思考过程：\n\
- 这个模块的输入是什么\n\
- 需要产生什么输出\n\
- 关键的数据结构和函数签名\n\
- 可能的边界情况\n\n\
然后再用代码块输出。\n\n\
## 输出格式（二选一）\n\
对已存在文件做小范围修改（推荐，节省 token）：\n\
```patch filename=\"path/to/file.rs\"\n---OLD---\n待替换的原始代码（必须与文件完全一致，包括空格和换行）\n---NEW---\n替换后的代码\n---END---\n```\n\
新文件或大规模重写：\n\
```rust filename=\"path/to/file.rs\"\n// full code here\n```\n\
All comments and text in {lang}.",
            role = ROLE_HEADER,
            mental_model = mental_model_section(&analysis_doc),
            request = user_request,
            task_type = analysis.task_type,
            submodule_prompt = step.submodule_prompt,
            files = step.files_to_create.join(", "),
            lang = output_lang,
            prior_code = prior_code,
        );

        // 4a: Generate
        let spinner = Spinner::new(SpinnerState::Crafting, Arc::clone(&global_tokens), Arc::clone(&global_start));
        let (code_raw, usage_c) = ollama.chat(exec_model, &gen_prompt, Some(&exec_opts))?;
        add_global_tokens(&global_tokens, &usage_c);
        spinner.stop();
        record_usage(state, &usage_c);

        // Dynamic context: if model emitted SYSTEM tool calls, resolve them and do one round-trip
        let (code_clean, tool_results_gen) = process_system_calls(&code_raw, &work_dir);
        let final_code_raw = if !tool_results_gen.is_empty() {
            println!("{GRAY}  🔍 模型请求上下文，解析后补充重新生成...{RESET}");
            for line in tool_results_gen.lines().take(6) {
                if !line.trim().is_empty() { println!("    {GRAY}{line}{RESET}"); }
            }
            let followup = format!(
                "{gen_prompt}\n\n## Tool call results:\n{tool_results_gen}\n\nNow generate the complete output:"
            );
            let spinner2 = Spinner::new(SpinnerState::Crafting, Arc::clone(&global_tokens), Arc::clone(&global_start));
            match ollama.chat(exec_model, &followup, Some(&exec_opts)) {
                Ok((r, u)) => { add_global_tokens(&global_tokens, &u); spinner2.stop(); record_usage(state, &u); r }
                Err(_)     => { spinner2.stop(); code_clean }
            }
        } else {
            code_raw
        };

        if let Some(think) = extract_think_block(&final_code_raw) {
            println!("{GRAY}  [THINK]{RESET}");
            for line in think.lines().take(8) {
                println!("    {GRAY}{}{RESET}", line.trim());
            }
            println!();
        }

        let blocks = extract_code_blocks(&final_code_raw);
        if blocks.is_empty() {
            println!("{YELLOW}  ⚠ No code blocks generated for step {}{RESET}", step.id);
        } else {
            write_or_patch_files(&blocks, &work_dir, &mut written_files, &mut all_completed);
        }

        // 4b: Review loop (max REVIEW_MAX_ATTEMPTS)
        for review_attempt in 1..=REVIEW_MAX_ATTEMPTS {
            let review_prompt = build_review_prompt(step, &written_files, &analysis_doc);

            let spinner = Spinner::new(SpinnerState::Reviewing, Arc::clone(&global_tokens), Arc::clone(&global_start));
            let (review_raw, usage_r) = ollama.chat(QWEN, &review_prompt, Some(&qwen_ctx_opts()))?;
            add_global_tokens(&global_tokens, &usage_r);
            spinner.stop();
            record_usage(state, &usage_r);

            print!("{CYAN}  [Review {review_attempt}/{REVIEW_MAX_ATTEMPTS}]{RESET} ");
            if !review_has_warnings(&review_raw) {
                println!("{GREEN}✓ passed{RESET}");
                break;
            }

            println!("{YELLOW}⚠ issues found{RESET}");
            for line in review_raw.lines().filter(|l| l.contains('⚠')).take(3) {
                println!("    {YELLOW}{}{RESET}", line.trim());
            }

            if review_attempt == REVIEW_MAX_ATTEMPTS {
                println!("  {YELLOW}⚠ Max reviews reached, proceeding anyway{RESET}");
                break;
            }

            let fix_prompt = build_fix_prompt(step, &written_files, &review_raw, &output_lang, &analysis_doc);

            let spinner = Spinner::new(SpinnerState::Fixing, Arc::clone(&global_tokens), Arc::clone(&global_start));
            let (fixed_raw, usage_f) = ollama.chat(exec_model, &fix_prompt, Some(&exec_opts))?;
            add_global_tokens(&global_tokens, &usage_f);
            spinner.stop();
            record_usage(state, &usage_f);

            let fix_blocks = extract_code_blocks(&fixed_raw);
            if !fix_blocks.is_empty() {
                write_or_patch_files(&fix_blocks, &work_dir, &mut written_files, &mut all_completed);
            }
        }
    }

    // ── 4c + 4d: Architect check and rework ──────────────────────────

    if !written_files.is_empty() {
        let plan_summary = plan.steps.iter().map(|s| {
            format!(
                "Step {}: {}\n  Model: {}\n  Files: {}\n  Task: {}",
                s.id, s.name, s.assigned_model,
                s.files_to_create.join(", "),
                s.submodule_prompt
            )
        }).collect::<Vec<_>>().join("\n\n");

        for arch_iter in 1..=ARCH_MAX_ITERATIONS {
            let all_code_ctx = build_code_context(&written_files);
            let arch_prompt = build_architect_prompt(user_request, &plan_summary, &all_code_ctx, &analysis_doc);

            println!("{CYAN}🏛️  架构师检查 / Architect check ({arch_iter}/{ARCH_MAX_ITERATIONS})...{RESET}");

            let spinner = Spinner::new(SpinnerState::Architecting, Arc::clone(&global_tokens), Arc::clone(&global_start));
            let (arch_raw, usage_arch) = ollama.chat(DSR1, &arch_prompt, Some(&dsr1_opts()))?;
            add_global_tokens(&global_tokens, &usage_arch);
            spinner.stop();
            record_usage(state, &usage_arch);

            for line in arch_raw.lines().take(15) {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                if trimmed.contains("[MAJOR]") {
                    println!("  {RED}{trimmed}{RESET}");
                } else if trimmed.contains("[MINOR]") {
                    println!("  {YELLOW}{trimmed}{RESET}");
                } else {
                    println!("  {GRAY}{trimmed}{RESET}");
                }
            }

            if !arch_has_issues(&arch_raw) {
                println!("{GREEN}✓ 架构检查通过 / Architect check passed{RESET}");
                break;
            }

            println!("{YELLOW}⚠ 架构师发现问题，进行返工 / Issues found, reworking...{RESET}");

            if arch_iter == ARCH_MAX_ITERATIONS {
                println!("{YELLOW}⚠ Max architect iterations reached, proceeding to compile check{RESET}");
                break;
            }

            let all_code_ctx = build_code_context(&written_files);
            let rework_prompt = build_rework_prompt(user_request, &arch_raw, &all_code_ctx, &output_lang, &analysis_doc);

            let spinner = Spinner::new(SpinnerState::FixingDsr1, Arc::clone(&global_tokens), Arc::clone(&global_start));
            let (rework_raw, usage_rw) = ollama.chat(DSR1, &rework_prompt, Some(&dsr1_opts()))?;
            add_global_tokens(&global_tokens, &usage_rw);
            spinner.stop();
            record_usage(state, &usage_rw);

            let rework_blocks = extract_code_blocks(&rework_raw);
            if !rework_blocks.is_empty() {
                write_or_patch_files(&rework_blocks, &work_dir, &mut written_files, &mut all_completed);
            }
        }
    }

    // ── 4e: Compilation gate (skipped for documentation-only steps) ──────

    let mut compile_ok = true;
    if work_dir.join("Cargo.toml").exists() && !written_files.is_empty() && any_needs_compile {
        println!();
        println!("{CYAN}🔍 编译检查 / Cargo check...{RESET}");
        compile_ok = false;

        for attempt in 1..=CARGO_FIX_MAX {
            let (ok, output, _) = executor.run("cargo check 2>&1")?;

            if ok {
                print_bash_result("cargo check 2>&1", &output, 5);
                println!("{GREEN}  ✓ 编译通过 (attempt {attempt}){RESET}");
                compile_ok = true;
                break;
            }

            if attempt == CARGO_FIX_MAX {
                println!("{RED}❌ 编译失败 after {CARGO_FIX_MAX} attempts{RESET}");
                // Show final errors to user for transparency
                println!("{RED}  ─── 最终编译错误 ───{RESET}");
                for line in output.lines().take(25) {
                    println!("  {RED}{line}{RESET}");
                }
                println!("{RED}  ────────────────────{RESET}");
                break;
            }

            let err: String = output.chars().take(1500).collect();
            println!("{RED}  ✗ 编译失败 (attempt {attempt}), 修复中...{RESET}");
            // Show errors to user so they can understand what's happening
            for line in output.lines().take(12) {
                if line.contains("error") || line.contains("warning") {
                    println!("    {GRAY}{line}{RESET}");
                }
            }

            let all_code_ctx = build_code_context(&written_files);
            let cargo_fix_prompt = format!(
                "{role}{mental_model}## Environment\n- OS: Windows 11\n- Shell: PowerShell\n\n\
编译错误（尝试 {attempt}/{CARGO_FIX_MAX}）：\n```\n{err}\n```\n\n\
当前代码：\n{all_code_ctx}\n\n\
可以先搜索相关符号：[SYSTEM:grep pattern=\"符号名\" path=\"src\"]\n\n\
## 输出格式（二选一）\n\
小范围修复（推荐）：\n\
```patch filename=\"path/to/file.rs\"\n---OLD---\n待替换的原始代码\n---NEW---\n修复后的代码\n---END---\n```\n\
大范围重写：\n\
```rust filename=\"path/to/file.rs\"\n// full corrected code\n```\n\
所有说明用 {output_lang}。",
                role = ROLE_HEADER,
                mental_model = mental_model_section(&analysis_doc),
            );

            let spinner = Spinner::new(SpinnerState::FixingDsr1, Arc::clone(&global_tokens), Arc::clone(&global_start));
            let (fix_raw, usage_fix) = ollama.chat(DSR1, &cargo_fix_prompt, Some(&dsr1_opts()))?;
            add_global_tokens(&global_tokens, &usage_fix);
            spinner.stop();
            record_usage(state, &usage_fix);

            let (fix_clean, fix_tool_results) = process_system_calls(&fix_raw, &work_dir);
            let final_fix_raw = if !fix_tool_results.is_empty() {
                let followup = format!(
                    "{cargo_fix_prompt}\n\n## Tool call results:\n{fix_tool_results}\n\nNow output the fix:"
                );
                let sp2 = Spinner::new(SpinnerState::FixingDsr1, Arc::clone(&global_tokens), Arc::clone(&global_start));
                match ollama.chat(DSR1, &followup, Some(&dsr1_opts())) {
                    Ok((r, u)) => { add_global_tokens(&global_tokens, &u); sp2.stop(); record_usage(state, &u); r }
                    Err(_)     => { sp2.stop(); fix_clean }
                }
            } else { fix_raw };

            let fix_blocks = extract_code_blocks(&final_fix_raw);
            if !fix_blocks.is_empty() {
                write_or_patch_files(&fix_blocks, &work_dir, &mut written_files, &mut all_completed);
            }
        }
    } else if !written_files.is_empty() && !any_needs_compile {
        println!("{CYAN}📝 文档步骤，跳过编译{RESET}");
    }

    // ════════════════════════════════════════════════════════════════════
    // Phase 5: Wrap-up
    // ════════════════════════════════════════════════════════════════════
    println!();
    print_change_summary(&all_completed);

    // Fix 6: Ask DSR1 to update .sakichan.md with this session's findings.
    let steps_summary = plan.steps.iter().enumerate().map(|(i, s)| {
        format!("{}. {} ({})", i + 1, s.name, s.files_to_create.join(", "))
    }).collect::<Vec<_>>().join("\n");

    let current_rules = rules_mgr.load();
    let rules_update_prompt = format!(
        "## 本次对话完成的工作\n{steps_summary}\n\n\
## 当前 .sakichan.md 内容\n{current_rules}\n\n\
请基于本次对话的发现，更新项目规则文件。\n\
保留原有的重要信息，追加新的发现。\n\
输出用 [RULES_UPDATE] 标记。"
    );

    let spinner = Spinner::new(SpinnerState::Thinking, Arc::clone(&global_tokens), Arc::clone(&global_start));
    let rules_updated = match ollama.chat(DSR1, &rules_update_prompt, Some(&dsr1_opts())) {
        Ok((rules_raw, usage_ru)) => {
            add_global_tokens(&global_tokens, &usage_ru);
            spinner.stop();
            record_usage(state, &usage_ru);
            if let Some(new_rules) = extract_rules_update(&rules_raw) {
                let ok = fs::write(work_dir.join(".sakichan.md"), &new_rules).is_ok();
                if ok { println!("{GRAY}📄 规则文件已由 AI 更新 / Rules updated by AI{RESET}"); }
                ok
            } else {
                false
            }
        }
        Err(_) => {
            spinner.stop();
            false
        }
    };

    if !rules_updated {
        let structure = list_files_in(&work_dir).join("\n");
        let _ = rules_mgr.update(&all_completed, &structure);
        println!("{GRAY}📄 规则文件已更新 / Rules updated{RESET}");
    }

    let elapsed_secs = global_start.elapsed().as_secs();
    let elapsed_f = global_start.elapsed().as_secs_f64();
    let elapsed_str = if elapsed_secs >= 60 {
        format!("{}m {}s", elapsed_secs / 60, elapsed_secs % 60)
    } else {
        format!("{}s", elapsed_secs)
    };

    let _ = logger.log_task(
        user_request,
        &analysis.task_type,
        &all_completed,
        compile_ok,
        &[],
        DSR1,
        elapsed_f,
    );
    let log_name = format!(
        "{}_log.md",
        work_dir.file_name().unwrap_or_default().to_string_lossy()
    );
    println!("{GRAY}📝 日志已更新 → {log_name}{RESET}");
    println!("{PINK}✻ Baked for {elapsed_str}{RESET}");

    context.push(format!("User: {user_request}"));
    context.push(format!(
        "Assistant: Completed {} steps ({}), compile_ok={}",
        plan.steps.len(), analysis.task_type, compile_ok
    ));

    let session_id = uuid::Uuid::new_v4().to_string();
    let _ = state.lock().unwrap().save_session(&session_id, context);

    // Fix 9: Auto git commit after Phase 5.
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
            if !first.is_empty() {
                println!("{GRAY}● Git(commit): {first}{RESET}");
            }
        }
        Err(e) => println!("{YELLOW}● Git(commit): Error: {e}{RESET}"),
    }

    Ok(())
}
