#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GBox, Button, CssProvider,
    Entry, EventControllerKey, FileDialog, FileFilter, GestureClick, Label,
    ListBox, ListBoxRow, Orientation, Overlay, PolicyType, Popover,
    Revealer, RevealerTransitionType, ScrolledWindow, Spinner, TextBuffer,
    TextView, ToggleButton, WrapMode, STYLE_PROVIDER_PRIORITY_APPLICATION,
};

use sakichan_core::config::{self, Config};
use sakichan_core::memory::store::MemoryStore;
use sakichan_core::models::traits::{ChatModel, ChatOptions, Message as CoreMessage, StreamEvent};
use sakichan_core::pipeline::{PipelineContext, PipelineEvent};
use sakichan_core::session::context::SessionContext;
use sakichan_core::session::store::{Session, SessionStore};

// ─────────────────────────────────────────
// Global tokio runtime
// ─────────────────────────────────────────

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn rt() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

// ─────────────────────────────────────────
// Application state
// ─────────────────────────────────────────

struct AppState {
    session_store: Arc<SessionStore>,
    memory_store:  Arc<MemoryStore>,
    config:        Arc<Config>,
    current_sid:   Option<String>,
    pending_imgs:  Vec<(String, String)>,
    pending_files: Vec<String>,
    is_generating: bool,
}

type State = Rc<RefCell<AppState>>;

// Shared UI references passed through helpers
#[derive(Clone)]
struct PanelCtx {
    chat_box:     GBox,
    chat_scroll:  ScrolledWindow,
    sess_title:   Label,
    session_list: ListBox,
    send_btn:     Button,
    dsv4f_btn:    ToggleButton,
}

impl AppState {
    fn init() -> anyhow::Result<Self> {
        let path = config::find_config_path()?;
        if !path.exists() {
            config::create_default_config()?;
        }
        let raw = config::read_config_from(&path)?;
        let mut cfg = config::resolve_config(raw)?;
        for (name, entry) in cfg.resolved_model_configs.iter_mut() {
            if entry.api_key.starts_with("ENC_KEYRING:") {
                let k = entry.api_key.clone();
                entry.api_key = keyring_get(&k, name);
            }
        }
        let db = cfg.get_db_path();
        let ss = Arc::new(SessionStore::open(&db)?);
        let ms = Arc::new(MemoryStore::open(&db)?);
        Ok(Self {
            session_store: ss,
            memory_store:  ms,
            config:        Arc::new(cfg),
            current_sid:   None,
            pending_imgs:  Vec::new(),
            pending_files: Vec::new(),
            is_generating: false,
        })
    }
}

// ─────────────────────────────────────────
// Keyring helpers
// ─────────────────────────────────────────

fn keyring_get(enc: &str, model: &str) -> String {
    let rest = &enc["ENC_KEYRING:".len()..];
    let (svc, acct) = rest.split_once('/').unwrap_or(("com.sakichan", rest));
    keyring::Entry::new(svc, acct)
        .ok()
        .and_then(|e| e.get_password().ok())
        .unwrap_or_else(|| {
            eprintln!("[warn] no keyring entry for {model}");
            String::new()
        })
}

fn has_api_key() -> bool {
    keyring::Entry::new("com.sakichan", "sensenova")
        .ok()
        .and_then(|e| e.get_password().ok())
        .map(|k| !k.is_empty())
        .unwrap_or(false)
}

fn set_api_key_all(key: &str) {
    for acct in &["sensenova", "deepseek"] {
        if let Ok(e) = keyring::Entry::new("com.sakichan", acct) {
            let _ = e.set_password(key);
        }
    }
}

fn clear_api_key_all() {
    for acct in &["sensenova", "deepseek"] {
        if let Ok(e) = keyring::Entry::new("com.sakichan", acct) {
            let _ = e.delete_password();
        }
    }
}

fn reload_keys_in_config(state: &State) {
    let path = match config::find_config_path() {
        Ok(p) => p,
        Err(_) => return,
    };
    let raw = match config::read_config_from(&path) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut cfg = match config::resolve_config(raw) {
        Ok(c) => c,
        Err(_) => return,
    };
    for (name, entry) in cfg.resolved_model_configs.iter_mut() {
        if entry.api_key.starts_with("ENC_KEYRING:") {
            let k = entry.api_key.clone();
            entry.api_key = keyring_get(&k, name);
        }
    }
    state.borrow_mut().config = Arc::new(cfg);
}

// ─────────────────────────────────────────
// Model factory
// ─────────────────────────────────────────

fn make_model(cfg: &Config, model_id: &str) -> anyhow::Result<Box<dyn ChatModel>> {
    let entry = cfg
        .get_model(model_id)
        .ok_or_else(|| anyhow::anyhow!("model '{model_id}' not configured"))?;
    Ok(match model_id {
        "deepseek-v4-flash" => Box::new(
            sakichan_core::models::deepseek_v4_flash::DeepSeekV4Flash::new(
                entry.api_base.clone(),
                entry.api_key.clone(),
                entry.extra_params.clone(),
            ),
        ),
        _ => Box::new(
            sakichan_core::models::sensenova_flash_lite::SensenovaFlashLite::new(
                entry.api_base.clone(),
                entry.api_key.clone(),
            ),
        ),
    })
}

async fn call_model_text(
    cfg: Arc<Config>,
    model_id: &str,
    messages: Vec<CoreMessage>,
) -> Result<String, String> {
    let model = make_model(&cfg, model_id).map_err(|e| e.to_string())?;
    let opts = ChatOptions::default();
    let mut result = String::new();
    model
        .chat_stream(&messages, &opts, &mut |ev| {
            if let StreamEvent::Token(t) = ev {
                result.push_str(&t);
            }
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(result)
}

// ─────────────────────────────────────────
// Utilities
// ─────────────────────────────────────────

fn image_to_data_uri(path: &PathBuf) -> anyhow::Result<String> {
    let bytes = std::fs::read(path)?;
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png")  => "image/png",
        Some("gif")  => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp")  => "image/bmp",
        _            => "image/jpeg",
    };
    let enc = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{enc}"))
}

fn scroll_to_bottom(scroll: &ScrolledWindow) {
    let adj = scroll.vadjustment();
    glib::idle_add_local_once(move || {
        adj.set_value(adj.upper() - adj.page_size());
    });
}

// ─────────────────────────────────────────
// Markdown rendering
// ─────────────────────────────────────────

fn escape_pango(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn flush_pango_label(container: &GBox, markup: &str) {
    let text = markup.trim_matches('\n').trim();
    if text.is_empty() { return; }
    let lbl = Label::new(None);
    lbl.add_css_class("message-text");
    lbl.set_markup(text);
    lbl.set_wrap(true);
    lbl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    lbl.set_xalign(0.0);
    lbl.set_yalign(0.0);
    lbl.set_selectable(true);
    lbl.set_hexpand(true);
    container.append(&lbl);
}

fn make_code_block(code: &str, lang: &str) -> GBox {
    let outer = GBox::new(Orientation::Vertical, 0);
    outer.add_css_class("code-block");

    let hdr = GBox::new(Orientation::Horizontal, 0);
    hdr.add_css_class("code-block-header");
    if !lang.is_empty() {
        let ll = Label::new(Some(lang));
        ll.add_css_class("code-lang");
        hdr.append(&ll);
    }
    let spacer = GBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    hdr.append(&spacer);

    let copy_btn = Button::with_label("复制");
    copy_btn.add_css_class("code-copy-btn");
    let code_owned = code.trim_end_matches('\n').to_string();
    copy_btn.connect_clicked(move |btn| {
        if let Some(display) = gdk4::Display::default() {
            display.clipboard().set_text(&code_owned);
            btn.set_label("已复制 ✓");
            let b2 = btn.clone();
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                b2.set_label("复制");
            });
        }
    });
    hdr.append(&copy_btn);
    outer.append(&hdr);

    let buf = TextBuffer::new(None);
    buf.set_text(code.trim_end_matches('\n'));
    let view = TextView::with_buffer(&buf);
    view.add_css_class("code-view");
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(WrapMode::None);
    view.set_left_margin(12);
    view.set_right_margin(12);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.set_monospace(true);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Automatic)
        .vscrollbar_policy(PolicyType::Never)
        .build();
    scroll.set_child(Some(&view));
    outer.append(&scroll);
    outer
}

