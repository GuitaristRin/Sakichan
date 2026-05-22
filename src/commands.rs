use crate::config::{active_config_path, save_config_field};
use crate::display::*;
use crate::slots::{probe_ollama_models, resolve_slot_model, SlotRole};
use crate::state::AppState;
use anyhow::Result;
use chrono::Local;
use std::collections::HashMap;
use std::fs;
use std::process::Command;
use std::sync::{Arc, Mutex};

// ── i18n ──────────────────────────────────────────────────────────────────────

pub fn get_i18n() -> HashMap<String, HashMap<String, String>> {
    let mut map: HashMap<String, HashMap<String, String>> = HashMap::new();

    let mut zh = HashMap::new();
    zh.insert("welcome".into(), "欢迎使用 Saki-chan！".into());
    zh.insert("prompt_readonly".into(), "🔒 [只读] > ".into());
    zh.insert("prompt_edit".into(), "✏️  [读写] > ".into());
    zh.insert("goodbye".into(), "再见！/ Goodbye!".into());
    zh.insert("help_title".into(), "📖 帮助 / Help".into());
    zh.insert("analyzing".into(), "🔍 分析需求中...".into());
    zh.insert("model_selected".into(), "切换到模型".into());
    zh.insert("locked".into(), "⚠️  只读模式，文件操作已禁用。使用 /edit 开启编辑。".into());
    zh.insert("edit_on".into(), "✏️  始终执行模式已开启（执行前不再询问确认）".into());
    zh.insert("edit_off".into(), "🔒 已关闭：每次执行前会询问确认".into());
    zh.insert("unknown_cmd".into(), "未知命令，输入 /help 查看帮助".into());
    map.insert("zh_TW".into(), zh);

    let mut en = HashMap::new();
    en.insert("welcome".into(), "Welcome to Saki-chan!".into());
    en.insert("prompt_readonly".into(), "🔒 [readonly] > ".into());
    en.insert("prompt_edit".into(), "✏️  [edit] > ".into());
    en.insert("goodbye".into(), "Goodbye!".into());
    en.insert("help_title".into(), "📖 Help".into());
    en.insert("analyzing".into(), "🔍 Analyzing request...".into());
    en.insert("model_selected".into(), "Switched to model".into());
    en.insert("locked".into(), "⚠️  Read-only mode. Use /edit to enable editing.".into());
    en.insert("edit_on".into(), "✏️  Always-execute enabled (no confirmation prompt)".into());
    en.insert("edit_off".into(), "🔒 Disabled: will ask for confirmation before each run".into());
    en.insert("unknown_cmd".into(), "Unknown command, type /help for help".into());
    map.insert("en".into(), en);

    map
}

pub fn t<'a>(
    i18n: &'a HashMap<String, HashMap<String, String>>,
    lang: &str,
    key: &'a str,
) -> &'a str {
    i18n.get(lang)
        .and_then(|m| m.get(key))
        .map(|s| s.as_str())
        .unwrap_or(key)
}

// ── Command handler ───────────────────────────────────────────────────────────

