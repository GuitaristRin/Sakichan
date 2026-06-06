use std::collections::VecDeque;

use crate::models::Message;

/// Maximum context length for models (256K tokens as spec).
/// We keep a generous character budget as a proxy for tokens.
const MAX_CONTEXT_TOKENS: usize = 256_000;
/// Reserve tokens for the response output.
const RESPONSE_RESERVE: usize = 4_000;
/// Approximate chars per token for truncation calculation.
const CHARS_PER_TOKEN: usize = 4;

/// Manages the sliding window context for a single session.
pub struct SessionContext {
    session_id: String,
    model_id: String,
    system_prompt: Option<String>,
    /// The working message buffer (sliding window).
    messages: VecDeque<Message>,
    /// Maximum tokens the model supports.
    max_context_tokens: usize,
    /// Whether a system message was injected for memory.
    has_memory_injection: bool,
    /// Current branch depth.
    current_depth: i64,
    /// Current branch order at the current depth.
    current_order: i64,
    /// Whether the next add_user_message should use --deri (branch) mode.
    branch_mode: Option<(i64, i64)>,
}

impl SessionContext {
    pub fn new(session_id: String, model_id: String) -> Self {
        Self {
            session_id,
            model_id,
            system_prompt: None,
            messages: VecDeque::new(),
            max_context_tokens: MAX_CONTEXT_TOKENS,
            has_memory_injection: false,
            current_depth: 0,
            current_order: 0,
            branch_mode: None,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn set_system_prompt(&mut self, prompt: Option<String>) {
        self.system_prompt = prompt;
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    pub fn set_max_context_tokens(&mut self, tokens: usize) {
        self.max_context_tokens = tokens;
    }

    /// Set the current depth and order for the next message.
    pub fn set_depth_order(&mut self, depth: i64, order: i64) {
        self.current_depth = depth;
        self.current_order = order;
    }

    /// Get the current depth.
    pub fn current_depth(&self) -> i64 {
        self.current_depth
    }

    /// Get the current order.
    pub fn current_order(&self) -> i64 {
        self.current_order
    }

    /// Set branch mode: next add_user_message will use the given (depth, order).
    pub fn set_branch_mode(&mut self, depth: i64, order: i64) {
        self.branch_mode = Some((depth, order));
    }

    /// Clear branch mode.
    pub fn clear_branch_mode(&mut self) {
        self.branch_mode = None;
    }

    /// Add a user message to the context.
    pub fn add_user_message(&mut self, content: &str) {
        self.has_memory_injection = false;
        // If branch_mode is set, use the specified depth/order
        if let Some((depth, order)) = self.branch_mode.take() {
            self.current_depth = depth;
            self.current_order = order;
        } else {
            // Normal continuation: increment depth, order = 0
            self.current_depth += 1;
            self.current_order = 0;
        }
        self.messages.push_back(Message::user(content));
    }

    /// Add a user message with attached images to the context.
    /// `images` should be base64 data URIs.
    pub fn add_user_message_with_images(&mut self, content: &str, images: Vec<String>) {
        self.has_memory_injection = false;
        if let Some((depth, order)) = self.branch_mode.take() {
            self.current_depth = depth;
            self.current_order = order;
        } else {
            self.current_depth += 1;
            self.current_order = 0;
        }
        self.messages.push_back(Message::user_with_images(content, images));
    }

    /// Add an assistant message to the context.
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push_back(Message::assistant(content));
    }

    /// Pop the last assistant message (for regeneration).
    pub fn pop_last_assistant(&mut self) -> Option<Message> {
        // Find and remove the last assistant message
        let pos = self.messages.iter().rposition(|m| m.role == "assistant")?;
        self.messages.remove(pos)
    }

    /// Pop the last user message (for editing).
    pub fn pop_last_user(&mut self) -> Option<Message> {
        let pos = self.messages.iter().rposition(|m| m.role == "user")?;
        self.messages.remove(pos)
    }

    /// Inject a memory system message at the end, right before the user's latest message.
    pub fn inject_memory_prompt(&mut self, memory_text: &str) {
        // Remove any previous memory injection
        self.messages
            .retain(|m| !(m.role == "system" && m.name.as_deref() == Some("memory_context")));

        let msg = Message {
            role: "system".to_string(),
            content: format!(
                "[长期记忆参考]\n以下是关于用户的长期记忆，请在回答时参考:\n{}",
                memory_text
            ),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: Some("memory_context".to_string()),
            images: Vec::new(),
        };
        self.messages.push_back(msg);
        self.has_memory_injection = true;
    }

    /// Inject non-memory context (e.g. image descriptions) that should not be
    /// labeled as long-term memory. Removes any previous context injection.
    pub fn inject_context_prompt(&mut self, context_text: &str) {
        // Remove any previous context injection
        self.messages
            .retain(|m| !(m.role == "system" && m.name.as_deref() == Some("injected_context")));

        let msg = Message {
            role: "system".to_string(),
            content: context_text.to_string(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: Some("injected_context".to_string()),
            images: Vec::new(),
        };
        self.messages.push_back(msg);
    }

    /// Build the message list for sending to the model.
    /// Applies the sliding window to fit within context limits.
    pub fn build_messages_for_model(&self) -> Vec<Message> {
        let max_chars =
            (self.max_context_tokens - RESPONSE_RESERVE) * CHARS_PER_TOKEN;
        let mut result: Vec<Message> = Vec::new();

        // Always include system prompt first
        if let Some(ref sp) = self.system_prompt {
            result.push(Message::system(sp));
        }

        // Collect all non-system messages
        let mut others: Vec<&Message> = self
            .messages
            .iter()
            .collect();

        // Measure total size from the back and drop old messages if needed
        let mut total_chars: usize = result.iter().map(|m| m.content.len()).sum();
        while total_chars > max_chars && !others.is_empty() {
            let removed = others.remove(0);
            total_chars = total_chars.saturating_sub(removed.content.len());
        }

        result.extend(others.into_iter().cloned());
        result
    }

    /// Get all messages in this context.
    pub fn messages(&self) -> &VecDeque<Message> {
        &self.messages
    }

    /// Check if there are any messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Number of messages in context.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Clear all non-system messages from the context.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.has_memory_injection = false;
    }

    /// Load messages from a previous session into context.
    pub fn load_messages(&mut self, messages: Vec<Message>) {
        self.messages.clear();
        for msg in messages {
            self.messages.push_back(msg);
        }
    }

    pub fn has_memory_injection(&self) -> bool {
        self.has_memory_injection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sliding_window() {
        let mut ctx = SessionContext::new("test-id".into(), "test-model".into());
        ctx.set_system_prompt(Some("You are a helpful assistant.".into()));

        ctx.add_user_message("Hello!");
        ctx.add_assistant_message("Hi there!");

        let msgs = ctx.build_messages_for_model();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "user");
    }

    #[test]
    fn test_pop_last_assistant() {
        let mut ctx = SessionContext::new("test".into(), "m".into());
        ctx.add_user_message("hi");
        ctx.add_assistant_message("hello");
        ctx.add_user_message("how are you?");
        ctx.add_assistant_message("I'm fine!");

        let popped = ctx.pop_last_assistant();
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().content, "I'm fine!");
        assert_eq!(ctx.len(), 3);
    }

    #[test]
    fn test_memory_injection_replaces_previous() {
        let mut ctx = SessionContext::new("test".into(), "m".into());
        ctx.add_user_message("hi");
        ctx.inject_memory_prompt("- 名字是张三");
        assert_eq!(ctx.messages().len(), 2);

        // Second injection replaces
        ctx.inject_memory_prompt("- 名字是李四");
        assert_eq!(ctx.messages().len(), 2);
    }
}