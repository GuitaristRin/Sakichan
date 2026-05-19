mod commands;
mod display;
mod executor;
mod logger;
mod ollama;
mod orchestrator;
mod rules;
mod state;

use commands::{get_i18n, handle_command, t};
use display::*;
use ollama::OllamaClient;
use orchestrator::run_orchestrator;
use rules::RulesManager;
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

// Fix 8: Slash command completer for rustyline.
struct CmdHelper {
    commands: Vec<String>,
}

impl CmdHelper {
    fn new() -> Self {
        CmdHelper {
            commands: vec![
                "/help", "/models", "/model", "/clear", "/init", "/load",
                "/usage", "/sessions", "/resume", "/export", "/edit",
                "/lang", "/exit", "/undo", "/history", "/diff",
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

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        if line.starts_with('/') {
            let word = &line[..pos];
            let candidates: Vec<Pair> = self
                .commands
                .iter()
                .filter(|c| c.starts_with(word))
                .map(|c| Pair {
                    display: c.clone(),
                    replacement: c.clone(),
                })
                .collect();
            Ok((0, candidates))
        } else {
            Ok((pos, vec![]))
        }
    }
}

impl Hinter for CmdHelper {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for CmdHelper {}

impl Validator for CmdHelper {
    fn validate(&self, _ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(ValidationResult::Valid(None))
    }
}

fn main() -> Result<()> {
    let work_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // 1. Check Ollama connection
    let ollama = OllamaClient::new("localhost:11434");
    print!("{CYAN}检查 Ollama 连接 / Checking Ollama...{RESET} ");
    let _ = std::io::Write::flush(&mut std::io::stdout());

    match ollama.check_connection() {
        Ok(_) => println!("{GREEN}✓ 已连接 / Connected{RESET}"),
        Err(e) => {
            println!("{RED}✗ 连接失败 / Connection failed: {e}{RESET}");
            println!(
                "{YELLOW}请确保 Ollama 在 localhost:11434 运行 / Ensure Ollama is running{RESET}"
            );
            println!("{GRAY}继续以离线模式运行 / Continuing in offline mode...{RESET}");
        }
    }

    // 2. Get model list
    let models = ollama.list_models().unwrap_or_default();

    // 3. Load/create .sakichan.md
    let rules_mgr = RulesManager::new(work_dir.join(".sakichan.md"));
    let _ = rules_mgr.init();

    // 4. Initialize state
    let state = Arc::new(Mutex::new(AppState::new(work_dir.clone())));

    // 5. Print welcome
    print_welcome("0.2.1", &models);

    // 5b. Check git status
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
        println!("{GREEN}● Git(status){RESET} {GRAY}branch: {} · {dirty}{RESET}", branch.trim());
    } else {
        println!("{YELLOW}⚠ 当前目录不是 Git 仓库 — 运行 'git init' 以启用回滚功能 / Not a git repo — run 'git init' to enable rollback{RESET}");
    }
    println!();

    // 6. REPL loop with slash command autocomplete (Fix 8)
    let i18n = get_i18n();
    let mut context: Vec<String> = Vec::new();

    let config = Config::builder()
        .completion_type(CompletionType::List)
        .build();
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

        let readline = rl.readline(&prompt);
        match readline {
            Ok(line) => {
                let input = line.trim().to_string();
                if input.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(&input);

                if input.starts_with('/') {
                    match handle_command(&input, &state, &mut context, &i18n, &models) {
                        Ok(true) => break,
                        Ok(false) => {}
                        Err(e) => println!("{RED}命令错误 / Command error: {e}{RESET}"),
                    }
                } else {
                    match run_orchestrator(&state, &input, &mut context) {
                        Ok(_) => {}
                        Err(e) => println!("{RED}错误 / Error: {e}{RESET}"),
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                let lang = state.lock().unwrap().lang.clone();
                println!("{PINK}{}{RESET}", t(&i18n, &lang, "goodbye"));
                break;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(e) => {
                println!("{RED}读取错误 / Readline error: {e}{RESET}");
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
