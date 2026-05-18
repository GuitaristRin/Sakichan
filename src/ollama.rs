use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

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

pub struct OllamaClient {
    pub host: String,
    client: reqwest::blocking::Client,
}

impl OllamaClient {
    pub fn new(host: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("Failed to build HTTP client");
        OllamaClient { host: host.to_string(), client }
    }

    pub fn check_connection(&self) -> Result<()> {
        let url = format!("http://{}/api/tags", self.host);
        let resp = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?
            .get(&url)
            .send()?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(anyhow!("Ollama returned status {}", resp.status()))
        }
    }

    pub fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("http://{}/api/tags", self.host);
        let resp = self.client.get(&url).send()?;
        let tags: TagsResponse = resp.json()?;
        Ok(tags.models.into_iter().map(|m| m.name).collect())
    }

    pub fn chat(&self, model: &str, prompt: &str) -> Result<(String, UsageStats)> {
        let url = format!("http://{}/api/generate", self.host);
        let body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": true
        });

        let resp = self.client.post(&url).json(&body).send()?;
        if !resp.status().is_success() {
            return Err(anyhow!("Ollama generate error: {}", resp.status()));
        }

        use std::io::BufRead;
        let mut full_response = String::new();
        let mut usage = UsageStats::default();

        let reader = std::io::BufReader::new(resp);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<GenerateChunk>(&line) {
                if let Some(text) = chunk.response {
                    full_response.push_str(&text);
                }
                if chunk.done == Some(true) {
                    usage.input_tokens = chunk.prompt_eval_count.unwrap_or(0);
                    usage.output_tokens = chunk.eval_count.unwrap_or(0);
                    usage.duration_ms = chunk.total_duration.unwrap_or(0) / 1_000_000;
                }
            }
        }

        Ok((full_response, usage))
    }
}
