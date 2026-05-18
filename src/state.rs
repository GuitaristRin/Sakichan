use crate::ollama::UsageStats;
use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DailyUsage {
    #[serde(flatten)]
    pub stats: UsageStats,
    // We re-export the fields for convenience
    #[serde(skip)]
    pub input_tokens: u64,
    #[serde(skip)]
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistUsage {
    pub daily: HashMap<String, UsageStats>,
    pub total: UsageStats,
}

impl PersistUsage {
    pub fn add(&mut self, stats: &UsageStats) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let entry = self.daily.entry(today).or_default();
        entry.input_tokens += stats.input_tokens;
        entry.output_tokens += stats.output_tokens;
        entry.duration_ms += stats.duration_ms;
        self.total.input_tokens += stats.input_tokens;
        self.total.output_tokens += stats.output_tokens;
        self.total.duration_ms += stats.duration_ms;
    }
}

pub struct AppState {
    pub ollama_host: String,
    pub current_model: String,
    pub edit_mode: bool,
    pub lang: String,
    pub work_dir: PathBuf,
    pub usage: PersistUsage,
    pub usage_file: PathBuf,
}

impl AppState {
    pub fn new(work_dir: PathBuf) -> Self {
        let usage_file = work_dir.join(".sakichan").join("usage.json");
        let usage = Self::load_usage(&usage_file);
        AppState {
            ollama_host: "localhost:11434".to_string(),
            current_model: "qwen2.5-coder:7b".to_string(),
            edit_mode: false,
            lang: "zh_TW".to_string(),
            work_dir,
            usage,
            usage_file,
        }
    }

    fn load_usage(path: &PathBuf) -> PersistUsage {
        if let Ok(data) = fs::read_to_string(path) {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            PersistUsage::default()
        }
    }

    pub fn save_usage(&self) -> Result<()> {
        if let Some(parent) = self.usage_file.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(&self.usage)?;
        fs::write(&self.usage_file, data)?;
        Ok(())
    }

    pub fn save_session(&self, id: &str, context: &[String]) -> Result<()> {
        let sessions_dir = self.work_dir.join(".sakichan").join("sessions");
        fs::create_dir_all(&sessions_dir)?;
        let session_file = sessions_dir.join(format!("{id}.json"));
        let data = serde_json::to_string_pretty(context)?;
        fs::write(session_file, data)?;
        Ok(())
    }
}
