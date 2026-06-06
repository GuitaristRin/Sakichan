use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::Result;

/// A message in the conversation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Base64-encoded data URIs for attached images, e.g. "data:image/jpeg;base64,/9j..."
    /// Skipped in serde — models use to_api_message() instead when images are present.
    #[serde(skip, default)]
    pub images: Vec<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: "user".to_string(),
            content: content.into(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: Vec::new(),
        }
    }

    /// Create a user message with attached images.
    /// `images` should be base64 data URIs like "data:image/jpeg;base64,/9j..."
    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Message {
            role: "user".to_string(),
            content: content.into(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Message {
            role: "assistant".to_string(),
            content: content.into(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: Vec::new(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Message {
            role: "system".to_string(),
            content: content.into(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: Vec::new(),
        }
    }

    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Message {
            role: "tool".to_string(),
            content: content.into(),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
            images: Vec::new(),
        }
    }

    /// Convert this message to the API JSON format.
    /// If images are present, serializes content as a multi-part array;
    /// otherwise serializes as a plain string (backward-compatible).
    pub fn to_api_value(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("role".to_string(), json!(self.role));

        if self.images.is_empty() {
            map.insert("content".to_string(), json!(self.content));
        } else {
            let mut parts = Vec::new();
            if !self.content.is_empty() {
                parts.push(json!({
                    "type": "text",
                    "text": self.content
                }));
            }
            for data_uri in &self.images {
                parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": data_uri }
                }));
            }
            map.insert("content".to_string(), json!(parts));
        }

        if let Some(ref rc) = self.reasoning_content {
            map.insert("reasoning_content".to_string(), json!(rc));
        }
        if let Some(ref tc) = self.tool_calls {
            map.insert("tool_calls".to_string(), json!(tc));
        }
        if let Some(ref tcid) = self.tool_call_id {
            map.insert("tool_call_id".to_string(), json!(tcid));
        }
        if let Some(ref n) = self.name {
            map.insert("name".to_string(), json!(n));
        }

        serde_json::Value::Object(map)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug)]
pub struct ChatOptions {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub extra_params: std::collections::HashMap<String, serde_json::Value>,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            max_tokens: None,
            temperature: None,
            top_p: None,
            extra_params: std::collections::HashMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UsageInfo {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Clone, Debug)]
pub enum StreamEvent {
    Token(String),
    ReasoningToken(String),
    ToolCall(Vec<ToolCall>),
    Done,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct FinalResult {
    pub full_content: String,
    pub reasoning_content: Option<String>,
    pub usage: Option<UsageInfo>,
    pub finish_reason: String,
}

/// Trait for chat-based language models.
#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn chat_stream(
        &self,
        messages: &[Message],
        options: &ChatOptions,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<FinalResult>;

    fn model_id(&self) -> &str;
}

/// Trait for image generation models.
#[async_trait]
pub trait ImageGenModel: Send + Sync {
    async fn generate_image(&self, prompt: &str, size: &str, n: u8) -> Result<Vec<String>>;

    fn model_id(&self) -> &str;
}
