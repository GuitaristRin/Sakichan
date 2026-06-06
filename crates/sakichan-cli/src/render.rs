use colored::Colorize;
use sakichan_core::models::FinalResult;

/// A streaming output handler.
/// On first token: prints prefix once in green, then each token appended inline in green.
/// On finish: prints a newline. The whole line remains green (terminal limitation).
pub struct StreamingLine {
    prefix: String,
    started: bool,
}

impl StreamingLine {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            started: false,
        }
    }

    /// Append a token and print it inline in green.
    /// Prefix is only printed once on the first call.
    pub fn stream(&mut self, token: &str) {
        if !self.started {
            self.started = true;
            print!("{} ", self.prefix.green());
        }
        print!("{}", token.green());
        std::io::Write::flush(&mut std::io::stdout()).ok();
    }

    /// Finish the line by printing a newline.
    pub fn finish(&self) {
        println!();
    }

    /// Check if any content has been streamed.
    pub fn is_empty(&self) -> bool {
        !self.started
    }
}

/// Print an info message with [INFO] prefix.
pub fn print_info(msg: &str) {
    println!("{}", format!("[INFO] {}", msg).dimmed());
}

/// Print a model selection message with [MODEL] prefix.
pub fn print_model(msg: &str) {
    println!("{}", format!("[MODEL] {}", msg).dimmed());
}

/// Print the thinking header before model invocation.
pub fn print_thinking_header(_model_id: &str) {
    print_info("Calling LLM");
}

/// Print token usage information.
pub fn print_usage(result: &FinalResult) {
    if let Some(ref usage) = result.usage {
        println!(
            "{}",
            format!(
                "[INFO] Tokens: {} 输入 / {} 输出 / {} 总计",
                usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
            )
            .dimmed()
        );
    }
}

/// Print a session list entry.
pub fn print_session_entry(id: &str, title: Option<&str>, time: &str) {
    let title_display = title.unwrap_or("[无标题]");
    println!(
        "{}  {}  {}  {}",
        "[INFO]".dimmed(),
        id.dimmed(),
        time.dimmed(),
        title_display
    );
}