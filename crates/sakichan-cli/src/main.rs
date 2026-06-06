mod keyring;
mod render;

use std::sync::Arc;

use anyhow::{Context, Result};
use base64::Engine;
use clap::Parser;
use sakichan_core::config::{self, Config};
use sakichan_core::memory::store::MemoryStore;
use sakichan_core::models::traits::ChatModel;
use sakichan_core::pipeline::{PipelineContext, PipelineEvent};
use sakichan_core::session::context::SessionContext;
use sakichan_core::session::store::{Session, SessionStore};

/// Sakichan CLI — a high-performance AI client with long-term memory.
#[derive(Parser, Debug)]
#[command(name = "sakichan", version = "0.1.0", about = "Personal AI client with long-term memory")]
struct Cli {
    /// Session ID, "create" to create new, "suspend" to save
    #[arg(long)]
    session: Option<String>,

    /// Message to send (omit to just load/display session)
    #[arg(long)]
    prompt: Option<String>,

    /// Image file paths (can be specified multiple times)
    #[arg(long)]
    image: Vec<String>,

    /// Enable DeepSeek reasoning mode (TRUE/FALSE)
    #[arg(long, default_value = "FALSE")]
    thinking: String,

    /// Create a new branch at the specified depth (requires --order)
    #[arg(long)]
    deri: Option<i64>,

    /// Order for the new branch (used with --deri)
    #[arg(long)]
    order: Option<i64>,

    /// Delete the specified session
    #[arg(long)]
    delete: bool,

    /// Manage API keys: "clear" to delete, or "sk-..." to set
    #[arg(long)]
    token: Option<String>,

    /// Generate or update session title by summarizing conversation
    #[arg(long)]
    summary: bool,

    /// Use DeepSeek V4 Flash model (default: SenseNova Flash-Lite)
    #[arg(long, default_value = "FALSE")]
    dsv4f: String,

    /// Text file paths to attach as context (can be specified multiple times)
    #[arg(long)]
    file: Vec<String>,
}

// ---------------------------------------------------------------------------
// Application state (per-invocation)
// ---------------------------------------------------------------------------

struct AppContext {
    config: Config,
    session_store: Arc<SessionStore>,
    memory_store: Arc<MemoryStore>,
}

// ---------------------------------------------------------------------------
// Initialization helpers
// ---------------------------------------------------------------------------

fn init_config() -> Result<Config> {
    let config_path = config::find_config_path()?;

    if !config_path.exists() {
        render::print_info("未检测到配置文件，正在创建默认配置...");
        let path = config::create_default_config()?;
        render::print_info(&format!("已创建默认配置文件: {}", path.display()));
    }

    let raw = config::read_config_from(&config_path)?;
    let mut config = config::resolve_config(raw)?;

    // Resolve keyring references
    for (name, entry) in config.resolved_model_configs.iter_mut() {
        if entry.api_key.starts_with("ENC_KEYRING:") {
            let resolved =
                keyring::resolve_keyring_value(&entry.api_key, name)
                    .context(format!("Failed to resolve API key for model '{name}'"))?;
            entry.api_key = resolved;
        }
    }

    Ok(config)
}

fn init_stores(config: Config) -> Result<AppContext> {
    let db_path = config.get_db_path();
    let session_store = Arc::new(
        SessionStore::open(&db_path)
            .context("Failed to open session database")?,
    );
    let memory_store = Arc::new(
        MemoryStore::open(&db_path)
            .context("Failed to open memory database")?,
    );
    Ok(AppContext {
        config,
        session_store,
        memory_store,
    })
}

fn create_chat_model(config: &Config, model_id: &str) -> Result<Box<dyn ChatModel>> {
    let entry = config
        .get_model(model_id)
        .with_context(|| format!("模型 '{model_id}' 未在配置中找到"))?;

    match model_id {
        "sensenova-flash-lite" => Ok(Box::new(
            sakichan_core::models::sensenova_flash_lite::SensenovaFlashLite::new(
                entry.api_base.clone(),
                entry.api_key.clone(),
            ),
        )),
        "deepseek-v4-flash" => Ok(Box::new(
            sakichan_core::models::deepseek_v4_flash::DeepSeekV4Flash::new(
                entry.api_base.clone(),
                entry.api_key.clone(),
                entry.extra_params.clone(),
            ),
        )),
        _ => {
            if let Some(fallback) = config.get_model("sensenova-flash-lite") {
                Ok(Box::new(
                    sakichan_core::models::sensenova_flash_lite::SensenovaFlashLite::new(
                        fallback.api_base.clone(),
                        fallback.api_key.clone(),
                    ),
                ))
            } else {
                anyhow::bail!("未知模型 '{model_id}'，且找不到备用模型")
            }
        }
    }
}

