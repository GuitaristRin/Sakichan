use async_trait::async_trait;
use serde_json::json;

use super::traits::*;
use crate::error::{Result, SakichanError};

/// Sensenova Flash Lite — a fast chat completion model.
pub struct SensenovaFlashLite {
    model_id: String,
    api_base: String,
    api_key: String,
    client: reqwest::Client,
}

impl SensenovaFlashLite {
    pub fn new(api_base: String, api_key: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            model_id: "sensenova-6.7-flash-lite".to_string(),
            api_base,
            api_key,
            client,
        }
    }
}

#[async_trait]
impl ChatModel for SensenovaFlashLite {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        options: &ChatOptions,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<FinalResult> {
        let api_messages: Vec<serde_json::Value> = messages.iter().map(|m| m.to_api_value()).collect();
        let mut body = json!({
            "model": self.model_id,
            "messages": api_messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });

        if let Some(max_tokens) = options.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if let Some(reasoning) = options.extra_params.get("reasoning_effort") {
            body["reasoning_effort"] = reasoning.clone();
        }
        if let Some(temp) = options.temperature {
            body["temperature"] = json!(temp);
        }
        if let Some(top_p) = options.top_p {
            body["top_p"] = json!(top_p);
        }

        let response = self
            .client
            .post(&self.api_base)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            return Err(SakichanError::Api {
                status,
                message: error_text,
            });
        }

        let mut full_content = String::new();
        let mut finish_reason = String::from("stop");
        let mut usage: Option<UsageInfo> = None;

        let mut stream = response.bytes_stream();
        use futures::StreamExt;
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let chunk_str = String::from_utf8_lossy(&chunk);
            buffer.push_str(&chunk_str);

            // Process complete SSE lines
            while let Some(line_end) = buffer.find('\n') {
                let line = buffer[..line_end].trim().to_string();
                buffer = buffer[line_end + 1..].to_string();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        on_event(StreamEvent::Done);
                        continue;
                    }

                    match serde_json::from_str::<serde_json::Value>(data) {
                        Ok(parsed) => {
                            if let Some(choices) = parsed["choices"].as_array() {
                                for choice in choices {
                                    let delta = &choice["delta"];
                                    if let Some(content) = delta["content"].as_str() {
                                        if !content.is_empty() {
                                            full_content.push_str(content);
                                            on_event(StreamEvent::Token(content.to_string()));
                                        }
                                    }

                                    if let Some(fr) = choice["finish_reason"].as_str() {
                                        if !fr.is_empty() && fr != "null" {
                                            finish_reason = fr.to_string();
                                        }
                                    }
                                }
                            }

                            if let Some(u) = parsed["usage"].as_object() {
                                usage = Some(UsageInfo {
                                    prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                    completion_tokens: u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                    total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                });
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to parse SSE JSON: {e}, data: {data}");
                        }
                    }
                }
            }
        }

        on_event(StreamEvent::Done);

        Ok(FinalResult {
            full_content,
            reasoning_content: None,
            usage,
            finish_reason,
        })
    }
}