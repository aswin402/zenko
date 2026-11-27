//! Native ZPowers Plan-First Orchestration & Per-Session Workspace Engine (`.zenko/sessions/<id>/`).
//!
//! Combines the strongest architectural patterns from `obra/superpowers`, Antigravity, Codex CLI,
//! Crush, Hermes Agent, and Prime Agent:
//! - **Dual-Zone Hygiene:** Secrets & global SQLite DB stay in `~/.zenko/`; per-session specs,
//!   DAG plans, debugging notebooks, two-stage review reports, walkthroughs, artifacts, and
//!   scratch files live under `<workspace>/.zenko/sessions/<id>/`.
//! - **Zero Git Pollution:** Idempotently adds `.zenko/sessions/` to local `.git/info/exclude`
//!   (handling both normal `.git/` dirs and worktree `gitdir:` pointers) without ever modifying
//!   the repository's tracked `.gitignore`.
//! - **Codex-Style Tamper Guard:** Project `.zenko/config.toml` and `.zenko/filters/` stay
//!   read-only to autonomous agent writes, and sessions cannot overwrite sibling session folders.
//! - **Lazy Materialization & Bounded Retention:** Read-only/trivial chats create zero folders;
//!   workspaces materialize on demand and prune automatically by count and age.
//!
//! Zero regex, zero `if/else` ladders, zero hardcoded strings, zero `.unwrap()` in production paths.

use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};
use zenko_core::{
    BrainstormSpec, ExecutionPlan, FILE_MODE_OWNER_ONLY, PlanStepStatus, Result, SessionTask,
    SessionTaskStatus, ZENKO_GIT_EXCLUDE_ENTRY, ZENKO_PROJECT_DIR,
    ZENKO_PROJECT_LATEST_SESSION_FILE, ZENKO_PROJECT_SESSIONS_DIR, ZENKO_PROTECTED_PROJECT_SUBPATHS,
    ZENKO_SESSION_ARTIFACTS_DIR, ZENKO_SESSION_MANIFEST_FILE, ZENKO_SESSION_SCRATCH_DIR,
    ZENKO_SESSION_ZPOWERS_DIR, ZPOWERS_DEBUG_FILE, ZPOWERS_FAST_TRACK_REASON_MAX_CHARS,
    ZPOWERS_MAX_DEBUG_ENTRIES, ZPOWERS_MAX_DOC_BYTES, ZPOWERS_MAX_REVIEWS, ZPOWERS_PLAN_FILE,
    ZPOWERS_PROMOTE_ALLOWED_ROOTS, ZPOWERS_REVIEWS_DIR, ZPOWERS_SPEC_FILE, ZPOWERS_STATE_FILE,
    ZPOWERS_WALKTHROUGH_FILE, ZPowersArtifactDoc, ZPowersConfig, ZPowersDebugEntry, ZPowersMode,
    ZPowersPhase, ZPowersReviewReceipt, ZPowersReviewStage, ZPowersReviewVerdict,
    ZPowersSessionView, ZPowersState, ZPowersVerificationReceipt, ZPowersWorkspaceManifest,
    ZenkoError, compact_preview, new_id,
};

// ─── Path & Security Boundary Helpers ─────────────────────────────────────────

/// Sanitizes a session ID into a strict filesystem-safe path segment (`[A-Za-z0-9_-]`, <= 64 chars).
pub fn sanitize_session_id(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    match cleaned.is_empty() {
        true => "session".to_string(),
        false => cleaned,
    }
}

pub fn sessions_root(workspace_dir: &Path) -> PathBuf {
    workspace_dir
        .join(ZENKO_PROJECT_DIR)
        .join(ZENKO_PROJECT_SESSIONS_DIR)
}

pub fn session_root(workspace_dir: &Path, session_id: &str) -> PathBuf {
    sessions_root(workspace_dir).join(sanitize_session_id(session_id))
}

pub fn zpowers_dir(workspace_dir: &Path, session_id: &str) -> PathBuf {
    session_root(workspace_dir, session_id).join(ZENKO_SESSION_ZPOWERS_DIR)
}

pub fn session_scratch_dir(workspace_dir: &Path, session_id: &str) -> PathBuf {
    session_root(workspace_dir, session_id).join(ZENKO_SESSION_SCRATCH_DIR)
}

pub fn session_artifacts_dir(workspace_dir: &Path, session_id: &str) -> PathBuf {
    session_root(workspace_dir, session_id).join(ZENKO_SESSION_ARTIFACTS_DIR)
}

pub fn workspace_rel_session_dir(session_id: &str) -> String {
    format!(
        "{ZENKO_PROJECT_DIR}/{ZENKO_PROJECT_SESSIONS_DIR}/{}",
        sanitize_session_id(session_id)
    )
}

pub fn workspace_rel_zpowers_dir(session_id: &str) -> String {
    format!(
        "{}/{ZENKO_SESSION_ZPOWERS_DIR}",
        workspace_rel_session_dir(session_id)
    )
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => out.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(s) => out.push(s),
        }
    }
    out
}

fn relative_normalized_slashes(workspace_dir: &Path, raw_path: &str) -> Option<String> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let ws = normalize_lexical(
        &workspace_dir
            .canonicalize()
            .unwrap_or_else(|_| workspace_dir.to_path_buf()),
    );
    let candidate = Path::new(trimmed);
    let full = match candidate.is_absolute() {
        true => normalize_lexical(candidate),
        false => normalize_lexical(&ws.join(candidate)),
    };
    let rel = full.strip_prefix(&ws).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// Codex-style tamper protection for `<workspace>/.zenko/`:
