use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ── General ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GeneralConfig {
    #[serde(default = "default_lang")]
    pub lang: String,
    #[serde(default)]
    pub edit_mode: bool,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_lang() -> String { "zh_TW".to_string() }
fn default_log_level() -> String { "info".to_string() }

impl Default for GeneralConfig {
    fn default() -> Self {
        Self { lang: default_lang(), edit_mode: false, log_level: default_log_level() }
    }
}

// ── Backend ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BackendConfig {
    #[serde(rename = "type", default = "default_backend_type")]
    pub backend_type: String,
    #[serde(default)]
    pub ollama: OllamaConfig,
    #[serde(default)]
    pub lmcpp: LmCppConfig,
    #[serde(default)]
    pub llama_server: LlamaServerConfig,
}

fn default_backend_type() -> String { "ollama".to_string() }

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            backend_type: default_backend_type(),
            ollama: Default::default(),
            lmcpp: Default::default(),
            llama_server: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_host")]
    pub host: String,
}

fn default_ollama_host() -> String { "http://localhost:11434".to_string() }

impl Default for OllamaConfig {
    fn default() -> Self { Self { host: default_ollama_host() } }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LmCppConfig {
    #[serde(default = "default_model_dirs")]
    pub model_dirs: Vec<String>,
    #[serde(default = "default_n_gpu_layers")]
    pub n_gpu_layers: i32,
    #[serde(default = "default_n_ctx")]
    pub n_ctx: u32,
    #[serde(default = "default_n_batch")]
    pub n_batch: u32,
}


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LlamaServerConfig {
    #[serde(default = "default_llama_server_executable")]
    pub executable: String,
    #[serde(default = "default_llama_server_host")]
    pub host: String,
    #[serde(default = "default_llama_server_port")]
    pub port: u16,
    #[serde(default = "default_llama_server_model_dir")]
    pub model_dir: String,
    #[serde(default = "default_n_gpu_layers")]
    pub n_gpu_layers: i32,
    #[serde(default = "default_n_ctx")]
    pub n_ctx: u32,
    #[serde(default = "default_n_batch")]
    pub n_batch: u32,
}

fn default_llama_server_executable() -> String { "llama-cpp/llama-server.exe".to_string() }
fn default_llama_server_host() -> String { "127.0.0.1".to_string() }
fn default_llama_server_port() -> u16 { 8081 }
fn default_llama_server_model_dir() -> String { "llama-cpp".to_string() }

impl Default for LlamaServerConfig {
    fn default() -> Self {
        Self {
            executable: default_llama_server_executable(),
            host: default_llama_server_host(),
            port: default_llama_server_port(),
            model_dir: default_llama_server_model_dir(),
            n_gpu_layers: default_n_gpu_layers(),
            n_ctx: default_n_ctx(),
            n_batch: default_n_batch(),
        }
    }
}

fn default_model_dirs() -> Vec<String> { vec![".".to_string(), "~/.cache/sakichan/models".to_string()] }
fn default_n_gpu_layers() -> i32 { -1 }
fn default_n_ctx() -> u32 { 8192 }
fn default_n_batch() -> u32 { 512 }

impl Default for LmCppConfig {
    fn default() -> Self {
        Self {
            model_dirs: default_model_dirs(),
            n_gpu_layers: default_n_gpu_layers(),
            n_ctx: default_n_ctx(),
            n_batch: default_n_batch(),
        }
    }
}

// ── Slots & Presets ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SlotConfig {
    pub primary: String,
    #[serde(default)]
    pub fallback: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset_override: Option<ModelPreset>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ModelPreset {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

impl ModelPreset {
    pub fn merge(&self, over: &ModelPreset) -> ModelPreset {
        ModelPreset {
            temperature: over.temperature.or(self.temperature),
            top_p: over.top_p.or(self.top_p),
            top_k: over.top_k.or(self.top_k),
            repeat_penalty: over.repeat_penalty.or(self.repeat_penalty),
            max_tokens: over.max_tokens.or(self.max_tokens),
        }
    }
}

// ── Verification ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VerificationStrategy {
    pub detector: Detector,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Detector {
    FileExists {
        file_exists: String,
        #[serde(default)]
        contains: Option<String>,
    },
    AnyExtension {
        any_extension: String,
    },
    AllFiles {
        all_files: Vec<String>,
    },
    Always {
        always: bool,
    },
}

impl Detector {
    pub fn matches(&self, project_root: &Path) -> bool {
        match self {
            Detector::FileExists { file_exists, contains } => {
                let path = project_root.join(file_exists);
                if !path.exists() { return false; }
                if let Some(needle) = contains {
                    return fs::read_to_string(&path).map_or(false, |c| c.contains(needle.as_str()));
                }
                true
            }
            Detector::AnyExtension { any_extension } => {
                has_files_with_ext(project_root, any_extension)
            }
            Detector::AllFiles { all_files } => {
                all_files.iter().all(|f| project_root.join(f).exists())
            }
            Detector::Always { .. } => true,
        }
    }
}

fn has_files_with_ext(dir: &Path, ext: &str) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    let target = ext.trim_start_matches('.');
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some(target) {
                return true;
            }
        } else if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name != "target" && !name.starts_with('.') {
                if has_files_with_ext(&path, ext) { return true; }
            }
        }
    }
    false
}

