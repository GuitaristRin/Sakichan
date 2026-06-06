use crate::error::Result;
use crate::memory::retrieval::MemoryRetriever;
use crate::memory::store::MemoryStore;
use crate::models::traits::{ChatModel, ChatOptions, FinalResult, Message, StreamEvent};
use crate::session::{SessionContext, SessionStore};
use std::sync::Arc;

/// Events emitted during the pipeline execution.
#[derive(Clone, Debug)]
pub enum PipelineEvent {
    /// A text token from the model.
    Token(String),
    /// A reasoning token (DeepSeek-specific).
    ReasoningToken(String),
    /// Long-term memories were retrieved and injected.
    MemoriesInjected { count: usize },
    /// Memory retrieval returned nothing.
    NoMemoriesFound,
    /// Image analysis output from the multimodal model.
    AnalyseImage(String),
    /// The model finished generating.
    Done(FinalResult),
    /// An error occurred.
    Error(String),
}

/// Full pipeline context holding all state for a chat session.
pub struct PipelineContext {
    pub session_ctx: SessionContext,
    pub session_store: Arc<SessionStore>,
    pub memory_store: Arc<MemoryStore>,
    pub retrieval_top_k: usize,
}

impl PipelineContext {
    pub fn new(
        session_ctx: SessionContext,
        session_store: Arc<SessionStore>,
        memory_store: Arc<MemoryStore>,
        retrieval_top_k: usize,
    ) -> Self {
        Self {
            session_ctx,
            session_store,
            memory_store,
            retrieval_top_k,
        }
    }