/// - Blocks autonomous writes to `.zenko/config.toml` and `.zenko/filters/*`.
/// - Blocks direct writes to `.zenko/sessions/latest.json`.
/// - Blocks cross-session writes to `.zenko/sessions/<other_sid>/*`.
pub fn check_protected_project_zenko_write(
    workspace_dir: &Path,
    raw_path: &str,
    active_session_id: Option<&str>,
) -> Option<String> {
    let rel = relative_normalized_slashes(workspace_dir, raw_path)?;
    let lower = rel.to_ascii_lowercase();

    for protected in ZENKO_PROTECTED_PROJECT_SUBPATHS {
        let p_lower = protected.to_ascii_lowercase();
        if lower == p_lower || lower.starts_with(&format!("{p_lower}/")) {
            return Some(format!(
                "Write to protected project config path `{rel}` is blocked (`.zenko/config.toml` and `.zenko/filters/` are read-only to autonomous agents)."
            ));
        }
    }

    let sessions_prefix = format!("{ZENKO_PROJECT_DIR}/{ZENKO_PROJECT_SESSIONS_DIR}/");
    if let Some(after_sessions) = rel.strip_prefix(&sessions_prefix) {
        if after_sessions.eq_ignore_ascii_case(ZENKO_PROJECT_LATEST_SESSION_FILE) {
            return Some(format!(
                "Direct write to `{rel}` is managed automatically by Zenko."
            ));
        }
        if let Some(active_sid) = active_session_id {
            let safe_active = sanitize_session_id(active_sid);
            let target_seg = after_sessions
                .split('/')
                .next()
                .unwrap_or("")
                .trim();
            if !target_seg.is_empty() && target_seg != safe_active {
                return Some(format!(
                    "Cross-session write to `{rel}` is blocked; session `{safe_active}` may only write inside `{}/{}`.",
                    sessions_prefix.trim_end_matches('/'),
                    safe_active
                ));
            }
        }
    }

    None
}

/// Returns `true` when `raw_path` targets the active session's own `.zenko/sessions/<sid>/` folder
/// (so writing session scratch/zpowers files never trips the code-mutation plan-first gate).
pub fn is_path_inside_active_session_workspace(
    workspace_dir: &Path,
    raw_path: &str,
    session_id: &str,
) -> bool {
    let Some(rel) = relative_normalized_slashes(workspace_dir, raw_path) else {
        return false;
    };
    let own_prefix = format!("{}/", workspace_rel_session_dir(session_id));
    rel.starts_with(&own_prefix)
}

// ─── Local Git Exclude & Atomic I/O ───────────────────────────────────────────

/// Resolves the common `.git` directory for `workspace_dir`, supporting normal repositories
/// and linked Git worktrees (via `gitdir:` + optional `commondir`, zero regex).
fn resolve_git_common_dir(workspace_dir: &Path) -> Option<PathBuf> {
    let mut cur = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    loop {
        let dot_git = cur.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            let raw = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir_line = raw
                .lines()
                .find_map(|l| l.trim().strip_prefix("gitdir:"))
                .map(str::trim)?;
            let gitdir_path = Path::new(gitdir_line);
            let resolved_gitdir = match gitdir_path.is_absolute() {
                true => normalize_lexical(gitdir_path),
                false => normalize_lexical(&cur.join(gitdir_path)),
            };
            let commondir_file = resolved_gitdir.join("commondir");
            if let Ok(common_raw) = std::fs::read_to_string(&commondir_file) {
                let common_trimmed = common_raw.trim();
                if !common_trimmed.is_empty() {
                    let common_path = Path::new(common_trimmed);
                    return Some(match common_path.is_absolute() {
                        true => normalize_lexical(common_path),
                        false => normalize_lexical(&resolved_gitdir.join(common_path)),
                    });
                }
            }
            return Some(resolved_gitdir);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Reads current git branch name deterministically from `.git/HEAD` (zero subprocess overhead).
fn read_git_branch_fast(workspace_dir: &Path) -> Option<String> {
    let mut cur = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    let git_dir = loop {
        let dot_git = cur.join(".git");
        if dot_git.is_dir() {
            break Some(dot_git);
        }
        if dot_git.is_file() {
            let raw = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir_line = raw
                .lines()
                .find_map(|l| l.trim().strip_prefix("gitdir:"))
                .map(str::trim)?;
            let p = Path::new(gitdir_line);
            break Some(match p.is_absolute() {
                true => normalize_lexical(p),
                false => normalize_lexical(&cur.join(p)),
            });
        }
        if !cur.pop() {
            break None;
        }
    }?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let trimmed = head.trim();
    match trimmed.strip_prefix("ref: refs/heads/") {
        Some(branch) if !branch.is_empty() => Some(branch.to_string()),
        _ => trimmed.get(..12).map(String::from),
    }
}

/// Idempotently adds `.zenko/sessions/` to local `.git/info/exclude` without touching `.gitignore`.
pub fn ensure_git_info_exclude(workspace_dir: &Path) -> bool {
    let Some(common_git) = resolve_git_common_dir(workspace_dir) else {
        return false;
    };
    let info_dir = common_git.join("info");
    if std::fs::create_dir_all(&info_dir).is_err() {
        return false;
    }
    let exclude_path = info_dir.join("exclude");
    let existing = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    let already_ignored = existing.lines().any(|line| {
        matches!(
            line.trim(),
            ".zenko/sessions/"
                | ".zenko/sessions"
                | "/.zenko/sessions/"
                | "/.zenko/sessions"
                | ".zenko/"
                | ".zenko"
                | "/.zenko/"
                | "/.zenko"
        )
    });
    if already_ignored {
        return false;
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str("# Zenko per-session workspace (auto-excluded; keeps git status clean)\n");
    updated.push_str(ZENKO_GIT_EXCLUDE_ENTRY);
    updated.push('\n');
    atomic_write_bytes(&exclude_path, updated.as_bytes()).is_ok()
}

fn clamp_doc_content(content: &str) -> String {
    match content.len() > ZPOWERS_MAX_DOC_BYTES {
        true => {
            let cut = content.floor_char_boundary(ZPOWERS_MAX_DOC_BYTES);
            format!("{}\n\n<!-- truncated at {} bytes -->\n", &content[..cut], ZPOWERS_MAX_DOC_BYTES)
        }
        false => content.to_string(),
    }
}

fn atomic_write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("tmp");
    let tmp_path = path.with_file_name(format!(".{file_name}.{}.tmp", new_id()));
    std::fs::write(&tmp_path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(FILE_MODE_OWNER_ONLY));
    }
    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err.into());
    }
    Ok(())
}

fn atomic_write_text(path: &Path, text: &str) -> Result<()> {
    let clamped = clamp_doc_content(text);
    atomic_write_bytes(path, clamped.as_bytes())
}

// ─── Workspace Materialization & State Persistence ────────────────────────────

