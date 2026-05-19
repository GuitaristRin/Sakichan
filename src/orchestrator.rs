use crate::display::*;
use crate::executor::Executor;
use crate::logger::Logger;
use crate::ollama::{ModelOptions, OllamaClient};
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

/// Extract the first complete `{…}` block from raw model output.
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

/// Process [SYSTEM:read_file path="..."] and [SYSTEM:list_files path="..."] tags in AI output.
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

    (clean, tool_results)
}

fn record_usage(state: &Arc<Mutex<AppState>>, usage: &crate::ollama::UsageStats) {
    if let Ok(mut st) = state.lock() {
        st.usage.add(usage);
        let _ = st.save_usage();
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

/// Write code blocks to disk, updating the written_files map and printing diffs.
fn write_step_files(
    blocks: &[(String, String, String)],
    work_dir: &Path,
    written_files: &mut HashMap<String, String>,
    all_completed: &mut Vec<String>,
) {
    for (_, filename, code) in blocks {
        if filename.is_empty() { continue; }
        let fpath = work_dir.join(filename);
        let old_content = fs::read_to_string(&fpath).unwrap_or_default();
        if let Some(p) = fpath.parent() { let _ = fs::create_dir_all(p); }
        let _ = fs::write(&fpath, code);
        print_code_diff(filename, &old_content, code);
        written_files.insert(filename.clone(), code.clone());
        all_completed.push(filename.clone());
    }
}

/// Build a formatted string of all written files for context.
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

/// Fixed git checkpoint using Command directly (avoids shell quoting bugs).
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
    arch.contains("[MINOR]") || arch.contains("[MAJOR]")
}

fn build_review_prompt(step: &PlanStep, written_files: &HashMap<String, String>) -> String {
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
        r#"你是代码审查员。请检查以下代码是否存在问题。

检查清单：
1. 拼写错误、语法错误、缺少分号/括号等会导致编译失败的问题
2. 函数签名、类型、模块引用是否正确
3. 是否实现了子模块 prompt 中规定的所有输入输出
4. 与接口规范是否一致
5. 是否有明显的逻辑漏洞（如未处理的 None/Error）

子模块要求：
{submodule_prompt}

代码：
{code_ctx}

用 [REVIEW] 开头，逐条列出检查结果。格式：
[REVIEW] filename
✓ 检查项描述
⚠ 发现问题：具体描述"#,
        submodule_prompt = step.submodule_prompt,
        code_ctx = code_ctx,
    )
}

fn build_fix_prompt(
    step: &PlanStep,
    written_files: &HashMap<String, String>,
    review: &str,
    output_lang: &str,
) -> String {
    let code_ctx = build_code_context(written_files);
    format!(
        r#"## Environment
- OS: Windows 11
- Shell: PowerShell

审查意见如下，请修复所有标记了 ⚠ 的问题。

审查结果：
{review}

子模块要求：
{submodule_prompt}

当前代码：
{code_ctx}

请输出修复后的完整代码，格式：
```rust filename="path/to/file.rs"
// full corrected code
```
所有注释和说明用 {output_lang}。"#,
        submodule_prompt = step.submodule_prompt,
    )
}

fn build_architect_prompt(user_request: &str, plan_summary: &str, all_code: &str) -> String {
    format!(
        r#"你是架构师。请检查以下模块是否组装正确。

原始需求：{user_request}

规划方案：
{plan_summary}

各模块代码：
{all_code}

检查：
1. 各模块间的接口是否匹配（A 的输出类型 = B 的输入类型）
2. 数据流是否完整（从入口到出口）
3. 是否有多余模块或缺失模块
4. 整体是否符合规划

用 [ARCHITECT] 开头，逐条列出。如有问题，标注严重程度：
- [MINOR] 可追加修正指令修复
- [MAJOR] 需要重新生成该模块

如果一切正常，输出：[ARCHITECT] ✓ 所有模块接口匹配，架构检查通过。"#
    )
}

fn build_rework_prompt(
    user_request: &str,
    arch_feedback: &str,
    all_code: &str,
    output_lang: &str,
) -> String {
    format!(
        r#"## Environment
- OS: Windows 11
- Shell: PowerShell

架构师发现以下问题，请修复所有受影响的代码。

原始需求：{user_request}

架构师意见：
{arch_feedback}

当前代码：
{all_code}

对每个需要修改的文件输出完整修复后的代码，格式：
```rust filename="path/to/file.rs"
// full corrected code
```
所有注释和说明用 {output_lang}。"#
    )
}

// ── Main orchestrator ─────────────────────────────────────────────────────────

