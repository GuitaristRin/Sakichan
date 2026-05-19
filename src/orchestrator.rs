use crate::display::*;
use crate::executor::Executor;
use crate::logger::Logger;
use crate::ollama::{ModelOptions, OllamaClient};
use crate::rules::RulesManager;
use crate::state::AppState;
use anyhow::Result;
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use chrono;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const QWEN: &str = "qwen2.5-coder:7b";
const DSR1: &str = "deepseek-r1:8b";
const QWEN_MAX_RETRIES: u32 = 5;
const DSR1_MAX_RETRIES: u32 = 10;

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

#[derive(Debug, Deserialize, Default)]
struct AnalysisResponse {
    #[serde(default)]
    understanding: String,
    #[serde(default)]
    complexity: u8,
    #[serde(default = "default_code_mod")]
    code_modification: bool,
    #[serde(default)]
    gathered_info: Vec<GatheredInfo>,
    #[serde(default)]
    clarifications: Vec<Clarification>,
}

fn default_code_mod() -> bool { true }

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
    description: String,
    #[serde(default)]
    files_to_create: Vec<String>,
    #[serde(default)]
    verification: String,
}

fn list_files_in(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().into_string().unwrap_or_default();
            if name.starts_with('.') || name == "target" {
                continue;
            }
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

fn process_tool_calls(response: &str, work_dir: &Path) -> (String, String) {
    let mut tool_results = String::new();
    let mut clean = response.to_string();
    while let Some(start) = clean.find("[TOOL:read_file") {
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
    while let Some(start) = clean.find("[TOOL:list_files") {
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
    let cargo = work_dir.join("Cargo.toml");
    if cargo.exists() {
        if let Ok(content) = fs::read_to_string(&cargo) {
            if !result.contains("Cargo.toml") {
                result.push_str(&format!("=== Cargo.toml ===\n{}\n\n", content));
            }
        }
    }
    if result.is_empty() { result = "(empty project)".to_string(); }
    result
}

fn ensure_cargo_toml(work_dir: &Path) -> bool {
    let cargo = work_dir.join("Cargo.toml");
    if !cargo.exists() {
        let content = r#"[package]
name = "project"
version = "0.1.0"
edition = "2021"

[dependencies]
"#;
        let _ = fs::write(&cargo, content);
        println!("{GREEN}📦 已自动创建 Cargo.toml{RESET}");
        return true;
    }
    false
}

fn record_usage(state: &Arc<Mutex<AppState>>, usage: &crate::ollama::UsageStats) {
    if let Ok(mut st) = state.lock() {
        st.usage.add(usage);
        let _ = st.save_usage();
    }
}

fn get_output_language(state: &Arc<Mutex<AppState>>) -> String {
    let st = state.lock().unwrap();
    if st.lang == "zh_TW" { "Traditional Chinese (繁體中文)".to_string() }
    else { "English".to_string() }
}

fn create_sandbox(work_dir: &Path) -> Result<PathBuf> {
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let sandbox = work_dir.join(".sakichan").join("sandboxes").join(&timestamp);
    fs::create_dir_all(&sandbox)?;
    Ok(sandbox)
}

fn git_checkpoint(executor: &Executor, description: &str) -> String {
    let _ = executor.run("git add -A");
    let safe_desc: String = description.chars()
        .map(|c| if c == '"' || c == '\\' { '-' } else { c })
        .collect();
    let msg = format!("sakichan: checkpoint before '{safe_desc}'");
    let cmd = format!("git commit -m \"{msg}\" --allow-empty 2>&1");
    match executor.run(&cmd) {
        Ok((true, _, _)) => format!("Created checkpoint: {msg}"),
        Ok((false, output, _)) => {
            let err = output.lines().next().unwrap_or("git error").trim().to_string();
            format!("Git: {err}")
        }
        Err(e) => format!("Error: {e}"),
    }
}

fn gather_context(
    ollama: &OllamaClient,
    user_request: &str,
    files_str: &str,
    work_dir: &Path,
) -> String {
    const FILE_EXTS: &[&str] = &[".md", ".rs", ".toml", ".txt", ".json", ".yaml", ".yml", ".py", ".js", ".ts", ".lock"];
    if !FILE_EXTS.iter().any(|ext| user_request.contains(ext)) {
        return user_request.to_string();
    }

    let prompt = format!(
        "Extract file names/paths explicitly mentioned in this user request. Return ONLY a JSON array of strings. If none, return [].\n\nUser request: {user_request}\nAvailable files: {files_str}\n\nReturn ONLY like: [\"src/main.rs\", \"Cargo.toml\"]"
    );

    let opts = qwen_ctx_opts();
    let Ok((response, _)) = ollama.chat(QWEN, &prompt, Some(&opts)) else {
        return user_request.to_string();
    };

    let files: Vec<String> = response.find('[')
        .and_then(|s| response[s..].find(']').map(|e| &response[s..s + e + 1]))
        .and_then(|slice| serde_json::from_str(slice).ok())
        .unwrap_or_default();

    if files.is_empty() {
        return user_request.to_string();
    }

    let mut file_contents = String::new();
    for filename in &files {
        let fpath = work_dir.join(filename);
        if fpath.exists() {
            if let Ok(content) = fs::read_to_string(&fpath) {
                file_contents.push_str(&format!("\n=== {filename} ===\n{content}\n"));
            }
        }
    }

    if file_contents.is_empty() {
        return user_request.to_string();
    }

    format!("用户需求: {user_request}\n\n相关文件内容:{file_contents}")
}

pub fn run_orchestrator(
    state: &Arc<Mutex<AppState>>,
    user_request: &str,
    context: &mut Vec<String>,
) -> Result<()> {
    let run_start = Instant::now();

    let (host, model, work_dir, edit_mode, output_lang) = {
        let st = state.lock().unwrap();
        (st.ollama_host.clone(), st.current_model.clone(), st.work_dir.clone(), st.edit_mode,
         if st.lang == "zh_TW" { "Traditional Chinese (繁體中文)".to_string() } else { "English".to_string() })
    };

    let ollama = OllamaClient::new(&host);
    let executor = Executor::new(work_dir.clone());
    let rules_mgr = RulesManager::new(work_dir.join(".sakichan.md"));
    let logger = Logger::new(
        work_dir.join(".sakichan").join("build.log"),
        work_dir.file_name().unwrap_or_default().to_string_lossy().to_string(),
    );
    let _ = logger.init();

    let rules = rules_mgr.load();
    let existing_files = list_files_in(&work_dir);
    let files_str = existing_files.join(", ");

    // ══════════════════════════════════════════════════════════════════
    // Phase 0: Context Gathering
    // ══════════════════════════════════════════════════════════════════
    let enriched_request = gather_context(&ollama, user_request, &files_str, &work_dir);

    // ══════════════════════════════════════════════════════════════════
    // Phase 1: Analysis
    // ══════════════════════════════════════════════════════════════════
    println!("{CYAN}🔍 分析需求中... / Analyzing...{RESET}");

    let prompt_a = format!(
        r#"## Environment
- OS: Windows 11
- Shell: PowerShell
- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)
- Path separator: \

You are an expert software engineer analyzing a user request.

IMPORTANT RULES:
1. If the user says any of these: "不要修改代码", "只输出分析", "只分析不修改", "analyze only", "no modifications", "do not modify", "只讀", "只读" — this is an ANALYSIS-ONLY request. You MUST set "code_modification": false.
2. If the user wants to CREATE, BUILD, WRITE code, or IMPLEMENT features, set "code_modification": true.
3. When code_modification is false, NO files should be changed.

Work directory: {}
Existing files: {}
Project rules:
{}

User request: {}


Available tools:
- [TOOL:read_file path="relative/path"]
- [TOOL:list_files path="dir"]

Based on ALL information gathered (user request, file contents, project rules, existing code),
identify what is GENUINELY unclear.

Only list clarifications that meet ALL criteria:
1. Answer significantly changes implementation direction
2. Cannot be inferred from existing context
3. Multiple reasonable approaches exist

Do NOT ask about things you can read from files.
Do NOT ask "what should I focus on" or "any specific requirements".
If everything is clear, return empty clarifications array.

Respond with ONLY valid JSON:
{{"understanding":"brief in {lang}","complexity":5,"code_modification":true,"gathered_info":[{{"label":"目标平台","value":"Windows","source":"环境推断"}}],"clarifications":[{{"question":"q","recommendation":"suggestion"}}]}}
Complexity is 1-10. All explanation text in {lang}."#,
        work_dir.display(), files_str, rules, enriched_request,
        lang = output_lang
    );

    let spinner = Spinner::new(SpinnerState::Thinking);
    let a_opts = dsr1_opts();
    let (analysis_raw, usage_a) = ollama.chat(DSR1, &prompt_a, Some(&a_opts))?;
    spinner.update_tokens(usage_a.input_tokens + usage_a.output_tokens);
    spinner.stop();
    record_usage(state, &usage_a);

    let (analysis_clean, tool_results_a) = process_tool_calls(&analysis_raw, &work_dir);
    let analysis_text = if tool_results_a.is_empty() { analysis_clean } else { format!("{analysis_clean}\n{tool_results_a}") };

    let analysis: AnalysisResponse = parse_json_from_response(&analysis_text)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();

    println!("{CYAN}📊 复杂度 / Complexity: {}/10{RESET}", analysis.complexity);
    if !analysis.understanding.is_empty() {
        println!("{GRAY}理解: {}{RESET}", analysis.understanding);
    }

    let mut current_model = model.clone();
    if analysis.complexity >= 7 {
        current_model = DSR1.to_string();
        println!("{YELLOW}复杂度高，切换模型到 {current_model}{RESET}");
        state.lock().unwrap().current_model = current_model.clone();
    }

    // ══════════════════════════════════════════════════════════════════
    // Phase 2: Clarification
    // ══════════════════════════════════════════════════════════════════
    if !analysis.gathered_info.is_empty() {
        println!("{CYAN}📋 已获取的信息:{RESET}");
        for info in &analysis.gathered_info {
            println!("  {GREEN}✓{RESET} {}: {} {GRAY}({}){RESET}", info.label, info.value, info.source);
        }
        println!();
    }

    const MAX_INLINE_QUESTIONS: usize = 2;
    let mut decisions = Vec::new();
    if !analysis.clarifications.is_empty() {
        let total = analysis.clarifications.len();
        for (i, c) in analysis.clarifications.iter().enumerate() {
            if i >= MAX_INLINE_QUESTIONS {
                let decision = c.recommendation.clone();
                decisions.push(format!("Q: {} → A: {} (auto)", c.question, decision));
                continue;
            }
            println!();
            print!("{YELLOW}❓ [{}/{}] {} {GRAY}[推荐: {}]{YELLOW}: {RESET}",
                i + 1, total.min(MAX_INLINE_QUESTIONS),
                c.question, c.recommendation);
            let _ = io::stdout().flush();
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim();
            let decision = if input.is_empty() { c.recommendation.clone() } else { input.to_string() };
            decisions.push(format!("Q: {} → A: {}", c.question, decision));
        }
    } else {
        println!("{GREEN}✅ 信息充分，无需澄清，直接进入规划{RESET}");
    }

    // 如果是纯分析模式，输出分析结果后直接返回
    if !analysis.code_modification {
        println!();
        println!("{CYAN}📝 纯分析模式 — 不修改任何文件{RESET}");
        println!("{GRAY}分析结果:{RESET}");
        println!("  {GRAY}理解: {}{RESET}", analysis.understanding);
        println!("  {GRAY}复杂度: {}/10{RESET}", analysis.complexity);
        if !decisions.is_empty() {
            println!("  {GRAY}决策:{RESET}");
            for d in &decisions {
                println!("    {GRAY}{d}{RESET}");
            }
        }
        println!();
        println!("{YELLOW}💡 提示: 若需要修改代码，请输入具体需求；若只需分析，以上即为结果。{RESET}");
        return Ok(());
    }

    // ══════════════════════════════════════════════════════════════════
    // Phase 3: Planning
    // ══════════════════════════════════════════════════════════════════
    println!();
    println!("{CYAN}📋 规划步骤中... / Planning...{RESET}");

    let decisions_str = if decisions.is_empty() { "None".to_string() } else { decisions.join("\n") };

    let prompt_p = format!(
        r#"## Environment
- OS: Windows 11
- Shell: PowerShell
- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)
- Path separator: \

You are planning implementation steps.

User request: {}
Understanding: {}
Decisions: {}
Rules: {}
Existing files: {}

IMPORTANT: If no Cargo.toml exists, the first step MUST create it.
Step names MUST start with a verb and describe the action concisely. Good: 'Implement Fibonacci function'. Bad: 'Fibonacci function'.

Respond with ONLY JSON (all explanation text in {}):
{{"steps":[{{"id":1,"name":"step","description":"detail","files_to_create":["path.rs"],"verification":"cargo check"}}]}}"#,
        user_request, analysis.understanding, decisions_str, rules, files_str, output_lang
    );

    let spinner = Spinner::new(SpinnerState::Thinking);
    let p_opts = dsr1_opts();
    let (plan_raw, usage_p) = ollama.chat(DSR1, &prompt_p, Some(&p_opts))?;
    spinner.update_tokens(usage_p.input_tokens + usage_p.output_tokens);
    spinner.stop();
    record_usage(state, &usage_p);

    let (plan_clean, _) = process_tool_calls(&plan_raw, &work_dir);

    let plan: PlanResponse = parse_json_from_response(&plan_clean)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or(PlanResponse {
            steps: vec![PlanStep {
                id: 1, name: "Generate code".to_string(),
                description: user_request.to_string(),
                files_to_create: vec!["Cargo.toml".to_string(), "src/main.rs".to_string()],
                verification: "cargo check".to_string(),
            }],
        });

    println!("{CYAN}步骤 / Steps:{RESET}");
    for step in &plan.steps {
        println!("  {GREEN}{}. {}{RESET}", step.id, step.name);
    }
    println!();

    // ══════════════════════════════════════════════════════════════════
    // Phase 4: Execute Steps
    // ══════════════════════════════════════════════════════════════════
    let mut all_completed_files: Vec<String> = Vec::new();
    let step_count = plan.steps.len();
    let mut any_step_failed = false;

    // 3a: Create sandbox before executing any steps
    let sandbox_path: Option<PathBuf> = if edit_mode {
        match create_sandbox(&work_dir) {
            Ok(path) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                println!("{CYAN}  ● Sandbox(create){RESET}");
                println!("  {GRAY}⎿  Created: .sakichan/sandboxes/{name}{RESET}");
                Some(path)
            }
            Err(e) => {
                println!("{YELLOW}⚠ 沙箱创建失败 / Sandbox creation failed: {e}{RESET}");
                None
            }
        }
    } else {
        None
    };

    for (step_idx, step) in plan.steps.iter().enumerate() {
        println!("{PINK}● {}/{}: {}{RESET}", step_idx + 1, step_count, step.name);

        if !edit_mode {
            println!("{YELLOW}⚠️  只读模式，跳过文件写入{RESET}");
            continue;
        }

        let checkpoint_info = git_checkpoint(&executor, &step.name);
        state.lock().unwrap().checkpoint_count += 1;
        println!("{CYAN}  ● Git(checkpoint){RESET}");
        println!("  {GRAY}⎿  {checkpoint_info}{RESET}");

        let _ = ensure_cargo_toml(&work_dir);
        let existing_code = gather_existing_code(&work_dir, &step.files_to_create);

        let base_prompt = format!(
            r#"## Environment
- OS: Windows 11
- Shell: PowerShell
- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)
- Path separator: \

Implement step {step_id} of a software project.

Project: {request}
Step: {name} - {desc}
Files: {files}
Verification: {verify}

Existing code:
{code}

Generate COMPLETE code. Format:
```rust filename="path/to/file.rs"
// full code
```
Write FULL compilable code. All explanation in {lang}."#,
            step_id = step.id, request = user_request, name = step.name,
            desc = step.description, files = step.files_to_create.join(", "),
            verify = step.verification, code = existing_code, lang = output_lang,
        );

        let mut compile_ok = false;
        let step_start = Instant::now();
        let mut step_model = current_model.clone();
        let mut prompt_c = base_prompt.clone();
    
        // ── Qwen (max 5) ──
        for attempt in 1..=QWEN_MAX_RETRIES {
            let spinner_state = if attempt == 1 { SpinnerState::Crafting } else { SpinnerState::Fixing };
            let spinner = Spinner::new(spinner_state);
            let exec_opts = if step_model == QWEN { qwen_gen_opts() } else { dsr1_opts() };
            let (code_raw, usage_c) = ollama.chat(&step_model, &prompt_c, Some(&exec_opts))?;
            spinner.update_tokens(usage_c.input_tokens + usage_c.output_tokens);
            spinner.stop();
            record_usage(state, &usage_c);
    
            let (code_clean, tool_results) = process_tool_calls(&code_raw, &work_dir);
            if !tool_results.is_empty() { prompt_c.push_str(&format!("\n\n{tool_results}")); }
    
            let blocks = extract_code_blocks(&code_clean);
            if blocks.is_empty() {
                if attempt < QWEN_MAX_RETRIES {
                    prompt_c.push_str("\n\nERROR: No code blocks. Use ```rust filename=\"path\" format.");
                    continue;
                }
                break;
            }
    
            for (_lang, filename, code) in &blocks {
                if filename.is_empty() { continue; }
                let fpath = work_dir.join(filename);
                let old_content = fs::read_to_string(&fpath).unwrap_or_default();
                if let Some(p) = fpath.parent() { let _ = fs::create_dir_all(p); }
                let _ = fs::write(&fpath, code);
                print_code_diff(filename, &old_content, code);
                all_completed_files.push(filename.clone());
            }

            let (ok, output, _dur) = executor.run("cargo check 2>&1")?;

            if ok {
                print_bash_result("cargo check 2>&1", &output, 5);
                println!("{GREEN}  ✓ 编译通过 (qwen, attempt {attempt}){RESET}");
                compile_ok = true;
                break;
            } else {
                let err: String = output.chars().take(1500).collect();
                prompt_c = format!("## Environment\n- OS: Windows 11\n- Shell: PowerShell\n- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)\n- Path separator: \\\n\nCOMPILE ERROR:\n```\n{}\n```\n\nFix ALL files. ```rust filename=\"path\" format. All text in {}.\n\n{}", err, output_lang, base_prompt);
            }
        }

        // ── DSR1 fallback (max 10) ──
        if !compile_ok {
            step_model = DSR1.to_string();
            println!("{RED}🔄 qwen 失败，切换到 deepseek-r1 (最多 10 次)...{RESET}");
            prompt_c = format!("## Environment\n- OS: Windows 11\n- Shell: PowerShell\n- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)\n- Path separator: \\\n\nURGENT: Fix ALL compile errors. All text in {}.\n\n{}", output_lang, base_prompt);

            for attempt in 1..=DSR1_MAX_RETRIES {
                let spinner = Spinner::new(SpinnerState::FixingDsr1);
                let fb_opts = dsr1_opts();
                let (code_raw, usage_c) = ollama.chat(&step_model, &prompt_c, Some(&fb_opts))?;
                spinner.update_tokens(usage_c.input_tokens + usage_c.output_tokens);
                spinner.stop();
                record_usage(state, &usage_c);

                let (code_clean, tool_results) = process_tool_calls(&code_raw, &work_dir);
                if !tool_results.is_empty() { prompt_c.push_str(&format!("\n\n{tool_results}")); }

                let blocks = extract_code_blocks(&code_clean);
                if blocks.is_empty() {
                    if attempt < DSR1_MAX_RETRIES { continue; }
                    break;
                }

                for (_lang, filename, code) in &blocks {
                    if filename.is_empty() { continue; }
                    let fpath = work_dir.join(filename);
                    let old_content = fs::read_to_string(&fpath).unwrap_or_default();
                    if let Some(p) = fpath.parent() { let _ = fs::create_dir_all(p); }
                    let _ = fs::write(&fpath, code);
                    print_code_diff(filename, &old_content, code);
                    all_completed_files.push(filename.clone());
                }

                let (ok, output, _dur) = executor.run("cargo check 2>&1")?;

                if ok {
                    print_bash_result("cargo check 2>&1", &output, 5);
                    println!("{GREEN}  ✓ 编译通过 (dsr1, attempt {attempt}){RESET}");
                    compile_ok = true;
                    break;
                } else {
                    let err: String = output.chars().take(1500).collect();
                    prompt_c = format!("## Environment\n- OS: Windows 11\n- Shell: PowerShell\n- Commands use PowerShell syntax (e.g., dir not ls, ; not &&)\n- Path separator: \\\n\nCOMPILE ERROR:\n```\n{}\n```\n\nFix ALL files. ```rust filename=\"path\" format. All text in {}.\n\n{}", err, output_lang, base_prompt);
                }
            }
        }
    
        let step_duration = step_start.elapsed().as_secs_f64();
        let _ = logger.log_task(&step.name, &step.description, &all_completed_files, compile_ok, &[], &step_model, step_duration);
    
        if !compile_ok {
            any_step_failed = true;
            println!("{RED}❌ 编译失败 after {} attempts{RESET}", QWEN_MAX_RETRIES + DSR1_MAX_RETRIES);
            // 3b: Rollback this step's changes via git
            if let Ok((reverted, _, _)) = executor.run("git checkout -- .") {
                if reverted {
                    println!("{RED}  ● Sandbox(rollback){RESET}");
                    println!("  {GRAY}⎿  Rolling back step {} after {} failed attempts{RESET}",
                        step.id, QWEN_MAX_RETRIES + DSR1_MAX_RETRIES);
                }
            }
        }
    }
    
    // ══════════════════════════════════════════════════════════════════
    // Phase 5: Final Build
    // ══════════════════════════════════════════════════════════════════
    if edit_mode && !all_completed_files.is_empty() {
        println!();
        println!("{CYAN}🏗️  最终构建 / Final build...{RESET}");
        let (ok, output, dur) = executor.run("cargo build --release 2>&1")?;
        print_cmd_result("cargo build --release", ok, &output, dur);
        if ok {
            println!("{GREEN}🎉 构建完成！/ Build complete!{RESET}");
        } else {
            println!("{RED}构建失败，请检查错误 / Build failed{RESET}");
        }
    
        let structure = list_files_in(&work_dir).join("\n");
        let _ = rules_mgr.update(&all_completed_files, &structure);
        println!("{GRAY}📄 规则文件已更新 / Rules updated{RESET}");
        println!("{GRAY}📝 日志已更新 / Log updated{RESET}");

        if ok {
            print_change_summary(&all_completed_files);
        }

        let elapsed = run_start.elapsed().as_secs_f64();
        let elapsed_str = if elapsed >= 60.0 {
            format!("{}m {:.0}s", elapsed as u64 / 60, elapsed % 60.0)
        } else {
            format!("{:.1}s", elapsed)
        };
        println!("{PINK}✻ Baked for {elapsed_str}{RESET}");
    }

    // 3c: Sandbox cleanup after Phase 5
    if let Some(ref sandbox) = sandbox_path {
        if any_step_failed {
            println!("{YELLOW}⚠ 沙箱保留（有步骤失败），使用 /undo 可回滚 / Sandbox kept — use /undo to rollback{RESET}");
        } else {
            let _ = fs::remove_dir_all(sandbox);
            println!("{GRAY}🧹 沙箱已清理 / Sandbox cleaned up{RESET}");
        }
    }

    context.push(format!("User: {user_request}"));
    context.push(format!("Assistant: Completed {} steps", plan.steps.len()));
    
    let session_id = uuid::Uuid::new_v4().to_string();
    let _ = state.lock().unwrap().save_session(&session_id, context);
    
    Ok(())
}