fn build_markdown_widget(content: &str) -> GBox {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    let container = GBox::new(Orientation::Vertical, 6);
    container.set_hexpand(true);

    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(content, opts);

    let mut pango       = String::new();
    let mut code_buf    = String::new();
    let mut lang        = String::new();
    let mut in_code     = false;
    let mut list_ord    = false;
    let mut list_count  = 0u64;
    let mut in_item     = false;

    for event in parser {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_pango_label(&container, &pango);
                pango.clear();
                in_code = true;
                lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented  => String::new(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                container.append(&make_code_block(&code_buf, &lang));
                code_buf.clear();
                lang.clear();
                in_code = false;
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush_pango_label(&container, &pango);
                pango.clear();
                let (sz, wt) = match level as u8 {
                    1 => ("x-large", "ultrabold"),
                    2 => ("large",   "bold"),
                    _ => ("medium",  "bold"),
                };
                pango.push_str(&format!("<span size='{sz}' weight='{wt}'>"));
            }
            Event::End(TagEnd::Heading(_)) => {
                pango.push_str("</span>");
                flush_pango_label(&container, &pango);
                pango.clear();
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                flush_pango_label(&container, &pango);
                pango.clear();
            }
            Event::Start(Tag::List(start)) => {
                list_ord   = start.is_some();
                list_count = start.unwrap_or(1);
            }
            Event::End(TagEnd::List(_)) => {
                flush_pango_label(&container, &pango);
                pango.clear();
            }
            Event::Start(Tag::Item) => {
                flush_pango_label(&container, &pango);
                pango.clear();
                if list_ord {
                    pango.push_str(&format!("{}. ", list_count));
                    list_count += 1;
                } else {
                    pango.push_str("• ");
                }
                in_item = true;
            }
            Event::End(TagEnd::Item) => {
                flush_pango_label(&container, &pango);
                pango.clear();
                in_item = false;
            }
            Event::Start(Tag::Strong) => pango.push_str("<b>"),
            Event::End(TagEnd::Strong) => pango.push_str("</b>"),
            Event::Start(Tag::Emphasis) => pango.push_str("<i>"),
            Event::End(TagEnd::Emphasis) => pango.push_str("</i>"),
            Event::Start(Tag::Strikethrough) => pango.push_str("<s>"),
            Event::End(TagEnd::Strikethrough) => pango.push_str("</s>"),
            Event::Start(Tag::Link { .. }) | Event::End(TagEnd::Link) => {}
            Event::Code(t) => {
                pango.push_str("<tt>");
                pango.push_str(&escape_pango(&t));
                pango.push_str("</tt>");
            }
            Event::Text(t) => {
                if in_code {
                    code_buf.push_str(&t);
                } else {
                    pango.push_str(&escape_pango(&t));
                }
            }
            Event::SoftBreak => {
                if !in_item { pango.push('\n'); }
            }
            Event::HardBreak => pango.push('\n'),
            Event::Rule => {
                flush_pango_label(&container, &pango);
                pango.clear();
            }
            _ => {}
        }
    }
    flush_pango_label(&container, &pango);
    container
}

// ─────────────────────────────────────────
// Message widgets
// ─────────────────────────────────────────

// Static (non-interactive) widget for a single message.  Used as fallback.
fn msg_role_header(role: &str) -> Label {
    let role_str = match role {
        "user"      => "用户",
        "assistant" => "Sakichan",
        "system"    => "系统",
        r           => r,
    };
    let lbl = Label::new(Some(role_str));
    lbl.add_css_class("msg-role");
    if role == "assistant" { lbl.add_css_class("msg-role-assistant"); }
    lbl.set_halign(Align::Start);
    lbl
}

// Builds an interactive message widget with right-click context menu.
// `user_text_for_regen` is only relevant for assistant messages.
fn make_msg_widget_with_actions(
    msg: &CoreMessage,
    depth: i64,
    order: i64,
    user_text_for_regen: Option<String>,
    state: &State,
    ctx: &PanelCtx,
) -> GBox {
    let outer = GBox::new(Orientation::Vertical, 4);
    outer.add_css_class(if msg.role == "user" { "msg-wrapper-user" } else { "msg-wrapper-assistant" });

    outer.append(&msg_role_header(&msg.role));

    // Thinking block for historical assistant messages
    if msg.role == "assistant" {
        if let Some(ref think) = msg.reasoning_content {
            if !think.is_empty() {
                let think_area = GBox::new(Orientation::Vertical, 0);
                think_area.add_css_class("thinking-box");

                // Collapsed toggle button
                let toggle = Button::new();
                toggle.add_css_class("think-toggle-btn");
                toggle.set_label("▸ 深度思考");
                think_area.append(&toggle);

                let rev = Revealer::builder()
                    .transition_type(RevealerTransitionType::SlideDown)
                    .transition_duration(200)
                    .reveal_child(false)
                    .build();
                let tc = GBox::new(Orientation::Vertical, 0);
                tc.add_css_class("thinking-content");
                let tl = Label::new(Some(think.as_str()));
                tl.add_css_class("thinking-text");
                tl.set_wrap(true);
                tl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
                tl.set_xalign(0.0);
                tl.set_selectable(true);
                tc.append(&tl);
                rev.set_child(Some(&tc));
                think_area.append(&rev);

                let rev_c = rev.clone();
                toggle.connect_clicked(move |_| {
                    rev_c.set_reveal_child(!rev_c.reveals_child());
                });

                outer.append(&think_area);
            }
        }
    }

    // Bubble content
    let bubble = GBox::new(Orientation::Vertical, 0);
    bubble.add_css_class(if msg.role == "user" { "msg-bubble-user" } else { "msg-bubble-assistant" });

    if msg.role == "user" {
        let lbl = Label::new(Some(&msg.content));
        lbl.add_css_class("message-text");
        lbl.set_wrap(true);
        lbl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        lbl.set_xalign(0.0);
        lbl.set_yalign(0.0);
        lbl.set_selectable(true);
        lbl.set_hexpand(true);
        bubble.append(&lbl);
    } else {
        bubble.append(&build_markdown_widget(&msg.content));
    }
    outer.append(&bubble);

    // Right-click context menu
    let pop = Popover::new();
    pop.set_parent(&outer);
    pop.add_css_class("context-menu-pop");
    let pbx = GBox::new(Orientation::Vertical, 2);
    pbx.add_css_class("context-menu");

    if msg.role == "user" {
        let edit_btn = Button::with_label("修改");
        edit_btn.add_css_class("ctx-item");
        let p       = pop.clone();
        let bubble_c = bubble.clone();
        let orig    = msg.content.clone();
        let state_c = state.clone();
        let ctx_c   = ctx.clone();
        edit_btn.connect_clicked(move |_| {
            p.popdown();
            enable_edit_mode(&bubble_c, &orig, depth, &state_c, &ctx_c);
        });
        pbx.append(&edit_btn);
    } else if msg.role == "assistant" {
        if let Some(user_text) = user_text_for_regen {
            let regen_btn = Button::with_label("重新生成");
            regen_btn.add_css_class("ctx-item");
            let p        = pop.clone();
            let state_c  = state.clone();
            let ctx_c    = ctx.clone();
            regen_btn.connect_clicked(move |_| {
                p.popdown();
                if state_c.borrow().is_generating { return; }
                let sid = match state_c.borrow().current_sid.clone() {
                    Some(s) => s,
                    None => return,
                };
                let next_order = state_c.borrow().session_store
                    .get_next_order(&sid, depth)
                    .unwrap_or(order + 1);
                let dsv4f = ctx_c.dsv4f_btn.is_active();
                do_send_branch(&state_c, user_text.clone(), vec![], depth, next_order, dsv4f, &ctx_c);
            });
            pbx.append(&regen_btn);
        }
    }

    pop.set_child(Some(&pbx));

    let gc  = GestureClick::new();
    gc.set_button(3);
    let pop_c = pop.clone();
    gc.connect_pressed(move |_, _, x, y| {
        let rect = gdk4::Rectangle::new(x as i32, y as i32, 1, 1);
        pop_c.set_pointing_to(Some(&rect));
        pop_c.popup();
    });
    outer.add_controller(gc);

    outer
}

// ─────────────────────────────────────────
// Edit mode
// ─────────────────────────────────────────

