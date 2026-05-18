use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

pub struct Executor {
    pub work_dir: PathBuf,
    forbidden: Vec<String>,
}

impl Executor {
    pub fn new(work_dir: PathBuf) -> Self {
        Executor {
            work_dir,
            forbidden: vec![
                "format".to_string(),
                "diskpart".to_string(),
                "rm -rf /".to_string(),
                r"del /f /s /q C:\".to_string(),
            ],
        }
    }

    pub fn run(&self, cmd: &str) -> Result<(bool, String, f64)> {
        let cmd_lower = cmd.to_lowercase();
        for forbidden in &self.forbidden {
            if cmd_lower.contains(&forbidden.to_lowercase()) {
                return Err(anyhow!("Command '{}' is forbidden", cmd));
            }
        }

        let start = Instant::now();

        #[cfg(target_os = "windows")]
        let output = Command::new("cmd")
            .args(["/c", cmd])
            .current_dir(&self.work_dir)
            .output();

        #[cfg(not(target_os = "windows"))]
        let output = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&self.work_dir)
            .output();

        let elapsed = start.elapsed().as_secs_f64();

        match output {
            Ok(out) => {
                let mut combined = String::new();
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                if !stdout.is_empty() {
                    combined.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !combined.is_empty() {
                        combined.push('\n');
                    }
                    combined.push_str(&stderr);
                }
                Ok((out.status.success(), combined, elapsed))
            }
            Err(e) => Ok((false, e.to_string(), elapsed)),
        }
    }
}
