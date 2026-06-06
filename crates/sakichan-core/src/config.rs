use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{Result, SakichanError};

/// Resolved model configuration with API key already resolved from keyring.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelEntry {
    pub api_base: String,
    pub api_key: String,
    #[serde(default)]
    pub extra_params: HashMap<String, serde_json::Value>,
}

/// Raw configuration as parsed from config.toml.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RawConfig {
    #[serde(default)]
    pub models: ModelsSection,

    #[serde(default)]
    pub database: DatabaseSection,

    #[serde(default)]
    pub memory: MemorySection,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelsSection {
    #[serde(default = "default_chat_model")]
    pub default_chat: String,

    #[serde(default = "default_image_model")]
    pub default_image: String,

    #[serde(default)]
    pub model_configs: HashMap<String, RawModelEntry>,
}

fn default_chat_model() -> String {
    "sensenova-flash-lite".to_string()
}

fn default_image_model() -> String {
    "sensenova-u1-fast".to_string()
}

impl Default for ModelsSection {
    fn default() -> Self {
        Self {
            default_chat: default_chat_model(),
            default_image: default_image_model(),
            model_configs: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RawModelEntry {
    pub api_base: String,
    pub api_key: String,
    #[serde(default)]
    pub extra_params: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseSection {
    #[serde(default = "default_db_path")]
    pub path: String,
}

fn default_db_path() -> String {
    "~/.local/share/sakichan/sakichan.db".to_string()
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemorySection {
    #[serde(default = "default_auto_summarize")]
    pub auto_summarize: bool,

    #[serde(default = "default_retrieval_top_k")]
    pub retrieval_top_k: usize,
}

fn default_auto_summarize() -> bool {
    true
}

fn default_retrieval_top_k() -> usize {
    5
}

impl Default for MemorySection {
    fn default() -> Self {
        Self {
            auto_summarize: default_auto_summarize(),
            retrieval_top_k: default_retrieval_top_k(),
        }
    }
}

/// Resolved configuration with API keys expanded.
#[derive(Clone)]
pub struct Config {
    pub models: ModelsSection,
    pub database: DatabaseSection,
    pub memory: MemorySection,
    /// Resolved model entries with real API keys.
    pub resolved_model_configs: HashMap<String, ModelEntry>,
}

/// Attempt to read and parse the config file from standard paths.
pub fn read_config() -> Result<Config> {
    let config_path = find_config_path()?;
    let raw: RawConfig = read_config_from(&config_path)?;
    resolve_config(raw)
}

pub fn find_config_path() -> Result<PathBuf> {
    let xdg_config = dirs_config_dir().map(|p| p.join("sakichan/config.toml"));
    let legacy = dirs_home_dir().map(|p| p.join(".config/sakichan/config.toml"));
    let cwd = std::env::current_dir().ok().map(|p| p.join("config.toml"));

    for candidate in [cwd, legacy, xdg_config].into_iter().flatten() {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // Default return path for first-run creation
    Ok(dirs_config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sakichan/config.toml"))
}

pub fn default_config_path() -> PathBuf {
    dirs_config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sakichan/config.toml")
}

pub fn default_data_dir() -> PathBuf {
    dirs_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sakichan")
}

fn dirs_config_dir() -> Option<PathBuf> {
    // Use shellexpand-aware paths
    Some(PathBuf::from(
        shellexpand::tilde("~/.config").to_string(),
    ))
}

fn dirs_data_dir() -> Option<PathBuf> {
    Some(PathBuf::from(
        shellexpand::tilde("~/.local/share").to_string(),
    ))
}

fn dirs_home_dir() -> Option<PathBuf> {
    Some(PathBuf::from(shellexpand::tilde("~").to_string()))
}

pub fn read_config_from(path: &std::path::Path) -> Result<RawConfig> {
    if !path.exists() {
        // Return default config if file doesn't exist
        return Ok(RawConfig {
            models: ModelsSection {
                default_chat: default_chat_model(),
                default_image: default_image_model(),
                model_configs: HashMap::new(),
            },
            database: DatabaseSection {
                path: default_db_path(),
            },
            memory: MemorySection {
                auto_summarize: true,
                retrieval_top_k: 5,
            },
        });
    }

    let content = std::fs::read_to_string(path)
        .map_err(|e| SakichanError::Config(format!("Failed to read config file: {e}")))?;

    let raw: RawConfig = toml::from_str(&content)
        .map_err(|e| SakichanError::Config(format!("Failed to parse config file: {e}")))?;

    Ok(raw)
}

pub fn resolve_config(raw: RawConfig) -> Result<Config> {
    // Check for ENC_KEYRING: markers and mark them; actual resolution is done by CLI
    let mut resolved = HashMap::new();

    for (name, entry) in &raw.models.model_configs {
        let api_key = if entry.api_key.starts_with("ENC_KEYRING:") {
            // Tag it; CLI will resolve against system keychain
            entry.api_key.clone()
        } else {
            entry.api_key.clone()
        };

        resolved.insert(
            name.clone(),
            ModelEntry {
                api_base: entry.api_base.clone(),
                api_key,
                extra_params: entry.extra_params.clone(),
            },
        );
    }

    Ok(Config {
        models: raw.models,
        database: raw.database,
        memory: raw.memory,
        resolved_model_configs: resolved,
    })
}

pub fn create_default_config() -> Result<PathBuf> {
    let path = default_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| SakichanError::Config(format!("Failed to create config directory: {e}")))?;
    }

    let raw = RawConfig {
        models: ModelsSection {
            default_chat: default_chat_model(),
            default_image: default_image_model(),
            model_configs: HashMap::from([
                (
                    "sensenova-flash-lite".to_string(),
                    RawModelEntry {
                        api_base: "https://token.sensenova.cn/v1/chat/completions".to_string(),
                        api_key: "ENC_KEYRING:com.sakichan/sensenova".to_string(),
                        extra_params: HashMap::new(),
                    },
                ),
                (
                    "sensenova-u1-fast".to_string(),
                    RawModelEntry {
                        api_base: "https://token.sensenova.cn/v1/images/generations".to_string(),
                        api_key: "ENC_KEYRING:com.sakichan/sensenova".to_string(),
                        extra_params: HashMap::new(),
                    },
                ),
                (
                    "deepseek-v4-flash".to_string(),
                    RawModelEntry {
                        api_base: "https://token.sensenova.cn/v1/chat/completions".to_string(),
                        api_key: "ENC_KEYRING:com.sakichan/deepseek".to_string(),
                        extra_params: HashMap::from([(
                            "reasoning_effort".to_string(),
                            serde_json::Value::String("medium".to_string()),
                        )]),
                    },
                ),
            ]),
        },
        database: DatabaseSection {
            path: default_db_path(),
        },
        memory: MemorySection {
            auto_summarize: true,
            retrieval_top_k: 5,
        },
    };

    let content = toml::to_string_pretty(&raw)
        .map_err(|e| SakichanError::Config(format!("Failed to serialize default config: {e}")))?;

    std::fs::write(&path, content)
        .map_err(|e| SakichanError::Config(format!("Failed to write default config: {e}")))?;

    Ok(path)
}

impl Config {
    pub fn get_model(&self, name: &str) -> Option<&ModelEntry> {
        self.resolved_model_configs.get(name)
    }

    pub fn get_db_path(&self) -> PathBuf {
        PathBuf::from(shellexpand::tilde(&self.database.path).to_string())
    }
}

/// TOML serialization is needed only when writing default configs.
/// Re-export serde structs that need toml serialization.
pub use toml;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_creation() {
        let raw = RawConfig::default();
        assert_eq!(raw.models.default_chat, "sensenova-flash-lite");
        assert_eq!(raw.database.path, "~/.local/share/sakichan/sakichan.db");
    }
}