// ── Top-level config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SakichanConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub backend: BackendConfig,
    #[serde(default)]
    pub slots: HashMap<String, SlotConfig>,
    #[serde(default)]
    pub presets: HashMap<String, ModelPreset>,
    #[serde(default)]
    pub slot_presets: HashMap<String, String>,
    #[serde(default, rename = "verification")]
    pub verification: Vec<VerificationStrategy>,
}

impl Default for SakichanConfig {
    fn default() -> Self {
        let mut cfg = SakichanConfig {
            general: Default::default(),
            backend: Default::default(),
            slots: HashMap::new(),
            presets: HashMap::new(),
            slot_presets: HashMap::new(),
            verification: vec![
                VerificationStrategy {
                    detector: Detector::FileExists {
                        file_exists: "Cargo.toml".to_string(),
                        contains: Some("[package]".to_string()),
                    },
                    name: "Rust".to_string(),
                    command: Some("cargo".to_string()),
                    args: vec!["check".to_string()],
                },
                VerificationStrategy {
                    detector: Detector::AnyExtension { any_extension: ".py".to_string() },
                    name: "Python".to_string(),
                    command: Some("python".to_string()),
                    args: vec!["-c".to_string(), "import py_compile, sys; [py_compile.compile(f) for f in sys.argv[1:]]".to_string()],
                },
                VerificationStrategy {
                    detector: Detector::AllFiles {
                        all_files: vec!["package.json".to_string(), "tsconfig.json".to_string()],
                    },
                    name: "TypeScript".to_string(),
                    command: Some("npx".to_string()),
                    args: vec!["tsc".to_string(), "--noEmit".to_string()],
                },
                VerificationStrategy {
                    detector: Detector::Always { always: true },
                    name: "Manual Review".to_string(),
                    command: None,
                    args: vec![],
                },
            ],
        };

        cfg.slots.insert("ProductOwner".to_string(), SlotConfig {
            primary: "qwen2.5-coder:7b".to_string(),
            fallback: vec!["deepseek-r1:8b".to_string()],
            models: vec!["qwen2.5-coder:7b".to_string(), "deepseek-r1:8b".to_string()],
            preset_override: None,
        });
        cfg.slots.insert("Architect".to_string(), SlotConfig {
            primary: "deepseek-r1:8b".to_string(),
            fallback: vec!["qwen2.5-coder:7b".to_string()],
            models: vec!["deepseek-r1:8b".to_string(), "qwen2.5-coder:7b".to_string()],
            preset_override: None,
        });
        cfg.slots.insert("SeniorEngineer".to_string(), SlotConfig {
            primary: "deepseek-r1:8b".to_string(),
            fallback: vec!["qwen2.5-coder:7b".to_string()],
            models: vec!["deepseek-r1:8b".to_string(), "qwen2.5-coder:7b".to_string()],
            preset_override: None,
        });
        cfg.slots.insert("Programmer".to_string(), SlotConfig {
            primary: "qwen2.5-coder:7b".to_string(),
            fallback: vec!["deepseek-coder:6.7b".to_string()],
            models: vec!["qwen2.5-coder:7b".to_string(), "deepseek-coder:6.7b".to_string(), "deepseek-r1:8b".to_string()],
            preset_override: None,
        });
        cfg.slots.insert("QA".to_string(), SlotConfig {
            primary: "qwen2.5-coder:7b".to_string(),
            fallback: vec!["deepseek-r1:8b".to_string()],
            models: vec!["qwen2.5-coder:7b".to_string(), "deepseek-r1:8b".to_string()],
            preset_override: None,
        });

        cfg.presets.insert("default".to_string(), ModelPreset {
            temperature: Some(0.3), top_p: Some(0.9), top_k: Some(40),
            repeat_penalty: Some(1.1), max_tokens: None,
        });
        cfg.presets.insert("architect".to_string(), ModelPreset {
            temperature: Some(0.4), top_p: Some(0.95), top_k: Some(50),
            repeat_penalty: Some(1.05), max_tokens: None,
        });
        cfg.presets.insert("programmer".to_string(), ModelPreset {
            temperature: Some(0.2), top_p: Some(0.9), top_k: Some(40),
            repeat_penalty: Some(1.1), max_tokens: None,
        });
        cfg.presets.insert("reviewer".to_string(), ModelPreset {
            temperature: Some(0.1), top_p: Some(0.85), top_k: Some(20),
            repeat_penalty: Some(1.2), max_tokens: None,
        });

        cfg.slot_presets.insert("ProductOwner".to_string(), "default".to_string());
        cfg.slot_presets.insert("Architect".to_string(), "architect".to_string());
        cfg.slot_presets.insert("SeniorEngineer".to_string(), "default".to_string());
        cfg.slot_presets.insert("Programmer".to_string(), "programmer".to_string());
        cfg.slot_presets.insert("QA".to_string(), "reviewer".to_string());

        cfg
    }
}