/// Lazily initializes `<workspace>/.zenko/sessions/<sid>/{zpowers/reviews,artifacts,scratch}`,
/// ensures `.git/info/exclude`, and writes `session.json` + `.zenko/sessions/latest.json`.
pub fn ensure_session_workspace(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    phase: ZPowersPhase,
    cfg: &ZPowersConfig,
) -> Result<PathBuf> {
    let root = session_root(workspace_dir, session_id);
    let zp = root.join(ZENKO_SESSION_ZPOWERS_DIR);
    std::fs::create_dir_all(zp.join(ZPOWERS_REVIEWS_DIR))?;
    std::fs::create_dir_all(root.join(ZENKO_SESSION_ARTIFACTS_DIR))?;
    std::fs::create_dir_all(root.join(ZENKO_SESSION_SCRATCH_DIR))?;

    if cfg.auto_git_exclude {
        let _ = ensure_git_info_exclude(workspace_dir);
    }

    let manifest_path = root.join(ZENKO_SESSION_MANIFEST_FILE);
    let now = Utc::now();
    let created_at = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ZPowersWorkspaceManifest>(&raw).ok())
        .map(|m| m.created_at)
        .unwrap_or(now);

    let manifest = ZPowersWorkspaceManifest {
        session_id: session_id.to_string(),
        title: title.trim().to_string(),
        parent_session_id: parent_session_id.map(String::from),
        workspace_dir: workspace_dir.display().to_string(),
        git_branch: read_git_branch_fast(workspace_dir),
        phase,
        created_at,
        updated_at: now,
    };
    if let Ok(raw) = serde_json::to_string_pretty(&manifest) {
        let _ = atomic_write_bytes(&manifest_path, raw.as_bytes());
    }

    let latest_path = sessions_root(workspace_dir).join(ZENKO_PROJECT_LATEST_SESSION_FILE);
    let latest_payload = json!({
        "session_id": session_id,
        "dir": sanitize_session_id(session_id),
        "updated_at": now,
    });
    if let Ok(raw) = serde_json::to_string_pretty(&latest_payload) {
        let _ = atomic_write_bytes(&latest_path, raw.as_bytes());
    }

    Ok(root)
}

pub fn load_state(
    workspace_dir: &Path,
    session_id: &str,
    default_mode: ZPowersMode,
) -> ZPowersState {
    let state_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_STATE_FILE);
    let mut state = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ZPowersState>(&raw).ok())
        .unwrap_or_else(|| ZPowersState::new(session_id, default_mode));
    state.mode = default_mode;
    state
}

pub fn is_materialized(workspace_dir: &Path, session_id: &str) -> bool {
    zpowers_dir(workspace_dir, session_id).exists()
}

pub fn save_state(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    state: &mut ZPowersState,
    cfg: &ZPowersConfig,
) -> Result<PathBuf> {
    state.updated_at = Utc::now();
    ensure_session_workspace(
        workspace_dir,
        session_id,
        title,
        parent_session_id,
        state.phase,
        cfg,
    )?;
    let state_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_STATE_FILE);
    let raw = serde_json::to_string_pretty(state)
        .map_err(|e| ZenkoError::Tool(format!("failed to serialize zpowers state: {e}")))?;
    atomic_write_bytes(&state_path, raw.as_bytes())?;
    Ok(state_path)
}

// ─── Deterministic Markdown Renderers & Sync Actions ──────────────────────────

fn render_brainstorm_spec_markdown(spec: &BrainstormSpec) -> String {
    let mut md = String::new();
    md.push_str("# Design Spec\n\n");
    md.push_str(&format!(
        "- **Spec ID:** `{}`\n- **Stage:** `{}`\n- **Updated:** `{}`\n\n",
        spec.id,
        spec.stage.as_str(),
        spec.created_at.to_rfc3339()
    ));
    md.push_str("## Problem\n\n");
    md.push_str(spec.problem.trim());
    md.push_str("\n\n");

    if !spec.invariants.is_empty() {
        md.push_str("## Invariants\n\n");
        for item in &spec.invariants {
            md.push_str(&format!("- {item}\n"));
        }
        md.push('\n');
    }
    if !spec.assumptions.is_empty() {
        md.push_str("## Assumptions\n\n");
        for item in &spec.assumptions {
            md.push_str(&format!("- {item}\n"));
        }
        md.push('\n');
    }
    if !spec.blind_spots.is_empty() {
        md.push_str("## Blind Spots & Edge Cases\n\n");
        for item in &spec.blind_spots {
            md.push_str(&format!("- {item}\n"));
        }
        md.push('\n');
    }
    if !spec.decision_criteria.is_empty() {
        md.push_str("## Decision Criteria\n\n");
        for item in &spec.decision_criteria {
            md.push_str(&format!("- {item}\n"));
        }
        md.push('\n');
    }
    if !spec.options.is_empty() {
        md.push_str("## Candidate Options\n\n");
        for opt in &spec.options {
            let selected_mark = match !spec.selected_option_id.trim().is_empty()
                && spec.selected_option_id.trim() == opt.id.as_str()
            {
                true => " **(SELECTED)**",
                false => match opt.recommended {
                    true => " *(recommended)*",
                    false => "",
                },
            };
            md.push_str(&format!("### `{}` — {}{}\n\n", opt.id, opt.title, selected_mark));
            md.push_str(opt.summary.trim());
            md.push_str(&format!(
                "\n\n- Blast radius: `{}` · Reversibility: `{}` · Safety: `{}/10` · Testability: `{}/10`\n",
                opt.blast_radius, opt.reversibility, opt.safety_score, opt.testability_score
            ));
            if !opt.touched_files.is_empty() {
                md.push_str(&format!("- Touched files: `{}`\n", opt.touched_files.join("`, `")));
            }
            for pro in &opt.pros {
                md.push_str(&format!("- Pro: {pro}\n"));
            }
            for con in &opt.cons {
                md.push_str(&format!("- Con: {con}\n"));
            }
            for risk in &opt.risks {
                md.push_str(&format!("- Risk: {risk}\n"));
            }
            md.push('\n');
        }
    }
    if !spec.selection_rationale.trim().is_empty() {
        md.push_str("## Selection Rationale\n\n");
        md.push_str(spec.selection_rationale.trim());
        md.push_str("\n\n");
    }
    if !spec.critique.is_empty() {
        md.push_str("## Adversarial Critique\n\n");
        for c in &spec.critique {
            md.push_str(&format!("- {c}\n"));
        }
        md.push('\n');
    }
    if !spec.verification_plan.is_empty() {
        md.push_str("## Verification Plan\n\n");
        for v in &spec.verification_plan {
            md.push_str(&format!("- {v}\n"));
        }
        md.push('\n');
    }
    md
}

fn plan_status_checkbox(status: PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Pending => "[ ]",
        PlanStepStatus::InProgress => "[/]",
        PlanStepStatus::Completed => "[x]",
        PlanStepStatus::Blocked => "[!]",
    }
}

