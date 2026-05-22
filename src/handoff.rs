use crate::slots::SlotRole;

// ── Constraints ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Constraint {
    Hard(String),
    Soft(String),
    Info(String),
}

impl Constraint {
    pub fn format(&self) -> String {
        match self {
            Constraint::Hard(s) => format!("[HARD] {s}"),
            Constraint::Soft(s) => format!("[SOFT] {s}"),
            Constraint::Info(s) => format!("[INFO] {s}"),
        }
    }

    pub fn parse_line(s: &str) -> Option<Self> {
        let t = s.trim();
        if let Some(r) = t.strip_prefix("[HARD]") { return Some(Constraint::Hard(r.trim().to_string())); }
        if let Some(r) = t.strip_prefix("[SOFT]") { return Some(Constraint::Soft(r.trim().to_string())); }
        if let Some(r) = t.strip_prefix("[INFO]") { return Some(Constraint::Info(r.trim().to_string())); }
        None
    }
}

// ── Module (Phase E → Phase F handoff) ───────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub implementation: String,
    pub constraints: Vec<Constraint>,
    pub verification: Vec<String>,
    pub assigned_slot: SlotRole,
    pub needs_compile: bool,
    pub full_spec: String,
}

impl Default for Module {
    fn default() -> Self {
        Module {
            name: String::new(),
            inputs: vec![],
            outputs: vec![],
            implementation: String::new(),
            constraints: vec![],
            verification: vec![],
            assigned_slot: SlotRole::Programmer,
            needs_compile: false,
            full_spec: String::new(),
        }
    }
}

// ── Review result (Phase G → Phase F handoff) ─────────────────────────────────

#[derive(Debug, Clone)]
pub enum ReviewStatus {
    Approved,
    RevisionRequired,
}

#[derive(Debug, Clone)]
pub enum IssueSeverity { Major, Minor, Info }

#[derive(Debug, Clone)]
pub struct Issue {
    pub severity: IssueSeverity,
    pub location: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ReviewResult {
    pub module_name: String,
    pub issues: Vec<Issue>,
    pub status: ReviewStatus,
}

// ── Major restart report (Phase I → Phase C handoff) ─────────────────────────

#[derive(Debug, Clone, Default)]
pub struct MajorRestartReport {
    pub triggered_by: Option<SlotRole>,
    pub reason: String,
    pub attempted_fixes: Vec<String>,
    pub constraints_for_redesign: Vec<Constraint>,
    pub preserved_artifacts: Vec<String>,
}

impl MajorRestartReport {
    pub fn format_for_prompt(&self) -> String {
        if self.reason.is_empty() { return String::new(); }
        let mut s = String::from("## 上次重启记录（请避免重蹈覆辙）\n");
        if let Some(role) = self.triggered_by {
            s.push_str(&format!("触发方: {}\n", role.name()));
        }
        s.push_str(&format!("原因: {}\n", self.reason));
        if !self.attempted_fixes.is_empty() {
            s.push_str("已尝试的修复（均失败）:\n");
            for f in &self.attempted_fixes { s.push_str(&format!("  - {f}\n")); }
        }
        if !self.constraints_for_redesign.is_empty() {
            s.push_str("重新设计时必须遵守:\n");
            for c in &self.constraints_for_redesign { s.push_str(&format!("  - {}\n", c.format())); }
        }
        if !self.preserved_artifacts.is_empty() {
            s.push_str("保留的产物（不要修改）:\n");
            for a in &self.preserved_artifacts { s.push_str(&format!("  - {a}\n")); }
        }
        s.push('\n');
        s
    }
}

// ── Prompt builders ───────────────────────────────────────────────────────────

pub fn build_generation_prompt(
    module: &Module,
    solution_design: &str,
    user_request: &str,
    project_tree: &str,
    output_lang: &str,
    restart_report: Option<&MajorRestartReport>,
) -> String {
    let role = module.assigned_slot;
    let constraints_str = if module.constraints.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = module.constraints.iter().map(|c| format!("  - {}", c.format())).collect();
        format!("## 约束条件\n{}\n\n", lines.join("\n"))
    };
    let verification_str = if module.verification.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = module.verification.iter().map(|v| format!("  - {v}")).collect();
        format!("## 验收标准\n{}\n\n", lines.join("\n"))
    };
    let restart_str = restart_report
        .map(|r| r.format_for_prompt())
        .unwrap_or_default();
    let sol_section = if solution_design.is_empty() {
        String::new()
    } else {
        format!("## 解决方案设计（Phase C）\n{solution_design}\n\n")
    };

    format!(
        "{role_prompt}\
        {restart_str}\
        {sol_section}\
        ## 原始需求\n{request}\n\n\
        ## 本模块规范\n{spec}\n\n\
        {constraints_str}\
        {verification_str}\
        ## 项目目录\n{tree}\n\n\
        可用工具（优先调用再生成）：\n\
        - [SYSTEM:read_file path=\"path\"]\n\
        - [SYSTEM:grep pattern=\"symbol\" path=\"src\"]\n\
        - [SYSTEM:web_search query=\"关键词\"]\n\n\
        在生成前，先用 [THINK] 输出思考过程：输入是什么/输出是什么/关键数据结构/边界情况。\n\n\
        ## 输出格式（二选一）\n\
        小范围修改：\n\
        ```patch filename=\"path/to/file\"\n---OLD---\n原始代码\n---NEW---\n修改后\n---END---\n```\n\
        新文件或大范围重写：\n\
        ```rust filename=\"path/to/file.rs\"\n// 完整代码\n```\n\
        所有注释和说明用 {lang}。",
        role_prompt = role.role_prompt(),
        sol_section = sol_section,
        request = user_request,
        spec = module.full_spec,
        tree = project_tree,
        lang = output_lang,
    )
}

pub fn build_fix_prompt(
    review_result: &ReviewResult,
    current_code: &str,
    module: &Module,
    solution_design: &str,
    output_lang: &str,
) -> String {
    let issues: Vec<String> = review_result.issues.iter().map(|i| {
        let sev = match i.severity {
            IssueSeverity::Major => "[MAJOR]",
            IssueSeverity::Minor => "[MINOR]",
            IssueSeverity::Info => "[INFO]",
        };
        let loc = i.location.as_deref().unwrap_or("");
        if loc.is_empty() { format!("  - {sev} {}", i.description) }
        else { format!("  - {sev} {loc}: {}", i.description) }
    }).collect();

    let sol_section = if solution_design.is_empty() {
        String::new()
    } else {
        format!("## 解决方案设计\n{solution_design}\n\n")
    };

    format!(
        "{role_prompt}\
        ## 审查意见 [REVIEW: {mod_name}]\n{issues}\n\n\
        ## 模块规范\n{spec}\n\n\
        {sol_section}\
        ## 当前代码\n{code}\n\n\
        请修复所有 [MAJOR] 和 [MINOR] 问题，输出修改后的文件。\n\
        所有说明用 {lang}。",
        role_prompt = module.assigned_slot.role_prompt(),
        mod_name = review_result.module_name,
        issues = issues.join("\n"),
        spec = module.full_spec,
        code = current_code,
        lang = output_lang,
    )
}