impl SakichanConfig {
    pub fn get_preset_for_slot(&self, slot_name: &str) -> ModelPreset {
        let preset = self.slot_presets
            .get(slot_name)
            .and_then(|name| self.presets.get(name))
            .or_else(|| self.presets.get("default"))
            .cloned()
            .unwrap_or_default();

        if let Some(slot_cfg) = self.slots.get(slot_name) {
            if let Some(over) = &slot_cfg.preset_override {
                return preset.merge(over);
            }
        }
        preset
    }
}

// ── Config loading ────────────────────────────────────────────────────────────

pub fn load_config() -> SakichanConfig {
    for path in config_search_paths() {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(cfg) = toml::from_str::<SakichanConfig>(&content) {
                    return cfg;
                }
            }
        }
    }

    let default = SakichanConfig::default();
    let user_path = user_config_path();
    if let Some(parent) = user_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(content) = toml::to_string_pretty(&default) {
        if fs::write(&user_path, &content).is_ok() {
            println!("ℹ  已在 {} 创建默认配置 / Default config created", user_path.display());
        }
    }
    default
}

pub fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("sakichan.conf")];
    if let Ok(env) = std::env::var("SAKICHAN_CONFIG") {
        paths.push(PathBuf::from(env));
    }
    paths.push(user_config_path());
    paths
}

pub fn active_config_path() -> Option<PathBuf> {
    config_search_paths().into_iter().find(|p| p.exists())
}

fn user_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("sakichan")
        .join("sakichan.conf")
}

pub fn save_config_field(key: &str, value: &str) -> Result<()> {
    let path = active_config_path()
        .unwrap_or_else(user_config_path);
    let content = if path.exists() {
        fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };
    let mut doc: toml::Table = toml::from_str(&content).unwrap_or_default();
    let (section, field) = key.split_once('.').unwrap_or(("general", key));
    let section_table = doc.entry(section).or_insert(toml::Value::Table(toml::Table::new()));
    if let toml::Value::Table(t) = section_table {
        t.insert(field.to_string(), toml::Value::String(value.to_string()));
    }
    if let Some(parent) = path.parent() { let _ = fs::create_dir_all(parent); }
    fs::write(&path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

