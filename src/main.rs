mod backend;
mod commands;
mod config;
mod display;
mod executor;
mod handoff;
mod logger;
mod orchestrator;
mod rules;
mod slots;
mod state;

use backend::ollama::OllamaBackend;
use commands::{get_i18n, handle_command, t};
use config::{load_config, ToolchainEntry};
use display::*;
use orchestrator::run_orchestrator;
use rules::RulesManager;
use slots::{build_slot_assignments, probe_ollama_models};
use state::{AppState, DetectedTool};

use anyhow::Result;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{CompletionType, Config, Context, Editor, Helper};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct CmdHelper {
    commands: Vec<String>,
}

impl CmdHelper {
    fn new() -> Self {
        CmdHelper {
            commands: vec![
                "/help", "/models", "/slots", "/slot", "/config",
                "/clear", "/init", "/load", "/usage", "/sessions",
                "/resume", "/export", "/edit", "/lang", "/exit",
                "/undo", "/history", "/diff",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }
}

impl Helper for CmdHelper {}

impl Completer for CmdHelper {
    type Candidate = Pair;
    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        if line.starts_with('/') {
            let word = &line[..pos];
            let candidates: Vec<Pair> = self.commands.iter()
                .filter(|c| c.starts_with(word))
                .map(|c| Pair { display: c.clone(), replacement: c.clone() })
                .collect();
            Ok((0, candidates))
        } else {
            Ok((pos, vec![]))
        }
    }
}

impl Hinter for CmdHelper {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> { None }
}

impl Highlighter for CmdHelper {}

impl Validator for CmdHelper {
    fn validate(&self, _ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(ValidationResult::Valid(None))
    }
}

const TOOLCHAIN_REGISTRY: &[(&str, &str, &str, &str)] = &[
    // (display_name, executable, version_flag, description)
    ("cargo",  "cargo",   "--version", "Rust 包管理器和编译器"),
    ("python3","python3", "--version", "Python 3 解释器"),
    ("python", "python",  "--version", "Python 解释器"),
    ("node",   "node",    "--version", "Node.js 运行时"),
    ("npm",    "npm",     "--version", "npm 包管理器"),
    ("npx",    "npx",     "--version", "npx 命令执行器"),
    ("tsc",    "tsc",     "--version", "TypeScript 编译器"),
    ("go",     "go",      "version",   "Go 编译器"),
    ("zig",    "zig",     "version",   "Zig 编译器"),
    ("java",   "java",    "--version", "Java 运行时"),
    ("gcc",    "gcc",     "--version", "GCC C/C++ 编译器"),
    ("clang",  "clang",   "--version", "Clang 编译器"),
    ("make",   "make",    "--version", "GNU Make"),
    ("cmake",  "cmake",   "--version", "CMake 构建系统"),
    ("git",    "git",     "--version", "版本控制系统"),
    ("docker", "docker",  "--version", "容器运行时"),
];

fn detect_toolchain(user_entries: &[ToolchainEntry]) -> Vec<DetectedTool> {
    let mut detected: Vec<DetectedTool> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for &(name, cmd, flag, description) in TOOLCHAIN_REGISTRY {
        let output = std::process::Command::new(cmd)
            .arg(flag)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let raw_ver = if stdout.trim().is_empty() { stderr.trim().to_string() } else { stdout.trim().to_string() };
            let version = raw_ver.lines().next().unwrap_or("").trim().to_string();
            detected.push(DetectedTool { name: name.to_string(), version, description: description.to_string() });
            seen.insert(name.to_string());
        }
    }

    for entry in user_entries {
        if seen.contains(&entry.name) { continue; }
        let parts: Vec<&str> = entry.check_command.split_whitespace().collect();
        let Some(&cmd) = parts.first() else { continue };
        let out = std::process::Command::new(cmd)
            .args(&parts[1..])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output();
        if let Ok(o) = out {
            if o.status.success() {
                let version = String::from_utf8_lossy(&o.stdout)
                    .lines().next().unwrap_or("").trim().to_string();
                detected.push(DetectedTool { name: entry.name.clone(), version, description: entry.description.clone() });
            }
        }
    }

    detected
}