fn task_status_checkbox(status: SessionTaskStatus) -> &'static str {
    match status {
        SessionTaskStatus::Pending => "[ ]",
        SessionTaskStatus::InProgress => "[/]",
        SessionTaskStatus::Done => "[x]",
        SessionTaskStatus::Blocked => "[!]",
        SessionTaskStatus::Failed => "[!]",
    }
}

fn render_execution_plan_markdown(plan: &ExecutionPlan) -> String {
    let mut md = String::new();
    let heading = match plan.goal.trim().is_empty() {
        true => "Implementation Plan".to_string(),
        false => format!("Implementation Plan — {}", compact_preview(plan.goal.trim(), 96)),
    };
    md.push_str(&format!("# {heading}\n\n"));
    md.push_str(&format!(
        "- **Plan ID:** `{}`\n- **Updated:** `{}`\n\n",
        plan.id,
        plan.updated_at.to_rfc3339()
    ));
    if !plan.goal.trim().is_empty() {
        md.push_str("## Goal\n\n");
        md.push_str(plan.goal.trim());
        md.push_str("\n\n");
    }
    md.push_str("## Tasks\n\n");
    for step in &plan.steps {
        let box_str = plan_status_checkbox(step.status);
        md.push_str(&format!("- {box_str} **`{}`** — {}", step.id, step.title.trim()));
        if !step.depends_on.is_empty() {
            md.push_str(&format!(" *(depends on: {})*", step.depends_on.join(", ")));
        }
        md.push('\n');
        if let Some(ref verify) = step.verification
            && !verify.trim().is_empty()
        {
            md.push_str(&format!("  - Verify: `{}`\n", verify.trim()));
        }
    }
    md
}

fn render_session_tasks_markdown(tasks: &[SessionTask]) -> String {
    let mut md = String::new();
    md.push_str("# Implementation Plan (Live Session Task DAG)\n\n");
    md.push_str(&format!("- **Updated:** `{}`\n- **Tasks:** `{}`\n\n", Utc::now().to_rfc3339(), tasks.len()));
    md.push_str("## Tasks\n\n");
    for task in tasks {
        let box_str = task_status_checkbox(task.status);
        md.push_str(&format!("- {box_str} **`{}`** — {}", task.id, task.title.trim()));
        if !task.depends_on.is_empty() {
            md.push_str(&format!(" *(depends on: {})*", task.depends_on.join(", ")));
        }
        if let Some(ref owner) = task.owner_agent
            && !owner.trim().is_empty()
        {
            md.push_str(&format!(" *(owner: `{}`)*", owner.trim()));
        }
        md.push('\n');
        if !task.description.trim().is_empty() {
            md.push_str(&format!("  - Scope: {}\n", task.description.trim()));
        }
        if let Some(ref cmd) = task.verification_cmd
            && !cmd.trim().is_empty()
        {
            md.push_str(&format!("  - Verify: `{}`\n", cmd.trim()));
        }
        if let Some(ref ev) = task.verification_evidence
            && !ev.trim().is_empty()
        {
            md.push_str(&format!("  - Evidence: {}\n", ev.trim()));
        }
        if let Some(ref notes) = task.notes
            && !notes.trim().is_empty()
        {
            md.push_str(&format!("  - Notes: {}\n", notes.trim()));
        }
    }
    md
}

pub fn sync_brainstorm_spec(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    spec: &BrainstormSpec,
    cfg: &ZPowersConfig,
) -> Result<(PathBuf, ZPowersState)> {
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    state.spec_recorded = true;
    state.phase = match state.phase {
        ZPowersPhase::Idle | ZPowersPhase::Recon => ZPowersPhase::Spec,
        other => other,
    };
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;
    let spec_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_SPEC_FILE);
    atomic_write_text(&spec_path, &render_brainstorm_spec_markdown(spec))?;
    Ok((spec_path, state))
}

pub fn write_spec_doc(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    doc_title: Option<&str>,
    content: &str,
    cfg: &ZPowersConfig,
) -> Result<(PathBuf, ZPowersState)> {
    let body = content.trim();
    if body.is_empty() {
        return Err(ZenkoError::Tool("`content` is required for `write_spec`".into()));
    }
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    state.spec_recorded = true;
    state.phase = match state.phase {
        ZPowersPhase::Idle | ZPowersPhase::Recon => ZPowersPhase::Spec,
        other => other,
    };
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;
    let heading = doc_title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Design Spec");
    let rendered = match body.starts_with('#') {
        true => format!("{body}\n"),
        false => format!("# {heading}\n\n{body}\n"),
    };
    let spec_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_SPEC_FILE);
    atomic_write_text(&spec_path, &rendered)?;
    Ok((spec_path, state))
}

pub fn sync_execution_plan(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    plan: &ExecutionPlan,
    cfg: &ZPowersConfig,
) -> Result<(PathBuf, ZPowersState)> {
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    state.plan_recorded = true;
    state.phase = match state.phase {
        ZPowersPhase::Idle | ZPowersPhase::Recon | ZPowersPhase::Spec => ZPowersPhase::Plan,
        other => other,
    };
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;
    let plan_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_PLAN_FILE);
    atomic_write_text(&plan_path, &render_execution_plan_markdown(plan))?;
    Ok((plan_path, state))
}

pub fn sync_session_tasks_plan(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    tasks: &[SessionTask],
    cfg: &ZPowersConfig,
) -> Result<Option<(PathBuf, ZPowersState)>> {
    if tasks.is_empty() {
        return Ok(None);
    }
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    state.plan_recorded = true;
    state.phase = match state.phase {
        ZPowersPhase::Idle | ZPowersPhase::Recon | ZPowersPhase::Spec => ZPowersPhase::Plan,
        other => other,
    };
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;
    let plan_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_PLAN_FILE);
    atomic_write_text(&plan_path, &render_session_tasks_markdown(tasks))?;
    Ok(Some((plan_path, state)))
}

