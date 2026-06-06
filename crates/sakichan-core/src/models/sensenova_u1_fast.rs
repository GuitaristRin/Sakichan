use async_trait::async_trait;
use serde_json::json;

use super::traits::*;
use crate::error::{Result, SakichanError};

/// Sensenova U1 Fast — image generation model.
pub struct SensenovaU1Fast {
    model_id: String,
    api_base: String,
    api_key: String,
    client: reqwest::Client,
}

impl SensenovaU1Fast {
    pub fn new(api_base: String, api_key: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            model_id: "sensenova-u1-fast".to_string(),
            api_base,
            api_key,
            client,
        }
    }
}

#[async_trait]
impl ImageGenModel for SensenovaU1Fast {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn generate_image(&self, prompt: &str, size: &str, n: u8) -> Result<Vec<String>> {
        let body = json!({
            "model": self.model_id,
            "prompt": prompt,
            "n": n,
            "size": size,
        });

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

        let parsed: serde_json::Value = response.json().await?;

        let urls = parsed["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item["url"]
                            .as_str()
                            .or_else(|| item["b64_json"].as_str())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(urls)
    }
}