    /// Run the full chat pipeline for a single user message.
    ///
    /// 1. Adds the user message (with optional images) to the session context and persists it.
    /// 2. Retrieves relevant long-term memories and injects them as context.
    /// 3. Builds the message list with sliding window truncation.
    /// 4. Calls the model with streaming, emitting tokens via `on_event`.
    /// 5. Persists the assistant response.
    ///
    /// `images` should be base64 data URIs (e.g. "data:image/jpeg;base64,/9j...").
    /// Pass an empty vec when no images are attached.
    pub async fn run(
        &mut self,
        user_input: &str,
        images: Vec<String>,
        model: &dyn ChatModel,
        on_event: &mut (dyn FnMut(PipelineEvent) + Send),
    ) -> Result<FinalResult> {
        // 1. Add user message to context (with images if any)
        if images.is_empty() {
            self.session_ctx.add_user_message(user_input);
        } else {
            self.session_ctx.add_user_message_with_images(user_input, images);
        }

        // 2. Persist user message
        let user_msg = Message::user(user_input);
        let depth = self.session_ctx.current_depth();
        let order = self.session_ctx.current_order();
        self.session_store
            .add_message(self.session_ctx.session_id(), &user_msg, depth, order)?;

        // 2a. Update session active branch
        self.session_store
            .set_active_branch(self.session_ctx.session_id(), depth, order)?;

        // 3. Long-term memory retrieval & injection
        let retriever = MemoryRetriever::new((*self.memory_store).clone(), self.retrieval_top_k);

        let relevant_memories = match retriever.retrieve(user_input).await {
            Ok(memories) => memories,
            Err(e) => {
                on_event(PipelineEvent::Error(format!("记忆检索失败: {e}")));
                Vec::new()
            }
        };

        if !relevant_memories.is_empty() {
            let memory_prompt =
                MemoryRetriever::format_memories_for_prompt(&relevant_memories);
            self.session_ctx.inject_memory_prompt(&memory_prompt);
            on_event(PipelineEvent::MemoriesInjected {
                count: relevant_memories.len(),
            });
        } else {
            on_event(PipelineEvent::NoMemoriesFound);
        }

        // 4. Build message list with sliding window
        let messages = self.session_ctx.build_messages_for_model();

        // 5. Call model with streaming
        let mut final_content = String::new();
        let mut final_reasoning: Option<String> = None;

        let options = ChatOptions::default();
        let model_result = model
            .chat_stream(&messages, &options, &mut |event| {
                match event {
                    StreamEvent::Token(t) => {
                        final_content.push_str(&t);
                        on_event(PipelineEvent::Token(t));
                    }
                    StreamEvent::ReasoningToken(t) => {
                        final_reasoning
                            .get_or_insert_with(String::new)
                            .push_str(&t);
                        on_event(PipelineEvent::ReasoningToken(t));
                    }
                    StreamEvent::Done => {}
                    StreamEvent::Error(e) => {
                        on_event(PipelineEvent::Error(e));
                    }
                    StreamEvent::ToolCall(_) => {
                        // Tool calls not yet implemented in v1
                    }
                }
            })
            .await?;

        // 6. Add assistant response to context
        self.session_ctx.add_assistant_message(&final_content);

        // 7. Persist assistant message
        let assistant_msg = Message {
            role: "assistant".to_string(),
            content: final_content.clone(),
            reasoning_content: final_reasoning.clone(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: Vec::new(),
        };
        self.session_store
            .add_message(self.session_ctx.session_id(), &assistant_msg, depth, order)?;

        // 8. Emit done event
        let final_result = FinalResult {
            full_content: final_content,
            reasoning_content: final_reasoning,
            usage: model_result.usage,
            finish_reason: model_result.finish_reason,
        };
        on_event(PipelineEvent::Done(final_result.clone()));

        Ok(final_result)
    }

    /// Run the two-stage image pipeline:
    /// 1. Use a multimodal model (SenseNova Flash-Lite) to describe the image
    /// 2. Send the description + user question to a text-only model (DeepSeek V4 Flash)
    ///
    /// `images` should be base64 data URIs.
    /// `analyse_model` is used for the first stage (multimodal description).
    /// `text_model` is used for the second stage (final answer generation).
    pub async fn run_image_pipeline(
        &mut self,
        user_input: &str,
        images: Vec<String>,
        analyse_model: &dyn ChatModel,
        text_model: &dyn ChatModel,
        on_event: &mut (dyn FnMut(PipelineEvent) + Send),
    ) -> Result<FinalResult> {
        // 1. Add the user message with images to context and persist
        self.session_ctx.add_user_message_with_images(user_input, images.clone());
        let depth = self.session_ctx.current_depth();
        let order = self.session_ctx.current_order();
        let user_msg = Message::user_with_images(user_input, images.clone());
        self.session_store
            .add_message(self.session_ctx.session_id(), &user_msg, depth, order)?;
        self.session_store
            .set_active_branch(self.session_ctx.session_id(), depth, order)?;

        // 2. Stage 1: Get image description from multimodal model
        // Use a minimal messages list for the description call (system + image message)
        let desc_messages = vec![
            Message::system("你是一个图像分析助手。请详细描述用户图片中的内容。"),
            Message::user_with_images("请详细描述这张图片的内容。", images),
        ];

        let mut description = String::new();
        let analyse_options = crate::models::ChatOptions::default();
        analyse_model
            .chat_stream(&desc_messages, &analyse_options, &mut |event| {
                if let crate::models::StreamEvent::Token(t) = event {
                    description.push_str(&t);
                    on_event(PipelineEvent::AnalyseImage(t));
                }
            })
            .await?;

        // 3. Inject description into session context as memory-like context
        let desc_prompt = format!(
            "[图片描述]\n{}\n\n[用户问题]\n{}",
            description.trim(),
            user_input
        );
        self.session_ctx.inject_memory_prompt(&desc_prompt);

        on_event(PipelineEvent::Token(String::new())); // flush separator

        // 4. Stage 2: Send description + question to text-only model
        let mut final_content = String::new();
        let mut final_reasoning: Option<String> = None;

        let text_messages = self.session_ctx.build_messages_for_model();
        let text_options = crate::models::ChatOptions::default();

        let model_result = text_model
            .chat_stream(&text_messages, &text_options, &mut |event| {
                match event {
                    crate::models::StreamEvent::Token(t) => {
                        final_content.push_str(&t);
                        on_event(PipelineEvent::Token(t));
                    }
                    crate::models::StreamEvent::ReasoningToken(t) => {
                        final_reasoning
                            .get_or_insert_with(String::new)
                            .push_str(&t);
                        on_event(PipelineEvent::ReasoningToken(t));
                    }
                    _ => {}
                }
            })
            .await?;

        // 5. Persist assistant response
        let assistant_msg = Message {
            role: "assistant".to_string(),
            content: final_content.clone(),
            reasoning_content: final_reasoning.clone(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: Vec::new(),
        };
        self.session_ctx.add_assistant_message(&final_content);
        self.session_store
            .add_message(self.session_ctx.session_id(), &assistant_msg, depth, order)?;

        let final_result = FinalResult {
            full_content: final_content,
            reasoning_content: final_reasoning,
            usage: model_result.usage,
            finish_reason: model_result.finish_reason,
        };
        on_event(PipelineEvent::Done(final_result.clone()));

        Ok(final_result)
    }
}

/// Generate a session title using a summarization model call.
pub async fn generate_session_title(
    model: &dyn ChatModel,
    messages: &[Message],
) -> Result<String> {
    let context = messages
        .iter()
        .take(3)
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let summary_prompt = format!(
        "请为以下对话开头生成一个简短的标题 (5-10个字), 只输出标题本身:\n\n{}",
        context
    );

    let mut title = String::new();
    model
        .chat_stream(
            &[
                Message::system("你是一个有用的助手，负责为对话生成简洁的标题。"),
                Message::user(&summary_prompt),
            ],
            &ChatOptions {
                max_tokens: Some(100),
                temperature: Some(0.3),
                extra_params: {
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "reasoning_effort".to_string(),
                        serde_json::Value::String("none".to_string()),
                    );
                    m
                },
                ..Default::default()
            },
            &mut |event| {
                match event {
                    StreamEvent::Token(t) => title.push_str(&t),
                    StreamEvent::ReasoningToken(t) => title.push_str(&t),
                    _ => {}
                }
            },
        )
        .await?;

    Ok(title.trim().to_string())
}