pub fn write_plan_doc(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    doc_title: Option<&str>,
    content: &str,
    cfg: &ZPowersConfig,
) -> Result<(PathBuf, ZPowersState)> {
    let body = content.trim();
    if body.is_empty() {
        return Err(ZenkoError::Tool("`content` is required for `write_plan`".into()));
    }
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    state.plan_recorded = true;
    state.phase = match state.phase {
        ZPowersPhase::Idle | ZPowersPhase::Recon | ZPowersPhase::Spec => ZPowersPhase::Plan,
        other => other,
    };
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;
    let heading = doc_title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Implementation Plan");
    let rendered = match body.starts_with('#') {
        true => format!("{body}\n"),
        false => format!("# {heading}\n\n{body}\n"),
    };
    let plan_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_PLAN_FILE);
    atomic_write_text(&plan_path, &rendered)?;
    Ok((plan_path, state))
}

fn render_debug_markdown(state: &ZPowersState, strike_limit: usize) -> String {
    let mut md = String::new();
    md.push_str("# Systematic Debugging Notebook\n\n");
    md.push_str(&format!(
        "- **Active Strikes:** `{}/{}`\n- **Phase:** `{}`\n- **Updated:** `{}`\n\n",
        state.debug_strikes,
        strike_limit.max(1),
        state.phase.as_str(),
        state.updated_at.to_rfc3339()
    ));
    if state.debug_strikes >= strike_limit.max(1) {
        md.push_str("> **3-Strike Architectural Breaker Triggered:** Stop patching symptoms. Question underlying assumptions/design before another code change.\n\n");
    }
    for (idx, entry) in state.debug_entries.iter().enumerate() {
        md.push_str(&format!(
            "## Entry {} (Strike {}, {})\n\n- **Hypothesis:** {}\n- **Evidence:** {}\n- **Outcome:** {}\n\n",
            idx + 1,
            entry.strike,
            entry.recorded_at.to_rfc3339(),
            entry.hypothesis,
            entry.evidence,
            entry.outcome
        ));
    }
    md
}

#[allow(clippy::too_many_arguments)]
pub fn log_debug_entry(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    hypothesis: &str,
    evidence: &str,
    outcome: &str,
    increment_strike: bool,
    cfg: &ZPowersConfig,
) -> Result<(PathBuf, ZPowersState)> {
    let hyp = hypothesis.trim();
    if hyp.is_empty() {
        return Err(ZenkoError::Tool("`hypothesis` is required for `log_debug`".into()));
    }
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    if increment_strike {
        state.debug_strikes = state.debug_strikes.saturating_add(1);
    }
    state.phase = ZPowersPhase::Debug;
    state.debug_entries.push(ZPowersDebugEntry {
        strike: state.debug_strikes,
        hypothesis: hyp.to_string(),
        evidence: evidence.trim().to_string(),
        outcome: outcome.trim().to_string(),
        recorded_at: Utc::now(),
    });
    if state.debug_entries.len() > ZPOWERS_MAX_DEBUG_ENTRIES {
        let excess = state.debug_entries.len() - ZPOWERS_MAX_DEBUG_ENTRIES;
        state.debug_entries.drain(..excess);
    }
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;
    let debug_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_DEBUG_FILE);
    atomic_write_text(&debug_path, &render_debug_markdown(&state, cfg.debug_strike_limit))?;
    Ok((debug_path, state))
}

#[allow(clippy::too_many_arguments)]
pub fn record_review(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    stage: ZPowersReviewStage,
    verdict: ZPowersReviewVerdict,
    summary: &str,
    findings: &[String],
    cfg: &ZPowersConfig,
) -> Result<(PathBuf, ZPowersReviewReceipt, ZPowersState)> {
    let clean_summary = summary.trim();
    if clean_summary.is_empty() {
        return Err(ZenkoError::Tool("`summary` is required for `record_review`".into()));
    }
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    let next_num = state.reviews.len().saturating_add(1);
    let filename = format!("{:02}-{}-review.md", next_num, stage.as_str());
    let now = Utc::now();
    let receipt = ZPowersReviewReceipt {
        number: next_num,
        stage,
        verdict,
        summary: compact_preview(clean_summary, 280),
        file: format!("{ZPOWERS_REVIEWS_DIR}/{filename}"),
        recorded_at: now,
    };
    state.reviews.push(receipt.clone());
    if state.reviews.len() > ZPOWERS_MAX_REVIEWS {
        let excess = state.reviews.len() - ZPOWERS_MAX_REVIEWS;
        state.reviews.drain(..excess);
    }
    state.phase = match verdict {
        ZPowersReviewVerdict::Approved => ZPowersPhase::Review,
        ZPowersReviewVerdict::ChangesRequested | ZPowersReviewVerdict::Blocked => {
            ZPowersPhase::Execute
        }
    };
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;

    let mut md = format!(
        "# Two-Stage Review #{next_num} — {} ({})\n\n- **Stage:** `{}`\n- **Verdict:** `{}`\n- **Recorded:** `{}`\n\n## Summary\n\n{}\n",
        stage.as_str(),
        verdict.as_str(),
        stage.as_str(),
        verdict.as_str(),
        now.to_rfc3339(),
        clean_summary
    );
    if !findings.is_empty() {
        md.push_str("\n## Findings\n\n");
        for f in findings {
            let trimmed = f.trim();
            if !trimmed.is_empty() {
                md.push_str(&format!("- {trimmed}\n"));
            }
        }
    }
    let review_path = zpowers_dir(workspace_dir, session_id)
        .join(ZPOWERS_REVIEWS_DIR)
        .join(&filename);
    atomic_write_text(&review_path, &md)?;
    Ok((review_path, receipt, state))
}

