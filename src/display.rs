use std::collections::HashSet;
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

pub fn format_num(n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

fn fmt_elapsed(secs: u64) -> String {
    if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

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

pub fn print_bash_result(cmd: &str, output: &str, max_lines: usize) {
    println!("{CYAN}  ● Bash({cmd}){RESET}");
    let non_empty: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let total = non_empty.len();
    for line in non_empty.iter().take(max_lines) {
        println!("    {GRAY}⎿  {line}{RESET}");
    }
    if total > max_lines {
        println!("    {GRAY}… +{} lines{RESET}", total - max_lines);
    }
}

pub fn print_code_diff(filename: &str, old_content: &str, new_content: &str) -> (usize, usize) {
    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();
    let old_set: HashSet<&str> = old_lines.iter().copied().collect();
    let new_set: HashSet<&str> = new_lines.iter().copied().collect();

    let removed: Vec<&str> = old_lines.iter().filter(|&&l| !new_set.contains(l)).copied().collect();
    let added: Vec<&str> = new_lines.iter().filter(|&&l| !old_set.contains(l)).copied().collect();
    let added_count = added.len();
    let removed_count = removed.len();

    if old_content.is_empty() {
        println!("{GREEN}  ● Create({filename}){RESET}");
    } else {
        println!("{CYAN}  ● Update({filename}){RESET}");
    }

    let max_show = 10usize;
    let mut shown = 0usize;
    for line in removed.iter() {
        if shown >= max_show { break; }
        println!("    {RED}-{RESET} {DIM}{}{RESET}", line);
        shown += 1;
    }
    for line in added.iter() {
        if shown >= max_show { break; }
        println!("    {GREEN}+{RESET} {}", line);
        shown += 1;
    }
    println!("  {GRAY}⎿  Added {added_count} lines, removed {removed_count} lines{RESET}");
    (added_count, removed_count)
}

pub fn print_change_summary(files: &[String]) {
    let mut seen = HashSet::new();
    let unique: Vec<&str> = files.iter()
        .filter(|f| seen.insert(f.as_str()))
        .map(|f| f.as_str())
        .collect();
    println!("{GRAY}---{RESET}");
    println!("  {BOLD}改动总结 / Change Summary{RESET}");
    println!("  共修改 {} 个文件 / {} files modified:", unique.len(), unique.len());
    for f in &unique {
        println!("    {GRAY}{f}{RESET}");
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
    Reviewing,
    Architecting,
    Evaluating,
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
            SpinnerState::Reviewing => "Reviewing...",
            SpinnerState::Architecting => "Architecting...",
            SpinnerState::Evaluating => "Evaluating...",
        }
    }

    fn color(&self) -> &str {
        match self {
            SpinnerState::Thinking
            | SpinnerState::ThinkingMore
            | SpinnerState::AlmostFinished
            | SpinnerState::Architecting
            | SpinnerState::Evaluating => ORANGE,
            SpinnerState::Crafting => GREEN,
            SpinnerState::Fixing | SpinnerState::FixingDsr1 => RED,
            SpinnerState::Mistaking => GRAY,
            SpinnerState::Reviewing => CYAN,
        }
    }
}

// Fix 7: Spinner now takes shared global token counter and start time.
// Timer and token count never reset between phases.
pub struct Spinner {
    stop: Arc<Mutex<bool>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Spinner {
    pub fn new(
        state: SpinnerState,
        global_tokens: Arc<Mutex<u64>>,
        global_start: Arc<Instant>,
    ) -> Self {
        let stop = Arc::new(Mutex::new(false));
        let stop_clone = Arc::clone(&stop);
        let label = state.label().to_string();
        let color = state.color().to_string();

        let handle = thread::spawn(move || {
            let mut i = 0usize;
            loop {
                if *stop_clone.lock().unwrap() { break; }
                let elapsed = global_start.elapsed().as_secs();
                let tokens = *global_tokens.lock().unwrap();
                let frame = SPINNER_FRAMES[i % SPINNER_FRAMES.len()];
                print!(
                    "\r  {color}🟠 {frame} {label}{RESET} {GRAY}({} · {} tokens){RESET}   ",
                    fmt_elapsed(elapsed),
                    format_num(tokens),
                );
                let _ = std::io::Write::flush(&mut std::io::stdout());
                i += 1;
                thread::sleep(Duration::from_millis(80));
            }
            print!("\r{}\r", " ".repeat(80));
            let _ = std::io::Write::flush(&mut std::io::stdout());
        });

        Spinner { stop, handle: Some(handle) }
    }

    pub fn stop(mut self) {
        *self.stop.lock().unwrap() = true;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