/// Read image files from paths, detect MIME type, and base64-encode them as data URIs.
fn load_images(paths: &[String]) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut images = Vec::new();
    for path in paths {
        let bytes =
            std::fs::read(path).with_context(|| format!("无法读取图片文件: {path}"))?;
        let mime = guess_mime_type(path);
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let data_uri = format!("data:{mime};base64,{encoded}");
        images.push(data_uri);
    }
    Ok(images)
}

fn guess_mime_type(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else {
        "image/jpeg"
    }
}

/// Read text files and return a formatted context string.
/// Returns None if no files are provided.
fn read_files(paths: &[String]) -> Result<Option<String>> {
    if paths.is_empty() {
        return Ok(None);
    }
    let mut parts = Vec::new();
    for path in paths {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("无法读取文件: {path}"))?;
        parts.push(format!("[文件: {}]\n{}", path, content));
    }
    Ok(Some(parts.join("\n\n---\n\n")))
}

/// Determine the model for a text-only query.
fn select_text_model(config: &Config, use_dsv4f: bool) -> &str {
    if use_dsv4f && config.get_model("deepseek-v4-flash").is_some() {
        "deepseek-v4-flash"
    } else if config.get_model("sensenova-flash-lite").is_some() {
        "sensenova-flash-lite"
    } else {
        &config.models.default_chat
    }
}

// ---------------------------------------------------------------------------
// Pipeline helpers
// ---------------------------------------------------------------------------

fn build_pipeline_context(
    ctx: &AppContext,
    session: &Session,
    model_id: &str,
) -> Result<PipelineContext> {
    let mut session_ctx = SessionContext::new(session.id.clone(), model_id.to_string());
    session_ctx.set_system_prompt(session.system_prompt.clone());

    // Load messages for the active branch
    let messages = ctx.session_store.get_branch_messages(
        &session.id,
        session.active_depth,
        session.active_order,
    )?;
    if !messages.is_empty() {
        session_ctx.load_messages(messages);
    }

    Ok(PipelineContext::new(
        session_ctx,
        ctx.session_store.clone(),
        ctx.memory_store.clone(),
        ctx.config.memory.retrieval_top_k,
    ))
}

// ---------------------------------------------------------------------------
// Session handlers
// ---------------------------------------------------------------------------

fn handle_session_create(app: &AppContext) -> Result<()> {
    let model_id = select_text_model(&app.config, false);
    let session = app
        .session_store
        .create_session(model_id, None)
        .context("Failed to create new session")?;
    render::print_info(&format!("Session Created as {}", session.id));
    Ok(())
}

fn handle_session_suspend(app: &AppContext) -> Result<()> {
    // Sessions are auto-saved after each operation.
    // We need to find the most recent session to confirm it exists.
    let sessions = app.session_store.list_sessions()?;
    if let Some(s) = sessions.first() {
        render::print_info(&format!("Session Suspended successfully, at {}.", s.id));
    } else {
        anyhow::bail!("无活跃 Session 可挂起")
    }
    Ok(())
}

fn handle_session_delete(app: &AppContext, session_id: &str) -> Result<()> {
    app.session_store
        .delete_session(session_id)
        .context("Failed to delete session")?;
    render::print_info(&format!("Session {} deleted.", session_id));
    Ok(())
}