fn enable_edit_mode(bubble: &GBox, original_text: &str, msg_depth: i64, state: &State, ctx: &PanelCtx) {
    while let Some(c) = bubble.first_child() { bubble.remove(&c); }

    let buf = TextBuffer::new(None);
    buf.set_text(original_text);
    let tv = TextView::with_buffer(&buf);
    tv.add_css_class("edit-input");
    tv.set_wrap_mode(WrapMode::WordChar);
    tv.set_left_margin(8);
    tv.set_right_margin(8);
    tv.set_top_margin(8);
    tv.set_bottom_margin(8);
    tv.set_hexpand(true);
    bubble.append(&tv);

    let btn_row = GBox::new(Orientation::Horizontal, 8);
    btn_row.set_halign(Align::End);
    btn_row.set_margin_top(6);
    btn_row.set_margin_bottom(4);
    let cancel_btn  = Button::with_label("取消");
    cancel_btn.add_css_class("edit-cancel-btn");
    let confirm_btn = Button::with_label("确认");
    confirm_btn.add_css_class("edit-confirm-btn");
    btn_row.append(&cancel_btn);
    btn_row.append(&confirm_btn);
    bubble.append(&btn_row);
    tv.grab_focus();

    // Enter → confirm, Shift+Enter → newline
    {
        let sb_c = confirm_btn.clone();
        let kc   = EventControllerKey::new();
        kc.connect_key_pressed(move |_, key, _, mods| {
            use gdk4::{Key, ModifierType};
            if key == Key::Return {
                if mods.contains(ModifierType::SHIFT_MASK) {
                    glib::Propagation::Proceed
                } else {
                    sb_c.emit_clicked();
                    glib::Propagation::Stop
                }
            } else {
                glib::Propagation::Proceed
            }
        });
        tv.add_controller(kc);
    }

    // Cancel: restore original text
    {
        let bubble_c = bubble.clone();
        let orig     = original_text.to_string();
        cancel_btn.connect_clicked(move |_| {
            while let Some(c) = bubble_c.first_child() { bubble_c.remove(&c); }
            let lbl = Label::new(Some(&orig));
            lbl.add_css_class("message-text");
            lbl.set_wrap(true);
            lbl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
            lbl.set_xalign(0.0);
            lbl.set_selectable(true);
            lbl.set_hexpand(true);
            bubble_c.append(&lbl);
        });
    }

    // Confirm: branch send at msg_depth with a new order
    {
        let bubble_c = bubble.clone();
        let state_c  = state.clone();
        let ctx_c    = ctx.clone();
        let buf_c    = buf.clone();
        confirm_btn.connect_clicked(move |_| {
            let text = {
                let s = buf_c.start_iter();
                let e = buf_c.end_iter();
                buf_c.text(&s, &e, false).to_string()
            };
            let text = text.trim().to_string();
            if text.is_empty() { return; }
            if state_c.borrow().is_generating { return; }

            let sid = match state_c.borrow().current_sid.clone() {
                Some(s) => s,
                None => return,
            };
            let next_order = state_c.borrow().session_store
                .get_next_order(&sid, msg_depth)
                .unwrap_or(2);

            // Clear edit mode (show that something is happening)
            while let Some(c) = bubble_c.first_child() { bubble_c.remove(&c); }
            let pending = Label::new(Some(&text));
            pending.add_css_class("message-text");
            pending.set_wrap(true);
            pending.set_xalign(0.0);
            pending.set_hexpand(true);
            bubble_c.append(&pending);

            let dsv4f = ctx_c.dsv4f_btn.is_active();
            do_send_branch(&state_c, text, vec![], msg_depth, next_order, dsv4f, &ctx_c);
        });
    }
}

// ─────────────────────────────────────────
// Branch navigation
// ─────────────────────────────────────────

fn make_branch_nav(
    session_id: &str,
    depth: i64,
    current_idx: usize,
    orders: &[i64],
    state: &State,
    ctx: &PanelCtx,
) -> GBox {
    let total = orders.len();
    let nav = GBox::new(Orientation::Horizontal, 8);
    nav.add_css_class("branch-nav");

    let left_btn = Button::with_label("←");
    left_btn.add_css_class("branch-nav-btn");
    left_btn.set_sensitive(current_idx > 0);

    let lbl = Label::new(Some(&format!("分支 {} / {}", current_idx + 1, total)));
    lbl.add_css_class("branch-nav-label");
    lbl.set_hexpand(true);
    lbl.set_halign(Align::Center);

    let right_btn = Button::with_label("→");
    right_btn.add_css_class("branch-nav-btn");
    right_btn.set_sensitive(current_idx < total - 1);

    nav.append(&left_btn);
    nav.append(&lbl);
    nav.append(&right_btn);

    let switch = {
        let sid   = session_id.to_string();
        let state = state.clone();
        let ctx   = ctx.clone();
        move |new_order: i64| {
            let _ = state.borrow().session_store.set_active_branch(&sid, depth, new_order);
            let s = state.borrow().session_store.get_session(&sid).ok().flatten();
            if let Some(sess) = s {
                state.borrow_mut().current_sid = Some(sess.id.clone());
                ctx.sess_title.set_text(sess.title.as_deref().unwrap_or("无标题"));
                load_session_messages(&ctx, &sess, &state.borrow().session_store, &state);
                scroll_to_bottom(&ctx.chat_scroll);
            }
        }
    };

    if current_idx > 0 {
        let prev  = orders[current_idx - 1];
        let sw    = switch.clone();
        left_btn.connect_clicked(move |_| sw(prev));
    }
    if current_idx < total - 1 {
        let next  = orders[current_idx + 1];
        let sw    = switch.clone();
        right_btn.connect_clicked(move |_| sw(next));
    }

    nav
}

// ─────────────────────────────────────────
// Session / chat display
// ─────────────────────────────────────────

fn clear_chat(chat_box: &GBox) {
    while let Some(c) = chat_box.first_child() {
        chat_box.remove(&c);
    }
}

fn show_welcome(chat_box: &GBox, text: &str) {
    let lbl = Label::new(Some(text));
    lbl.add_css_class("welcome-label");
    lbl.set_vexpand(true);
    lbl.set_valign(Align::Center);
    lbl.set_halign(Align::Center);
    chat_box.append(&lbl);
}

fn load_session_messages(
    ctx: &PanelCtx,
    session: &Session,
    store: &SessionStore,
    state: &State,
) {
    clear_chat(&ctx.chat_box);

    let msgs = store
        .get_branch_messages_with_coords(&session.id, session.active_depth, session.active_order)
        .unwrap_or_default();

    let orders_at_tip = if session.active_depth > 0 {
        store.get_orders_at_depth(&session.id, session.active_depth)
            .unwrap_or_default()
    } else {
        vec![]
    };

    let mut nav_inserted = false;
    let mut last_user_text = String::new();
    let mut visible_count  = 0usize;

    for (m, depth, _order) in &msgs {
        if m.role == "system" { continue; }

        // Insert branch nav before first message at active_depth (when multiple orders exist)
        if *depth == session.active_depth && !nav_inserted && orders_at_tip.len() > 1 {
            let curr_idx = orders_at_tip.iter()
                .position(|&o| o == session.active_order)
                .unwrap_or(0);
            let nav = make_branch_nav(
                &session.id, session.active_depth,
                curr_idx, &orders_at_tip,
                state, ctx,
            );
            ctx.chat_box.append(&nav);
            nav_inserted = true;
        }

        if m.role == "user" { last_user_text = m.content.clone(); }
        let user_text_for_regen = if m.role == "assistant" {
            Some(last_user_text.clone())
        } else {
            None
        };

        let w = make_msg_widget_with_actions(m, *depth, _order.to_owned(), user_text_for_regen, state, ctx);
        ctx.chat_box.append(&w);
        visible_count += 1;
    }

    // Nav at bottom if there are multiple orders but no messages yet at active_depth
    if !nav_inserted && orders_at_tip.len() > 1 {
        let curr_idx = orders_at_tip.iter()
            .position(|&o| o == session.active_order)
            .unwrap_or(0);
        let nav = make_branch_nav(
            &session.id, session.active_depth,
            curr_idx, &orders_at_tip,
            state, ctx,
        );
        ctx.chat_box.append(&nav);
    }

    if visible_count == 0 {
        show_welcome(&ctx.chat_box, "新会话已就绪，请输入消息");
    }
    scroll_to_bottom(&ctx.chat_scroll);
}

