use crate::config::{SakichanConfig, SlotConfig};
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::PathBuf;

// ── Slot roles ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlotRole {
    ProductOwner,
    Architect,
    SeniorEngineer,
    Programmer,
    QA,
}

impl SlotRole {
    pub fn name(self) -> &'static str {
        match self {
            SlotRole::ProductOwner => "ProductOwner",
            SlotRole::Architect => "Architect",
            SlotRole::SeniorEngineer => "SeniorEngineer",
            SlotRole::Programmer => "Programmer",
            SlotRole::QA => "QA",
        }
    }

    pub fn role_prompt(self) -> &'static str {
        match self {
            SlotRole::ProductOwner =>
                "你是 Saki-chan 的 ProductOwner（产品负责人）模块。\n\
                 职责：需求总结、澄清提问、验收评估。关注用户意图而非技术实现细节。\n\n",
            SlotRole::Architect =>
                "你是 Saki-chan 的 Architect（系统架构师）模块。\n\
                 职责：方案设计、模块拆分、方向决策、跨模块一致性与架构审查。不直接生成业务代码。\n\n",
            SlotRole::SeniorEngineer =>
                "你是 Saki-chan 的 SeniorEngineer（高级工程师）模块。\n\
                 职责：复杂逻辑实现、编译诊断、跨模块修复。擅长底层细节和调试。\n\n",
            SlotRole::Programmer =>
                "你是 Saki-chan 的 Programmer（程序员）模块。\n\
                 职责：按规范实现代码，严格遵循接口定义、文件路径和语言约定。不做架构决策。\n\n",
            SlotRole::QA =>
                "你是 Saki-chan 的 QA（质量保证）模块。\n\
                 职责：代码审查、静态检查、验证代码是否符合规范。客观评估，不修复问题。\n\n",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "productowner" | "product_owner" | "po" => Some(SlotRole::ProductOwner),
            "architect" => Some(SlotRole::Architect),
            "seniorengineer" | "senior_engineer" | "se" => Some(SlotRole::SeniorEngineer),
            "programmer" => Some(SlotRole::Programmer),
            "qa" => Some(SlotRole::QA),
            // Legacy v0.3.0 model names mapped to slots
            "qwen" | "qwen2.5-coder:7b" => Some(SlotRole::Programmer),
            "dsr1" | "deepseek-r1:8b" | "deepseek" => Some(SlotRole::Architect),
            _ => None,
        }
    }

    pub fn all() -> &'static [SlotRole] {
        &[
            SlotRole::ProductOwner,
            SlotRole::Architect,
            SlotRole::SeniorEngineer,
            SlotRole::Programmer,
            SlotRole::QA,
        ]
    }
}

// ── Resolution ────────────────────────────────────────────────────────────────

pub fn resolve_slot(slot_cfg: &SlotConfig, available: &HashSet<String>) -> Result<String> {
    if available.contains(&slot_cfg.primary) {
        return Ok(slot_cfg.primary.clone());
    }
    for m in &slot_cfg.fallback {
        if available.contains(m) { return Ok(m.clone()); }
    }
    for m in &slot_cfg.models {
        if m != &slot_cfg.primary && available.contains(m) { return Ok(m.clone()); }
    }
    // Return primary even if not confirmed available (avoids blocking when probe fails)
    Ok(slot_cfg.primary.clone())
}

pub fn resolve_slot_model(
    role: SlotRole,
    config: &SakichanConfig,
    available: &HashSet<String>,
) -> String {
    let name = role.name();
    if let Some(slot_cfg) = config.slots.get(name) {
        resolve_slot(slot_cfg, available).unwrap_or_else(|_| slot_cfg.primary.clone())
    } else {
        default_model_for_role(role)
    }
}

fn default_model_for_role(role: SlotRole) -> String {
    match role {
        SlotRole::Architect | SlotRole::SeniorEngineer => "deepseek-r1:8b".to_string(),
        _ => "qwen2.5-coder:7b".to_string(),
    }
}

// ── Model discovery ───────────────────────────────────────────────────────────

pub fn probe_ollama_models(host: &str) -> HashSet<String> {
    let url = format!("{}/api/tags", host.trim_end_matches('/'));
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    let Ok(resp) = client.get(&url).send() else { return HashSet::new() };
    let Ok(json) = resp.json::<serde_json::Value>() else { return HashSet::new() };
    let mut set = HashSet::new();
    if let Some(models) = json["models"].as_array() {
        for m in models {
            if let Some(name) = m["name"].as_str() {
                set.insert(name.to_string());
            }
        }
    }
    set
}

pub fn probe_gguf_models(model_dirs: &[String]) -> HashSet<String> {
    let mut set = HashSet::new();
    for dir in model_dirs {
        let path = expand_tilde(dir);
        if let Ok(entries) = std::fs::read_dir(&path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().map_or(false, |e| e == "gguf") {
                    if let Some(stem) = p.file_stem() {
                        set.insert(stem.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    set
}

fn expand_tilde(s: &str) -> PathBuf {
    if s.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&s[2..]);
        }
    }
    PathBuf::from(s)
}

// ── Slot assignments (resolved at startup) ────────────────────────────────────

pub fn build_slot_assignments(
    config: &SakichanConfig,
    available: &HashSet<String>,
) -> std::collections::HashMap<String, String> {
    SlotRole::all()
        .iter()
        .map(|&role| (role.name().to_string(), resolve_slot_model(role, config, available)))
        .collect()
}
