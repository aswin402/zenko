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
    BRAIN_COMPILED_TRUTH_MARKER, BRAIN_DIR_NAME, BRAIN_HUB_WIKI_DIR_NAME, BRAIN_IMPACT_MAX_HOPS,
    BRAIN_INDEX_FILE_NAME, BRAIN_LINT_MAX_ISSUES, BRAIN_LOG_FILE_NAME, BRAIN_MAX_AFFECTS,
    BRAIN_MAX_INDEX_PAGES, BRAIN_MAX_LOG_BYTES, BRAIN_MAX_PAGE_BYTES, BRAIN_MAX_RAW_BYTES,
    BRAIN_MAX_ROOT_BYTES, BRAIN_MAX_SLUG_CHARS, BRAIN_MAX_TAGS, BRAIN_MAX_TIMELINE_ENTRIES,
    BRAIN_MAX_TITLE_CHARS, BRAIN_PAGES_DIR_NAME, BRAIN_PREFS_DIR_NAME, BRAIN_PREFS_FILE_NAME,
    BRAIN_PREFS_ROOT_KEY, BRAIN_PROTOCOL_FILE, BRAIN_RAW_DIR_NAME, BRAIN_ROOT_SLOTS,
    BRAIN_TIMELINE_HEADING, BrainImpactGraph, BrainLintIssue, BrainLintIssueKind, BrainLintReport,
    BrainPage, BrainPageCategory, BrainPageStatus, BrainPageSummary, BrainRootSlot, BrainScope,
    BrainSearchHit, BrainTimelineEntry, BrainTimelineKind, OKF_KEY_SOURCES, OKF_KEY_STALE_AFTER,
    OKF_KEY_VERIFIED_AT, OKF_KEY_VERIFIED_BY, OkfDocStatus, OkfFreshnessState, OkfTrustTier,
    Result, TURBOQUANT_MIN_COSINE, ZenkoError, compact_preview, find_brain_root_slot,
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

