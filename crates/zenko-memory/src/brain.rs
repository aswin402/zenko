//! Pure-Rust Repo-Native Project Brain Engine (interoperable with `mindmuxai/brain.md` open standard).
//!
//! Guarantees:
//! - **Correct-by-construction single writer:** Frontmatter, `<!-- compiled_truth -->`, and
//!   append-only `## Timeline` entries are rendered and written atomically via temp-file rename
//!   with secret redaction (`sanitize_memory_text_with_secrets`).
//! - **Zero regex & zero `if/else` ladders:** Deterministic byte/line state-machine parser,
//!   exhaustive `match` dispatch, token-overlap search, and BFS `affects`/backlink impact graph.
//! - **Workspace-jailed `brainRoot`:** Reads `.mindmux/preferences.json` (`brainRoot`) safely,
//!   blocking any path traversal outside the active workspace directory.

use crate::algorithms::{sanitize_memory_text_with_secrets, tokenize_terms};
use crate::okf::{
    evaluate_freshness, evaluate_source_provenance, evaluate_trust_tier, extract_markdown_links,
};
use crate::turboquant::TurboQuantCodec;
use chrono::Utc;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use zenko_core::{
    BRAIN_COMPILED_TRUTH_MARKER, BRAIN_DIR_NAME, BRAIN_IMPACT_MAX_HOPS, BRAIN_MAX_AFFECTS,
    BRAIN_MAX_INDEX_PAGES, BRAIN_MAX_PAGE_BYTES, BRAIN_MAX_ROOT_BYTES, BRAIN_MAX_SLUG_CHARS,
    BRAIN_MAX_TAGS, BRAIN_MAX_TIMELINE_ENTRIES, BRAIN_MAX_TITLE_CHARS, BRAIN_PAGES_DIR_NAME,
    BRAIN_PREFS_DIR_NAME, BRAIN_PREFS_FILE_NAME, BRAIN_PREFS_ROOT_KEY, BRAIN_PROTOCOL_FILE,
    BRAIN_ROOT_SLOTS, BRAIN_TIMELINE_HEADING, BrainImpactGraph, BrainPage, BrainPageCategory,
    BrainPageStatus, BrainPageSummary, BrainRootSlot, BrainSearchHit, BrainTimelineEntry,
    BrainTimelineKind, OKF_KEY_SOURCES, OKF_KEY_STALE_AFTER, OKF_KEY_VERIFIED_AT,
    OKF_KEY_VERIFIED_BY, OkfDocStatus, OkfFreshnessState, Result, TURBOQUANT_MIN_COSINE,
    ZenkoError, compact_preview, find_brain_root_slot,
};

/// Resolves the workspace's Project Brain root directory, honoring `.mindmux/preferences.json`
/// (`brainRoot`) when jailed safely inside `workspace_dir`, otherwise defaulting to `<ws>/brain`.
pub fn resolve_brain_root(workspace_dir: &Path) -> PathBuf {
    let default_root = workspace_dir.join(BRAIN_DIR_NAME);
    let prefs_path = workspace_dir
        .join(BRAIN_PREFS_DIR_NAME)
        .join(BRAIN_PREFS_FILE_NAME);
    let Ok(raw) = std::fs::read_to_string(&prefs_path) else {
        return default_root;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return default_root;
    };
    let Some(custom) = parsed
        .get(BRAIN_PREFS_ROOT_KEY)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return default_root;
    };

    // Reject path traversal segments deterministically (zero symlink/escape outside workspace)
    if custom.split(['/', '\\']).any(|seg| seg == "..") {
        return default_root;
    }
    let candidate = match Path::new(custom).is_absolute() {
        true => PathBuf::from(custom),
        false => workspace_dir.join(custom),
    };
    let canon_ws = std::fs::canonicalize(workspace_dir).unwrap_or_else(|_| workspace_dir.to_path_buf());
    let check_anchor = match candidate.exists() {
        true => std::fs::canonicalize(&candidate).ok(),
        false => candidate
            .parent()
            .and_then(|p| std::fs::canonicalize(p).ok()),
    };
    match check_anchor.is_some_and(|c| c.starts_with(&canon_ws)) {
        true => candidate,
        false => default_root,
    }
}

/// Returns `true` when the workspace has an initialized Project Brain directory.
pub fn has_project_brain(workspace_dir: &Path) -> bool {
    let root = resolve_brain_root(workspace_dir);
    root.is_dir()
        && (root.join(BRAIN_PAGES_DIR_NAME).is_dir()
            || workspace_dir.join(BRAIN_PROTOCOL_FILE).is_file()
            || BRAIN_ROOT_SLOTS
                .iter()
                .any(|slot| root.join(slot.filename).is_file()))
}