// ─────────────────────────────────────────
// Context menu async actions
// ─────────────────────────────────────────

fn refresh_session_list(list: &ListBox, state: &State, ctx: &PanelCtx) {
    while let Some(c) = list.first_child() { list.remove(&c); }
    let sessions = state.borrow().session_store.list_sessions().unwrap_or_default();
    let current  = state.borrow().current_sid.clone();

    for s in &sessions {
        let row = ListBoxRow::new();
        row.set_widget_name(&s.id);

        let inner = GBox::new(Orientation::Vertical, 3);
        inner.add_css_class("session-row-inner");

        let title_lbl = Label::new(Some(s.title.as_deref().unwrap_or("无标题")));
        title_lbl.add_css_class("session-row-title");
        title_lbl.set_halign(Align::Start);
        title_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        title_lbl.set_max_width_chars(26);
        inner.append(&title_lbl);

        let time = chrono::DateTime::from_timestamp(s.updated_at, 0)
            .map(|dt| dt.format("%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let time_lbl = Label::new(Some(&time));
        time_lbl.add_css_class("session-row-time");
        time_lbl.set_halign(Align::Start);
        inner.append(&time_lbl);

        row.set_child(Some(&inner));
        list.append(&row);

        if current.as_deref() == Some(&s.id) {
            list.select_row(Some(&row));
        }

        // Right-click context menu
        let sid     = s.id.clone();
        let list_c  = list.clone();
        let state_c = state.clone();
        let ctx_c   = ctx.clone();
        let row_c   = row.clone();

        let pop = Popover::new();
        pop.set_parent(&row);
        pop.add_css_class("context-menu-pop");

        let pbx = GBox::new(Orientation::Vertical, 2);
        pbx.add_css_class("context-menu");

        let del_btn = Button::with_label("删除会话");
        del_btn.add_css_class("ctx-item");
        del_btn.add_css_class("ctx-item-danger");
        let cmp_btn = Button::with_label("压缩对话");
        cmp_btn.add_css_class("ctx-item");
        let ttl_btn = Button::with_label("重新总结标题");
        ttl_btn.add_css_class("ctx-item");
        pbx.append(&del_btn);
        pbx.append(&cmp_btn);
        pbx.append(&ttl_btn);
        pop.set_child(Some(&pbx));

        // Delete
        {
            let p = pop.clone();
            let sid2   = sid.clone();
            let list2  = list_c.clone();
            let state2 = state_c.clone();
            let ctx2   = ctx_c.clone();
            del_btn.connect_clicked(move |_| {
                p.popdown();
                let _ = state2.borrow().session_store.delete_session(&sid2);
                if state2.borrow().current_sid.as_deref() == Some(&sid2) {
                    state2.borrow_mut().current_sid = None;
                    clear_chat(&ctx2.chat_box);
                    show_welcome(&ctx2.chat_box, "从左侧选择会话或创建新会话");
                    ctx2.sess_title.set_text("Sakichan");
                }
                refresh_session_list(&list2, &state2, &ctx2);
            });
        }
        // Compress
        {
            let p = pop.clone();
            let sid2   = sid.clone();
            let list2  = list_c.clone();
            let state2 = state_c.clone();
            let ctx2   = ctx_c.clone();
            cmp_btn.connect_clicked(move |_| {
                p.popdown();
                start_compress(&state2, sid2.clone(), &list2, &ctx2);
            });
        }
        // Re-title
        {
            let p = pop.clone();
            let sid2   = sid.clone();
            let list2  = list_c.clone();
            let state2 = state_c.clone();
            let ctx2   = ctx_c.clone();
            ttl_btn.connect_clicked(move |_| {
                p.popdown();
                start_retitle(&state2, sid2.clone(), &list2, &ctx2);
            });
        }

        let gc = GestureClick::new();
        gc.set_button(3);
        let pop_c = pop.clone();
        gc.connect_pressed(move |_, _, x, y| {
            let rect = gdk4::Rectangle::new(x as i32, y as i32, 1, 1);
            pop_c.set_pointing_to(Some(&rect));
            pop_c.popup();
        });
        row_c.add_controller(gc);
    }
}

fn start_compress(state: &State, session_id: String, list: &ListBox, ctx: &PanelCtx) {
    let ss  = state.borrow().session_store.clone();
    let cfg = state.borrow().config.clone();
    let (tx, rx) = async_channel::unbounded::<Result<(), String>>();

    rt().spawn(async move {
        let session = match ss.get_session(&session_id) {
            Ok(Some(s)) => s,
            _ => { let _ = tx.send(Err("session not found".into())).await; return; }
        };
        let msgs = ss.get_branch_messages(&session_id, session.active_depth, session.active_order)
            .unwrap_or_default();
        if msgs.is_empty() {
            let _ = tx.send(Err("no messages".into())).await; return;
        }
        let prev = ss.get_latest_summary(&session_id).ok().flatten();
        let history: String = msgs.iter()
            .filter(|m| m.role != "system")
            .map(|m| format!("[{}]: {}", if m.role == "user" { "用户" } else { "助手" }, m.content))
            .collect::<Vec<_>>().join("\n\n");
        let mut pt = String::new();
        if let Some(p) = &prev {
            pt.push_str("以下是之前的对话总结（供参考）：\n\n");
            pt.push_str(&p.content);
            pt.push_str("\n\n---\n\n");
        }
        pt.push_str("请对以下对话内容生成精准简洁的总结：\n\n");
        pt.push_str(&history);
        match call_model_text(cfg, "sensenova-flash-lite", vec![CoreMessage::user(pt)]).await {
            Ok(summary) => {
                let _ = ss.save_summary(&session_id, session.active_depth, session.active_order, summary.trim(), msgs.len());
                let _ = tx.send(Ok(())).await;
            }
            Err(e) => { let _ = tx.send(Err(e)).await; }
        }
    });

    let list_c  = list.clone();
    let state_c = state.clone();
    let ctx_c   = ctx.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(res) = rx.recv().await {
            if let Err(e) = res { eprintln!("compress: {e}"); }
            refresh_session_list(&list_c, &state_c, &ctx_c);
            break;
        }
    });
}

fn start_retitle(state: &State, session_id: String, list: &ListBox, ctx: &PanelCtx) {
    let ss          = state.borrow().session_store.clone();
    let cfg         = state.borrow().config.clone();
    let current_sid = state.borrow().current_sid.clone();
    let sid_check   = session_id.clone();
    let (tx, rx)    = async_channel::unbounded::<Result<String, String>>();

    rt().spawn(async move {
        let session = match ss.get_session(&session_id) {
            Ok(Some(s)) => s,
            _ => { let _ = tx.send(Err("session not found".into())).await; return; }
        };
        let msgs = ss.get_branch_messages(&session_id, session.active_depth, session.active_order)
            .unwrap_or_default();
        let hist: String = msgs.iter().filter(|m| m.role != "system").take(8)
            .map(|m| {
                let p = m.content.char_indices().nth(200).map(|(i,_)| &m.content[..i]).unwrap_or(&m.content);
                format!("[{}]: {}", if m.role == "user" { "用户" } else { "助手" }, p)
            }).collect::<Vec<_>>().join("\n\n");
        let pmsg = vec![CoreMessage::user(format!(
            "请为以下对话生成一个简洁的标题（不超过20个字，直接输出标题，不要加引号或解释）：\n\n{hist}"
        ))];
        match call_model_text(cfg, "sensenova-flash-lite", pmsg).await {
            Ok(title) => {
                let title = title.trim().to_string();
                let _ = ss.update_title(&session_id, &title);
                let _ = tx.send(Ok(title)).await;
            }
            Err(e) => { let _ = tx.send(Err(e)).await; }
        }
    });

    let list_c  = list.clone();
    let state_c = state.clone();
    let ctx_c   = ctx.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(res) = rx.recv().await {
            match res {
                Ok(title) => {
                    if current_sid.as_deref() == Some(&sid_check) {
                        ctx_c.sess_title.set_text(&title);
                    }
                    refresh_session_list(&list_c, &state_c, &ctx_c);
                }
                Err(e) => eprintln!("retitle: {e}"),
            }
            break;
        }
    });
}

