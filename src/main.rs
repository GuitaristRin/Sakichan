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
use config::load_config;
use display::*;
use orchestrator::run_orchestrator;
use rules::RulesManager;
use slots::{build_slot_assignments, probe_ollama_models};
use state::AppState;

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

    // 4. Initialize state
    let mut initial_state = AppState::new(work_dir.clone(), cfg);
    initial_state.slot_assignments = slot_assignments;
    let state = Arc::new(Mutex::new(initial_state));

    // 5. Init .sakichan.md
    let rules_mgr = RulesManager::new(work_dir.join(".sakichan.md"));
    let _ = rules_mgr.init();

    // 6. Print welcome
    let mut sorted_models = models.clone();
    sorted_models.sort();
    print_welcome("0.4.0", &sorted_models);

    // 7. Git status
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

    // 8. REPL
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