/// Normalizes and validates a strict kebab-case page identifier (`a-z`, `0-9`, `-`).
pub fn sanitize_brain_slug(raw: &str) -> Result<String> {
    let mut out = String::with_capacity(raw.len().min(BRAIN_MAX_SLUG_CHARS));
    let mut prev_dash = true;
    for ch in raw.trim().chars() {
        if out.len() >= BRAIN_MAX_SLUG_CHARS {
            break;
        }
        match ch {
            'a'..='z' | '0'..='9' => {
                out.push(ch);
                prev_dash = false;
            }
            'A'..='Z' => {
                out.push(ch.to_ascii_lowercase());
                prev_dash = false;
            }
            '-' | '_' | ' ' if !prev_dash => {
                out.push('-');
                prev_dash = true;
            }
            _ => {}
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    match trimmed.is_empty() {
        true => Err(ZenkoError::Tool(
            "invalid project_brain page id (expected kebab-case ASCII letters/digits/hyphens)"
                .into(),
        )),
        false => Ok(trimmed),
    }
}

fn sanitize_slug_list(items: &[String], max_items: usize) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for raw in items {
        if out.len() >= max_items {
            break;
        }
        if let Ok(slug) = sanitize_brain_slug(raw)
            && seen.insert(slug.clone())
        {
            out.push(slug);
        }
    }
    out
}

fn write_atomic_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_name = format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("brain"),
        zenko_core::new_id()
    );
    let tmp_path = path.with_file_name(tmp_name);
    std::fs::write(&tmp_path, content.as_bytes())?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Scaffolds a repo-native Project Brain (`BRAIN.md` + `brain/pages/` + empty root blueprint slots)
/// without overwriting any existing files. Returns newly created relative paths.
pub fn init_project_brain(workspace_dir: &Path) -> Result<Vec<String>> {
    let root = resolve_brain_root(workspace_dir);
    let pages_dir = root.join(BRAIN_PAGES_DIR_NAME);
    std::fs::create_dir_all(&pages_dir)?;

    let mut created = Vec::new();
    let protocol_path = workspace_dir.join(BRAIN_PROTOCOL_FILE);
    if !protocol_path.exists() {
        let protocol_body = format!(
            "# Project Brain Protocol\n\n\
             This repository stores durable architectural truth, decisions, constraints, and reversals in `{BRAIN_DIR_NAME}/`.\n\n\
             - **Pages (`{BRAIN_DIR_NAME}/{BRAIN_PAGES_DIR_NAME}/<id>.md`):** Each page pairs a rewritable `<!-- compiled_truth -->` section with an append-only `## Timeline` audit log (`decision`, `evidence`, `update`, `reversal`, `archived`).\n\
             - **Root Blueprint Slots (`{BRAIN_DIR_NAME}/*.md`):** `background.md`, `architecture.md`, `flow.md`, `mindmap.md`, `stack.md`, `roadmap.md`.\n\
             - **Single-Writer Invariant:** Mutate pages atomically through Zenko's `project_brain` tool (or a compatible `brain.md` writer) rather than hand-editing YAML frontmatter.\n"
        );
        write_atomic_file(&protocol_path, &protocol_body)?;
        created.push(BRAIN_PROTOCOL_FILE.to_string());
    }

    for desc in BRAIN_ROOT_SLOTS {
        let slot_path = root.join(desc.filename);
        if !slot_path.exists() {
            let body = format!("# {}\n\n<!-- {} -->\n", desc.title, desc.guidance);
            write_atomic_file(&slot_path, &body)?;
            let rel = slot_path
                .strip_prefix(workspace_dir)
                .unwrap_or(&slot_path)
                .to_string_lossy()
                .replace('\\', "/");
            created.push(rel);
        }
    }

    Ok(created)
}