pub fn run_orchestrator(
    state: &Arc<Mutex<AppState>>,
    user_request: &str,
    context: &mut Vec<String>,
) -> Result<()> {
    let run_start = Instant::now();

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

    // ════════════════════════════════════════════════════════════════════
    // Phase 0: Context Gathering  (qwen, temperature=0.1)
    // ════════════════════════════════════════════════════════════════════
    let enriched_request = gather_context(&ollama, user_request, &files_str, &work_dir);

    // ════════════════════════════════════════════════════════════════════
    // Phase 1: Analysis  (dsr1, temperature=0.3)
    // ════════════════════════════════════════════════════════════════════
    println!("{CYAN}🔍 分析需求中... / Analyzing...{RESET}");

    let prompt_a = format!(
        r#"## Environment
- OS: Windows 11
- Shell: PowerShell
- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)
- Path separator: \

You are an expert analyzing a user request. Output your analysis in {lang}.

Work directory: {work_dir}
Existing files: {files_str}
Project rules:
{rules}

User request:
{request}

Available tools (include in response if you need to inspect files):
- [SYSTEM:read_file path="relative/path"]
- [SYSTEM:list_files path="dir"]

Based on the request, output ONLY valid JSON:
{{"understanding":"brief understanding in {lang}","task_type":"natural language description of task type (e.g. '修改现有Rust代码', '生成学术论文', '代码分析解读', '跨语言翻译')","complexity":5,"gathered_info":[{{"label":"目标","value":"...","source":"需求推断"}}],"clarifications":[{{"question":"q","recommendation":"suggestion"}}]}}

Rules:
- task_type is free-form natural language, NOT an enum
- complexity is 1-10
- clarifications must be empty [] if everything is clear; max 3 items
- Only ask clarifications that GENUINELY change the implementation direction
- All text in {lang}"#,
        lang = output_lang,
        work_dir = work_dir.display(),
        request = enriched_request,
    );

    let spinner = Spinner::new(SpinnerState::Thinking);
    let (analysis_raw, usage_a) = ollama.chat(DSR1, &prompt_a, Some(&dsr1_opts()))?;
    spinner.update_tokens(usage_a.input_tokens + usage_a.output_tokens);
    spinner.stop();
    record_usage(state, &usage_a);

    let (analysis_clean, tool_results_a) = process_system_calls(&analysis_raw, &work_dir);
    let analysis_text = if tool_results_a.is_empty() {
        analysis_clean
    } else {
        format!("{analysis_clean}\n{tool_results_a}")
    };

    let analysis: AnalysisResponse = parse_json_from_response(&analysis_text)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();

    println!("{CYAN}📊 复杂度 / Complexity: {}/10{RESET}", analysis.complexity);
    if !analysis.task_type.is_empty() {
        println!("{GRAY}任务类型 / Task type: {}{RESET}", analysis.task_type);
    }
    if !analysis.understanding.is_empty() {
        println!("{GRAY}理解: {}{RESET}", analysis.understanding);
    }

    // ════════════════════════════════════════════════════════════════════
    // Phase 2: Clarification  (interactive, max 3 questions)
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
    // Phase 3: Planning  (dsr1, temperature=0.3)
    // ════════════════════════════════════════════════════════════════════
    println!();
    println!("{CYAN}📋 规划步骤中... / Planning...{RESET}");

    let decisions_str = if decisions.is_empty() {
        "None".to_string()
    } else {
        decisions.join("\n")
    };

    let prompt_p = format!(
        r#"## Environment
- OS: Windows 11
- Shell: PowerShell

You are planning implementation steps. Output in {lang}.

User request: {request}
Task type: {task_type}
Understanding: {understanding}
Decisions made: {decisions}
Project rules: {rules}
Existing files: {files_str}

Output ONLY valid JSON with this schema:
{{"steps":[{{"id":1,"name":"动词开头的步骤名","submodule_prompt":"完整独立prompt，包含：职责、输入接口、输出规范、不负责的范围。执行模型无需其他上下文即可完成工作。","assigned_model":"QWEN","files_to_create":["path/to/file.rs"],"verification":"如何验证"}}]}}

Rules:
- submodule_prompt MUST be self-contained; orchestrator will prepend project background
- assigned_model is "QWEN" (for straightforward tasks) or "DSR1" (for complex logic/architecture)
- step names MUST start with a verb (e.g. "实现 Fibonacci 函数", not "Fibonacci 函数")
- If task has no code to write (analysis, essay, etc.), steps should create output files
- All text in {lang}"#,
        lang = output_lang,
        request = user_request,
        task_type = analysis.task_type,
        understanding = analysis.understanding,
        decisions = decisions_str,
    );

    let spinner = Spinner::new(SpinnerState::Thinking);
    let (plan_raw, usage_p) = ollama.chat(DSR1, &prompt_p, Some(&dsr1_opts()))?;
    spinner.update_tokens(usage_p.input_tokens + usage_p.output_tokens);
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
            }],
        });

    println!("{CYAN}步骤 / Steps:{RESET}");
    for step in &plan.steps {
        println!(
            "  {GREEN}{}. {} {GRAY}[{}]{RESET}",
            step.id, step.name, step.assigned_model
        );
    }
    println!();

    // Read-only: stop here
    if !edit_mode {
        println!("{YELLOW}⚠️  只读模式，跳过代码生成 / Read-only mode — use /edit to enable{RESET}");
        return Ok(());
    }

    // ════════════════════════════════════════════════════════════════════
    // Phase 4: Execution
    // ════════════════════════════════════════════════════════════════════

    // Git checkpoint — one per session, before any writes
    let checkpoint_info = git_checkpoint(&work_dir, user_request);
    state.lock().unwrap().checkpoint_count += 1;
    println!("{CYAN}  ● Git(checkpoint){RESET}");
    println!("  {GRAY}⎿  {checkpoint_info}{RESET}");
    println!();

    // Auto-create Cargo.toml if this appears to be a Rust project
    let is_rust = plan.steps.iter()
        .flat_map(|s| s.files_to_create.iter())
        .any(|f| f.ends_with(".rs"));
    if is_rust { ensure_cargo_toml(&work_dir); }

    let mut written_files: HashMap<String, String> = HashMap::new();
    let mut all_completed: Vec<String> = Vec::new();
    let step_count = plan.steps.len();

    // ── 4a + 4b: Generate each step, then review ─────────────────────

    for (step_idx, step) in plan.steps.iter().enumerate() {
        println!("{PINK}● {}/{}: {}{RESET}", step_idx + 1, step_count, step.name);

        let exec_model = resolve_model(&step.assigned_model);
        let exec_opts = if exec_model == QWEN { qwen_gen_opts() } else { dsr1_opts() };

        // Pull in any files this step needs from disk (from prior steps)
        let prior_code = gather_existing_code(&work_dir, &step.files_to_create);

        let gen_prompt = format!(
            r#"## Environment
- OS: Windows 11
- Shell: PowerShell
- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)
- Path separator: \

Project: {request}
Task type: {task_type}

Existing relevant code:
{prior_code}

Your submodule task:
{submodule_prompt}

Target files: {files}

Generate COMPLETE, compilable code for each file. Format every file as:
```rust filename="path/to/file.rs"
// full code here
```
All comments and text in {lang}."#,
            request = user_request,
            task_type = analysis.task_type,
            submodule_prompt = step.submodule_prompt,
            files = step.files_to_create.join(", "),
            lang = output_lang,
        );

        // 4a: Generate
        let spinner = Spinner::new(SpinnerState::Crafting);
        let (code_raw, usage_c) = ollama.chat(exec_model, &gen_prompt, Some(&exec_opts))?;
        spinner.update_tokens(usage_c.input_tokens + usage_c.output_tokens);
        spinner.stop();
        record_usage(state, &usage_c);

        let blocks = extract_code_blocks(&code_raw);
        if blocks.is_empty() {
            println!("{YELLOW}  ⚠ No code blocks generated for step {}{RESET}", step.id);
        } else {
            write_step_files(&blocks, &work_dir, &mut written_files, &mut all_completed);
        }

        // 4b: Review loop (max REVIEW_MAX_ATTEMPTS)
        let mut fix_prompt_extra = String::new();
        for review_attempt in 1..=REVIEW_MAX_ATTEMPTS {
            let review_prompt = build_review_prompt(step, &written_files);

            let spinner = Spinner::new(SpinnerState::Reviewing);
            let (review_raw, usage_r) = ollama.chat(QWEN, &review_prompt, Some(&qwen_ctx_opts()))?;
            spinner.update_tokens(usage_r.input_tokens + usage_r.output_tokens);
            spinner.stop();
            record_usage(state, &usage_r);

            // Show condensed review output
            print!("{CYAN}  [Review {review_attempt}/{REVIEW_MAX_ATTEMPTS}]{RESET} ");
            if !review_has_warnings(&review_raw) {
                println!("{GREEN}✓ passed{RESET}");
                break;
            }

            println!("{YELLOW}⚠ issues found{RESET}");
            // Show first warning line
            for line in review_raw.lines().filter(|l| l.contains('⚠')).take(3) {
                println!("    {YELLOW}{}{RESET}", line.trim());
            }

            if review_attempt == REVIEW_MAX_ATTEMPTS {
                println!("  {YELLOW}⚠ Max reviews reached, proceeding anyway{RESET}");
                break;
            }

            // Fix
            fix_prompt_extra.push_str(&format!("\n\n[Review {}]\n{}", review_attempt, review_raw));
            let fix_prompt = build_fix_prompt(step, &written_files, &review_raw, &output_lang);

            let spinner = Spinner::new(SpinnerState::Fixing);
            let (fixed_raw, usage_f) = ollama.chat(exec_model, &fix_prompt, Some(&exec_opts))?;
            spinner.update_tokens(usage_f.input_tokens + usage_f.output_tokens);
            spinner.stop();
            record_usage(state, &usage_f);

            let fix_blocks = extract_code_blocks(&fixed_raw);
            if !fix_blocks.is_empty() {
                write_step_files(&fix_blocks, &work_dir, &mut written_files, &mut all_completed);
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
            let arch_prompt = build_architect_prompt(user_request, &plan_summary, &all_code_ctx);

            println!("{CYAN}🏛️  架构师检查 / Architect check ({arch_iter}/{ARCH_MAX_ITERATIONS})...{RESET}");

            let spinner = Spinner::new(SpinnerState::Architecting);
            let (arch_raw, usage_arch) = ollama.chat(DSR1, &arch_prompt, Some(&dsr1_opts()))?;
            spinner.update_tokens(usage_arch.input_tokens + usage_arch.output_tokens);
            spinner.stop();
            record_usage(state, &usage_arch);

            // Show condensed architect output
            for line in arch_raw.lines().take(8) {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
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

            // 4d: Fix — let dsr1 fix all issues at once
            let all_code_ctx = build_code_context(&written_files);
            let rework_prompt = build_rework_prompt(user_request, &arch_raw, &all_code_ctx, &output_lang);

            let spinner = Spinner::new(SpinnerState::FixingDsr1);
            let (rework_raw, usage_rw) = ollama.chat(DSR1, &rework_prompt, Some(&dsr1_opts()))?;
            spinner.update_tokens(usage_rw.input_tokens + usage_rw.output_tokens);
            spinner.stop();
            record_usage(state, &usage_rw);

            let rework_blocks = extract_code_blocks(&rework_raw);
            if !rework_blocks.is_empty() {
                write_step_files(&rework_blocks, &work_dir, &mut written_files, &mut all_completed);
            }
        }
    }

    // ── 4e: Compilation gate (Rust projects only) ─────────────────────

    let mut compile_ok = true;
    if work_dir.join("Cargo.toml").exists() && !written_files.is_empty() {
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
                break;
            }

            let err: String = output.chars().take(1500).collect();
            println!("{RED}  ✗ 编译失败 (attempt {attempt}), 修复中...{RESET}");

            let all_code_ctx = build_code_context(&written_files);
            let cargo_fix_prompt = format!(
                r#"## Environment
- OS: Windows 11
- Shell: PowerShell

编译错误（尝试 {attempt}/{CARGO_FIX_MAX}）：
```
{err}
```

当前代码：
{all_code_ctx}

请修复所有编译错误，输出完整修复后的代码，格式：
```rust filename="path/to/file.rs"
// full corrected code
```
所有说明用 {output_lang}。"#
            );

            let spinner = Spinner::new(SpinnerState::FixingDsr1);
            let (fix_raw, usage_fix) = ollama.chat(DSR1, &cargo_fix_prompt, Some(&dsr1_opts()))?;
            spinner.update_tokens(usage_fix.input_tokens + usage_fix.output_tokens);
            spinner.stop();
            record_usage(state, &usage_fix);

            let fix_blocks = extract_code_blocks(&fix_raw);
            if !fix_blocks.is_empty() {
                write_step_files(&fix_blocks, &work_dir, &mut written_files, &mut all_completed);
            }
        }
    }

    // ════════════════════════════════════════════════════════════════════
    // Phase 5: Wrap-up
    // ════════════════════════════════════════════════════════════════════
    println!();
    print_change_summary(&all_completed);

    let structure = list_files_in(&work_dir).join("\n");
    let _ = rules_mgr.update(&all_completed, &structure);
    println!("{GRAY}📄 规则文件已更新 / Rules updated{RESET}");

    let elapsed = run_start.elapsed().as_secs_f64();
    let _ = logger.log_task(
        user_request,
        &analysis.task_type,
        &all_completed,
        compile_ok,
        &[],
        DSR1,
        elapsed,
    );
    let log_name = format!(
        "{}_log.md",
        work_dir.file_name().unwrap_or_default().to_string_lossy()
    );
    println!("{GRAY}📝 日志已更新 → {log_name}{RESET}");

    let elapsed_str = if elapsed >= 60.0 {
        format!("{}m {:.0}s", elapsed as u64 / 60, elapsed % 60.0)
    } else {
        format!("{:.1}s", elapsed)
    };
    println!("{PINK}✻ Baked for {elapsed_str}{RESET}");

    context.push(format!("User: {user_request}"));
    context.push(format!(
        "Assistant: Completed {} steps ({}), compile_ok={}",
        plan.steps.len(), analysis.task_type, compile_ok
    ));

    let session_id = uuid::Uuid::new_v4().to_string();
    let _ = state.lock().unwrap().save_session(&session_id, context);

    Ok(())
}