/// Resolves either the workspace Project Brain (`<ws>/brain`) or the personal Hub Wiki (`<hub>/wiki`).
pub fn resolve_scoped_brain_root(workspace_dir: &Path, hub_dir: &Path, scope: BrainScope) -> PathBuf {
    match scope {
        BrainScope::Workspace => resolve_brain_root(workspace_dir),
        BrainScope::Hub => hub_dir.join(BRAIN_HUB_WIKI_DIR_NAME),
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

/// Returns `true` when the user's personal Hub Wiki (`<hub>/wiki/`) has been initialized.
pub fn has_hub_wiki(hub_dir: &Path) -> bool {
    let root = hub_dir.join(BRAIN_HUB_WIKI_DIR_NAME);
    root.is_dir()
        && (root.join(BRAIN_PAGES_DIR_NAME).is_dir()
            || root.join(BRAIN_RAW_DIR_NAME).is_dir()
            || root.join(BRAIN_INDEX_FILE_NAME).is_file())
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

/// Scaffolds a repo-native Project Brain (`BRAIN.md` + `brain/{pages,raw}/` + `index.md` + `log.md`
/// + root blueprint slots) without overwriting existing files.
pub fn init_project_brain(workspace_dir: &Path) -> Result<Vec<String>> {
    init_brain_scoped(workspace_dir, workspace_dir, BrainScope::Workspace)
}

/// Scaffolds either a workspace Project Brain (`<ws>/brain/`) or personal Hub Wiki (`<hub>/wiki/`)
/// with Karpathy LLM Wiki layers (`raw/`, `pages/`, `index.md`, `log.md`) without overwriting files.
pub fn init_brain_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
) -> Result<Vec<String>> {
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let pages_dir = root.join(BRAIN_PAGES_DIR_NAME);
    let raw_dir = root.join(BRAIN_RAW_DIR_NAME);
    std::fs::create_dir_all(&pages_dir)?;
    std::fs::create_dir_all(&raw_dir)?;

    let mut created = Vec::new();
    let display_base = match scope {
        BrainScope::Workspace => workspace_dir,
        BrainScope::Hub => hub_dir,
    };

    if scope == BrainScope::Workspace {
        let protocol_path = workspace_dir.join(BRAIN_PROTOCOL_FILE);
        if !protocol_path.exists() {
            let protocol_body = format!(
                "# Project Brain Protocol\n\n\
                 This repository stores durable architectural truth, decisions, constraints, and reversals in `{BRAIN_DIR_NAME}/`.\n\n\
                 - **Raw Sources (`{BRAIN_DIR_NAME}/{BRAIN_RAW_DIR_NAME}/`):** Immutable source drops (specs, research notes, web clips) compiled into wiki pages.\n\
                 - **Pages (`{BRAIN_DIR_NAME}/{BRAIN_PAGES_DIR_NAME}/<id>.md`):** Each page pairs a rewritable `<!-- compiled_truth -->` section with an append-only `## Timeline` audit log (`decision`, `evidence`, `update`, `reversal`, `archived`).\n\
                 - **Index & Log (`{BRAIN_DIR_NAME}/{BRAIN_INDEX_FILE_NAME}`, `{BRAIN_DIR_NAME}/{BRAIN_LOG_FILE_NAME}`):** Auto-maintained catalog and chronological operation log.\n\
                 - **Root Blueprint Slots (`{BRAIN_DIR_NAME}/*.md`):** `background.md`, `architecture.md`, `flow.md`, `mindmap.md`, `stack.md`, `roadmap.md`.\n\
                 - **Single-Writer Invariant:** Mutate pages atomically through Zenko's `project_brain` tool rather than hand-editing YAML frontmatter.\n"
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
                    .strip_prefix(display_base)
                    .unwrap_or(&slot_path)
                    .to_string_lossy()
                    .replace('\\', "/");
                created.push(rel);
            }
        }
    }

    let index_path = root.join(BRAIN_INDEX_FILE_NAME);
    if !index_path.exists() {
        rebuild_brain_index_md(&root, scope);
        let rel = index_path
            .strip_prefix(display_base)
            .unwrap_or(&index_path)
            .to_string_lossy()
            .replace('\\', "/");
        created.push(rel);
    }

    let log_path = root.join(BRAIN_LOG_FILE_NAME);
    if !log_path.exists() {
        let header = match scope {
            BrainScope::Workspace => "# Project Brain Log\n\nChronological append-only log of ingest, compilation, updates, and lint passes.\n",
            BrainScope::Hub => "# Personal Hub Wiki Log\n\nChronological append-only log of raw ingests, wiki syntheses, updates, and lint passes.\n",
        };
        write_atomic_file(&log_path, header)?;
        let rel = log_path
            .strip_prefix(display_base)
            .unwrap_or(&log_path)
            .to_string_lossy()
            .replace('\\', "/");
        created.push(rel);
    }

    Ok(created)
}

/// Lists immutable raw source files in `<root>/raw/` sorted deterministically.
pub fn list_raw_sources_at_root(root: &Path) -> Vec<String> {
    let raw_dir = root.join(BRAIN_RAW_DIR_NAME);
    let Ok(rd) = std::fs::read_dir(&raw_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && !name.starts_with('.')
        {
            out.push(format!("{BRAIN_RAW_DIR_NAME}/{name}"));
        }
    }
    out.sort();
    out
}

/// Appends one Karpathy-style parseable line (`## [YYYY-MM-DD] <op> | <target> — <summary>`) to `log.md`.
fn append_brain_log_md(root: &Path, op: &str, target: &str, summary: &str) {
    let log_path = root.join(BRAIN_LOG_FILE_NAME);
    let date = Utc::now().format("%Y-%m-%d");
    let clean_summary = summary.replace(['\r', '\n'], " ");
    let entry = format!(
        "\n## [{date}] {op} | {target}\n{}\n",
        compact_preview(clean_summary.trim(), 220)
    );
    let mut existing = std::fs::read_to_string(&log_path).unwrap_or_else(|_| {
        "# Wiki Log\n\nChronological append-only log of ingests, compilations, and lints.\n"
            .to_string()
    });
    existing.push_str(&entry);
    if existing.len() > BRAIN_MAX_LOG_BYTES {
        let keep_from = existing.floor_char_boundary(existing.len() - (BRAIN_MAX_LOG_BYTES * 3 / 4));
        if let Some(rel_nl) = existing[keep_from..].find('\n') {
            existing = format!(
                "# Wiki Log (trimmed older entries)\n{}",
                &existing[keep_from + rel_nl + 1..]
            );
        }
    }
    let _ = write_atomic_file(&log_path, &existing);
}

/// Regenerates a clean, human- and LLM-readable `index.md` catalog grouped by category with backlink counts.
fn rebuild_brain_index_md(root: &Path, scope: BrainScope) {
    let pages = load_all_brain_pages_at_root(root);
    let raw_files = list_raw_sources_at_root(root);

    let mut inbound_counts: BTreeMap<String, usize> = BTreeMap::new();
    for page in &pages {
        let mut out_targets: BTreeSet<String> = BTreeSet::new();
        for ev in &page.timeline {
            for dep in &ev.affects {
                if dep != &page.id {
                    out_targets.insert(dep.clone());
                }
            }
        }
        for link in extract_markdown_links(&page.compiled_truth) {
            if let Some(slug) = extract_brain_link_slug(&link)
                && slug != page.id
            {
                out_targets.insert(slug);
            }
        }
        for target in out_targets {
            *inbound_counts.entry(target).or_insert(0) += 1;
        }
    }

    let heading = match scope {
        BrainScope::Workspace => "# Project Brain Index",
        BrainScope::Hub => "# Personal Hub Wiki Index",
    };
    let mut grouped: BTreeMap<&'static str, Vec<&BrainPage>> = BTreeMap::new();
    let mut archived_count = 0usize;
    for page in &pages {
        if page.status == BrainPageStatus::Archived {
            archived_count += 1;
            continue;
        }
        grouped
            .entry(page.category.as_str())
            .or_default()
            .push(page);
    }

    let active_count = pages.len().saturating_sub(archived_count);
    let mut out = format!(
        "{heading}\n\n\
         - **Active pages:** {active_count} (archived: {archived_count})\n\
         - **Raw sources (`{BRAIN_RAW_DIR_NAME}/`):** {}\n\n",
        raw_files.len()
    );

    if grouped.is_empty() {
        out.push_str("_No compiled pages yet. Drop sources into `raw/` or call `project_brain(action=\"create_page\")`._\n");
    } else {
        for (cat, list) in grouped {
            out.push_str(&format!("## {cat}\n"));
            for p in list {
                let first_line = p
                    .compiled_truth
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty() && !l.starts_with('#'))
                    .unwrap_or(&p.title);
                let in_links = inbound_counts.get(&p.id).copied().unwrap_or(0);
                out.push_str(&format!(
                    "- [{}]({BRAIN_PAGES_DIR_NAME}/{}.md) (`{}`) — {} [backlinks: {in_links}]\n",
                    p.title,
                    p.id,
                    p.id,
                    compact_preview(first_line, 96)
                ));
            }
            out.push('\n');
        }
    }

    if !raw_files.is_empty() {
        out.push_str(&format!("## Raw Sources ({BRAIN_RAW_DIR_NAME}/)\n"));
        for rf in raw_files.iter().take(40) {
            out.push_str(&format!("- `{rf}`\n"));
        }
    }

    let _ = write_atomic_file(&root.join(BRAIN_INDEX_FILE_NAME), &out);
}

fn extract_brain_link_slug(raw_link: &str) -> Option<String> {
    let trimmed = raw_link.trim();
    if trimmed.is_empty() || trimmed.contains("://") || trimmed.starts_with('#') {
        return None;
    }
    let clean = trimmed.split('#').next().unwrap_or(trimmed).trim();
    if !clean.ends_with(".md") {
        return None;
    }
    if clean.starts_with("raw/") || clean.starts_with("../raw/") {
        return None;
    }
    let stem = clean
        .rsplit('/')
        .next()
        .unwrap_or(clean)
        .strip_suffix(".md")
        .unwrap_or(clean);
    sanitize_brain_slug(stem).ok()
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

/// Loads all parsed pages from `<root>/pages/*.md` deterministically sorted by page ID.
pub fn load_all_brain_pages_at_root(root: &Path) -> Vec<BrainPage> {
    let pages_dir = root.join(BRAIN_PAGES_DIR_NAME);
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

/// Loads all parsed pages from `<brain_root>/pages/*.md` deterministically sorted by page ID.
pub fn load_all_brain_pages(workspace_dir: &Path) -> Vec<BrainPage> {
    load_all_brain_pages_at_root(&resolve_brain_root(workspace_dir))
}

pub fn list_brain_pages(
    workspace_dir: &Path,
    status_filter: Option<BrainPageStatus>,
    category_filter: Option<BrainPageCategory>,
) -> Vec<BrainPageSummary> {
    list_brain_pages_scoped(
        workspace_dir,
        workspace_dir,
        BrainScope::Workspace,
        status_filter,
        category_filter,
    )
}

pub fn list_brain_pages_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    status_filter: Option<BrainPageStatus>,
    category_filter: Option<BrainPageCategory>,
) -> Vec<BrainPageSummary> {
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let provenance_ws = match scope {
        BrainScope::Workspace => Some(workspace_dir),
        BrainScope::Hub => Some(root.as_path()),
    };
    load_all_brain_pages_at_root(&root)
        .into_iter()
        .filter(|p| status_filter.is_none_or(|st| p.status == st))
        .filter(|p| category_filter.is_none_or(|c| p.category == c))
        .map(|p| summarize_brain_page_in_workspace(&p, provenance_ws))
        .collect()
}

pub fn read_brain_page(workspace_dir: &Path, raw_id: &str) -> Result<BrainPage> {
    read_brain_page_scoped(workspace_dir, workspace_dir, BrainScope::Workspace, raw_id)
}

pub fn read_brain_page_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    raw_id: &str,
) -> Result<BrainPage> {
    let id = sanitize_brain_slug(raw_id)?;
    let path = resolve_scoped_brain_root(workspace_dir, hub_dir, scope)
        .join(BRAIN_PAGES_DIR_NAME)
        .join(format!("{id}.md"));
    if !path.is_file() {
        return Err(ZenkoError::NotFound(format!(
            "project_brain ({}) page `{id}` does not exist",
            scope.as_str()
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
    create_brain_page_scoped(
        workspace_dir,
        workspace_dir,
        BrainScope::Workspace,
        raw_id,
        title,
        category,
        status,
        tags,
        compiled_truth,
        timeline_summary,
        source,
        affects,
        known_secrets,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn create_brain_page_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
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
    let _ = init_brain_scoped(workspace_dir, hub_dir, scope)?;
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let path = root.join(BRAIN_PAGES_DIR_NAME).join(format!("{id}.md"));
    if path.exists() {
        return Err(ZenkoError::Tool(format!(
            "project_brain ({}) page `{id}` already exists (use `update_truth` or `append_timeline` instead)",
            scope.as_str()
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
            summary: clean_summary.clone(),
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
    rebuild_brain_index_md(&root, scope);
    append_brain_log_md(&root, "create_page", &id, &clean_summary);
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
    update_brain_page_truth_scoped(
        workspace_dir,
        workspace_dir,
        BrainScope::Workspace,
        raw_id,
        new_truth,
        kind,
        summary,
        source,
        affects,
        tags_override,
        status_override,
        known_secrets,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn update_brain_page_truth_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
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
    let mut page = read_brain_page_scoped(workspace_dir, hub_dir, scope, raw_id)?;
    let clean_truth = sanitize_memory_text_with_secrets(new_truth.trim(), known_secrets);
    let clean_summary = sanitize_memory_text_with_secrets(summary.trim(), known_secrets);
    if clean_truth.is_empty() || clean_summary.is_empty() {
        return Err(ZenkoError::Tool(
            "project_brain `update_truth` requires non-empty `compiled_truth` and `summary`".into(),
        ));
    }
    let clean_source = sanitize_memory_text_with_secrets(source.trim(), known_secrets);
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    page.compiled_truth = clean_truth;
    page.updated = now.clone();
    if !clean_source.is_empty() && !page.sources.iter().any(|s| s == &clean_source) {
        page.sources.push(clean_source.clone());
    }
    if let Some(new_tags) = tags_override {
        page.tags = sanitize_slug_list(new_tags, BRAIN_MAX_TAGS);
    }
    if let Some(st) = status_override {
        page.status = st;
    }
    page.timeline.push(BrainTimelineEntry {
        time: now,
        kind,
        summary: clean_summary.clone(),
        source: clean_source,
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
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let path = root.join(BRAIN_PAGES_DIR_NAME).join(format!("{}.md", page.id));
    write_atomic_file(&path, &rendered)?;
    rebuild_brain_index_md(&root, scope);
    append_brain_log_md(&root, "update_truth", &page.id, &clean_summary);
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
    append_brain_timeline_scoped(
        workspace_dir,
        workspace_dir,
        BrainScope::Workspace,
        raw_id,
        kind,
        summary,
        source,
        affects,
        known_secrets,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn append_brain_timeline_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    raw_id: &str,
    kind: BrainTimelineKind,
    summary: &str,
    source: &str,
    affects: &[String],
    known_secrets: &[String],
) -> Result<BrainPage> {
    let mut page = read_brain_page_scoped(workspace_dir, hub_dir, scope, raw_id)?;
    let clean_summary = sanitize_memory_text_with_secrets(summary.trim(), known_secrets);
    if clean_summary.is_empty() {
        return Err(ZenkoError::Tool(
            "project_brain `append_timeline` requires non-empty `summary`".into(),
        ));
    }
    let clean_source = sanitize_memory_text_with_secrets(source.trim(), known_secrets);
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    page.updated = now.clone();
    if !clean_source.is_empty() && !page.sources.iter().any(|s| s == &clean_source) {
        page.sources.push(clean_source.clone());
    }
    page.timeline.push(BrainTimelineEntry {
        time: now,
        kind,
        summary: clean_summary.clone(),
        source: clean_source,
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
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let path = root.join(BRAIN_PAGES_DIR_NAME).join(format!("{}.md", page.id));
    write_atomic_file(&path, &rendered)?;
    rebuild_brain_index_md(&root, scope);
    append_brain_log_md(&root, kind.as_str(), &page.id, &clean_summary);
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
    archive_brain_page_scoped(
        workspace_dir,
        workspace_dir,
        BrainScope::Workspace,
        raw_id,
        reversal_summary,
        source,
        affects,
        is_reversal,
        known_secrets,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn archive_brain_page_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    raw_id: &str,
    reversal_summary: &str,
    source: &str,
    affects: &[String],
    is_reversal: bool,
    known_secrets: &[String],
) -> Result<BrainPage> {
    let mut page = read_brain_page_scoped(workspace_dir, hub_dir, scope, raw_id)?;
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
        summary: clean_summary.clone(),
        source: sanitize_memory_text_with_secrets(source.trim(), known_secrets),
        affects: sanitize_slug_list(affects, BRAIN_MAX_AFFECTS),
    });
    let rendered = serialize_brain_page(&page);
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let path = root.join(BRAIN_PAGES_DIR_NAME).join(format!("{}.md", page.id));
    write_atomic_file(&path, &rendered)?;
    rebuild_brain_index_md(&root, scope);
    append_brain_log_md(
        &root,
        match is_reversal {
            true => "reversal",
            false => "archive_page",
        },
        &page.id,
        &clean_summary,
    );
    Ok(page)
}

pub fn read_brain_root_slot(workspace_dir: &Path, slot: BrainRootSlot) -> Result<String> {
    read_brain_root_slot_scoped(workspace_dir, workspace_dir, BrainScope::Workspace, slot)
}

pub fn read_brain_root_slot_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    slot: BrainRootSlot,
) -> Result<String> {
    let path = resolve_scoped_brain_root(workspace_dir, hub_dir, scope).join(slot.filename());
    if !path.is_file() {
        return Err(ZenkoError::NotFound(format!(
            "project_brain ({}) root slot `{}` does not exist (run `init` or `update_root`)",
            scope.as_str(),
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
    update_brain_root_slot_scoped(
        workspace_dir,
        workspace_dir,
        BrainScope::Workspace,
        slot,
        content,
        known_secrets,
    )
}

pub fn update_brain_root_slot_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    slot: BrainRootSlot,
    content: &str,
    known_secrets: &[String],
) -> Result<String> {
    let _ = init_brain_scoped(workspace_dir, hub_dir, scope)?;
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
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let path = root.join(slot.filename());
    write_atomic_file(&path, &body)?;
    append_brain_log_md(&root, "update_root", slot.as_str(), desc.title);
    Ok(body)
}

/// Saves an immutable raw source into `<root>/raw/<slug>.md` (Karpathy LLM Wiki ingest layer).
/// Refuses silent overwrite if the slug already exists so raw sources remain append-only.
#[allow(clippy::too_many_arguments)]
pub fn ingest_raw_source(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    raw_id: &str,
    title: &str,
    source_url: &str,
    content: &str,
    known_secrets: &[String],
) -> Result<String> {
    let slug = sanitize_brain_slug(raw_id)?;
    let _ = init_brain_scoped(workspace_dir, hub_dir, scope)?;
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let rel_path = format!("{BRAIN_RAW_DIR_NAME}/{slug}.md");
    let full_path = root.join(&rel_path);
    if full_path.exists() {
        return Err(ZenkoError::Tool(format!(
            "immutable raw source `{rel_path}` already exists in {} wiki; pick a new `id` to preserve raw history",
            scope.as_str()
        )));
    }
    let clean_body = sanitize_memory_text_with_secrets(content.trim(), known_secrets);
    if clean_body.is_empty() {
        return Err(ZenkoError::Tool(
            "project_brain `ingest_raw` requires non-empty `content`".into(),
        ));
    }
    let clean_title = match title.trim().is_empty() {
        true => slug.clone(),
        false => compact_preview(
            &sanitize_memory_text_with_secrets(title.trim(), known_secrets),
            BRAIN_MAX_TITLE_CHARS,
        ),
    };
    let clean_source = sanitize_memory_text_with_secrets(source_url.trim(), known_secrets);
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let mut rendered = String::with_capacity(256 + clean_body.len());
    rendered.push_str("---\n");
    rendered.push_str(&format!("id: {slug}\n"));
    rendered.push_str(&format!("title: {}\n", escape_yaml_inline(&clean_title)));
    if !clean_source.is_empty() {
        rendered.push_str(&format!("source: \"{}\"\n", escape_yaml_inline(&clean_source)));
    }
    rendered.push_str(&format!("ingested_at: \"{now}\"\n"));
    rendered.push_str("immutable: true\n---\n\n");
    rendered.push_str(&clean_body);
    rendered.push('\n');

    if rendered.len() > BRAIN_MAX_RAW_BYTES {
        return Err(ZenkoError::Tool(format!(
            "raw source `{rel_path}` exceeds {} KiB cap",
            BRAIN_MAX_RAW_BYTES / 1024
        )));
    }
    write_atomic_file(&full_path, &rendered)?;
    rebuild_brain_index_md(&root, scope);
    append_brain_log_md(
        &root,
        "ingest",
        &rel_path,
        &format!("Saved immutable raw source `{clean_title}`"),
    );
    Ok(rel_path)
}

/// Deterministic zero-regex BFS impact graph over `affects` edges and reverse backlinks.
pub fn compute_brain_impact_graph(workspace_dir: &Path, raw_id: &str) -> Result<BrainImpactGraph> {
    compute_brain_impact_graph_scoped(workspace_dir, workspace_dir, BrainScope::Workspace, raw_id)
}

pub fn compute_brain_impact_graph_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    raw_id: &str,
) -> Result<BrainImpactGraph> {
    let target_id = sanitize_brain_slug(raw_id)?;
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let pages = load_all_brain_pages_at_root(&root);
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
        for raw_link in extract_markdown_links(&page.compiled_truth) {
            if let Some(slug) = extract_brain_link_slug(&raw_link)
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

/// Deterministic zero-token Karpathy Wiki `lint` pass checking orphans, broken links, missing
/// workspace source files, stale/drifted pages, uncompiled `raw/` sources, and suggested concepts.
pub fn lint_brain(workspace_dir: &Path, hub_dir: &Path, scope: BrainScope) -> BrainLintReport {
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let pages = load_all_brain_pages_at_root(&root);
    let raw_files = list_raw_sources_at_root(&root);
    let active_pages_vec: Vec<&BrainPage> = pages
        .iter()
        .filter(|p| p.status != BrainPageStatus::Archived)
        .collect();
    let known_ids: BTreeSet<&str> = pages.iter().map(|p| p.id.as_str()).collect();
    let provenance_ws = match scope {
        BrainScope::Workspace => Some(workspace_dir),
        BrainScope::Hub => Some(root.as_path()),
    };

    let mut inbound_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut dangling_ref_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut broken_links: BTreeSet<String> = BTreeSet::new();
    let mut missing_source_files: BTreeSet<String> = BTreeSet::new();
    let mut stale_pages: BTreeSet<String> = BTreeSet::new();
    let mut issues: Vec<BrainLintIssue> = Vec::new();
    let mut push_issue = |kind: BrainLintIssueKind, target: String, detail: String| {
        if issues.len() < BRAIN_LINT_MAX_ISSUES {
            issues.push(BrainLintIssue {
                kind,
                target,
                detail,
            });
        }
    };

    for page in &active_pages_vec {
        let mut out_targets: BTreeSet<String> = BTreeSet::new();
        for ev in &page.timeline {
            for dep in &ev.affects {
                if dep != &page.id {
                    out_targets.insert(dep.clone());
                }
            }
        }
        for raw_link in extract_markdown_links(&page.compiled_truth) {
            if let Some(slug) = extract_brain_link_slug(&raw_link)
                && slug != page.id
            {
                out_targets.insert(slug);
            }
        }
        for target in out_targets {
            match known_ids.contains(target.as_str()) {
                true => *inbound_counts.entry(target).or_insert(0) += 1,
                false => {
                    *dangling_ref_counts.entry(target.clone()).or_insert(0) += 1;
                    let edge = format!("{} -> {}", page.id, target);
                    if broken_links.insert(edge.clone()) {
                        push_issue(
                            BrainLintIssueKind::BrokenLink,
                            edge,
                            format!("Page `{}` references missing page `{target}`", page.id),
                        );
                    }
                }
            }
        }

        let summary = summarize_brain_page_in_workspace(page, provenance_ws);
        if summary.freshness == OkfFreshnessState::Stale
            || summary.trust_tier == OkfTrustTier::SourceDrifted
        {
            if stale_pages.insert(page.id.clone()) {
                push_issue(
                    BrainLintIssueKind::StalePage,
                    page.id.clone(),
                    format!(
                        "Page `{}` is {}/{} and should be re-verified",
                        page.id,
                        summary.trust_tier.as_str(),
                        summary.freshness.as_str()
                    ),
                );
            }
        }

        if let Some(ws) = provenance_ws {
            let mut candidate_sources: Vec<String> = page.sources.clone();
            for ev in &page.timeline {
                if !ev.source.is_empty() && !candidate_sources.iter().any(|s| s == &ev.source) {
                    candidate_sources.push(ev.source.clone());
                }
            }
            let (missing, _) = evaluate_source_provenance(ws, &candidate_sources, None);
            for m in missing {
                // Only flag file-like local paths (contain `/` or `.`) so freeform notes like "audit" aren't false positives
                if (m.contains('/') || m.contains('.')) && missing_source_files.insert(m.clone()) {
                    push_issue(
                        BrainLintIssueKind::MissingSourceFile,
                        m.clone(),
                        format!("Page `{}` cites missing local source `{m}`", page.id),
                    );
                }
            }
        }
    }

    let mut orphan_pages: Vec<String> = Vec::new();
    if active_pages_vec.len() >= 2 {
        for page in &active_pages_vec {
            if inbound_counts.get(&page.id).copied().unwrap_or(0) == 0 {
                orphan_pages.push(page.id.clone());
                push_issue(
                    BrainLintIssueKind::OrphanPage,
                    page.id.clone(),
                    format!("Active page `{}` has zero inbound links or `affects` backlinks", page.id),
                );
            }
        }
    }

    let mut suggested_concepts: Vec<String> = Vec::new();
    for (slug, count) in dangling_ref_counts {
        if count >= 2 {
            suggested_concepts.push(slug.clone());
            push_issue(
                BrainLintIssueKind::SuggestedConcept,
                slug.clone(),
                format!("Missing concept `{slug}` is referenced by {count} pages; consider creating it"),
            );
        }
    }

    let mut uncompiled_raw_sources: Vec<String> = Vec::new();
    for raw_rel in &raw_files {
        let fname = raw_rel.rsplit('/').next().unwrap_or(raw_rel);
        let cited = pages.iter().any(|p| {
            p.sources
                .iter()
                .any(|s| s.contains(raw_rel.as_str()) || s.contains(fname))
                || p.compiled_truth.contains(raw_rel.as_str())
                || p.compiled_truth.contains(fname)
                || p.timeline
                    .iter()
                    .any(|e| e.source.contains(raw_rel.as_str()) || e.source.contains(fname))
        });
        if !cited {
            uncompiled_raw_sources.push(raw_rel.clone());
            push_issue(
                BrainLintIssueKind::UncompiledRaw,
                raw_rel.clone(),
                format!("Raw source `{raw_rel}` has not been compiled into any wiki page yet"),
            );
        }
    }

    BrainLintReport {
        scope,
        total_pages: pages.len(),
        active_pages: active_pages_vec.len(),
        raw_sources_count: raw_files.len(),
        uncompiled_raw_sources,
        orphan_pages,
        broken_links: broken_links.into_iter().collect(),
        missing_source_files: missing_source_files.into_iter().collect(),
        stale_pages: stale_pages.into_iter().collect(),
        suggested_concepts,
        issues,
    }
}

/// Deterministic zero-regex token-overlap search across Project Brain pages (weights title,
/// tags, compiled truth, timeline summaries, active status, and reversal presence).
pub fn search_brain_pages(
    workspace_dir: &Path,
    query: &str,
    include_archived: bool,
    limit: usize,
) -> Vec<BrainSearchHit> {
    search_brain_pages_scoped(
        workspace_dir,
        workspace_dir,
        BrainScope::Workspace,
        query,
        include_archived,
        limit,
    )
}

pub fn search_brain_pages_scoped(
    workspace_dir: &Path,
    hub_dir: &Path,
    scope: BrainScope,
    query: &str,
    include_archived: bool,
    limit: usize,
) -> Vec<BrainSearchHit> {
    let q_terms: BTreeSet<String> = tokenize_terms(query).into_iter().collect();
    if q_terms.is_empty() {
        return Vec::new();
    }
    let root = resolve_scoped_brain_root(workspace_dir, hub_dir, scope);
    let provenance_ws = match scope {
        BrainScope::Workspace => Some(workspace_dir),
        BrainScope::Hub => Some(root.as_path()),
    };
    let pages = load_all_brain_pages_at_root(&root);
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
        let summary = summarize_brain_page_in_workspace(&page, provenance_ws);
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

/// Builds a compact Tier-2 `<hub_wiki_index>` summary list for `SnapshotManager`.
/// Returns `(active_page_count, uncompiled_raw_count, formatted_lines)` — strictly empty when unused.
pub fn build_hub_wiki_index(hub_dir: &Path) -> (usize, usize, Vec<String>) {
    if !has_hub_wiki(hub_dir) {
        return (0, 0, Vec::new());
    }
    let report = lint_brain(hub_dir, hub_dir, BrainScope::Hub);
    let pages: Vec<BrainPageSummary> =
        list_brain_pages_scoped(hub_dir, hub_dir, BrainScope::Hub, None, None)
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
    (total_active, report.uncompiled_raw_sources.len(), lines)
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

    #[test]
    fn karpathy_hub_wiki_supports_immutable_raw_index_log_and_lint() {
        let base = std::env::temp_dir().join(format!("zenko_hub_wiki_{}", zenko_core::new_id()));
        let ws = base.join("ws");
        let hub = base.join("hub");
        std::fs::create_dir_all(&ws).expect("ws");
        std::fs::create_dir_all(&hub).expect("hub");

        let raw_path = ingest_raw_source(
            &ws,
            &hub,
            BrainScope::Hub,
            "context-paper",
            "Effective Context Engineering",
            "https://anthropic.com/research/effective-context-engineering",
            "Use progressive disclosure and compaction to beat context rot.",
            &[],
        )
        .expect("ingest raw");
        assert_eq!(raw_path, "raw/context-paper.md");

        // Immutable raw drop-box must refuse silent overwrite
        assert!(
            ingest_raw_source(
                &ws,
                &hub,
                BrainScope::Hub,
                "context-paper",
                "Overwrite",
                "",
                "Should fail",
                &[],
            )
            .is_err()
        );

        // Before compiling, lint flags uncompiled raw source
        let pre_lint = lint_brain(&ws, &hub, BrainScope::Hub);
        assert_eq!(pre_lint.uncompiled_raw_sources, vec!["raw/context-paper.md"]);

        // Compile 2 pages referencing a shared missing concept (`kv-cache`) so lint suggests creating it
        let _ = create_brain_page_scoped(
            &ws,
            &hub,
            BrainScope::Hub,
            "context-engineering",
            "Context Engineering",
            BrainPageCategory::Concept,
            BrainPageStatus::Active,
            &["agents".into()],
            "Combines progressive disclosure with [KV Cache](kv-cache.md) discipline.",
            "Compiled from Anthropic article",
            &raw_path,
            &["kv-cache".into(), "compaction".into()],
            &[],
        )
        .expect("page 1");
        let _ = create_brain_page_scoped(
            &ws,
            &hub,
            BrainScope::Hub,
            "compaction",
            "Memory Compaction",
            BrainPageCategory::Concept,
            BrainPageStatus::Active,
            &["agents".into()],
            "Reclaims bloated tool outputs while respecting [KV Cache](kv-cache.md).",
            "Compiled compaction notes",
            &raw_path,
            &["kv-cache".into()],
            &[],
        )
        .expect("page 2");

        let post_lint = lint_brain(&ws, &hub, BrainScope::Hub);
        assert!(post_lint.uncompiled_raw_sources.is_empty());
        assert_eq!(post_lint.suggested_concepts, vec!["kv-cache"]);
        assert_eq!(post_lint.orphan_pages, vec!["context-engineering"]);

        // Auto-maintained index.md and log.md must exist and reflect operations
        let index_md = std::fs::read_to_string(hub.join("wiki/index.md")).expect("index.md");
        assert!(index_md.contains("Context Engineering"));
        assert!(index_md.contains("Memory Compaction"));
        let log_md = std::fs::read_to_string(hub.join("wiki/log.md")).expect("log.md");
        assert!(log_md.contains("ingest | raw/context-paper.md"));
        assert!(log_md.contains("create_page | context-engineering"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