fn render_inline_list(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

fn escape_yaml_inline(s: &str) -> String {
    s.replace(['\r', '\n'], " ").trim().to_string()
}

/// Renders a `BrainPage` into canonical, interoperable `mindmuxai/brain.md` Markdown format.
pub fn serialize_brain_page(page: &BrainPage) -> String {
    let mut out = String::with_capacity(512 + page.compiled_truth.len());
    out.push_str("---\n");
    out.push_str(&format!("id: {}\n", page.id));
    out.push_str(&format!("title: {}\n", escape_yaml_inline(&page.title)));
    out.push_str(&format!("category: {}\n", page.category.as_str()));
    out.push_str(&format!("status: {}\n", page.status.as_str()));
    if !page.tags.is_empty() {
        out.push_str(&format!("tags: {}\n", render_inline_list(&page.tags)));
    }
    out.push_str(&format!("created: \"{}\"\n", escape_yaml_inline(&page.created)));
    out.push_str(&format!("updated: \"{}\"\n", escape_yaml_inline(&page.updated)));
    if let Some(ref vb) = page.verified_by
        && !vb.trim().is_empty()
    {
        out.push_str(&format!(
            "{OKF_KEY_VERIFIED_BY}: \"{}\"\n",
            escape_yaml_inline(vb)
        ));
    }
    if let Some(ref va) = page.verified_at
        && !va.trim().is_empty()
    {
        out.push_str(&format!(
            "{OKF_KEY_VERIFIED_AT}: \"{}\"\n",
            escape_yaml_inline(va)
        ));
    }
    if let Some(ref sa) = page.stale_after
        && !sa.trim().is_empty()
    {
        out.push_str(&format!(
            "{OKF_KEY_STALE_AFTER}: \"{}\"\n",
            escape_yaml_inline(sa)
        ));
    }
    if !page.sources.is_empty() {
        out.push_str(&format!(
            "{OKF_KEY_SOURCES}: {}\n",
            render_inline_list(&page.sources)
        ));
    }
    out.push_str("---\n");
    out.push_str(BRAIN_COMPILED_TRUTH_MARKER);
    out.push('\n');
    let truth = page.compiled_truth.trim();
    if !truth.is_empty() {
        out.push_str(truth);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(BRAIN_TIMELINE_HEADING);
    out.push('\n');
    for entry in page.timeline.iter().take(BRAIN_MAX_TIMELINE_ENTRIES) {
        out.push_str(&format!("- time: {}\n", escape_yaml_inline(&entry.time)));
        out.push_str(&format!("  kind: {}\n", entry.kind.as_str()));
        out.push_str(&format!("  summary: {}\n", escape_yaml_inline(&entry.summary)));
        if !entry.source.trim().is_empty() {
            out.push_str(&format!("  source: {}\n", escape_yaml_inline(&entry.source)));
        }
        if !entry.affects.is_empty() {
            out.push_str(&format!("  affects: {}\n", render_inline_list(&entry.affects)));
        }
    }
    out
}

fn parse_inline_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .map(|s| s.trim().trim_matches(['"', '\'']).trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn strip_quotes(raw: &str) -> String {
    let t = raw.trim();
    t.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| t.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(t)
        .to_string()
}

/// Deterministic zero-regex state-machine parser for `brain/pages/<id>.md`.
pub fn parse_brain_page(fallback_id: &str, raw: &str) -> BrainPage {
    let mut id = fallback_id.to_string();
    let mut title = fallback_id.to_string();
    let mut category = BrainPageCategory::Decision;
    let mut status = BrainPageStatus::Active;
    let mut tags: Vec<String> = Vec::new();
    let mut created = String::new();
    let mut updated = String::new();
    let mut verified_by: Option<String> = None;
    let mut verified_at: Option<String> = None;
    let mut stale_after: Option<String> = None;
    let mut sources: Vec<String> = Vec::new();

    let mut lines = raw.lines().peekable();
    if lines.peek().is_some_and(|l| l.trim() == "---") {
        lines.next();
        for l in lines.by_ref() {
            if l.trim() == "---" {
                break;
            }
            let Some((k, v)) = l.split_once(':') else {
                continue;
            };
            match k.trim().to_ascii_lowercase().as_str() {
                "id" => {
                    let s = strip_quotes(v);
                    if !s.is_empty() {
                        id = s;
                    }
                }
                "title" => {
                    let s = strip_quotes(v);
                    if !s.is_empty() {
                        title = s;
                    }
                }
                "category" => {
                    if let Some(c) = BrainPageCategory::parse(&strip_quotes(v)) {
                        category = c;
                    }
                }
                "status" => {
                    if let Some(st) = BrainPageStatus::parse(&strip_quotes(v)) {
                        status = st;
                    }
                }
                "tags" => {
                    tags = parse_inline_list(v);
                }
                "created" => created = strip_quotes(v),
                "updated" => updated = strip_quotes(v),
                k if k == OKF_KEY_VERIFIED_BY => {
                    let s = strip_quotes(v);
                    if !s.is_empty() {
                        verified_by = Some(s);
                    }
                }
                k if k == OKF_KEY_VERIFIED_AT => {
                    let s = strip_quotes(v);
                    if !s.is_empty() {
                        verified_at = Some(s);
                    }
                }
                k if k == OKF_KEY_STALE_AFTER => {
                    let s = strip_quotes(v);
                    if !s.is_empty() {
                        stale_after = Some(s);
                    }
                }
                k if k == OKF_KEY_SOURCES => {
                    sources = parse_inline_list(v);
                }
                _ => {}
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Section {
        Truth,
        Timeline,
    }

    let mut section = Section::Truth;
    let mut truth_lines: Vec<&str> = Vec::new();
    let mut timeline: Vec<BrainTimelineEntry> = Vec::new();
    let mut current: Option<BrainTimelineEntry> = None;

    for raw_line in lines {
        let trimmed = raw_line.trim();
        match section {
            Section::Truth => match trimmed {
                t if t == BRAIN_COMPILED_TRUTH_MARKER => {}
                t if t.eq_ignore_ascii_case(BRAIN_TIMELINE_HEADING) => {
                    section = Section::Timeline;
                }
                _ => truth_lines.push(raw_line),
            },
            Section::Timeline => match (trimmed.strip_prefix("- "), current.as_mut(), trimmed.is_empty()) {
                (Some(after_dash), _, _) => {
                    if let Some(prev) = current.take() {
                        timeline.push(prev);
                    }
                    let mut entry = BrainTimelineEntry {
                        time: String::new(),
                        kind: BrainTimelineKind::Update,
                        summary: String::new(),
                        source: String::new(),
                        affects: Vec::new(),
                    };
                    apply_timeline_field(&mut entry, after_dash);
                    current = Some(entry);
                }
                (None, Some(entry), false) => apply_timeline_field(entry, trimmed),
                _ => {}
            },
        }
    }
    if let Some(last) = current.take() {
        timeline.push(last);
    }

    BrainPage {
        id,
        title,
        category,
        status,
        tags,
        created,
        updated,
        verified_by,
        verified_at,
        stale_after,
        sources,
        compiled_truth: truth_lines.join("\n").trim().to_string(),
        timeline,
    }
}

fn apply_timeline_field(entry: &mut BrainTimelineEntry, line: &str) {
    let Some((k, v)) = line.split_once(':') else {
        return;
    };
    match k.trim().to_ascii_lowercase().as_str() {
        "time" => entry.time = strip_quotes(v),
        "kind" => {
            if let Some(kind) = BrainTimelineKind::parse(&strip_quotes(v)) {
                entry.kind = kind;
            }
        }
        "summary" => entry.summary = strip_quotes(v),
        "source" => entry.source = strip_quotes(v),
        "affects" => entry.affects = parse_inline_list(v),
        _ => {}
    }
}

fn brain_status_to_okf_status(status: BrainPageStatus) -> OkfDocStatus {
    match status {
        BrainPageStatus::Active => OkfDocStatus::Active,
        BrainPageStatus::Draft => OkfDocStatus::Draft,
        BrainPageStatus::Archived => OkfDocStatus::Deprecated,
    }
}

pub fn summarize_brain_page(page: &BrainPage) -> BrainPageSummary {
    summarize_brain_page_in_workspace(page, None)
}

pub fn summarize_brain_page_in_workspace(
    page: &BrainPage,
    workspace_dir: Option<&Path>,
) -> BrainPageSummary {
    let has_reversals = page
        .timeline
        .iter()
        .any(|e| e.kind == BrainTimelineKind::Reversal);
    let first_line = page
        .compiled_truth
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or(&page.title);
    let freshness = evaluate_freshness(page.stale_after.as_deref(), Utc::now());
    let (missing, drifted) = workspace_dir
        .map(|ws| evaluate_source_provenance(ws, &page.sources, page.verified_at.as_deref()))
        .unwrap_or_default();
    let trust_tier = evaluate_trust_tier(
        brain_status_to_okf_status(page.status),
        page.verified_by.as_deref(),
        page.verified_at.as_deref(),
        &missing,
        &drifted,
    );
    BrainPageSummary {
        id: page.id.clone(),
        title: page.title.clone(),
        category: page.category,
        status: page.status,
        tags: page.tags.clone(),
        updated: page.updated.clone(),
        timeline_count: page.timeline.len(),
        has_reversals,
        truth_preview: compact_preview(first_line, zenko_core::BRAIN_TRUTH_PREVIEW_CHARS),
        freshness,
        trust_tier,
    }
}

/// Loads all parsed pages from `<brain_root>/pages/*.md` deterministically sorted by page ID.
pub fn load_all_brain_pages(workspace_dir: &Path) -> Vec<BrainPage> {
    let pages_dir = resolve_brain_root(workspace_dir).join(BRAIN_PAGES_DIR_NAME);
    let Ok(rd) = std::fs::read_dir(&pages_dir) else {
        return Vec::new();
    };
    let mut by_id: BTreeMap<String, BrainPage> = BTreeMap::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(meta) = path.metadata() else {
            continue;
        };
        if meta.len() > BRAIN_MAX_PAGE_BYTES as u64 {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let page = parse_brain_page(stem, &raw);
        by_id.insert(page.id.clone(), page);
    }
    by_id.into_values().collect()
}

pub fn list_brain_pages(
    workspace_dir: &Path,
    status_filter: Option<BrainPageStatus>,
    category_filter: Option<BrainPageCategory>,
) -> Vec<BrainPageSummary> {
    load_all_brain_pages(workspace_dir)
        .into_iter()
        .filter(|p| status_filter.is_none_or(|st| p.status == st))
        .filter(|p| category_filter.is_none_or(|c| p.category == c))
        .map(|p| summarize_brain_page_in_workspace(&p, Some(workspace_dir)))
        .collect()
}

pub fn read_brain_page(workspace_dir: &Path, raw_id: &str) -> Result<BrainPage> {
    let id = sanitize_brain_slug(raw_id)?;
    let path = resolve_brain_root(workspace_dir)
        .join(BRAIN_PAGES_DIR_NAME)
        .join(format!("{id}.md"));
    if !path.is_file() {
        return Err(ZenkoError::NotFound(format!(
            "project_brain page `{id}` does not exist"
        )));
    }
    let raw = std::fs::read_to_string(&path)?;
    Ok(parse_brain_page(&id, &raw))
}

#[allow(clippy::too_many_arguments)]
pub fn create_brain_page(
    workspace_dir: &Path,
    raw_id: &str,
    title: &str,
    category: BrainPageCategory,
    status: BrainPageStatus,
    tags: &[String],
    compiled_truth: &str,
    timeline_summary: &str,
    source: &str,
    affects: &[String],
    known_secrets: &[String],
) -> Result<BrainPage> {
    let id = sanitize_brain_slug(raw_id)?;
    let _ = init_project_brain(workspace_dir)?;
    let path = resolve_brain_root(workspace_dir)
        .join(BRAIN_PAGES_DIR_NAME)
        .join(format!("{id}.md"));
    if path.exists() {
        return Err(ZenkoError::Tool(format!(
            "project_brain page `{id}` already exists (use `update_truth` or `append_timeline` instead)"
        )));
    }

    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let clean_title = compact_preview(
        &sanitize_memory_text_with_secrets(title.trim(), known_secrets),
        BRAIN_MAX_TITLE_CHARS,
    );
    let clean_truth = sanitize_memory_text_with_secrets(compiled_truth.trim(), known_secrets);
    if clean_title.is_empty() || clean_truth.is_empty() {
        return Err(ZenkoError::Tool(
            "project_brain `create_page` requires non-empty `title` and `compiled_truth`".into(),
        ));
    }
    let clean_summary = match timeline_summary.trim().is_empty() {
        true => format!("Created `{id}` ({})", category.as_str()),
        false => sanitize_memory_text_with_secrets(timeline_summary.trim(), known_secrets),
    };
    let clean_source = sanitize_memory_text_with_secrets(source.trim(), known_secrets);
    let initial_kind = match category {
        BrainPageCategory::Decision => BrainTimelineKind::Decision,
        _ => BrainTimelineKind::Evidence,
    };

    let page = BrainPage {
        id: id.clone(),
        title: clean_title,
        category,
        status,
        tags: sanitize_slug_list(tags, BRAIN_MAX_TAGS),
        created: now.clone(),
        updated: now.clone(),
        verified_by: None,
        verified_at: None,
        stale_after: None,
        sources: match clean_source.is_empty() {
            true => Vec::new(),
            false => vec![clean_source.clone()],
        },
        compiled_truth: clean_truth,
        timeline: vec![BrainTimelineEntry {
            time: now,
            kind: initial_kind,
            summary: clean_summary,
            source: clean_source,
            affects: sanitize_slug_list(affects, BRAIN_MAX_AFFECTS),
        }],
    };
    let rendered = serialize_brain_page(&page);
    if rendered.len() > BRAIN_MAX_PAGE_BYTES {
        return Err(ZenkoError::Tool(format!(
            "project_brain page `{id}` exceeds {} KiB cap",
            BRAIN_MAX_PAGE_BYTES / 1024
        )));
    }
    write_atomic_file(&path, &rendered)?;
    Ok(page)
}

/// Atomically replaces `compiled_truth` AND appends a new `## Timeline` entry in a single write.
#[allow(clippy::too_many_arguments)]
pub fn update_brain_page_truth(
    workspace_dir: &Path,
    raw_id: &str,
    new_truth: &str,
    kind: BrainTimelineKind,
    summary: &str,
    source: &str,
    affects: &[String],
    tags_override: Option<&[String]>,
    status_override: Option<BrainPageStatus>,
    known_secrets: &[String],
) -> Result<BrainPage> {
    let mut page = read_brain_page(workspace_dir, raw_id)?;
    let clean_truth = sanitize_memory_text_with_secrets(new_truth.trim(), known_secrets);
    let clean_summary = sanitize_memory_text_with_secrets(summary.trim(), known_secrets);
    if clean_truth.is_empty() || clean_summary.is_empty() {
        return Err(ZenkoError::Tool(
            "project_brain `update_truth` requires non-empty `compiled_truth` and `summary`".into(),
        ));
    }
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    page.compiled_truth = clean_truth;
    page.updated = now.clone();
    if let Some(new_tags) = tags_override {
        page.tags = sanitize_slug_list(new_tags, BRAIN_MAX_TAGS);
    }
    if let Some(st) = status_override {
        page.status = st;
    }
    page.timeline.push(BrainTimelineEntry {
        time: now,
        kind,
        summary: clean_summary,
        source: sanitize_memory_text_with_secrets(source.trim(), known_secrets),
        affects: sanitize_slug_list(affects, BRAIN_MAX_AFFECTS),
    });
    if page.timeline.len() > BRAIN_MAX_TIMELINE_ENTRIES {
        let drop_n = page.timeline.len() - BRAIN_MAX_TIMELINE_ENTRIES;
        page.timeline.drain(..drop_n);
    }

    let rendered = serialize_brain_page(&page);
    if rendered.len() > BRAIN_MAX_PAGE_BYTES {
        return Err(ZenkoError::Tool(format!(
            "project_brain page `{}` exceeds {} KiB cap",
            page.id,
            BRAIN_MAX_PAGE_BYTES / 1024
        )));
    }
    let path = resolve_brain_root(workspace_dir)
        .join(BRAIN_PAGES_DIR_NAME)
        .join(format!("{}.md", page.id));
    write_atomic_file(&path, &rendered)?;
    Ok(page)
}

/// Appends an evidence/update/reversal timeline entry without rewriting `compiled_truth`.
pub fn append_brain_timeline(
    workspace_dir: &Path,
    raw_id: &str,
    kind: BrainTimelineKind,
    summary: &str,
    source: &str,
    affects: &[String],
    known_secrets: &[String],
) -> Result<BrainPage> {
    let mut page = read_brain_page(workspace_dir, raw_id)?;
    let clean_summary = sanitize_memory_text_with_secrets(summary.trim(), known_secrets);
    if clean_summary.is_empty() {
        return Err(ZenkoError::Tool(
            "project_brain `append_timeline` requires non-empty `summary`".into(),
        ));
    }
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    page.updated = now.clone();
    page.timeline.push(BrainTimelineEntry {
        time: now,
        kind,
        summary: clean_summary,
        source: sanitize_memory_text_with_secrets(source.trim(), known_secrets),
        affects: sanitize_slug_list(affects, BRAIN_MAX_AFFECTS),
    });
    let rendered = serialize_brain_page(&page);
    if rendered.len() > BRAIN_MAX_PAGE_BYTES {
        return Err(ZenkoError::Tool(format!(
            "project_brain page `{}` exceeds {} KiB cap",
            page.id,
            BRAIN_MAX_PAGE_BYTES / 1024
        )));
    }
    let path = resolve_brain_root(workspace_dir)
        .join(BRAIN_PAGES_DIR_NAME)
        .join(format!("{}.md", page.id));
    write_atomic_file(&path, &rendered)?;
    Ok(page)
}

/// Atomically archives a page (`status: archived`) and records why (`reversal` or `archived`).
pub fn archive_brain_page(
    workspace_dir: &Path,
    raw_id: &str,
    reversal_summary: &str,
    source: &str,
    affects: &[String],
    is_reversal: bool,
    known_secrets: &[String],
) -> Result<BrainPage> {
    let mut page = read_brain_page(workspace_dir, raw_id)?;
    let clean_summary = sanitize_memory_text_with_secrets(reversal_summary.trim(), known_secrets);
    if clean_summary.is_empty() {
        return Err(ZenkoError::Tool(
            "project_brain `archive_page` requires a non-empty `summary` explaining the archival or reversal"
                .into(),
        ));
    }
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    page.status = BrainPageStatus::Archived;
    page.updated = now.clone();
    page.timeline.push(BrainTimelineEntry {
        time: now,
        kind: match is_reversal {
            true => BrainTimelineKind::Reversal,
            false => BrainTimelineKind::Archived,
        },
        summary: clean_summary,
        source: sanitize_memory_text_with_secrets(source.trim(), known_secrets),
        affects: sanitize_slug_list(affects, BRAIN_MAX_AFFECTS),
    });
    let rendered = serialize_brain_page(&page);
    let path = resolve_brain_root(workspace_dir)
        .join(BRAIN_PAGES_DIR_NAME)
        .join(format!("{}.md", page.id));
    write_atomic_file(&path, &rendered)?;
    Ok(page)
}

pub fn read_brain_root_slot(workspace_dir: &Path, slot: BrainRootSlot) -> Result<String> {
    let path = resolve_brain_root(workspace_dir).join(slot.filename());
    if !path.is_file() {
        return Err(ZenkoError::NotFound(format!(
            "project_brain root slot `{}` does not exist (run `init` or `update_root`)",
            slot.as_str()
        )));
    }
    Ok(std::fs::read_to_string(path)?)
}

pub fn update_brain_root_slot(
    workspace_dir: &Path,
    slot: BrainRootSlot,
    content: &str,
    known_secrets: &[String],
) -> Result<String> {
    let _ = init_project_brain(workspace_dir)?;
    let clean = sanitize_memory_text_with_secrets(content.trim(), known_secrets);
    if clean.is_empty() {
        return Err(ZenkoError::Tool(
            "project_brain `update_root` requires non-empty `content`".into(),
        ));
    }
    if clean.len() > BRAIN_MAX_ROOT_BYTES {
        return Err(ZenkoError::Tool(format!(
            "project_brain root slot `{}` exceeds {} KiB cap",
            slot.as_str(),
            BRAIN_MAX_ROOT_BYTES / 1024
        )));
    }
    let desc = find_brain_root_slot(slot);
    let body = match clean.starts_with('#') {
        true => format!("{clean}\n"),
        false => format!("# {}\n\n{clean}\n", desc.title),
    };
    let path = resolve_brain_root(workspace_dir).join(slot.filename());
    write_atomic_file(&path, &body)?;
    Ok(body)
}

/// Deterministic zero-regex BFS impact graph over `affects` edges and reverse backlinks.
pub fn compute_brain_impact_graph(workspace_dir: &Path, raw_id: &str) -> Result<BrainImpactGraph> {
    let target_id = sanitize_brain_slug(raw_id)?;
    let pages = load_all_brain_pages(workspace_dir);
    let mut forward: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for page in &pages {
        let mut out_edges: BTreeSet<String> = BTreeSet::new();
        for ev in &page.timeline {
            for dep in &ev.affects {
                if dep != &page.id {
                    out_edges.insert(dep.clone());
                    reverse
                        .entry(dep.clone())
                        .or_default()
                        .insert(page.id.clone());
                }
            }
        }
        // Also fold OKF-style relative Markdown `[Title](other-page.md)` links into the graph
        for raw_link in extract_markdown_links(&page.compiled_truth) {
            let stem = raw_link
                .rsplit('/')
                .next()
                .unwrap_or(&raw_link)
                .strip_suffix(".md")
                .unwrap_or(&raw_link);
            if let Ok(slug) = sanitize_brain_slug(stem)
                && slug != page.id
            {
                out_edges.insert(slug.clone());
                reverse.entry(slug).or_default().insert(page.id.clone());
            }
        }
        forward.insert(page.id.clone(), out_edges);
    }

    let direct_set: BTreeSet<String> = forward.get(&target_id).cloned().unwrap_or_default();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visited.insert(target_id.clone());
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    for d in &direct_set {
        if visited.insert(d.clone()) {
            queue.push_back((d.clone(), 1));
        }
    }

    let mut transitive: BTreeSet<String> = BTreeSet::new();
    while let Some((node, hop)) = queue.pop_front() {
        if hop >= BRAIN_IMPACT_MAX_HOPS {
            continue;
        }
        if let Some(nexts) = forward.get(&node) {
            for nxt in nexts {
                if visited.insert(nxt.clone()) {
                    transitive.insert(nxt.clone());
                    queue.push_back((nxt.clone(), hop + 1));
                }
            }
        }
    }

    let referenced_by = reverse
        .get(&target_id)
        .map(|s| s.iter().cloned().collect())
        .unwrap_or_default();

    Ok(BrainImpactGraph {
        page_id: target_id,
        direct_affects: direct_set.into_iter().collect(),
        transitive_affects: transitive.into_iter().collect(),
        referenced_by,
    })
}

/// Deterministic zero-regex token-overlap search across Project Brain pages (weights title,
/// tags, compiled truth, timeline summaries, active status, and reversal presence).
pub fn search_brain_pages(
    workspace_dir: &Path,
    query: &str,
    include_archived: bool,
    limit: usize,
) -> Vec<BrainSearchHit> {
    let q_terms: BTreeSet<String> = tokenize_terms(query).into_iter().collect();
    if q_terms.is_empty() {
        return Vec::new();
    }
    let pages = load_all_brain_pages(workspace_dir);
    let tq_codec = TurboQuantCodec::default_memory_codec();
    let tq_query = tq_codec.prepare_text_query(query);
    let mut scored: Vec<BrainSearchHit> = Vec::new();

    for page in pages {
        if !include_archived && page.status == BrainPageStatus::Archived {
            continue;
        }
        let title_terms: BTreeSet<String> = tokenize_terms(&page.title).into_iter().collect();
        let tag_terms: BTreeSet<String> = page
            .tags
            .iter()
            .flat_map(|t| tokenize_terms(t))
            .collect();
        let truth_terms: BTreeSet<String> = tokenize_terms(&page.compiled_truth).into_iter().collect();
        let timeline_terms: BTreeSet<String> = page
            .timeline
            .iter()
            .flat_map(|e| tokenize_terms(&e.summary))
            .collect();

        let title_hits = q_terms.intersection(&title_terms).count() as f64;
        let tag_hits = q_terms.intersection(&tag_terms).count() as f64;
        let truth_hits = q_terms.intersection(&truth_terms).count() as f64;
        let timeline_hits = q_terms.intersection(&timeline_terms).count() as f64;

        let tq_cos = match tq_query.is_zero() {
            true => 0.0f32,
            false => {
                let doc_vec = tq_codec.encode_text(&format!(
                    "{} {} {}",
                    page.title,
                    page.tags.join(" "),
                    page.compiled_truth
                ));
                tq_query.cosine(&doc_vec)
            }
        };
        let tq_bonus = match tq_cos >= TURBOQUANT_MIN_COSINE {
            true => (tq_cos as f64) * 2.0,
            false => 0.0,
        };

        let raw_score =
            (title_hits * 3.0) + (tag_hits * 2.5) + (truth_hits * 1.5) + timeline_hits + tq_bonus;
        if raw_score <= 0.0 {
            continue;
        }
        let summary = summarize_brain_page_in_workspace(&page, Some(workspace_dir));
        let status_mult = match page.status {
            BrainPageStatus::Active => 1.25,
            BrainPageStatus::Draft => 1.0,
            BrainPageStatus::Archived => 0.75,
        };
        let okf_mult = match summary.freshness {
            OkfFreshnessState::Stale => summary.freshness.rank_weight(),
            _ => 1.0,
        };
        let reversal_bonus = match page
            .timeline
            .iter()
            .any(|e| e.kind == BrainTimelineKind::Reversal)
        {
            true => 0.5,
            false => 0.0,
        };
        let mut affects: BTreeSet<String> = BTreeSet::new();
        for ev in &page.timeline {
            for dep in &ev.affects {
                affects.insert(dep.clone());
            }
        }
        scored.push(BrainSearchHit {
            summary,
            score: (raw_score * status_mult * okf_mult) + reversal_bonus,
            affects: affects.into_iter().collect(),
        });
    }

    scored.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.summary.id.cmp(&b.summary.id))
    });
    scored.truncate(limit);
    scored
}

/// Builds a compact Tier-2 `<project_brain_index>` summary list for `SnapshotManager`.
/// Returns `(active_page_count, populated_root_slots, formatted_lines)` — strictly empty when
/// the repository has no initialized `brain/` directory (zero token cost when unused).
pub fn build_project_brain_index(workspace_dir: &Path) -> (usize, Vec<String>, Vec<String>) {
    if !has_project_brain(workspace_dir) {
        return (0, Vec::new(), Vec::new());
    }
    let root = resolve_brain_root(workspace_dir);
    let mut populated_slots = Vec::new();
    for desc in BRAIN_ROOT_SLOTS {
        let p = root.join(desc.filename);
        if let Ok(raw) = std::fs::read_to_string(&p) {
            let has_body = raw.lines().any(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#') && !t.starts_with("<!--")
            });
            if has_body {
                populated_slots.push(desc.slug.to_string());
            }
        }
    }

    let pages: Vec<BrainPageSummary> = list_brain_pages(workspace_dir, None, None)
        .into_iter()
        .filter(|p| p.status != BrainPageStatus::Archived)
        .collect();
    let total_active = pages.len();
    let lines: Vec<String> = pages
        .into_iter()
        .take(BRAIN_MAX_INDEX_PAGES)
        .map(|p| {
            let rev_tag = match p.has_reversals {
                true => " [has-reversal]",
                false => "",
            };
            format!("- `{}` [{}]{rev_tag}: {}", p.id, p.category.as_str(), p.title)
        })
        .collect();

    (total_active, populated_slots, lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_brain_roundtrips_truth_timeline_reversals_secrets_and_bfs_impact() {
        let base = std::env::temp_dir().join(format!("zenko_brain_test_{}", zenko_core::new_id()));
        std::fs::create_dir_all(&base).expect("mkdir");
        assert!(!has_project_brain(&base));

        let created = init_project_brain(&base).expect("init");
        assert!(created.iter().any(|p| p == BRAIN_PROTOCOL_FILE));
        assert!(has_project_brain(&base));

        let p1 = create_brain_page(
            &base,
            "Auth Session Strategy",
            "Auth Session Strategy",
            BrainPageCategory::Decision,
            BrainPageStatus::Active,
            &["auth".into(), "security".into()],
            "Use HMAC cookies with sk-secret-12345678901234567890 redacted.",
            "Chose stateless HMAC cookies over Redis sessions",
            "src/auth.rs",
            &["api-gateway".into()],
            &["sk-secret-12345678901234567890".into()],
        )
        .expect("create auth page");
        assert_eq!(p1.id, "auth-session-strategy");
        assert!(!p1.compiled_truth.contains("sk-secret-12345678901234567890"));

        let _ = create_brain_page(
            &base,
            "api-gateway",
            "API Gateway Contract",
            BrainPageCategory::Concept,
            BrainPageStatus::Active,
            &["api".into()],
            "Validates cookies at edge middleware.",
            "Defined edge auth boundary",
            "src/gateway.rs",
            &["edge-cache".into()],
            &[],
        )
        .expect("create gateway page");

        let updated = update_brain_page_truth(
            &base,
            "auth-session-strategy",
            "Use rotating HMAC-SHA256 cookies; rejected localStorage JWT.",
            BrainTimelineKind::Reversal,
            "Reversed localStorage JWT after XSS threat review",
            "security-audit",
            &["api-gateway".into()],
            None,
            None,
            &[],
        )
        .expect("update truth");
        assert_eq!(updated.timeline.len(), 2);
        assert_eq!(updated.timeline[1].kind, BrainTimelineKind::Reversal);

        // Verify disk round-trip fidelity
        let reloaded = read_brain_page(&base, "auth-session-strategy").expect("reload");
        assert_eq!(reloaded, updated);

        // Verify BFS impact graph (1-hop direct + 2-hop transitive + reverse backlinks)
        let graph = compute_brain_impact_graph(&base, "auth-session-strategy").expect("impact");
        assert_eq!(graph.direct_affects, vec!["api-gateway"]);
        assert_eq!(graph.transitive_affects, vec!["edge-cache"]);
        let gw_graph = compute_brain_impact_graph(&base, "api-gateway").expect("gw impact");
        assert_eq!(gw_graph.referenced_by, vec!["auth-session-strategy"]);

        // Verify token search ranks reversal-bearing matching page highest
        let hits = search_brain_pages(&base, "HMAC cookies XSS auth", false, 5);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].summary.id, "auth-session-strategy");
        assert!(hits[0].summary.has_reversals);

        // Verify root slot update + Tier-2 index
        let _ = update_brain_root_slot(
            &base,
            BrainRootSlot::Stack,
            "Rust 2024 + SQLite WAL + React 19 embedded via rust-embed.",
            &[],
        )
        .expect("update root");
        let (active_n, slots, lines) = build_project_brain_index(&base);
        assert_eq!(active_n, 2);
        assert_eq!(slots, vec!["stack"]);
        assert!(lines.iter().any(|l| l.contains("[has-reversal]")));

        // Verify archival flips status and appends reversal/archived entry
        let archived = archive_brain_page(
            &base,
            "api-gateway",
            "Superseded by unified router",
            "refactor",
            &[],
            false,
            &[],
        )
        .expect("archive");
        assert_eq!(archived.status, BrainPageStatus::Archived);
        assert_eq!(
            archived.timeline.last().map(|e| e.kind),
            Some(BrainTimelineKind::Archived)
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_brain_root_honors_jailed_preferences_and_blocks_traversal() {
        let base = std::env::temp_dir().join(format!("zenko_brain_jail_{}", zenko_core::new_id()));
        std::fs::create_dir_all(base.join(".mindmux")).expect("mkdir");
        std::fs::create_dir_all(base.join("docs/brain")).expect("docs/brain");

        std::fs::write(
            base.join(".mindmux/preferences.json"),
            r#"{"brainRoot":"docs/brain"}"#,
        )
        .expect("write prefs");
        assert_eq!(resolve_brain_root(&base), base.join("docs/brain"));

        std::fs::write(
            base.join(".mindmux/preferences.json"),
            r#"{"brainRoot":"../../etc/passwd"}"#,
        )
        .expect("write evil prefs");
        assert_eq!(resolve_brain_root(&base), base.join("brain"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