// ─────────────────────────────────────────
// Attachment chips
// ─────────────────────────────────────────

fn add_chip(bar: &GBox, rev: &Revealer, state: &State, label: &str, is_image: bool, key: String) {
    let chip = GBox::new(Orientation::Horizontal, 4);
    chip.add_css_class("attach-chip");
    let lbl = Label::new(Some(label));
    chip.append(&lbl);
    let rm = Button::with_label("×");
    rm.add_css_class("attach-remove-btn");
    chip.append(&rm);

    let bar_c = bar.clone(); let rev_c = rev.clone();
    let state_c = state.clone(); let chip_c = chip.clone(); let key_c = key.clone();
    rm.connect_clicked(move |_| {
        if is_image { state_c.borrow_mut().pending_imgs.retain(|(n,_)| n != &key_c); }
        else        { state_c.borrow_mut().pending_files.retain(|p| p != &key_c); }
        bar_c.remove(&chip_c);
        if bar_c.first_child().is_none() { rev_c.set_reveal_child(false); }
    });
    bar.append(&chip);
    rev.set_reveal_child(true);
}

fn pick_image(state: &State, bar: &GBox, rev: &Revealer, path: PathBuf) {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("image").to_string();
    match image_to_data_uri(&path) {
        Ok(uri) => {
            state.borrow_mut().pending_imgs.push((name.clone(), uri));
            add_chip(bar, rev, state, &format!("[图] {name}"), true, name);
        }
        Err(e) => eprintln!("image load: {e}"),
    }
}

fn pick_file(state: &State, bar: &GBox, rev: &Revealer, path: PathBuf) {
    let path_str = path.to_string_lossy().to_string();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    state.borrow_mut().pending_files.push(path_str.clone());
    add_chip(bar, rev, state, &format!("[文] {name}"), false, path_str);
}

// ─────────────────────────────────────────
// Streaming events
// ─────────────────────────────────────────

enum Msg { Token(String), Think(String), Done, Fail(String) }

// ─────────────────────────────────────────
// Streaming bubble builder
// ─────────────────────────────────────────

struct StreamBubble {
    outer:        GBox,
    bubble:       GBox,
    think_toggle: Option<Button>,
    think_lbl:    Option<Label>,
}

