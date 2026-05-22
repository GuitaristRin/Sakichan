use crate::config::{ModelPreset, SakichanConfig};
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod ollama;

// ── Model options ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub duration_ms: u64,
}

impl std::ops::AddAssign for UsageStats {
    fn add_assign(&mut self, rhs: Self) {
        self.input_tokens += rhs.input_tokens;
        self.output_tokens += rhs.output_tokens;
        self.duration_ms += rhs.duration_ms;
    }
}

#[derive(Debug, Clone)]
pub struct ModelOptions {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub repeat_penalty: f32,
    pub max_tokens: Option<u32>,
    pub num_ctx: Option<u32>,
    pub seed: Option<u32>,
}

impl Default for ModelOptions {
    fn default() -> Self {
        ModelOptions {
            temperature: 0.3,
            top_p: 0.9,
            top_k: 40,
            repeat_penalty: 1.1,
            max_tokens: None,
            num_ctx: Some(8192),
            seed: None,
        }
    }
}

impl ModelOptions {
    pub fn from_preset(preset: &ModelPreset) -> Self {
        ModelOptions {
            temperature: preset.temperature.unwrap_or(0.3),
            top_p: preset.top_p.unwrap_or(0.9),
            top_k: preset.top_k.unwrap_or(40),
            repeat_penalty: preset.repeat_penalty.unwrap_or(1.1),
            max_tokens: preset.max_tokens,
            num_ctx: Some(8192),
            seed: None,
        }
    }
}

// ── Backend trait ─────────────────────────────────────────────────────────────

pub trait LlmBackend: Send + Sync {
    fn generate_complete(
        &self,
        model: &str,
        prompt: &str,
        options: &ModelOptions,
    ) -> Result<(String, UsageStats)>;

    fn list_models(&self) -> Vec<String>;

    fn backend_name(&self) -> &'static str;
}

// ── Factory ───────────────────────────────────────────────────────────────────

pub fn create_backend(config: &SakichanConfig) -> Result<Box<dyn LlmBackend>> {
    match config.backend.backend_type.as_str() {
        "ollama" => Ok(Box::new(ollama::OllamaBackend::new(&config.backend.ollama)?)),
        "lmcpp" => Err(anyhow::anyhow!(
            "llama.cpp 后端尚未实现。请在 sakichan.conf 中设置 [backend] type = \"ollama\"。"
        )),
        other => Err(anyhow::anyhow!("不支持的后端类型: {}", other)),
    }
}