pub fn handle_command(
    cmd: &str,
    state: &Arc<Mutex<AppState>>,
    context: &mut Vec<String>,
    i18n: &HashMap<String, HashMap<String, String>>,
    models: &[String],
) -> Result<bool> {
    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
    let command = parts[0];
    let arg = parts.get(1).copied().unwrap_or("").trim();
    let arg2 = parts.get(2).copied().unwrap_or("").trim();

    let lang = state.lock().unwrap().lang.clone();

    match command {
        "/help" => {
            print_divider();
            println!("{CYAN}{BOLD}{}{RESET}", t(i18n, &lang, "help_title"));
            println!();
            let cmds = [
                ("/help", "显示帮助 / Show help"),
                ("/models", "列出可用模型（按插槽）/ List models by slot"),
                ("/slots", "显示插槽状态 / Show slot status"),
                ("/slot <slot> [model]", "查看或修改插槽 / View or change slot model"),
                ("/config", "显示当前配置 / Show current config"),
                ("/clear", "清空上下文 / Clear context"),
                ("/init", "生成规则文件 / Generate rules file"),
                ("/load <file>", "读取文件到上下文 / Load file to context"),
                ("/usage", "显示 token 用量 / Show token usage"),
                ("/sessions", "列出历史会话 / List sessions"),
                ("/resume <id>", "恢复会话 / Resume session"),
                ("/export <id>", "导出会话 / Export session"),
                ("/edit", "切换始终执行模式 / Toggle always-execute"),
                ("/lang <zh|en>", "切换语言 / Switch language"),
                ("/undo [n]", "回滚 git checkpoint / Rollback checkpoint(s)"),
                ("/history", "显示 sakichan 提交历史 / Show commits"),
                ("/diff", "显示当前 git diff / Show git diff --stat"),
                ("/exit", "退出 / Exit"),
            ];
            for (c, desc) in &cmds {
                println!("  {YELLOW}{c:<30}{RESET} {desc}");
            }
            print_divider();
        }

        "/models" => {
            let st = state.lock().unwrap();
            println!("{CYAN}可用模型（按插槽分组）/ Available Models by Slot:{RESET}");
            println!();
            for role in SlotRole::all() {
                let slot_name = role.name();
                let assigned = st.slot_assignments.get(slot_name).map(|s| s.as_str()).unwrap_or("(未分配)");
                let slot_cfg = st.config.slots.get(slot_name);
                let candidates: Vec<&str> = slot_cfg
                    .map(|c| c.models.iter().map(|s| s.as_str()).filter(|m| models.contains(&m.to_string())).collect())
                    .unwrap_or_default();
                print!("  {GREEN}{slot_name:<16}{RESET} → {BOLD}{assigned}{RESET}");
                if !candidates.is_empty() {
                    print!("  {GRAY}[可用: {}]{RESET}", candidates.join(", "));
                }
                println!();
            }
            println!();
        }

        "/slots" => {
            let st = state.lock().unwrap();
            print_divider();
            println!("{CYAN}{BOLD}  团队插槽状态{RESET}");
            print_divider();
            for role in SlotRole::all() {
                let slot_name = role.name();
                let assigned = st.slot_assignments.get(slot_name).map(|s| s.as_str()).unwrap_or("(未分配)");
                let slot_cfg = st.config.slots.get(slot_name);
                let avail: Vec<String> = slot_cfg
                    .map(|c| c.models.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|m| models.contains(m))
                    .collect();
                let avail_str = if avail.is_empty() { "(无可用模型)".to_string() } else { avail.join(", ") };
                println!("  {GREEN}{slot_name:<16}{RESET} → {BOLD}{assigned:<28}{RESET}  {GRAY}[可用: {avail_str}]{RESET}");
            }
            print_divider();
            let backend = state.lock().unwrap().config.backend.backend_type.clone();
            println!("  后端: {backend}");
            print_divider();
        }

        "/slot" => {
            if arg.is_empty() {
                println!("{YELLOW}用法: /slot <插槽名> [模型名]{RESET}");
                println!("  插槽名: ProductOwner, Architect, SeniorEngineer, Programmer, QA");
                return Ok(false);
            }
            let slot_name = SlotRole::from_str(arg).map(|r| r.name()).unwrap_or(arg);
            if !crate::slots::SlotRole::all().iter().any(|r| r.name() == slot_name) {
                println!("{RED}未知插槽: {arg}。有效值: ProductOwner, Architect, SeniorEngineer, Programmer, QA{RESET}");
                return Ok(false);
            }
            if arg2.is_empty() {
                // Show slot details
                let st = state.lock().unwrap();
                let assigned = st.slot_assignments.get(slot_name).map(|s| s.as_str()).unwrap_or("(未分配)");
                let slot_cfg = st.config.slots.get(slot_name);
                println!("{CYAN}插槽: {slot_name}{RESET}");
                println!("  当前模型: {GREEN}{assigned}{RESET}");
                if let Some(cfg) = slot_cfg {
                    println!("  主模型:   {}", cfg.primary);
                    if !cfg.fallback.is_empty() { println!("  备选:     {}", cfg.fallback.join(", ")); }
                    let avail: Vec<&str> = cfg.models.iter()
                        .filter(|m| models.contains(m))
                        .map(|s| s.as_str())
                        .collect();
                    if !avail.is_empty() { println!("  已安装:   {}", avail.join(", ")); }
                }
            } else {
                // Change slot model
                let model_name = arg2.to_string();
                state.lock().unwrap().slot_assignments.insert(slot_name.to_string(), model_name.clone());
                println!("{GREEN}✓ {slot_name} 插槽主模型已切换为 {model_name}{RESET}");
                if !models.contains(&model_name) {
                    println!("{YELLOW}⚠ 此模型当前在 Ollama 中不可用。请运行 ollama pull {model_name}{RESET}");
                }
            }
        }

        "/config" => {
            let st = state.lock().unwrap();
            let config_path = active_config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(默认配置)".to_string());
            print_divider();
            println!("{CYAN}{BOLD}  当前配置{RESET}");
            print_divider();
            println!("  配置文件: {config_path}");
            println!("  后端:     {}", st.config.backend.backend_type);
            println!("  Ollama:   {}", st.config.backend.ollama.host);
            println!("  语言:     {}", st.lang);
            println!("  编辑模式: {}", if st.edit_mode { "开启" } else { "关闭" });
            println!("  模型目录: {}", st.config.backend.lmcpp.model_dirs.join(", "));
            print_divider();
            println!("{CYAN}  验证策略:{RESET}");
            for v in &st.config.verification {
                let cmd = v.command.as_deref().unwrap_or("(仅架构审查)");
                println!("    {GREEN}{:<12}{RESET} → {cmd}", v.name);
            }
            print_divider();
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
            match fs::read_dir(&sessions_dir) {
                Ok(entries) => {
                    let mut sessions: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .filter_map(|e| e.file_name().into_string().ok())
                        .filter(|n| n.ends_with(".json"))
                        .collect();
                    sessions.sort();
                    if sessions.is_empty() {
                        println!("{GRAY}暂无历史会话{RESET}");
                    } else {
                        println!("{CYAN}历史会话:{RESET}");
                        for s in &sessions {
                            println!("  {GRAY}•{RESET} {}", s.trim_end_matches(".json"));
                        }
                    }
                }
                Err(_) => println!("{GRAY}暂无历史会话{RESET}"),
            }
        }

        "/resume" => {
            if arg.is_empty() {
                println!("{YELLOW}用法: /resume <id>{RESET}");
            } else {
                let work_dir = state.lock().unwrap().work_dir.clone();
                let f = work_dir.join(".sakichan").join("sessions").join(format!("{arg}.json"));
                match fs::read_to_string(&f) {
                    Ok(data) => match serde_json::from_str::<Vec<String>>(&data) {
                        Ok(messages) => {
                            *context = messages;
                            println!("{GREEN}已恢复会话: {arg} ({} 条消息){RESET}", context.len());
                        }
                        Err(_) => println!("{RED}会话格式错误{RESET}"),
                    },
                    Err(e) => println!("{RED}无法读取会话: {e}{RESET}"),
                }
            }
        }

        "/export" => {
            if arg.is_empty() {
                println!("{YELLOW}用法: /export <id>{RESET}");
            } else {
                let work_dir = state.lock().unwrap().work_dir.clone();
                let f = work_dir.join(".sakichan").join("sessions").join(format!("{arg}.json"));
                match fs::read_to_string(&f) {
                    Ok(data) => {
                        if let Ok(messages) = serde_json::from_str::<Vec<String>>(&data) {
                            let mut md = format!("# Session: {arg}\n\n");
                            for (i, msg) in messages.iter().enumerate() {
                                md.push_str(&format!("## Message {}\n\n{}\n\n", i + 1, msg));
                            }
                            let out = format!("session_{arg}.md");
                            fs::write(&out, &md)?;
                            println!("{GREEN}已导出到: {out}{RESET}");
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
            let new_lang = match arg {
                "zh" | "zh_TW" | "zh-TW" => Some("zh_TW"),
                "en" => Some("en"),
                _ => None,
            };
            if let Some(l) = new_lang {
                state.lock().unwrap().lang = l.to_string();
                println!("{GREEN}语言已切换 / Language switched: {l}{RESET}");
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
                println!("{YELLOW}没有可回滚的 checkpoint{RESET}");
            } else {
                match Command::new("git")
                    .args(["reset", "--hard", &format!("HEAD~{n}")])
                    .current_dir(&work_dir)
                    .output()
                {
                    Ok(o) if o.status.success() => {
                        state.lock().unwrap().checkpoint_count = 0;
                        println!("{GREEN}✓ 已回滚 {n} 个 checkpoint{RESET}");
                    }
                    Ok(o) => {
                        let err = String::from_utf8_lossy(&o.stderr);
                        println!("{RED}回滚失败: {}{RESET}", err.trim());
                    }
                    Err(e) => println!("{RED}Git 错误: {e}{RESET}"),
                }
            }
        }

        "/history" => {
            let work_dir = state.lock().unwrap().work_dir.clone();
            match Command::new("git")
                .args(["log", "--oneline", "--grep=sakichan", "-n", "20"])
                .current_dir(&work_dir)
                .output()
            {
                Ok(o) => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    if text.trim().is_empty() {
                        println!("{GRAY}无 sakichan 相关 commit{RESET}");
                    } else {
                        println!("{CYAN}● Git(log --grep=sakichan){RESET}");
                        for line in text.lines() {
                            println!("  {GRAY}{line}{RESET}");
                        }
                    }
                }
                Err(e) => println!("{RED}Git 错误: {e}{RESET}"),
            }
        }

        "/diff" => {
            let work_dir = state.lock().unwrap().work_dir.clone();
            match Command::new("git")
                .args(["diff", "--stat"])
                .current_dir(&work_dir)
                .output()
            {
                Ok(o) => {
                    let text = String::from_utf8_lossy(&o.stdout);
                    if text.trim().is_empty() {
                        println!("{GRAY}无未提交修改{RESET}");
                    } else {
                        println!("{CYAN}● Git(diff --stat){RESET}");
                        for line in text.lines() {
                            println!("  {GRAY}{line}{RESET}");
                        }
                    }
                }
                Err(e) => println!("{RED}Git 错误: {e}{RESET}"),
            }
        }

        "/model" => {
            println!("{YELLOW}⚠ /model 已废弃。请使用 /slot <插槽名> <模型名> 来修改插槽模型。{RESET}");
            println!("  示例: /slot Programmer qwen2.5-coder:7b");
            println!("        /slot Architect deepseek-r1:8b");
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
