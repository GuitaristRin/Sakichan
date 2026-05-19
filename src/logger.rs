use anyhow::Result;
use chrono::Local;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Logger {
    pub log_file: PathBuf,
    pub project_name: String,
}

impl Logger {
    pub fn new(log_file: PathBuf, project_name: String) -> Self {
        Logger { log_file, project_name }
    }

    /// Derive log path from work_dir: `{work_dir}/{work_dir_name}_log.md`.
    pub fn from_work_dir(work_dir: &Path) -> Self {
        let name = work_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let log_file = work_dir.join(format!("{name}_log.md"));
        Logger { log_file, project_name: name }
    }

    pub fn init(&self) -> Result<()> {
        if let Some(parent) = self.log_file.parent() {
            fs::create_dir_all(parent)?;
        }
        if !self.log_file.exists() {
            let date = Local::now().format("%Y-%m-%d").to_string();
            let header = format!(
                "# 施工日志 / Build Log\n\n**项目**: {}\n**开始日期**: {}\n\n",
                self.project_name, date
            );
            fs::write(&self.log_file, header)?;
        }
        Ok(())
    }

    pub fn log_task(
        &self,
        title: &str,
        description: &str,
        files: &[String],
        result: bool,
        fixes: &[(String, String)],
        model: &str,
        duration: f64,
    ) -> Result<()> {
        let date = Local::now().format("%Y-%m-%d %H:%M").to_string();
        let result_str = if result { "✓ 通过" } else { "✗ 失败" };

        let mut entry = format!(
            "## {date}\n\n### {title}\n\n**任务类型**：{description}\n\n**使用模型**：{model} | 耗时: {duration:.1}s\n\n"
        );

        if !files.is_empty() {
            entry.push_str("**修改文件**：\n");
            for f in files {
                entry.push_str(&format!("- `{f}`\n"));
            }
            entry.push('\n');
        }

        entry.push_str(&format!("**结果**：{result_str}\n\n"));

        if !fixes.is_empty() {
            entry.push_str("**修复记录**：\n");
            for (i, (err, fix)) in fixes.iter().enumerate() {
                entry.push_str(&format!("- 第{}次：{} → {}\n", i + 1, err, fix));
            }
            entry.push('\n');
        }

        entry.push_str("---\n\n");

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)?;
        file.write_all(entry.as_bytes())?;
        Ok(())
    }
}
