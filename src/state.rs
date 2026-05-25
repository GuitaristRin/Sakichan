use crate::backend::UsageStats;
use crate::config::SakichanConfig;
use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct DetectedTool {
    pub name: String,
    pub version: String,
    pub description: String,
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
    pub config: SakichanConfig,
    pub slot_assignments: HashMap<String, String>,
    pub toolchain_info: Vec<DetectedTool>,
    pub edit_mode: bool,
    pub lang: String,
    pub work_dir: PathBuf,
    pub usage: PersistUsage,
    pub usage_file: PathBuf,
    pub checkpoint_count: u32,
}

impl AppState {
    pub fn new(work_dir: PathBuf, config: SakichanConfig) -> Self {
        let usage_file = work_dir.join(".sakichan").join("usage.json");
        let usage = Self::load_usage(&usage_file);
        let edit_mode = config.general.edit_mode;
        let lang = config.general.lang.clone();
        AppState {
            config,
            slot_assignments: HashMap::new(),
            toolchain_info: Vec::new(),
            edit_mode,
            lang,
            work_dir,
            usage,
            usage_file,
            checkpoint_count: 0,
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

    pub fn ollama_host(&self) -> &str {
        &self.config.backend.ollama.host
    }

    pub fn toolchain_prompt_section(&self) -> String {
        if self.toolchain_info.is_empty() {
            return String::new();
        }
        let mut s = "## 当前环境可用工具\n".to_string();
        for t in &self.toolchain_info {
            if t.version.is_empty() {
                s.push_str(&format!("- {}: {}\n", t.name, t.description));
            } else {
                s.push_str(&format!("- {} {}: {}\n", t.name, t.version, t.description));
            }
        }
        s.push('\n');
        s
    }
}