#[allow(clippy::too_many_arguments)]
pub fn write_walkthrough(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    doc_title: Option<&str>,
    summary: &str,
    files_changed: &[String],
    verification: &[String],
    next_steps: &[String],
    cfg: &ZPowersConfig,
) -> Result<(PathBuf, ZPowersState)> {
    let clean_summary = summary.trim();
    if clean_summary.is_empty() {
        return Err(ZenkoError::Tool(
            "`summary` (or `content`) is required for `write_walkthrough`".into(),
        ));
    }
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    state.walkthrough_recorded = true;
    state.phase = ZPowersPhase::Done;
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;

    let heading = doc_title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Session Walkthrough & Verification Handoff");
    let mut md = format!(
        "# {heading}\n\n- **Session:** `{session_id}`\n- **Updated:** `{}`\n\n## Summary\n\n{clean_summary}\n",
        state.updated_at.to_rfc3339()
    );

    let mut merged_files: BTreeSet<&str> = BTreeSet::new();
    for f in &state.mutated_files {
        if !f.trim().is_empty() {
            merged_files.insert(f.trim());
        }
    }
    for f in files_changed {
        if !f.trim().is_empty() {
            merged_files.insert(f.trim());
        }
    }
    if !merged_files.is_empty() {
        md.push_str("\n## Files Changed\n\n");
        for f in merged_files {
            md.push_str(&format!("- `{f}`\n"));
        }
    }

    md.push_str("\n## Verification Evidence\n\n");
    if let Some(ref receipt) = state.last_verification {
        md.push_str(&format!(
            "- Runtime check (`{}`): **{}** — {}\n",
            receipt.command,
            match receipt.passed {
                true => "PASSED",
                false => "FAILED",
            },
            receipt.summary
        ));
    }
    for v in verification {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            md.push_str(&format!("- {trimmed}\n"));
        }
    }
    if !state.reviews.is_empty() {
        md.push_str("\n## Reviews\n\n");
        for r in &state.reviews {
            md.push_str(&format!(
                "- #{} `{}` → `{}` (`{}`): {}\n",
                r.number,
                r.stage.as_str(),
                r.verdict.as_str(),
                r.file,
                r.summary
            ));
        }
    }
    if !next_steps.is_empty() {
        md.push_str("\n## Next Steps / Caveats\n\n");
        for n in next_steps {
            let trimmed = n.trim();
            if !trimmed.is_empty() {
                md.push_str(&format!("- {trimmed}\n"));
            }
        }
    }

    let wt_path = zpowers_dir(workspace_dir, session_id).join(ZPOWERS_WALKTHROUGH_FILE);
    atomic_write_text(&wt_path, &md)?;
    Ok((wt_path, state))
}

pub fn record_fast_track(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    reason: &str,
    cfg: &ZPowersConfig,
) -> Result<ZPowersState> {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return Err(ZenkoError::Tool(
            "`reason` is required for `fast_track` so plan-gate bypasses stay auditable.".into(),
        ));
    }
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    state.fast_track_reason = Some(compact_preview(trimmed, ZPOWERS_FAST_TRACK_REASON_MAX_CHARS));
    state.phase = match state.phase {
        ZPowersPhase::Idle | ZPowersPhase::Recon => ZPowersPhase::Execute,
        other => other,
    };
    save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg)?;
    Ok(state)
}

#[allow(clippy::too_many_arguments)]
pub fn record_verification_if_materialized(
    workspace_dir: &Path,
    session_id: &str,
    title: &str,
    parent_session_id: Option<&str>,
    command: &str,
    passed: bool,
    summary: &str,
    mutated_files: &BTreeSet<String>,
    cfg: &ZPowersConfig,
) {
    if !cfg.sync_artifacts_to_workspace || (!is_materialized(workspace_dir, session_id) && mutated_files.is_empty()) {
        return;
    }
    let mut state = load_state(workspace_dir, session_id, cfg.mode);
    let mut merged: BTreeSet<String> = state.mutated_files.iter().cloned().collect();
    for f in mutated_files {
        merged.insert(f.clone());
    }
    state.mutated_files = merged.into_iter().collect();
    state.last_verification = Some(ZPowersVerificationReceipt {
        command: compact_preview(command.trim(), 180),
        passed,
        summary: compact_preview(summary.trim(), 240),
        recorded_at: Utc::now(),
    });
    if passed {
        state.debug_strikes = 0;
        if !matches!(state.phase, ZPowersPhase::Review | ZPowersPhase::Done) {
            state.phase = ZPowersPhase::Verify;
        }
    }
    let _ = save_state(workspace_dir, session_id, title, parent_session_id, &mut state, cfg);
}

/// Safely promotes a session ZPowers doc (`spec.md`, `plan.md`, `walkthrough.md`, or `reviews/<name>`)
/// into a tracked project folder under an allowed root (`docs/` or `knowledge/`).
pub fn promote_doc(
    workspace_dir: &Path,
    session_id: &str,
    doc_name: &str,
    dest_rel: &str,
) -> Result<PathBuf> {
    let clean_doc = doc_name.trim().trim_start_matches('/');
    let src_path = match clean_doc {
        ZPOWERS_SPEC_FILE | "spec" => zpowers_dir(workspace_dir, session_id).join(ZPOWERS_SPEC_FILE),
        ZPOWERS_PLAN_FILE | "plan" => zpowers_dir(workspace_dir, session_id).join(ZPOWERS_PLAN_FILE),
        ZPOWERS_DEBUG_FILE | "debug" => zpowers_dir(workspace_dir, session_id).join(ZPOWERS_DEBUG_FILE),
        ZPOWERS_WALKTHROUGH_FILE | "walkthrough" => {
            zpowers_dir(workspace_dir, session_id).join(ZPOWERS_WALKTHROUGH_FILE)
        }
        other => match other.strip_prefix(&format!("{ZPOWERS_REVIEWS_DIR}/")) {
            Some(fname)
                if !fname.is_empty()
                    && !fname.contains('/')
                    && !fname.contains('\\')
                    && !fname.contains("..") =>
            {
                zpowers_dir(workspace_dir, session_id)
                    .join(ZPOWERS_REVIEWS_DIR)
                    .join(fname)
            }
            _ => {
                return Err(ZenkoError::Tool(format!(
                    "Unsupported zpowers doc `{doc_name}`. Expected `spec.md`, `plan.md`, `debug.md`, `walkthrough.md`, or `reviews/<file>`."
                )));
            }
        },
    };

    let content = std::fs::read_to_string(&src_path).map_err(|_| {
        ZenkoError::Tool(format!(
            "ZPowers doc `{clean_doc}` does not exist yet for session `{session_id}`."
        ))
    })?;

    let clean_dest = dest_rel.trim().trim_start_matches("./").trim_start_matches('/');
    let allowed_root = ZPOWERS_PROMOTE_ALLOWED_ROOTS.iter().any(|root| {
        clean_dest.starts_with(&format!("{root}/"))
    });
    if !allowed_root || clean_dest.contains("..") {
        return Err(ZenkoError::PolicyDenied(format!(
            "Promote destination `{dest_rel}` must be a relative path inside one of: {}",
            ZPOWERS_PROMOTE_ALLOWED_ROOTS.join(", ")
        )));
    }
    let resolved_dest = zenko_tools::jail::resolve_jailed_path(workspace_dir, clean_dest, false)?;
    atomic_write_text(&resolved_dest, &content)?;
    Ok(resolved_dest)
}

// ─── Inspection, Compaction Recall & Retention GC ─────────────────────────────

