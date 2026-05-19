use crate::display::*;
use crate::state::AppState;
use anyhow::Result;
use chrono::Local;
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};

pub fn get_i18n() -> HashMap<String, HashMap<String, String>> {
    let mut map: HashMap<String, HashMap<String, String>> = HashMap::new();

    let mut zh = HashMap::new();
    zh.insert("welcome".into(), "欢迎使用 Saki-chan！".into());
    zh.insert("prompt_readonly".into(), "🔒 [只读] > ".into());
    zh.insert("prompt_edit".into(), "✏️  [读写] > ".into());
    zh.insert("goodbye".into(), "再见！/ Goodbye!".into());
    zh.insert("help_title".into(), "📖 帮助 / Help".into());
    zh.insert("analyzing".into(), "🔍 分析需求中...".into());
    zh.insert("complexity".into(), "复杂度评分".into());
    zh.insert("model_selected".into(), "切换到模型".into());
    zh.insert("clarifying".into(), "🤔 需要澄清...".into());
    zh.insert("locked".into(), "⚠️  只读模式，文件操作已禁用。使用 /edit 开启编辑。".into());
    zh.insert("planning".into(), "📋 规划步骤中...".into());
    zh.insert("crafting_step".into(), "🔨 执行步骤".into());
    zh.insert("saved_file".into(), "💾 已保存".into());
    zh.insert("compile_pass".into(), "✅ 编译通过".into());
    zh.insert("compile_fail".into(), "❌ 编译失败，修复中...".into());
    zh.insert("fixing".into(), "🔧 修复错误中...".into());
    zh.insert("build_done".into(), "🎉 构建完成！".into());
    zh.insert("log_updated".into(), "📝 日志已更新".into());
    zh.insert("rules_updated".into(), "📄 规则文件已更新".into());
    zh.insert("edit_on".into(), "✏️  编辑模式已开启".into());
    zh.insert("edit_off".into(), "🔒 编辑模式已关闭".into());
    zh.insert("unknown_cmd".into(), "未知命令，输入 /help 查看帮助".into());
    map.insert("zh_TW".into(), zh);

    let mut en = HashMap::new();
    en.insert("welcome".into(), "Welcome to Saki-chan!".into());
    en.insert("prompt_readonly".into(), "🔒 [readonly] > ".into());
    en.insert("prompt_edit".into(), "✏️  [edit] > ".into());
    en.insert("goodbye".into(), "Goodbye!".into());
    en.insert("help_title".into(), "📖 Help".into());
    en.insert("analyzing".into(), "🔍 Analyzing request...".into());
    en.insert("complexity".into(), "Complexity score".into());
    en.insert("model_selected".into(), "Switched to model".into());
    en.insert("clarifying".into(), "🤔 Clarifying...".into());
    en.insert("locked".into(), "⚠️  Read-only mode. Use /edit to enable editing.".into());
    en.insert("planning".into(), "📋 Planning steps...".into());
    en.insert("crafting_step".into(), "🔨 Executing step".into());
    en.insert("saved_file".into(), "💾 Saved".into());
    en.insert("compile_pass".into(), "✅ Compile passed".into());
    en.insert("compile_fail".into(), "❌ Compile failed, fixing...".into());
    en.insert("fixing".into(), "🔧 Fixing errors...".into());
    en.insert("build_done".into(), "🎉 Build complete!".into());
    en.insert("log_updated".into(), "📝 Log updated".into());
    en.insert("rules_updated".into(), "📄 Rules file updated".into());
    en.insert("edit_on".into(), "✏️  Edit mode enabled".into());
    en.insert("edit_off".into(), "🔒 Edit mode disabled".into());
    en.insert("unknown_cmd".into(), "Unknown command, type /help for help".into());
    map.insert("en".into(), en);

    map
}

pub fn t<'a>(i18n: &'a HashMap<String, HashMap<String, String>>, lang: &str, key: &'a str) -> &'a str {
    i18n.get(lang)
        .and_then(|m| m.get(key))
        .map(|s| s.as_str())
        .unwrap_or(key)
}

