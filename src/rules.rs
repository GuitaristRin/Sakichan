use anyhow::Result;
use chrono::Local;
use std::fs;
use std::path::PathBuf;

pub struct RulesManager {
    pub rules_file: PathBuf,
}

impl RulesManager {
    pub fn new(rules_file: PathBuf) -> Self {
        RulesManager { rules_file }
    }

    pub fn init(&self) -> Result<()> {
        if !self.rules_file.exists() {
            let template = "# .sakichan.md - 项目规则\n\n## 项目概述\n<!-- 项目描述 -->\n\n## 目录结构\n<!-- 说明 -->\n\n## 编码规范\n<!-- 规则 -->\n\n## 禁止事项\n<!-- 不允许的操作 -->\n";
            fs::write(&self.rules_file, template)?;
        }
        Ok(())
    }

    pub fn load(&self) -> String {
        fs::read_to_string(&self.rules_file).unwrap_or_default()
    }

    pub fn update(&self, completed: &[String], structure: &str) -> Result<()> {
        let existing = self.load();
        let date = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        let mut new_content = existing.clone();

        let timestamp_marker = "\n## 最后更新 / Last Updated\n";
        if let Some(pos) = new_content.find(timestamp_marker) {
            new_content.truncate(pos);
        }

        new_content.push_str(timestamp_marker);
        new_content.push_str(&format!("{date}\n"));

        if !completed.is_empty() {
            new_content.push_str("\n## 已完成模块 / Completed Modules\n");
            for item in completed {
                new_content.push_str(&format!("- {item}\n"));
            }
        }

        if !structure.is_empty() {
            new_content.push_str("\n## 当前项目结构 / Project Structure\n");
            new_content.push_str("```\n");
            new_content.push_str(structure);
            new_content.push_str("\n```\n");
        }

        fs::write(&self.rules_file, new_content)?;
        Ok(())
    }
}