fn main() -> Result<()> {
    let work_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // 1. Load config
    let cfg = load_config();
    let ollama_host = cfg.backend.ollama.host.clone();

    // 2. Check backend connection
    print!("{CYAN}检查后端连接 / Checking backend ({})...{RESET} ", cfg.backend.backend_type);
    let _ = std::io::Write::flush(&mut std::io::stdout());

    let available_models = match OllamaBackend::new(&cfg.backend.ollama) {
        Ok(b) => match b.check_connection() {
            Ok(_) => {
                println!("{GREEN}✓ 已连接 / Connected{RESET}");
                probe_ollama_models(&ollama_host)
            }
            Err(e) => {
                println!("{RED}✗ 连接失败: {e}{RESET}");
                println!("{YELLOW}请确保 Ollama 在 {} 运行{RESET}", ollama_host);
                println!("{GRAY}继续以离线模式运行...{RESET}");
                std::collections::HashSet::new()
            }
        },
        Err(e) => {
            println!("{RED}✗ 后端初始化失败: {e}{RESET}");
            std::collections::HashSet::new()
        }
    };

    let models: Vec<String> = available_models.iter().cloned().collect();

    // 3. Build slot assignments
    let slot_assignments = build_slot_assignments(&cfg, &available_models);

    // 4. Detect toolchain
    print!("{CYAN}检测工具链 / Detecting toolchain...{RESET} ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let toolchain = detect_toolchain(&cfg.toolchain);
    println!("{GREEN}找到 {} 个工具{RESET}", toolchain.len());

    // 5. Initialize state
    let mut initial_state = AppState::new(work_dir.clone(), cfg);
    initial_state.slot_assignments = slot_assignments;
    initial_state.toolchain_info = toolchain;
    let state = Arc::new(Mutex::new(initial_state));

    // 6. Init .sakichan.md
    let rules_mgr = RulesManager::new(work_dir.join(".sakichan.md"));
    let _ = rules_mgr.init();

    // 7. Print welcome
    let mut sorted_models = models.clone();
    sorted_models.sort();
    print_welcome("0.4.0", &sorted_models);

    // 8. Git status
    let git_ok = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&work_dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if git_ok {
        let branch = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&work_dir)
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let dirty = std::process::Command::new("git")
            .args(["status", "--short"])
            .current_dir(&work_dir)
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| if s.trim().is_empty() { "clean" } else { "dirty" }.to_string())
            .unwrap_or_default();
        println!("{GREEN}● Git{RESET} {GRAY}branch: {} · {dirty}{RESET}", branch.trim());
    } else {
        println!("{YELLOW}⚠ 当前目录不是 Git 仓库 — 运行 'git init' 以启用回滚功能{RESET}");
    }
    println!();

    // 9. REPL
    let i18n = get_i18n();
    let mut context: Vec<String> = Vec::new();

    let config = Config::builder().completion_type(CompletionType::List).build();
    let mut rl = Editor::<CmdHelper, DefaultHistory>::with_config(config)?;
    rl.set_helper(Some(CmdHelper::new()));

    let history_file = work_dir.join(".sakichan").join("history.txt");
    let _ = rl.load_history(&history_file);

    loop {
        let prompt = {
            let st = state.lock().unwrap();
            let lang = st.lang.clone();
            if st.edit_mode {
                t(&i18n, &lang, "prompt_edit").to_string()
            } else {
                t(&i18n, &lang, "prompt_readonly").to_string()
            }
        };

        match rl.readline(&prompt) {
            Ok(line) => {
                let input = line.trim().to_string();
                if input.is_empty() { continue; }
                let _ = rl.add_history_entry(&input);

                if input.starts_with('/') {
                    match handle_command(&input, &state, &mut context, &i18n, &models) {
                        Ok(true) => break,
                        Ok(false) => {}
                        Err(e) => println!("{RED}命令错误: {e}{RESET}"),
                    }
                } else {
                    match run_orchestrator(&state, &input, &mut context) {
                        Ok(_) => {}
                        Err(e) => println!("{RED}错误: {e}{RESET}"),
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                let lang = state.lock().unwrap().lang.clone();
                println!("{PINK}{}{RESET}", t(&i18n, &lang, "goodbye"));
                break;
            }
            Err(ReadlineError::Eof) => break,
            Err(e) => {
                println!("{RED}读取错误: {e}{RESET}");
                break;
            }
        }
    }

    if let Some(parent) = history_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = rl.save_history(&history_file);

    Ok(())
}