fn collect_doc_meta(
    workspace_dir: &Path,
    abs_path: &Path,
    display_name: &str,
    include_content: bool,
) -> Option<ZPowersArtifactDoc> {
    let meta = std::fs::metadata(abs_path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let updated_at: DateTime<Utc> = meta
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now);
    let rel_path = abs_path
        .strip_prefix(workspace_dir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| display_name.to_string());
    let content = match include_content {
        true => std::fs::read_to_string(abs_path).ok(),
        false => None,
    };
    Some(ZPowersArtifactDoc {
        name: display_name.to_string(),
        rel_path,
        bytes: meta.len(),
        updated_at,
        content,
    })
}

pub fn inspect_session_zpowers(
    workspace_dir: &Path,
    session_id: &str,
    default_mode: ZPowersMode,
    include_content: bool,
) -> ZPowersSessionView {
    let zp = zpowers_dir(workspace_dir, session_id);
    let materialized = zp.exists();
    let state = load_state(workspace_dir, session_id, default_mode);
    let mut docs = Vec::new();
    if materialized {
        for name in [
            ZPOWERS_SPEC_FILE,
            ZPOWERS_PLAN_FILE,
            ZPOWERS_DEBUG_FILE,
            ZPOWERS_WALKTHROUGH_FILE,
        ] {
            if let Some(doc) = collect_doc_meta(workspace_dir, &zp.join(name), name, include_content)
            {
                docs.push(doc);
            }
        }
        let rev_dir = zp.join(ZPOWERS_REVIEWS_DIR);
        if let Ok(entries) = std::fs::read_dir(&rev_dir) {
            let mut rev_files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|de| de.path()))
                .filter(|p| p.is_file())
                .collect();
            rev_files.sort();
            for path in rev_files {
                if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                    let display = format!("{ZPOWERS_REVIEWS_DIR}/{fname}");
                    if let Some(doc) = collect_doc_meta(workspace_dir, &path, &display, include_content)
                    {
                        docs.push(doc);
                    }
                }
            }
        }
    }
    ZPowersSessionView {
        session_id: session_id.to_string(),
        workspace_rel_dir: workspace_rel_zpowers_dir(session_id),
        materialized,
        state,
        docs,
    }
}