pub fn handle_command(
    cmd: &str,
    state: &Arc<Mutex<AppState>>,
    context: &mut Vec<String>,
    i18n: &HashMap<String, HashMap<String, String>>,
    models: &[String],
) -> Result<bool> {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
    let command = parts[0];
    let arg = parts.get(1).copied().unwrap_or("").trim();

    let lang = state.lock().unwrap().lang.clone();

    match command {
        "/help" => {
            print_divider();
            println!("{CYAN}{BOLD}{}{RESET}", t(i18n, &lang, "help_title"));
            println!();
            let cmds = [
                ("/help", "显示帮助 / Show help"),
                ("/models", "列出可用模型 / List models"),
                ("/model <name>", "切换模型 / Switch model"),
                ("/clear", "清空上下文 / Clear context"),
                ("/init", "生成规则文件 / Generate rules file"),
                ("/load <file>", "读取文件到上下文 / Load file to context"),
                ("/usage", "显示 token 用量 / Show token usage"),
                ("/sessions", "列出历史会话 / List sessions"),
                ("/resume <id>", "恢复会话 / Resume session"),
                ("/export <id>", "导出会话 / Export session"),
                ("/edit", "切换编辑模式 / Toggle edit mode"),
                ("/lang <zh|en>", "切换语言 / Switch language"),
                ("/undo [n]", "回滚 git checkpoint / Rollback checkpoint(s)"),
                ("/history", "显示 sakichan 提交历史 / Show sakichan commits"),
                ("/diff", "显示当前 git diff / Show git diff --stat"),
                ("/exit", "退出 / Exit"),
            ];
            for (c, desc) in &cmds {
                println!("  {YELLOW}{c:<20}{RESET} {desc}");
            }
            print_divider();
        }
        "/models" => {
            println!("{CYAN}可用模型 / Available Models:{RESET}");
            for m in models {
                let current = state.lock().unwrap().current_model.clone();
                let marker = if m == &current { " ← current" } else { "" };
                println!("  {GRAY}•{RESET} {m}{GREEN}{marker}{RESET}");
            }
        }
        "/model" => {
            if arg.is_empty() {
                println!("{YELLOW}用法: /model <name>{RESET}");
            } else {
                state.lock().unwrap().current_model = arg.to_string();
                println!("{GREEN}{} {arg}{RESET}", t(i18n, &lang, "model_selected"));
            }
        }
        "/clear" => {
            context.clear();
            println!("{GREEN}上下文已清空 / Context cleared{RESET}");
        }
        "/init" => {
            let work_dir = state.lock().unwrap().work_dir.clone();
            let rules_file = work_dir.join(".sakichan.md");
            let template = "# .sakichan.md - 项目规则\n\n## 项目概述\n<!-- 项目描述 -->\n\n## 目录结构\n<!-- 说明 -->\n\n## 编码规范\n<!-- 规则 -->\n\n## 禁止事项\n<!-- 不允许的操作 -->\n";
            fs::write(&rules_file, template)?;
            println!("{GREEN}已创建 .sakichan.md{RESET}");
        }
        "/load" => {
            if arg.is_empty() {
                println!("{YELLOW}用法: /load <file>{RESET}");
            } else {
                match fs::read_to_string(arg) {
                    Ok(content) => {
                        context.push(format!("=== File: {arg} ===\n{content}"));
                        println!("{GREEN}已加载: {arg} ({} bytes){RESET}", content.len());
                    }
                    Err(e) => println!("{RED}读取失败: {e}{RESET}"),
                }
            }
        }
        "/usage" => {
            let st = state.lock().unwrap();
            let today = Local::now().format("%Y-%m-%d").to_string();
            let daily = st.usage.daily.get(&today).cloned().unwrap_or_default();
            let total = &st.usage.total;
            println!();
            println!("{CYAN}📊 今日用量 ({today}){RESET}");
            println!("  输入: {:>10} tokens", format_num(daily.input_tokens));
            println!("  输出: {:>10} tokens", format_num(daily.output_tokens));
            println!("  合计: {:>10} tokens", format_num(daily.input_tokens + daily.output_tokens));
            println!();
            println!("{CYAN}📊 总用量{RESET}");
            println!("  输入: {:>10} tokens", format_num(total.input_tokens));
            println!("  输出: {:>10} tokens", format_num(total.output_tokens));
            println!("  合计: {:>10} tokens", format_num(total.input_tokens + total.output_tokens));
            println!();
        }
        "/sessions" => {
            let work_dir = state.lock().unwrap().work_dir.clone();
            let sessions_dir = work_dir.join(".sakichan").join("sessions");
            if let Ok(entries) = fs::read_dir(&sessions_dir) {
                let mut sessions: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| n.ends_with(".json"))
                    .collect();
                sessions.sort();
                if sessions.is_empty() {
                    println!("{GRAY}暂无历史会话 / No sessions yet{RESET}");
                } else {
                    println!("{CYAN}历史会话 / Sessions:{RESET}");
                    for s in &sessions {
                        let id = s.trim_end_matches(".json");
                        println!("  {GRAY}•{RESET} {id}");
                    }
                }
            } else {
                println!("{GRAY}暂无历史会话 / No sessions yet{RESET}");
            }
        }
        "/resume" => {
            if arg.is_empty() {
                println!("{YELLOW}用法: /resume <id>{RESET}");
            } else {
                let work_dir = state.lock().unwrap().work_dir.clone();
                let session_file = work_dir.join(".sakichan").join("sessions").join(format!("{arg}.json"));
                match fs::read_to_string(&session_file) {
                    Ok(data) => {
                        if let Ok(messages) = serde_json::from_str::<Vec<String>>(&data) {
                            *context = messages;
                            println!("{GREEN}已恢复会话: {arg} ({} 条消息){RESET}", context.len());
                        } else {
                            println!("{RED}会话格式错误{RESET}");
                        }
                    }
                    Err(e) => println!("{RED}无法读取会话: {e}{RESET}"),
                }
            }
        }
        "/export" => {
            if arg.is_empty() {
                println!("{YELLOW}用法: /export <id>{RESET}");
            } else {
                let work_dir = state.lock().unwrap().work_dir.clone();
                let session_file = work_dir.join(".sakichan").join("sessions").join(format!("{arg}.json"));
                match fs::read_to_string(&session_file) {
                    Ok(data) => {
                        if let Ok(messages) = serde_json::from_str::<Vec<String>>(&data) {
                            let mut md = format!("# Session: {arg}\n\n");
                            for (i, msg) in messages.iter().enumerate() {
                                md.push_str(&format!("## Message {}\n\n{}\n\n", i + 1, msg));
                            }
                            let out_file = format!("session_{arg}.md");
                            fs::write(&out_file, &md)?;
                            println!("{GREEN}已导出到: {out_file}{RESET}");
                        }
                    }
                    Err(e) => println!("{RED}无法读取会话: {e}{RESET}"),
                }
            }
        }
        "/edit" => {
            let mut st = state.lock().unwrap();
            st.edit_mode = !st.edit_mode;
            if st.edit_mode {
                println!("{GREEN}{}{RESET}", t(i18n, &lang, "edit_on"));
            } else {
                println!("{YELLOW}{}{RESET}", t(i18n, &lang, "edit_off"));
            }
        }
        "/lang" => {
            if arg == "zh" || arg == "zh_TW" {
                state.lock().unwrap().lang = "zh_TW".to_string();
                println!("{GREEN}语言已切换为中文{RESET}");
            } else if arg == "en" {
                state.lock().unwrap().lang = "en".to_string();
                println!("{GREEN}Language switched to English{RESET}");
            } else {
                println!("{YELLOW}用法: /lang <zh|en>{RESET}");
            }
        }
        "/undo" => {
            let (work_dir, checkpoint_count) = {
                let st = state.lock().unwrap();
                (st.work_dir.clone(), st.checkpoint_count)
            };
            let n: u32 = if arg.is_empty() {
                checkpoint_count
            } else {
                arg.parse().unwrap_or(1)
            };
            if n == 0 {
                println!("{YELLOW}没有可回滚的 checkpoint / No checkpoints to undo{RESET}");
            } else {
                let out = Command::new("git")
                    .args(["reset", "--hard", &format!("HEAD~{n}")])
                    .current_dir(&work_dir)
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        state.lock().unwrap().checkpoint_count = 0;
                        println!("{GREEN}✓ 已回滚 {n} 个 checkpoint / Rolled back {n} checkpoint(s){RESET}");
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        println!("{RED}回滚失败 / Rollback failed: {}{RESET}", err.trim());
                    }
                    Err(e) => println!("{RED}Git 错误 / Git error: {e}{RESET}"),
                }
            }
        }
        "/history" => {
            let work_dir = state.lock().unwrap().work_dir.clone();
            let out = Command::new("git")
                .args(["log", "--oneline", "--grep=sakichan", "-n", "20"])
                .current_dir(&work_dir)
                .output();
            match out {
                Ok(o) => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    if text.trim().is_empty() {
                        println!("{GRAY}无 sakichan 相关 commit / No sakichan commits found{RESET}");
                    } else {
                        println!("{CYAN}● Git(log --grep=sakichan){RESET}");
                        for line in text.lines() {
                            println!("  {GRAY}{line}{RESET}");
                        }
                    }
                }
                Err(e) => println!("{RED}Git 错误 / Git error: {e}{RESET}"),
            }
        }
        "/diff" => {
            let work_dir = state.lock().unwrap().work_dir.clone();
            let out = Command::new("git")
                .args(["diff", "--stat"])
                .current_dir(&work_dir)
                .output();
            match out {
                Ok(o) => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    if text.trim().is_empty() {
                        println!("{GRAY}无未提交修改 / No uncommitted changes{RESET}");
                    } else {
                        println!("{CYAN}● Git(diff --stat){RESET}");
                        for line in text.lines() {
                            println!("  {GRAY}{line}{RESET}");
                        }
                    }
                }
                Err(e) => println!("{RED}Git 错误 / Git error: {e}{RESET}"),
            }
        }
        "/exit" | "/quit" => {
            println!("{PINK}{}{RESET}", t(i18n, &lang, "goodbye"));
            return Ok(true);
        }
        _ => {
            println!("{YELLOW}{}{RESET}", t(i18n, &lang, "unknown_cmd"));
        }
    }
    Ok(false)
}

fn format_num(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