fn make_streaming_bubble(show_thinking: bool) -> StreamBubble {
    let outer = GBox::new(Orientation::Vertical, 4);
    outer.add_css_class("msg-wrapper-assistant");

    let role_lbl = Label::new(Some("Sakichan"));
    role_lbl.add_css_class("msg-role");
    role_lbl.add_css_class("msg-role-assistant");
    role_lbl.set_halign(Align::Start);
    outer.append(&role_lbl);

    // Thinking section (collapsed by default, only when thinking mode on)
    let (think_toggle, think_lbl) = if show_thinking {
        let area = GBox::new(Orientation::Vertical, 0);
        area.add_css_class("thinking-box");

        let toggle = Button::new();
        toggle.add_css_class("think-toggle-btn");
        toggle.set_label("▸ 深度思考中…");
        area.append(&toggle);

        let rev = Revealer::builder()
            .transition_type(RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .reveal_child(false)
            .build();
        let tc = GBox::new(Orientation::Vertical, 0);
        tc.add_css_class("thinking-content");
        let tl = Label::new(Some(""));
        tl.add_css_class("thinking-text");
        tl.set_wrap(true);
        tl.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        tl.set_xalign(0.0);
        tl.set_selectable(true);
        tc.append(&tl);
        rev.set_child(Some(&tc));
        area.append(&rev);

        let rev_c = rev.clone();
        toggle.connect_clicked(move |_| {
            rev_c.set_reveal_child(!rev_c.reveals_child());
        });

        outer.append(&area);
        (Some(toggle), Some(tl))
    } else {
        (None, None)
    };

    // Main bubble: starts with loading spinner
    let bubble = GBox::new(Orientation::Vertical, 0);
    bubble.add_css_class("msg-bubble-assistant");

    let loading = GBox::new(Orientation::Horizontal, 8);
    loading.add_css_class("loading-box");
    loading.set_margin_start(4);
    loading.set_margin_top(6);
    loading.set_margin_bottom(6);
    let sp = Spinner::new();
    sp.start();
    sp.add_css_class("loading-spinner");
    loading.append(&sp);
    let ll = Label::new(Some("生成中…"));
    ll.add_css_class("loading-label");
    loading.append(&ll);
    bubble.append(&loading);

    outer.append(&bubble);
    StreamBubble { outer, bubble, think_toggle, think_lbl }
}

// ─────────────────────────────────────────
// Common streaming receive loop
// ─────────────────────────────────────────

fn spawn_stream_receiver(
    rx:            async_channel::Receiver<Msg>,
    sb:            StreamBubble,
    state:         State,
    ctx:           PanelCtx,
    sid:           String,
    reload_on_done: bool,   // true → reload session messages after Done
) {
    let bubble2   = sb.bubble.clone();
    let think_tog = sb.think_toggle.clone();
    let think_lbl = sb.think_lbl.clone();
    let scroll2   = ctx.chat_scroll.clone();
    let send_btn2 = ctx.send_btn.clone();
    let sl2       = ctx.session_list.clone();
    let state2    = state.clone();
    let ctx2      = ctx.clone();
    let sid2      = sid.clone();

    let tbuf        = Rc::new(RefCell::new(String::new()));
    let thbuf       = Rc::new(RefCell::new(String::new()));
    let first_tok   = Rc::new(RefCell::new(true));
    let last_rend   = Rc::new(RefCell::new(Instant::now()));
    let think_start : Rc<RefCell<Option<Instant>>> = Rc::new(RefCell::new(None));

    glib::MainContext::default().spawn_local(async move {
        while let Ok(msg) = rx.recv().await {
            match msg {
                Msg::Token(t) => {
                    tbuf.borrow_mut().push_str(&t);

                    if *first_tok.borrow() {
                        *first_tok.borrow_mut() = false;
                        let text = tbuf.borrow().clone();
                        while let Some(c) = bubble2.first_child() { bubble2.remove(&c); }
                        bubble2.append(&build_markdown_widget(&text));
                        scroll_to_bottom(&scroll2);
                        *last_rend.borrow_mut() = Instant::now();
                    } else {
                        let now = Instant::now();
                        if now.duration_since(*last_rend.borrow()) >= Duration::from_millis(180) {
                            let text = tbuf.borrow().clone();
                            while let Some(c) = bubble2.first_child() { bubble2.remove(&c); }
                            bubble2.append(&build_markdown_widget(&text));
                            scroll_to_bottom(&scroll2);
                            *last_rend.borrow_mut() = now;
                        }
                    }
                }
                Msg::Think(t) => {
                    if think_start.borrow().is_none() {
                        *think_start.borrow_mut() = Some(Instant::now());
                    }
                    thbuf.borrow_mut().push_str(&t);
                    if let Some(ref tl) = think_lbl {
                        tl.set_text(thbuf.borrow().as_str());
                    }
                }
                Msg::Done => {
                    state2.borrow_mut().is_generating = false;
                    send_btn2.set_sensitive(true);

                    // Final markdown render
                    let final_text = tbuf.borrow().clone();
                    while let Some(c) = bubble2.first_child() { bubble2.remove(&c); }
                    bubble2.append(&build_markdown_widget(&final_text));
                    scroll_to_bottom(&scroll2);

                    // Update thinking toggle with elapsed time
                    if let Some(ref tb) = think_tog {
                        let secs = think_start.borrow()
                            .map(|t| t.elapsed().as_secs())
                            .unwrap_or(0);
                        tb.set_label(&format!("▸ 深度思考 (用时 {}s)", secs));
                    }

                    // Reload session messages on Done (used for branch sends)
                    if reload_on_done {
                        let session = state2.borrow().session_store
                            .get_session(&sid2).ok().flatten();
                        if let Some(s) = session {
                            load_session_messages(&ctx2, &s, &state2.borrow().session_store, &state2);
                        }
                    }

                    // Auto-title if untitled
                    let needs_title = {
                        let st = state2.borrow();
                        st.session_store.get_session(&sid2)
                            .ok().flatten()
                            .map(|s| s.title.is_none())
                            .unwrap_or(false)
                    };
                    if needs_title {
                        let ss2  = state2.borrow().session_store.clone();
                        let cfg2 = state2.borrow().config.clone();
                        let (ttx, trx) = async_channel::unbounded::<String>();
                        let sid3 = sid2.clone();
                        rt().spawn(async move {
                            let msgs = match ss2.get_session(&sid3) {
                                Ok(Some(s)) => ss2.get_branch_messages(&sid3, s.active_depth, s.active_order).unwrap_or_default(),
                                _ => return,
                            };
                            let hist: String = msgs.iter().filter(|m| m.role != "system").take(6)
                                .map(|m| {
                                    let p = m.content.char_indices().nth(200).map(|(i,_)| &m.content[..i]).unwrap_or(&m.content);
                                    format!("[{}]: {}", if m.role == "user" { "用户" } else { "助手" }, p)
                                }).collect::<Vec<_>>().join("\n\n");
                            let pmsg = vec![CoreMessage::user(format!(
                                "请为以下对话生成一个简洁的标题（不超过20个字，直接输出标题，不要加引号或解释）：\n\n{hist}"
                            ))];
                            if let Ok(title) = call_model_text(cfg2, "sensenova-flash-lite", pmsg).await {
                                let title = title.trim().to_string();
                                let _ = ss2.update_title(&sid3, &title);
                                let _ = ttx.send(title).await;
                            }
                        });
                        let ctx3   = ctx2.clone();
                        let state3 = state2.clone();
                        let sl3    = sl2.clone();
                        glib::MainContext::default().spawn_local(async move {
                            if let Ok(title) = trx.recv().await {
                                ctx3.sess_title.set_text(&title);
                                refresh_session_list(&sl3, &state3, &ctx3);
                            }
                        });
                    }

                    refresh_session_list(&sl2, &state2, &ctx2);
                    break;
                }
                Msg::Fail(e) => {
                    eprintln!("gen error: {e}");
                    while let Some(c) = bubble2.first_child() { bubble2.remove(&c); }
                    let err = Label::new(Some(&format!("[错误: {e}]")));
                    err.add_css_class("message-text");
                    err.set_xalign(0.0);
                    bubble2.append(&err);
                    state2.borrow_mut().is_generating = false;
                    send_btn2.set_sensitive(true);
                    break;
                }
            }
        }
    });
}

// ─────────────────────────────────────────
// Main send handler
// ─────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn do_send(
    state:        &State,
    input_buf:    &TextBuffer,
    attach_rev:   &Revealer,
    attach_bar:   &GBox,
    dsv4f:        bool,
    thinking:     bool,
    ctx:          &PanelCtx,
) {
    if state.borrow().is_generating { return; }

    let text = {
        let s = input_buf.start_iter();
        let e = input_buf.end_iter();
        input_buf.text(&s, &e, false).to_string()
    };
    let text = text.trim().to_string();
    if text.is_empty() { return; }

    let sid = {
        let mut st = state.borrow_mut();
        if let Some(id) = st.current_sid.clone() { id }
        else {
            match st.session_store.create_session("sensenova-flash-lite", None) {
                Ok(s) => { st.current_sid = Some(s.id.clone()); s.id }
                Err(e) => { eprintln!("create session: {e}"); return; }
            }
        }
    };

    let imgs: Vec<String>;
    let files: Vec<String>;
    {
        let mut st = state.borrow_mut();
        imgs  = st.pending_imgs.iter().map(|(_, u)| u.clone()).collect();
        files = st.pending_files.clone();
        st.pending_imgs.clear();
        st.pending_files.clear();
        st.is_generating = true;
    }
    while let Some(c) = attach_bar.first_child() { attach_bar.remove(&c); }
    attach_rev.set_reveal_child(false);
    input_buf.set_text("");
    ctx.send_btn.set_sensitive(false);

    let file_prefix: Option<String> = {
        let parts: Vec<_> = files.iter()
            .filter_map(|p| std::fs::read_to_string(p).ok().map(|c| format!("[文件: {p}]\n{c}")))
            .collect();
        if parts.is_empty() { None } else { Some(parts.join("\n\n---\n\n")) }
    };
    let full_text = match file_prefix {
        Some(fp) => format!("{fp}\n\n---\n\n{text}"),
        None     => text.clone(),
    };

    // User message widget
    let uw = make_msg_widget_with_actions(
        &CoreMessage::user(&full_text), 0, 0, None, state, ctx,
    );
    ctx.chat_box.append(&uw);
    scroll_to_bottom(&ctx.chat_scroll);

    // Streaming bubble
    let sb = make_streaming_bubble(thinking);
    ctx.chat_box.append(&sb.outer);
    scroll_to_bottom(&ctx.chat_scroll);

    let (tx, rx) = async_channel::unbounded::<Msg>();

    let ss    = state.borrow().session_store.clone();
    let ms    = state.borrow().memory_store.clone();
    let cfg   = state.borrow().config.clone();
    let top_k = cfg.memory.retrieval_top_k;
    let mid   = if dsv4f { "deepseek-v4-flash" } else { "sensenova-flash-lite" }.to_string();
    let full_text_c = full_text.clone();
    let imgs_c      = imgs.clone();
    let sid_c       = sid.clone();
    let tx2         = tx.clone();

    rt().spawn(async move {
        let session = match ss.get_session(&sid_c) {
            Ok(Some(s)) => s,
            _ => { let _ = tx2.send(Msg::Fail("session not found".into())).await; return; }
        };
        let next_depth = ss.get_max_depth(&sid_c).unwrap_or(0) + 1;
        let next_order = 1i64;
        let _ = ss.set_active_branch(&sid_c, next_depth, next_order);

        let model = match make_model(&cfg, &mid) {
            Ok(m) => m,
            Err(e) => { let _ = tx2.send(Msg::Fail(format!("{e}"))).await; return; }
        };

        let mut sctx = SessionContext::new(sid_c.clone(), mid.clone());
        sctx.set_system_prompt(session.system_prompt.clone());
        let branch = ss.get_branch_messages(&sid_c, session.active_depth, session.active_order).unwrap_or_default();
        if !branch.is_empty() { sctx.load_messages(branch); }
        sctx.set_branch_mode(next_depth, next_order);

        let mut pipeline = PipelineContext::new(sctx, ss, ms, top_k);
        let tx3 = tx2.clone();
        let res = pipeline.run(&full_text_c, imgs_c, &*model, &mut move |ev| match ev {
            PipelineEvent::Token(t)          => { let _ = tx3.send_blocking(Msg::Token(t)); }
            PipelineEvent::ReasoningToken(t) => { let _ = tx3.send_blocking(Msg::Think(t)); }
            PipelineEvent::Done(_)           => { let _ = tx3.send_blocking(Msg::Done); }
            PipelineEvent::Error(e)          => { let _ = tx3.send_blocking(Msg::Fail(e)); }
            _ => {}
        }).await;

        if let Err(e) = res {
            let _ = tx2.send(Msg::Fail(format!("{e}"))).await;
        }
    });

    spawn_stream_receiver(rx, sb, state.clone(), ctx.clone(), sid, false);
}