fn handle_session_info(app: &AppContext, session_id: &str) -> Result<()> {
    let session = app
        .session_store
        .get_session(session_id)?
        .with_context(|| format!("会话不存在: {session_id}"))?;
    let title = session.title.as_deref().unwrap_or("[无标题]");
    render::print_info(&format!(
        "Session: {} | {} | depth={} order={}",
        session.id, title, session.active_depth, session.active_order
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Prompt handler
// ---------------------------------------------------------------------------

async fn handle_send_prompt(
    app: &AppContext,
    session_id: &str,
    prompt: &str,
    image_paths: Vec<String>,
    thinking: bool,
    dsv4f: bool,
    deri: Option<i64>,
    deri_order: Option<i64>,
) -> Result<()> {
    // Load session
    let mut session = app
        .session_store
        .get_session(session_id)?
        .with_context(|| format!("会话不存在: {session_id}"))?;

    let images = load_images(&image_paths)?;

    // Deri/branching logic
    let (active_depth, active_order) = if let Some(d) = deri {
        let o = match deri_order {
            Some(o) => o,
            None => app.session_store.get_next_order(session_id, d)?,
        };
        let existing = app.session_store.get_branch_messages(session_id, d, o)?;
        if !existing.is_empty() {
            let available = app.session_store.get_next_order(session_id, d)?;
            anyhow::bail!(
                "order {} 在 depth {} 处已被占用。可用 order: {}",
                o, d, available
            );
        }
        (d, o)
    } else {
        let max_depth = app.session_store.get_max_depth(session_id)?;
        let next_depth = max_depth + 1;
        (next_depth, 1)
    };

    // Update session active branch
    app.session_store
        .set_active_branch(session_id, active_depth, active_order)?;
    session.active_depth = active_depth;
    session.active_order = active_order;

    // --- Determine model pipeline ---
    if dsv4f && !images.is_empty() {
        // === DSV4F + IMAGE: two-stage pipeline (SenseNova analyse -> DeepSeek answer) ===
        let analyse_model = create_chat_model(&app.config, "sensenova-flash-lite")?;
        let text_model = create_chat_model(&app.config, "deepseek-v4-flash")?;

        render::print_model(&format!(
            "Image Files detected,using {}",
            analyse_model.model_id()
        ));

        let mut pipeline_ctx = build_pipeline_context(app, &session, "deepseek-v4-flash")?;
        pipeline_ctx.session_ctx.set_branch_mode(active_depth, active_order);

        // Stage 1: Analyse image
        render::print_thinking_header(analyse_model.model_id());
        let analyse_msgs = vec![
            sakichan_core::models::Message::system("你是一个图像分析助手。请详细描述用户图片中的内容。"),
            sakichan_core::models::Message::user_with_images(
                "请详细描述这张图片的内容。",
                images.clone(),
            ),
        ];

        let mut description = String::new();
        let mut analyse_line = render::StreamingLine::new("[ANALYSE]");
        analyse_model
            .chat_stream(&analyse_msgs, &sakichan_core::models::ChatOptions::default(), &mut |event| {
                if let sakichan_core::models::StreamEvent::Token(t) = event {
                    description.push_str(&t);
                    analyse_line.stream(&t);
                }
            })
            .await?;
        analyse_line.finish();

        // Stage 2: Inject description and call DeepSeek
        render::print_info("Get image description,send to text-only model.");
        render::print_info("Calling LLM");
        render::print_model(&format!("Text mission,using {}", text_model.model_id()));

        let desc_prompt = format!(
            "用户上传了一张新的图片。以下是该图片的分析结果，请基于此回答用户的问题：\n\n\
             图片分析：{}\n\n用户问题：{}",
            description.trim(),
            prompt
        );

        let user_msg = sakichan_core::models::Message::user_with_images(prompt, images);
        app.session_store
            .add_message(&session.id, &user_msg, active_depth, active_order)?;
        app.session_store
            .set_active_branch(&session.id, active_depth, active_order)?;

        pipeline_ctx.session_ctx.add_user_message_with_images(prompt, Vec::new());
        pipeline_ctx.session_ctx.inject_context_prompt(&desc_prompt);

        let text_messages = pipeline_ctx.session_ctx.build_messages_for_model();
        let mut final_content = String::new();
        let final_reasoning: Option<String> = None;
        let mut result_line = render::StreamingLine::new("[RESULT]");
        let mut thinking_line = render::StreamingLine::new("[THINKING]");

        let model_result = text_model
            .chat_stream(&text_messages, &sakichan_core::models::ChatOptions::default(), &mut |event| {
                match event {
                    sakichan_core::models::StreamEvent::Token(t) => {
                        final_content.push_str(&t);
                        result_line.stream(&t);
                    }
                    sakichan_core::models::StreamEvent::ReasoningToken(t) if thinking => {
                        thinking_line.stream(&t);
                    }
                    _ => {}
                }
            })
            .await?;

        if !thinking_line.is_empty() {
            thinking_line.finish();
        }
        result_line.finish();

        pipeline_ctx.session_ctx.add_assistant_message(&final_content);
        let assistant_msg = sakichan_core::models::Message {
            role: "assistant".to_string(),
            content: final_content.clone(),
            reasoning_content: final_reasoning.clone(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: Vec::new(),
        };
        app.session_store
            .add_message(&session.id, &assistant_msg, active_depth, active_order)?;

        let final_result = sakichan_core::models::traits::FinalResult {
            full_content: final_content,
            reasoning_content: final_reasoning,
            usage: model_result.usage,
            finish_reason: model_result.finish_reason,
        };
        render::print_usage(&final_result);

    } else {
        // === DEFAULT / DSV4F TEXT: Direct pipeline.run() ===
        // dsv4f=FALSE (any): SenseNova
        // dsv4f=TRUE + no images: DeepSeek
        let text_model_id = if dsv4f {
            "deepseek-v4-flash"
        } else {
            "sensenova-flash-lite"
        };
        let model = create_chat_model(&app.config, text_model_id)?;

        render::print_model(&format!("Using {}", model.model_id()));

        let mut pipeline_ctx = build_pipeline_context(app, &session, text_model_id)?;
        pipeline_ctx.session_ctx.set_branch_mode(active_depth, active_order);

        render::print_thinking_header(text_model_id);

        let mut result_line = render::StreamingLine::new("[RESULT]");
        let mut thinking_line = render::StreamingLine::new("[THINKING]");

        let result = pipeline_ctx
            .run(prompt, images, &*model, &mut |event| {
                match &event {
                    PipelineEvent::Token(t) => {
                        result_line.stream(t);
                    }
                    PipelineEvent::ReasoningToken(t) if thinking => {
                        thinking_line.stream(t);
                    }
                    PipelineEvent::Done(_) => {
                        if !thinking_line.is_empty() {
                            thinking_line.finish();
                        }
                        result_line.finish();
                    }
                    _ => {}
                }
            })
            .await;

        match &result {
            Ok(_final_result) => {
                render::print_usage(_final_result);
            }
            Err(e) => {
                render::print_info(&format!("生成回复失败: {e}"));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Session list and summary handlers
// ---------------------------------------------------------------------------

fn handle_session_list(app: &AppContext) -> Result<()> {
    let sessions = app.session_store.list_sessions()?;
    if sessions.is_empty() {
        render::print_info("暂无会话记录。");
        return Ok(());
    }
    for session in &sessions {
        let time = chrono::DateTime::from_timestamp(session.updated_at, 0)
            .map(|dt| dt.format("%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let title = session.title.as_deref();
        render::print_session_entry(&session.id, title, &time);
    }
    Ok(())
}

async fn handle_session_summary(app: &AppContext, session_id: &str) -> Result<()> {
    // Verify session exists
    app.session_store
        .get_session(session_id)?
        .with_context(|| format!("会话不存在: {session_id}"))?;

    let messages = app.session_store.get_messages(session_id)?;
    if messages.len() < 2 {
        render::print_info("消息太少，无法生成标题");
        return Ok(());
    }

    let text_model_id = select_text_model(&app.config, false);
    let model = create_chat_model(&app.config, text_model_id)?;

    render::print_info("正在生成会话标题...");

    let title = sakichan_core::pipeline::generate_session_title(&*model, &messages).await?;

    if !title.is_empty() {
        app.session_store.update_title(session_id, &title)?;
        render::print_session_entry(session_id, Some(&title), "");
    } else {
        render::print_info("无法生成标题");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .init();

    let cli = Cli::parse();

    // Validate: at least one of --session or --token must be provided
    if cli.session.is_none() && cli.token.is_none() {
        anyhow::bail!("至少需要提供 --session 或 --token 参数\n使用 --help 查看用法");
    }

    // Handle --token (API key management) before session logic
    if let Some(token_val) = &cli.token {
        match token_val.as_str() {
            "clear" => {
                let count = keyring::clear_all_api_keys()
                    .context("Failed to clear API keys")?;
                render::print_info(&format!("已清除 {} 个 API Key", count));
            }
            v if v.starts_with("sk-") => {
                let count = keyring::set_all_api_keys(v)
                    .context("Failed to set API keys")?;
                render::print_info(&format!("已设置 {} 个模型的 API Key", count));
            }
            _ => {
                anyhow::bail!("--token 参数无效。使用 'clear' 清除，或 'sk-xxx' 设置 API Key。");
            }
        }
        // If only --token was provided (no --session), exit early
        if cli.session.is_none() {
            return Ok(());
        }
    }

    let app = init_config()?;
    let app = init_stores(app)?;

    let session_id = cli.session.as_deref().unwrap(); // Safe: checked above
    let session_val = session_id.trim().to_lowercase();

    match session_val.as_str() {
        "create" => {
            handle_session_create(&app)?;
        }
        "suspend" => {
            handle_session_suspend(&app)?;
        }
        "list" => {
            handle_session_list(&app)?;
        }
        _ => {
            // It's a session ID
            if cli.delete {
                handle_session_delete(&app, session_id)?;
            } else if cli.summary {
                handle_session_summary(&app, session_id).await?;
            } else if let Some(prompt) = &cli.prompt {
                let thinking = cli.thinking.to_uppercase() == "TRUE";
                let dsv4f = cli.dsv4f.to_uppercase() == "TRUE";

                // Read file attachments and prepend to prompt
                let file_context = read_files(&cli.file)?;
                let full_prompt = if let Some(ref fc) = file_context {
                    format!("{}\n\n---\n\n{}", fc, prompt)
                } else {
                    prompt.clone()
                };

                handle_send_prompt(
                    &app,
                    session_id,
                    &full_prompt,
                    cli.image,
                    thinking,
                    dsv4f,
                    cli.deri,
                    cli.order,
                )
                .await?;
            } else {
                // Just show session info
                handle_session_info(&app, session_id)?;
            }
        }
    }

    Ok(())
}