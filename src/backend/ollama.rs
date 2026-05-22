use super::{LlmBackend, ModelOptions, UsageStats};
use crate::config::OllamaConfig;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_ctx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u32>,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    name: String,
}

#[derive(Deserialize)]
struct GenerateChunk {
    response: Option<String>,
    done: Option<bool>,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
    total_duration: Option<u64>,
}

pub struct OllamaBackend {
    pub host: String,
    client: reqwest::blocking::Client,
}

impl OllamaBackend {
    pub fn new(config: &OllamaConfig) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|e| anyhow!("Failed to build HTTP client: {e}"))?;
        Ok(OllamaBackend {
            host: config.host.trim_end_matches('/').to_string(),
            client,
        })
    }

    pub fn check_connection(&self) -> Result<()> {
        let url = format!("{}/api/tags", self.host);
        let resp = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?
            .get(&url)
            .send()?;
        if resp.status().is_success() { Ok(()) }
        else { Err(anyhow!("Ollama returned status {}", resp.status())) }
    }
}

impl LlmBackend for OllamaBackend {
    fn generate_complete(
        &self,
        model: &str,
        prompt: &str,
        options: &ModelOptions,
    ) -> Result<(String, UsageStats)> {
        let url = format!("{}/api/generate", self.host);
        let opts = OllamaOptions {
            temperature: Some(options.temperature),
            top_p: Some(options.top_p),
            top_k: Some(options.top_k),
            num_predict: options.max_tokens.map(|n| n as i32),
            num_ctx: options.num_ctx,
            repeat_penalty: Some(options.repeat_penalty),
            seed: options.seed,
        };
        let body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": true,
            "options": opts,
        });

        let resp = self.client.post(&url).json(&body).send()?;
        if !resp.status().is_success() {
            return Err(anyhow!("Ollama generate error: {}", resp.status()));
        }

        use std::io::BufRead;
        let mut full = String::new();
        let mut usage = UsageStats::default();
        let reader = std::io::BufReader::new(resp);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() { continue; }
            if let Ok(chunk) = serde_json::from_str::<GenerateChunk>(&line) {
                if let Some(t) = chunk.response { full.push_str(&t); }
                if chunk.done == Some(true) {
                    usage.input_tokens = chunk.prompt_eval_count.unwrap_or(0);
                    usage.output_tokens = chunk.eval_count.unwrap_or(0);
                    usage.duration_ms = chunk.total_duration.unwrap_or(0) / 1_000_000;
                }
            }
        }
        Ok((full, usage))
    }

    fn list_models(&self) -> Vec<String> {
        let url = format!("{}/api/tags", self.host);
        let Ok(resp) = self.client.get(&url).send() else { return vec![] };
        let Ok(tags) = resp.json::<TagsResponse>() else { return vec![] };
        tags.models.into_iter().map(|m| m.name).collect()
    }

    fn backend_name(&self) -> &'static str { "Ollama" }
}