/// Generates a compact Tier-3 `<zpowers_state>` block for compaction survival and subagent
/// cold-start alignment (strictly zero Tier-1 KV-cache mutation).
pub fn build_zpowers_recall_block(
    workspace_dir: &Path,
    session_id: &str,
    default_mode: ZPowersMode,
) -> Option<String> {
    let view = inspect_session_zpowers(workspace_dir, session_id, default_mode, false);
    if !view.materialized && view.docs.is_empty() {
        return None;
    }
    let st = &view.state;
    let doc_list = match view.docs.is_empty() {
        true => "(none yet)".to_string(),
        false => view
            .docs
            .iter()
            .map(|d| d.rel_path.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    };
    let verify_line = match &st.last_verification {
        Some(v) => format!(
            "{} (`{}`)",
            match v.passed {
                true => "passed",
                false => "failed",
            },
            v.command
        ),
        None => "none yet".to_string(),
    };
    let fast_track_line = match &st.fast_track_reason {
        Some(r) => format!(" · fast_track=\"{r}\""),
        None => String::new(),
    };
    Some(format!(
        "<zpowers_state>\n\
         Session ZPowers workspace: `{}` · mode=`{}` · phase=`{}` · spec={} · plan={} · debug_strikes={} · reviews={} · walkthrough={}{fast_track_line}\n\
         Last verification: {verify_line}\n\
         Disk artifacts (inspect via `read_file`): {doc_list}\n\
         </zpowers_state>",
        view.workspace_rel_dir,
        st.mode.as_str(),
        st.phase.as_str(),
        st.spec_recorded,
        st.plan_recorded,
        st.debug_strikes,
        st.reviews.len(),
        st.walkthrough_recorded,
    ))
}

pub fn remove_session_workspace(workspace_dir: &Path, session_id: &str) -> bool {
    let dir = session_root(workspace_dir, session_id);
    match dir.exists() {
        true => std::fs::remove_dir_all(&dir).is_ok(),
        false => false,
    }
}

/// Bounded retention GC for `<workspace>/.zenko/sessions/`: prunes directories older than
/// `retention_days` and trims excess directories beyond `max_sessions` (oldest mtime first).
pub fn prune_workspace_sessions(
    workspace_dir: &Path,
    max_sessions: usize,
    retention_days: u64,
) -> usize {
    let root = sessions_root(workspace_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return 0;
    };
    let now = SystemTime::now();
    let max_age = Duration::from_secs(retention_days.saturating_mul(86_400));
    let mut dirs: Vec<(PathBuf, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let mtime = std::fs::metadata(path.join(ZENKO_SESSION_MANIFEST_FILE))
            .or_else(|_| entry.metadata())
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        dirs.push((path, mtime));
    }
    // Newest first
    dirs.sort_by(|a, b| b.1.cmp(&a.1));

    let mut pruned = 0usize;
    let keep_cap = max_sessions.max(1);
    for (idx, (path, mtime)) in dirs.into_iter().enumerate() {
        let too_old = retention_days > 0
            && now
                .duration_since(mtime)
                .map(|age| age > max_age)
                .unwrap_or(false);
        let over_cap = idx >= keep_cap;
        if (too_old || over_cap) && std::fs::remove_dir_all(&path).is_ok() {
            pruned += 1;
        }
    }
    pruned
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::{BrainstormOption, BrainstormStage, PlanStep};

    fn tmp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zenko_zp_{prefix}_{}", new_id()));
        std::fs::create_dir_all(&dir).expect("mkdir tmp");
        dir
    }

    #[test]
    fn git_info_exclude_is_idempotent_and_never_touches_gitignore() {
        let ws = tmp_dir("git");
        std::fs::create_dir_all(ws.join(".git/info")).expect("mkdir .git/info");
        let cfg = ZPowersConfig::default();

        ensure_session_workspace(&ws, "sess_1", "Test", None, ZPowersPhase::Plan, &cfg)
            .expect("ensure workspace");
        let exclude = std::fs::read_to_string(ws.join(".git/info/exclude")).expect("read exclude");
        assert!(exclude.contains(ZENKO_GIT_EXCLUDE_ENTRY));
        assert!(!ws.join(".gitignore").exists());

        // Second call must NOT duplicate the exclude line
        let second_added = ensure_git_info_exclude(&ws);
        assert!(!second_added);
        let exclude2 = std::fs::read_to_string(ws.join(".git/info/exclude")).expect("read exclude2");
        assert_eq!(
            exclude2.matches(ZENKO_GIT_EXCLUDE_ENTRY).count(),
            1,
            "exclude entry should appear exactly once"
        );

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn tamper_guard_blocks_project_config_and_sibling_sessions_only() {
        let ws = tmp_dir("tamper");
        assert!(check_protected_project_zenko_write(&ws, ".zenko/config.toml", Some("s1")).is_some());
        assert!(check_protected_project_zenko_write(&ws, ".zenko/filters/custom.toml", Some("s1")).is_some());
        assert!(check_protected_project_zenko_write(&ws, ".zenko/sessions/latest.json", Some("s1")).is_some());
        assert!(check_protected_project_zenko_write(&ws, ".zenko/sessions/s2/zpowers/plan.md", Some("s1")).is_some());
        assert!(check_protected_project_zenko_write(&ws, ".zenko/sessions/s1/zpowers/plan.md", Some("s1")).is_none());
        assert!(check_protected_project_zenko_write(&ws, "src/main.rs", Some("s1")).is_none());
        assert!(is_path_inside_active_session_workspace(&ws, ".zenko/sessions/s1/scratch/run.py", "s1"));
        assert!(!is_path_inside_active_session_workspace(&ws, ".zenko/sessions/s2/scratch/run.py", "s1"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn spec_plan_debug_review_walkthrough_and_promote_roundtrip() {
        let ws = tmp_dir("roundtrip");
        let cfg = ZPowersConfig::default();
        let spec = BrainstormSpec {
            id: "bs_1".into(),
            stage: BrainstormStage::Committed,
            parent_spec_id: None,
            problem: "Add ZPowers workflow".into(),
            invariants: vec!["Zero git pollution".into()],
            assumptions: vec![],
            blind_spots: vec![],
            decision_criteria: vec!["Simplicity".into()],
            options: vec![BrainstormOption {
                id: "opt_a".into(),
                title: "Dual-zone .zenko".into(),
                summary: "Local sessions + global DB".into(),
                pros: vec!["Fast".into()],
                cons: vec![],
                blast_radius: "low".into(),
                reversibility: "high".into(),
                safety_score: 9,
                testability_score: 9,
                touched_files: vec!["zpowers.rs".into()],
                rollback_steps: vec![],
                risks: vec![],
                recommended: true,
            }],
            selected_option_id: "opt_a".into(),
            selection_rationale: "Cleanest isolation".into(),
            critique: vec![],
            open_questions: vec![],
            pre_mortem: vec![],
            verification_plan: vec!["just verify".into()],
            rubric_warnings: vec![],
            created_at: Utc::now(),
        };
        let (spec_path, st1) =
            sync_brainstorm_spec(&ws, "sess_a", "ZPowers", None, &spec, &cfg).expect("sync spec");
        assert!(spec_path.exists());
        assert!(st1.spec_recorded);

        let plan = ExecutionPlan {
            id: "plan_1".into(),
            goal: "Plan first then build".into(),
            steps: vec![PlanStep {
                id: "s1".into(),
                title: "Create engine".into(),
                status: PlanStepStatus::InProgress,
                depends_on: vec![],
                verification: Some("cargo test".into()),
            }],
            rubric_warnings: vec![],
            updated_at: Utc::now(),
        };
        let (plan_path, st2) =
            sync_execution_plan(&ws, "sess_a", "ZPowers", None, &plan, &cfg).expect("sync plan");
        assert!(plan_path.exists());
        assert!(st2.plan_recorded);

        let (dbg_path, st3) = log_debug_entry(
            &ws,
            "sess_a",
            "ZPowers",
            None,
            "Check git exclude path",
            "gitdir pointer",
            "resolved commondir",
            true,
            &cfg,
        )
        .expect("log debug");
        assert!(dbg_path.exists());
        assert_eq!(st3.debug_strikes, 1);

        let (rev_path, receipt, st4) = record_review(
            &ws,
            "sess_a",
            "ZPowers",
            None,
            ZPowersReviewStage::Spec,
            ZPowersReviewVerdict::Approved,
            "All spec invariants met",
            &["No missing edge cases".into()],
            &cfg,
        )
        .expect("record review");
        assert!(rev_path.exists());
        assert_eq!(receipt.file, "reviews/01-spec-review.md");
        assert_eq!(st4.reviews.len(), 1);

        let (wt_path, st5) = write_walkthrough(
            &ws,
            "sess_a",
            "ZPowers",
            None,
            Some("ZPowers Walkthrough"),
            "Completed end-to-end.",
            &["crates/zenko-runtime/src/zpowers.rs".into()],
            &["cargo test --workspace: passed".into()],
            &[],
            &cfg,
        )
        .expect("write walkthrough");
        assert!(wt_path.exists());
        assert_eq!(st5.phase, ZPowersPhase::Done);

        // Promote spec.md to docs/ must succeed; promoting outside docs/knowledge must fail
        let promoted =
            promote_doc(&ws, "sess_a", "spec.md", "docs/zpowers-spec.md").expect("promote");
        assert!(promoted.exists());
        assert!(promote_doc(&ws, "sess_a", "spec.md", "src/escape.md").is_err());

        let view = inspect_session_zpowers(&ws, "sess_a", ZPowersMode::Guided, true);
        assert!(view.materialized);
        assert_eq!(view.docs.len(), 5);
        assert!(build_zpowers_recall_block(&ws, "sess_a", ZPowersMode::Guided).is_some());

        assert!(remove_session_workspace(&ws, "sess_a"));
        assert!(!is_materialized(&ws, "sess_a"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn prune_workspace_sessions_respects_max_cap() {
        let ws = tmp_dir("prune");
        let cfg = ZPowersConfig::default();
        for i in 0..5 {
            let sid = format!("sess_{i}");
            ensure_session_workspace(&ws, &sid, "T", None, ZPowersPhase::Plan, &cfg)
                .expect("ensure");
            std::thread::sleep(Duration::from_millis(12));
        }
        let pruned = prune_workspace_sessions(&ws, 2, 30);
        assert_eq!(pruned, 3);
        let remaining: usize = std::fs::read_dir(sessions_root(&ws))
            .expect("readdir")
            .flatten()
            .filter(|e| e.path().is_dir())
            .count();
        assert_eq!(remaining, 2);
        let _ = std::fs::remove_dir_all(&ws);
    }
}