// Branch send: send at a specific (depth, order) with explicit history loading
fn do_send_branch(
    state:    &State,
    text:     String,
    imgs:     Vec<String>,
    depth:    i64,
    order:    i64,
    dsv4f:    bool,
    ctx:      &PanelCtx,
) {
    if state.borrow().is_generating { return; }

    let sid = match state.borrow().current_sid.clone() {
        Some(s) => s,
        None    => return,
    };

    state.borrow_mut().is_generating = true;
    ctx.send_btn.set_sensitive(false);

    // Streaming bubble (appended at bottom)
    let sb = make_streaming_bubble(false);
    ctx.chat_box.append(&sb.outer);
    scroll_to_bottom(&ctx.chat_scroll);

    let (tx, rx) = async_channel::unbounded::<Msg>();

    let ss    = state.borrow().session_store.clone();
    let ms    = state.borrow().memory_store.clone();
    let cfg   = state.borrow().config.clone();
    let top_k = cfg.memory.retrieval_top_k;
    let mid   = if dsv4f { "deepseek-v4-flash" } else { "sensenova-flash-lite" }.to_string();
    let text_c = text.clone();
    let sid_c  = sid.clone();
    let tx2    = tx.clone();

    rt().spawn(async move {
        let session = match ss.get_session(&sid_c) {
            Ok(Some(s)) => s,
            _ => { let _ = tx2.send(Msg::Fail("session not found".into())).await; return; }
        };

        // Load history up to depth-1 (on the current active path)
        let history = if depth > 1 {
            ss.get_branch_messages(&sid_c, depth - 1, session.active_order).unwrap_or_default()
        } else { vec![] };

        let _ = ss.set_active_branch(&sid_c, depth, order);

        let model = match make_model(&cfg, &mid) {
            Ok(m) => m,
            Err(e) => { let _ = tx2.send(Msg::Fail(format!("{e}"))).await; return; }
        };

        let mut sctx = SessionContext::new(sid_c.clone(), mid.clone());
        sctx.set_system_prompt(session.system_prompt.clone());
        if !history.is_empty() { sctx.load_messages(history); }
        sctx.set_branch_mode(depth, order);

        let mut pipeline = PipelineContext::new(sctx, ss, ms, top_k);
        let tx3 = tx2.clone();
        let res = pipeline.run(&text_c, imgs, &*model, &mut move |ev| match ev {
            PipelineEvent::Token(t)          => { let _ = tx3.send_blocking(Msg::Token(t)); }
            PipelineEvent::ReasoningToken(t) => { let _ = tx3.send_blocking(Msg::Think(t)); }
            PipelineEvent::Done(_)           => { let _ = tx3.send_blocking(Msg::Done); }
            PipelineEvent::Error(e)          => { let _ = tx3.send_blocking(Msg::Fail(e)); }
            _ => {}
        }).await;

        if let Err(e) = res {
            let _ = tx2.send(Msg::Fail(format!("{e}"))).await;
        }
    });

    // reload_on_done=true → reload session messages when generation finishes
    spawn_stream_receiver(rx, sb, state.clone(), ctx.clone(), sid, true);
}

// ─────────────────────────────────────────
// UI builder
// ─────────────────────────────────────────

