use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const PINK: &str = "\x1b[38;5;213m";
pub const CYAN: &str = "\x1b[36m";
pub const YELLOW: &str = "\x1b[33m";
pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const GRAY: &str = "\x1b[90m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const ORANGE: &str = "\x1b[38;5;208m";

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn print_welcome(version: &str, models: &[String]) {
    println!();
    println!("{PINK}{BOLD}╭──────────────────────────────────────────╮{RESET}");
    println!("{PINK}{BOLD}│   🌸  Saki-chan AI Assistant  v{version:<13}│{RESET}");
    println!("{PINK}{BOLD}╰──────────────────────────────────────────╯{RESET}");
    println!();
    if !models.is_empty() {
        println!("{CYAN}可用模型 / Available Models:{RESET}");
        for m in models {
            println!("  {GRAY}•{RESET} {m}");
        }
    }
    println!();
    println!("{DIM}输入需求开始，/help 查看命令 | Type your request, /help for commands{RESET}");
    println!();
}

pub fn print_divider() {
    println!("{GRAY}{}{RESET}", "─".repeat(50));
}

pub fn print_cmd_result(cmd: &str, success: bool, output: &str, duration: f64) {
    let icon = if success {
        format!("{GREEN}✓{RESET}")
    } else {
        format!("{RED}✗{RESET}")
    };
    println!("{icon} {BOLD}{cmd}{RESET} {GRAY}({duration:.1}s){RESET}");
    if !output.trim().is_empty() {
        for line in output.lines().take(50) {
            println!("  {GRAY}{line}{RESET}");
        }
    }
}

pub fn print_code_diff(filename: &str, old_lines: &[&str], new_lines: &[&str]) {
    println!("{CYAN}📄 {filename}{RESET}");
    if old_lines.is_empty() {
        println!("  {GREEN}(new file){RESET}");
        return;
    }
    let max_show = 20usize;
    let mut shown = 0;
    for line in old_lines {
        if shown >= max_show { break; }
        println!("  {RED}-{RESET} {DIM}{line}{RESET}");
        shown += 1;
    }
    shown = 0;
    for line in new_lines {
        if shown >= max_show { break; }
        println!("  {GREEN}+{RESET} {line}");
        shown += 1;
    }
}

pub enum SpinnerState {
    Thinking,
    ThinkingMore,
    AlmostFinished,
    Crafting,
    Fixing,
    FixingDsr1,
    Mistaking,
}

impl SpinnerState {
    fn label(&self) -> &str {
        match self {
            SpinnerState::Thinking => "Thinking...",
            SpinnerState::ThinkingMore => "Thinking more...",
            SpinnerState::AlmostFinished => "Almost finished...",
            SpinnerState::Crafting => "Crafting...",
            SpinnerState::Fixing => "Fixing...",
            SpinnerState::FixingDsr1 => "Deliberating...",
            SpinnerState::Mistaking => "Mistaking...",
        }
    }

    fn color(&self) -> &str {
        match self {
            SpinnerState::Thinking
            | SpinnerState::ThinkingMore
            | SpinnerState::AlmostFinished => ORANGE,
            SpinnerState::Crafting => GREEN,
            SpinnerState::Fixing => RED,
            SpinnerState::FixingDsr1 => RED,
            SpinnerState::Mistaking => GRAY,
        }
    }
}

pub struct Spinner {
    stop: Arc<Mutex<bool>>,
    token_count: Arc<Mutex<u64>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Spinner {
    pub fn new(state: SpinnerState) -> Self {
        let stop = Arc::new(Mutex::new(false));
        let token_count = Arc::new(Mutex::new(0u64));
        let stop_clone = Arc::clone(&stop);
        let token_clone = Arc::clone(&token_count);
        let label = state.label().to_string();
        let color = state.color().to_string();

        let handle = thread::spawn(move || {
            let start = Instant::now();
            let mut i = 0usize;
            loop {
                {
                    let s = stop_clone.lock().unwrap();
                    if *s { break; }
                }
                let elapsed = start.elapsed().as_secs_f64();
                let elapsed_str = if elapsed >= 60.0 {
                    format!("{}m {:.1}s", elapsed as u64 / 60, elapsed % 60.0)
                } else {
                    format!("{:.1}s", elapsed)
                };
                let tokens = token_clone.lock().unwrap();
                let frame = SPINNER_FRAMES[i % SPINNER_FRAMES.len()];
                print!("\r  {color}🟠 {frame} {label}{RESET} {GRAY}({elapsed_str} · {} tokens){RESET}   ", *tokens);
                let _ = std::io::Write::flush(&mut std::io::stdout());
                i += 1;
                thread::sleep(Duration::from_millis(80));
            }
            print!("\r{}\r", " ".repeat(80));
            let _ = std::io::Write::flush(&mut std::io::stdout());
        });

        Spinner { stop, token_count, handle: Some(handle) }
    }

    pub fn update_tokens(&self, count: u64) {
        if let Ok(mut t) = self.token_count.lock() {
            *t = count;
        }
    }

    pub fn stop(mut self) {
        {
            let mut s = self.stop.lock().unwrap();
            *s = true;
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}