fn build_ui(app: &Application) {
    let css = CssProvider::new();
    css.load_from_string(include_str!("style.css"));
    gtk4::style_context_add_provider_for_display(
        &gdk4::Display::default().expect("display"),
        &css,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Force dark theme variant (especially important on Windows)
    if let Some(s) = gtk4::Settings::default() {
        s.set_gtk_application_prefer_dark_theme(true);
    }

    let state: State = match AppState::init() {
        Ok(s)  => Rc::new(RefCell::new(s)),
        Err(e) => { eprintln!("init error: {e}"); return; }
    };

    // ── Window ────────────────────────────────────────────────
    let win = ApplicationWindow::builder()
        .application(app)
        .title("Sakichan")
        .default_width(1100)
        .default_height(720)
        .build();
    win.add_css_class("main-window");

    let overlay = Overlay::new();

    let main_col = GBox::new(Orientation::Vertical, 0);
    main_col.set_hexpand(true);
    main_col.set_vexpand(true);
    overlay.set_child(Some(&main_col));

    // ── Header ───────────────────────────────────────────────
    let header = GBox::new(Orientation::Horizontal, 0);
    header.add_css_class("header-bar");

    let hamburger = Button::with_label("☰");
    hamburger.add_css_class("hamburger-btn");
    header.append(&hamburger);

    let sess_title = Label::new(Some("Sakichan"));
    sess_title.add_css_class("session-title");
    sess_title.set_hexpand(true);
    sess_title.set_halign(Align::Start);
    sess_title.set_margin_start(16);
    sess_title.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    header.append(&sess_title);
    main_col.append(&header);

    // ── Body ─────────────────────────────────────────────────
    let body = GBox::new(Orientation::Vertical, 0);
    body.set_vexpand(true);
    body.set_hexpand(true);
    main_col.append(&body);

    let chat_scroll = ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .build();
    chat_scroll.add_css_class("chat-scroll");

    let chat_box = GBox::new(Orientation::Vertical, 0);
    chat_box.set_vexpand(true);
    chat_box.add_css_class("chat-box");
    show_welcome(&chat_box, "从左侧选择会话或创建新会话");
    chat_scroll.set_child(Some(&chat_box));
    body.append(&chat_scroll);

    // Attachment revealer
    let attach_rev = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideDown)
        .transition_duration(160)
        .reveal_child(false)
        .build();
    let attach_bar = GBox::new(Orientation::Horizontal, 8);
    attach_bar.add_css_class("attachment-bar");
    attach_rev.set_child(Some(&attach_bar));
    body.append(&attach_rev);

    // Input container
    let input_con = GBox::new(Orientation::Vertical, 0);
    input_con.add_css_class("input-container");

    let input_scroll = ScrolledWindow::builder()
        .min_content_height(52)
        .max_content_height(180)
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .build();
    input_scroll.add_css_class("input-scroll");

    let input_buf  = TextBuffer::new(None);
    let input_view = TextView::with_buffer(&input_buf);
    input_view.add_css_class("input-text");
    input_view.set_wrap_mode(WrapMode::WordChar);
    input_view.set_left_margin(12);
    input_view.set_right_margin(12);
    input_view.set_top_margin(10);
    input_view.set_bottom_margin(10);
    input_view.set_hexpand(true);
    input_scroll.set_child(Some(&input_view));
    input_con.append(&input_scroll);

    let btn_row = GBox::new(Orientation::Horizontal, 8);
    btn_row.add_css_class("btn-row");

    let dsv4f_btn = ToggleButton::with_label("DeepSeek V4");
    dsv4f_btn.add_css_class("toggle-btn");
    dsv4f_btn.set_tooltip_text(Some("使用 DeepSeek V4 Flash"));
    btn_row.append(&dsv4f_btn);

    let think_btn = ToggleButton::with_label("深度思考");
    think_btn.add_css_class("toggle-btn");
    think_btn.set_tooltip_text(Some("显示 DeepSeek 思考过程"));
    think_btn.set_sensitive(false);
    btn_row.append(&think_btn);

    let spacer = GBox::new(Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    btn_row.append(&spacer);

    let img_btn = Button::with_label("图片");
    img_btn.add_css_class("action-btn");
    img_btn.set_tooltip_text(Some("添加图片附件"));
    btn_row.append(&img_btn);

    let file_btn = Button::with_label("文件");
    file_btn.add_css_class("action-btn");
    file_btn.set_tooltip_text(Some("添加文本文件"));
    btn_row.append(&file_btn);

    let send_btn = Button::with_label("发送");
    send_btn.add_css_class("send-btn");
    btn_row.append(&send_btn);

    input_con.append(&btn_row);
    body.append(&input_con);

    // ── Overlays: dim + sidebar ───────────────────────────────
    let dim = GBox::new(Orientation::Horizontal, 0);
    dim.add_css_class("dim-overlay");
    dim.set_halign(Align::Fill);
    dim.set_valign(Align::Fill);
    dim.set_visible(false);
    overlay.add_overlay(&dim);
    overlay.set_measure_overlay(&dim, false);

    let sidebar_rev = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideRight)
        .transition_duration(220)
        .reveal_child(false)
        .halign(Align::Start)
        .valign(Align::Fill)
        .build();

    let sidebar = GBox::new(Orientation::Vertical, 0);
    sidebar.add_css_class("sidebar");

    // API KEY section
    let ak_section = GBox::new(Orientation::Vertical, 6);
    ak_section.add_css_class("apikey-section");

    let key_ok = has_api_key();
    let ak_status = Label::new(Some(
        if key_ok { "API KEY 已对接" } else { "请输入 API KEY 再对话" },
    ));
    ak_status.add_css_class(if key_ok { "apikey-status-ok" } else { "apikey-status-missing" });
    ak_status.set_halign(Align::Start);
    ak_section.append(&ak_status);

    let ak_entry = Entry::new();
    ak_entry.set_placeholder_text(Some("输入 API KEY …"));
    ak_entry.set_visibility(false);
    ak_entry.set_hexpand(true);
    ak_entry.add_css_class("apikey-entry");
    ak_section.append(&ak_entry);

    let ak_btns  = GBox::new(Orientation::Horizontal, 6);
    let ak_write = Button::with_label("写入");
    ak_write.add_css_class("apikey-write-btn");
    ak_write.set_hexpand(true);
    let ak_clear = Button::with_label("清除");
    ak_clear.add_css_class("apikey-clear-btn");
    ak_clear.set_hexpand(true);
    ak_btns.append(&ak_write);
    ak_btns.append(&ak_clear);
    ak_section.append(&ak_btns);
    sidebar.append(&ak_section);

    // Session list header
    let sb_hdr = GBox::new(Orientation::Horizontal, 0);
    sb_hdr.add_css_class("sidebar-header");
    let sb_lbl = Label::new(Some("会话列表"));
    sb_lbl.set_hexpand(true);
    sb_lbl.set_halign(Align::Start);
    sb_hdr.append(&sb_lbl);
    sidebar.append(&sb_hdr);

    let new_sess_btn = Button::with_label("+ 新建会话");
    new_sess_btn.add_css_class("new-session-btn");
    sidebar.append(&new_sess_btn);

    let sess_scroll = ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(PolicyType::Never)
        .build();
    let session_list = ListBox::new();
    session_list.add_css_class("session-list");
    session_list.set_selection_mode(gtk4::SelectionMode::Single);
    sess_scroll.set_child(Some(&session_list));
    sidebar.append(&sess_scroll);

    sidebar_rev.set_child(Some(&sidebar));
    overlay.add_overlay(&sidebar_rev);
    overlay.set_measure_overlay(&sidebar_rev, false);

    win.set_child(Some(&overlay));

    // Panel context (shared UI refs)
    let panel_ctx = PanelCtx {
        chat_box:     chat_box.clone(),
        chat_scroll:  chat_scroll.clone(),
        sess_title:   sess_title.clone(),
        session_list: session_list.clone(),
        send_btn:     send_btn.clone(),
        dsv4f_btn:    dsv4f_btn.clone(),
    };

    // ════════════════════════════════════════
    // Signal connections
    // ════════════════════════════════════════

    // DeepSeek V4 toggle
    {
        let think_btn2 = think_btn.clone();
        dsv4f_btn.connect_toggled(move |btn| {
            let on = btn.is_active();
            think_btn2.set_sensitive(on);
            if !on { think_btn2.set_active(false); }
        });
    }

    // Hamburger → sidebar
    {
        let sr = sidebar_rev.clone();
        let d  = dim.clone();
        hamburger.connect_clicked(move |_| {
            let open = sr.reveals_child();
            sr.set_reveal_child(!open);
            d.set_visible(!open);
        });
    }

    // Dim → close sidebar
    {
        let gc = GestureClick::new();
        let sr = sidebar_rev.clone();
        let d  = dim.clone();
        gc.connect_released(move |_, _, _, _| {
            sr.set_reveal_child(false);
            d.set_visible(false);
        });
        dim.add_controller(gc);
    }

    // Write API key
    {
        let ec = ak_entry.clone();
        let sc = ak_status.clone();
        let st = state.clone();
        ak_write.connect_clicked(move |_| {
            let key = ec.text().to_string();
            let key = key.trim().to_string();
            if key.is_empty() { return; }
            set_api_key_all(&key);
            reload_keys_in_config(&st);
            ec.set_text("");
            sc.set_text("API KEY 已对接");
            sc.remove_css_class("apikey-status-missing");
            sc.add_css_class("apikey-status-ok");
        });
    }

    // Clear API key
    {
        let sc = ak_status.clone();
        let st = state.clone();
        ak_clear.connect_clicked(move |_| {
            clear_api_key_all();
            reload_keys_in_config(&st);
            sc.set_text("请输入 API KEY 再对话");
            sc.remove_css_class("apikey-status-ok");
            sc.add_css_class("apikey-status-missing");
        });
    }

    // Image picker
    {
        let win2   = win.clone();
        let ar     = attach_rev.clone();
        let ab     = attach_bar.clone();
        let state2 = state.clone();
        img_btn.connect_clicked(move |_| {
            let f = FileFilter::new();
            f.add_mime_type("image/*");
            f.set_name(Some("图片文件"));
            let dlg = FileDialog::builder().title("选择图片").default_filter(&f).build();
            let ar2 = ar.clone(); let ab2 = ab.clone(); let st2 = state2.clone();
            dlg.open(Some(&win2), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(p) = file.path() { pick_image(&st2, &ab2, &ar2, p); }
                }
            });
        });
    }

    // File picker
    {
        let win2   = win.clone();
        let ar     = attach_rev.clone();
        let ab     = attach_bar.clone();
        let state2 = state.clone();
        file_btn.connect_clicked(move |_| {
            let dlg = FileDialog::builder().title("选择文件").build();
            let ar2 = ar.clone(); let ab2 = ab.clone(); let st2 = state2.clone();
            dlg.open(Some(&win2), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(p) = file.path() { pick_file(&st2, &ab2, &ar2, p); }
                }
            });
        });
    }

    // Enter → send, Shift+Enter → newline
    {
        let sb = send_btn.clone();
        let kc = EventControllerKey::new();
        kc.connect_key_pressed(move |_, key, _, mods| {
            use gdk4::{Key, ModifierType};
            if key == Key::Return {
                if mods.contains(ModifierType::SHIFT_MASK) {
                    glib::Propagation::Proceed
                } else {
                    sb.emit_clicked();
                    glib::Propagation::Stop
                }
            } else {
                glib::Propagation::Proceed
            }
        });
        input_view.add_controller(kc);
    }

    // New session
    {
        let sl     = session_list.clone();
        let sr     = sidebar_rev.clone();
        let d      = dim.clone();
        let state2 = state.clone();
        let ctx2   = panel_ctx.clone();
        new_sess_btn.connect_clicked(move |_| {
            let res = state2.borrow().session_store.create_session("sensenova-flash-lite", None);
            if let Ok(s) = res {
                state2.borrow_mut().current_sid = Some(s.id.clone());
                load_session_messages(&ctx2, &s, &state2.borrow().session_store, &state2);
                ctx2.sess_title.set_text(s.title.as_deref().unwrap_or("新会话"));
                refresh_session_list(&sl, &state2, &ctx2);
                sr.set_reveal_child(false);
                d.set_visible(false);
            }
        });
    }

    // Session row activated
    {
        let sr     = sidebar_rev.clone();
        let d      = dim.clone();
        let state2 = state.clone();
        let ctx2   = panel_ctx.clone();
        session_list.connect_row_activated(move |_, row| {
            let sid = row.widget_name().to_string();
            if sid.is_empty() { return; }
            let session = state2.borrow().session_store.get_session(&sid).ok().flatten();
            if let Some(s) = session {
                state2.borrow_mut().current_sid = Some(s.id.clone());
                ctx2.sess_title.set_text(s.title.as_deref().unwrap_or("无标题"));
                load_session_messages(&ctx2, &s, &state2.borrow().session_store, &state2);
                sr.set_reveal_child(false);
                d.set_visible(false);
            }
        });
    }

    // Send button
    {
        let ib     = input_buf.clone();
        let ar     = attach_rev.clone();
        let ab     = attach_bar.clone();
        let d4     = dsv4f_btn.clone();
        let th     = think_btn.clone();
        let state2 = state.clone();
        let ctx2   = panel_ctx.clone();
        send_btn.connect_clicked(move |_| {
            do_send(
                &state2, &ib, &ar, &ab,
                d4.is_active(), th.is_active(), &ctx2,
            );
        });
    }

    refresh_session_list(&session_list, &state, &panel_ctx);
    win.present();
}

// ─────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────

fn setup_portable_env() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let dir = dir.to_string_lossy().to_string();
            // DLLs 和 exe 放同目录即可被 Windows 自动找到
            // 以下环境变量让 GTK4 能找到运行时数据
            unsafe {
                std::env::set_var("GTK_DATA_PREFIX", &dir);
                std::env::set_var("GTK_EXE_PREFIX", &dir);
                std::env::set_var("XDG_DATA_DIRS", format!("{dir}\\share"));
                std::env::set_var("GDK_PIXBUF_MODULE_FILE", format!("{dir}\\lib\\gdk-pixbuf-2.0\\2.10.0\\loaders.cache"));
                std::env::set_var("FONTCONFIG_PATH", format!("{dir}\\etc\\fonts"));
            }
        }
    }
}

fn main() {
    setup_portable_env();
    let _ = rt();
    let app = Application::builder()
        .application_id("com.sakichan.gtk")
        .build();
    app.connect_activate(build_ui);
    app.run();
}
