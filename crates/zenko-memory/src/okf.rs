//! Native Pure-Rust Open Knowledge Format (OKF v0.2/v0.3) Engine.
//!
//! Interoperable with `GoogleCloudPlatform/knowledge-catalog/okf` & `okf.md`:
//! - Enforces OKF's "One Required Field" (`type:`) rule so arbitrary READMEs are ignored.
//! - Zero-regex indentation-stack YAML frontmatter state machine supporting `type`, `title`,
//!   `summary`/`description`, `status`, `owner`, `tags`, `verified_by`/`verified_at` (flat or
//!   nested `verified:`), `stale_after`, and `sources` (inline, scalar, or `resource:` items).
//! - Zero-regex code-fence-aware byte state machine for Markdown `[label](relative.md)` link graphs.
//! - Live trust, freshness, and source-file mtime drift detection (`Verified`, `Unverified`,
//!   `SourceDrifted`, `Deprecated` + `Fresh`, `ExpiringSoon`, `Stale`, `Unspecified`).
//! - Hybrid search combining code-aware token overlap + 4-bit TurboQuant cosine + trust/freshness weights.

use crate::algorithms::{is_stopword, sanitize_memory_text_with_secrets, tokenize_terms};
use crate::brain::sanitize_brain_slug;
use crate::turboquant::TurboQuantCodec;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use zenko_core::{
    MEMORY_RRF_K, OKF_DEFAULT_WRITE_DIR, OKF_DISCOVERY_DIRS, OKF_EXPIRING_SOON_DAYS,
    OKF_FRONTMATTER_DELIM, OKF_KEY_AT, OKF_KEY_BY, OKF_KEY_DESCRIPTION, OKF_KEY_OWNER,
    OKF_KEY_RESOURCE, OKF_KEY_SOURCES, OKF_KEY_STALE_AFTER, OKF_KEY_STATUS, OKF_KEY_SUMMARY,
    OKF_KEY_TAGS, OKF_KEY_TITLE, OKF_KEY_TYPE, OKF_KEY_VERIFIED, OKF_KEY_VERIFIED_AT,
    OKF_KEY_VERIFIED_BY, OKF_MAX_FILE_BYTES, OKF_MAX_FILES_SCAN, OKF_MAX_GRAPH_HOPS,
    OKF_MAX_LINKS_PER_DOC, OKF_MAX_SOURCES_PER_DOC, OKF_RECON_MAX_HITS, OKF_SUMMARY_PREVIEW_CHARS,
    OkfAuditIssue, OkfAuditIssueKind, OkfConcept, OkfConceptSummary, OkfDocStatus,
    OkfFreshnessAuditReport, OkfFreshnessState, OkfGraphNeighborhood, OkfSearchHit, OkfTrustTier,
    Result, TURBOQUANT_MIN_COSINE, TURBOQUANT_RRF_WEIGHT, ZenkoError, compact_preview, new_id,
};

/// Parses either RFC 3339 timestamps or `YYYY-MM-DD` calendar dates deterministically (zero regex).
pub fn parse_okf_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(dt.with_timezone(&Utc));
    }
    let date_prefix = trimmed.get(..10).unwrap_or(trimmed);
    NaiveDate::parse_from_str(date_prefix, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(23, 59, 59))
        .map(|ndt| Utc.from_utc_datetime(&ndt))
}

/// Evaluates calendar freshness against `stale_after` without `if/else` chains.
pub fn evaluate_freshness(stale_after: Option<&str>, now: DateTime<Utc>) -> OkfFreshnessState {
    let Some(expiry) = stale_after.and_then(parse_okf_timestamp) else {
        return OkfFreshnessState::Unspecified;
    };
    let warn_cutoff = now + Duration::days(OKF_EXPIRING_SOON_DAYS);
    match (expiry <= now, expiry <= warn_cutoff) {
        (true, _) => OkfFreshnessState::Stale,
        (false, true) => OkfFreshnessState::ExpiringSoon,
        (false, false) => OkfFreshnessState::Fresh,
    }
}

fn is_external_uri(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    lower.contains("://")
        || lower.starts_with("urn:")
        || lower.starts_with("mailto:")
        || lower.starts_with('#')
}

/// Verifies local workspace `sources:` entries for missing paths and post-verification mtime drift.
pub fn evaluate_source_provenance(
    workspace_dir: &Path,
    sources: &[String],
    verified_at: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let canon_ws = std::fs::canonicalize(workspace_dir).unwrap_or_else(|_| workspace_dir.to_path_buf());
    let verified_dt = verified_at.and_then(parse_okf_timestamp);
    let mut missing = Vec::new();
    let mut drifted = Vec::new();

    for src in sources.iter().take(OKF_MAX_SOURCES_PER_DOC) {
        let trimmed = src.trim();
        if trimmed.is_empty() || is_external_uri(trimmed) {
            continue;
        }
        if trimmed.split(['/', '\\']).any(|seg| seg == "..") || Path::new(trimmed).is_absolute() {
            continue;
        }
        let candidate = workspace_dir.join(trimmed);
        match std::fs::canonicalize(&candidate) {
            Ok(canon) if canon.starts_with(&canon_ws) => {
                if let (Some(vdt), Ok(meta)) = (verified_dt, std::fs::metadata(&canon))
                    && let Ok(modified) = meta.modified()
                {
                    let mod_dt: DateTime<Utc> = modified.into();
                    if mod_dt > vdt {
                        drifted.push(trimmed.to_string());
                    }
                }
            }
            _ => missing.push(trimmed.to_string()),
        }
    }
    (missing, drifted)
}

/// Computes deterministic trust tier from lifecycle status, verification metadata, and provenance.
pub fn evaluate_trust_tier(
    status: OkfDocStatus,
    verified_by: Option<&str>,
    verified_at: Option<&str>,
    missing_sources: &[String],
    drifted_sources: &[String],
) -> OkfTrustTier {
    let has_verification = verified_by.is_some_and(|s| !s.trim().is_empty())
        || verified_at.is_some_and(|s| !s.trim().is_empty());
    let has_drift = !missing_sources.is_empty() || !drifted_sources.is_empty();
    match (status, has_drift, has_verification) {
        (OkfDocStatus::Deprecated | OkfDocStatus::Superseded, _, _) => OkfTrustTier::Deprecated,
        (_, true, _) => OkfTrustTier::SourceDrifted,
        (_, false, true) => OkfTrustTier::Verified,
        (_, false, false) => OkfTrustTier::Unverified,
    }
}

fn unquote_scalar(raw: &str) -> String {
    let trimmed = raw.trim();
    match (trimmed.starts_with('"'), trimmed.starts_with('\'')) {
        (true, _) => trimmed
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(trimmed)
            .trim()
            .to_string(),
        (false, true) => trimmed
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .unwrap_or(trimmed)
            .trim()
            .to_string(),
        (false, false) => match trimmed.find(" #") {
            Some(idx) => trimmed[..idx].trim().to_string(),
            None => trimmed.to_string(),
        },
    }
}

fn parse_inline_list(raw: &str, cap: usize) -> Vec<String> {
    let trimmed = raw.trim();
    let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for part in inner.split(',') {
        if out.len() >= cap {
            break;
        }
        let val = unquote_scalar(part);
        if !val.is_empty() && !out.contains(&val) {
            out.push(val);
        }
    }
    out
}

fn push_unique_capped(target: &mut Vec<String>, val: String, cap: usize) {
    if !val.is_empty() && target.len() < cap && !target.contains(&val) {
        target.push(val);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontmatterSection {
    Root,
    Verified,
    Sources,
    Tags,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FenceKind {
    None,
    Backtick,
    Tilde,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkScanState {
    Text,
    InlineCode,
    Label,
    AfterLabel,
    Target,
}

/// Deterministic, zero-regex byte/char state machine extracting relative Markdown links
/// (`[label](target.md)`) while skipping fenced code blocks and inline backtick spans.
pub fn extract_markdown_links(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut fence = FenceKind::None;

    for raw_line in body.lines() {
        let trimmed = raw_line.trim_start();
        match fence {
            FenceKind::None => match (trimmed.starts_with("```"), trimmed.starts_with("~~~")) {
                (true, _) => {
                    fence = FenceKind::Backtick;
                    continue;
                }
                (_, true) => {
                    fence = FenceKind::Tilde;
                    continue;
                }
                _ => {}
            },
            FenceKind::Backtick => {
                if trimmed.starts_with("```") {
                    fence = FenceKind::None;
                }
                continue;
            }
            FenceKind::Tilde => {
                if trimmed.starts_with("~~~") {
                    fence = FenceKind::None;
                }
                continue;
            }
        }

        let mut state = LinkScanState::Text;
        let mut target_buf = String::new();
        let mut prev_bang = false;

        for ch in raw_line.chars() {
            match state {
                LinkScanState::Text => match ch {
                    '`' => {
                        state = LinkScanState::InlineCode;
                        prev_bang = false;
                    }
                    '!' => prev_bang = true,
                    '[' => {
                        state = match prev_bang {
                            true => LinkScanState::Text,
                            false => LinkScanState::Label,
                        };
                        prev_bang = false;
                    }
                    _ => prev_bang = false,
                },
                LinkScanState::InlineCode => {
                    if ch == '`' {
                        state = LinkScanState::Text;
                    }
                }
                LinkScanState::Label => match ch {
                    ']' => state = LinkScanState::AfterLabel,
                    '`' => state = LinkScanState::InlineCode,
                    _ => {}
                },
                LinkScanState::AfterLabel => match ch {
                    '(' => {
                        target_buf.clear();
                        state = LinkScanState::Target;
                    }
                    '[' => state = LinkScanState::Label,
                    _ => state = LinkScanState::Text,
                },
                LinkScanState::Target => match ch {
                    ')' => {
                        let cleaned = normalize_markdown_link_target(&target_buf);
                        if let Some(rel_md) = cleaned {
                            push_unique_capped(&mut links, rel_md, OKF_MAX_LINKS_PER_DOC);
                        }
                        target_buf.clear();
                        state = LinkScanState::Text;
                    }
                    ' ' | '\t' if target_buf.is_empty() => {}
                    _ => target_buf.push(ch),
                },
            }
        }
    }
    links
}

fn normalize_markdown_link_target(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_start_matches('<')
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('>');
    if trimmed.is_empty() || is_external_uri(trimmed) {
        return None;
    }
    let without_frag = trimmed.split('#').next().unwrap_or("");
    let without_query = without_frag.split('?').next().unwrap_or("").trim();
    match without_query.to_ascii_lowercase().ends_with(".md") {
        true => Some(without_query.replace('\\', "/")),
        false => None,
    }
}

fn normalize_rel_path(path: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(seg) => parts.push(seg.to_string_lossy().to_string()),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    match parts.is_empty() {
        true => None,
        false => Some(parts.join("/")),
    }
}

/// Derives a canonical concept ID from a workspace-relative OKF path (strips `.md`).
pub fn concept_id_from_rel_path(rel_path: &str) -> String {
    let norm = rel_path.replace('\\', "/");
    norm.strip_suffix(".md")
        .or_else(|| norm.strip_suffix(".MD"))
        .unwrap_or(&norm)
        .to_string()
}

fn resolve_linked_concept_id(source_rel_path: &str, link_target: &str) -> Option<String> {
    let base_dir = Path::new(source_rel_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let joined = base_dir.join(link_target);
    normalize_rel_path(&joined).map(|p| concept_id_from_rel_path(&p))
}

fn extract_first_h1(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let clean = rest.trim();
            if !clean.is_empty() {
                return Some(clean.to_string());
            }
        }
    }
    None
}

/// Parses a single OKF Markdown file using deterministic indentation-stack frontmatter parsing.
/// Returns `None` if the document lacks OKF's single required `type:` frontmatter field.
pub fn parse_okf_document(
    workspace_dir: &Path,
    rel_path: &str,
    raw: &str,
    now: DateTime<Utc>,
) -> Option<OkfConcept> {
    let mut lines = raw.lines().peekable();
    if lines.peek().map(|l| l.trim()) != Some(OKF_FRONTMATTER_DELIM) {
        return None;
    }
    lines.next();

    let mut doc_type: Option<String> = None;
    let mut title: Option<String> = None;
    let mut summary = String::new();
    let mut status = OkfDocStatus::Active;
    let mut owner: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut verified_by: Option<String> = None;
    let mut verified_at: Option<String> = None;
    let mut stale_after: Option<String> = None;
    let mut sources: Vec<String> = Vec::new();

    let mut stack: Vec<(usize, FrontmatterSection)> = vec![(0, FrontmatterSection::Root)];
    let mut closed = false;

    for raw_line in lines.by_ref() {
        let stripped = raw_line.trim_end_matches('\r');
        let trimmed = stripped.trim();
        if trimmed == OKF_FRONTMATTER_DELIM {
            closed = true;
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = stripped.bytes().take_while(|&b| b == b' ').count();
        let is_seq_item = trimmed.starts_with("- ");
        while stack.len() > 1
            && stack.last().is_some_and(|&(ind, sec)| {
                let same_indent_list = is_seq_item
                    && indent == ind
                    && matches!(sec, FrontmatterSection::Tags | FrontmatterSection::Sources);
                indent <= ind && !same_indent_list
            })
        {
            stack.pop();
        }
        let section = stack
            .last()
            .map(|&(_, sec)| sec)
            .unwrap_or(FrontmatterSection::Root);

        match section {
            FrontmatterSection::Tags => {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    push_unique_capped(&mut tags, unquote_scalar(item), 16);
                }
            }
            FrontmatterSection::Sources => match trimmed.strip_prefix("- ") {
                Some(after_dash) => match after_dash.split_once(':') {
                    Some((k, v)) if k.trim().eq_ignore_ascii_case(OKF_KEY_RESOURCE) => {
                        push_unique_capped(&mut sources, unquote_scalar(v), OKF_MAX_SOURCES_PER_DOC);
                    }
                    Some((k, _)) if !k.contains('/') && !k.contains('.') => {}
                    _ => {
                        push_unique_capped(
                            &mut sources,
                            unquote_scalar(after_dash),
                            OKF_MAX_SOURCES_PER_DOC,
                        );
                    }
                },
                None => {
                    if let Some((k, v)) = trimmed.split_once(':')
                        && k.trim().eq_ignore_ascii_case(OKF_KEY_RESOURCE)
                    {
                        push_unique_capped(&mut sources, unquote_scalar(v), OKF_MAX_SOURCES_PER_DOC);
                    }
                }
            },
            FrontmatterSection::Verified => {
                if let Some((k, v)) = trimmed.split_once(':') {
                    let val = unquote_scalar(v);
                    match k.trim().to_ascii_lowercase().as_str() {
                        k if k == OKF_KEY_BY || k == OKF_KEY_VERIFIED_BY => {
                            if !val.is_empty() {
                                verified_by = Some(val);
                            }
                        }
                        k if k == OKF_KEY_AT || k == OKF_KEY_VERIFIED_AT => {
                            if !val.is_empty() {
                                verified_at = Some(val);
                            }
                        }
                        _ => {}
                    }
                }
            }
            FrontmatterSection::Ignored => {}
            FrontmatterSection::Root => {
                let Some((k_raw, v_raw)) = trimmed.split_once(':') else {
                    continue;
                };
                let key = k_raw.trim().to_ascii_lowercase();
                let val_trimmed = v_raw.trim();
                let val = unquote_scalar(val_trimmed);

                match key.as_str() {
                    k if k == OKF_KEY_TYPE => {
                        if !val.is_empty() {
                            doc_type = Some(val);
                        }
                    }
                    k if k == OKF_KEY_TITLE => {
                        if !val.is_empty() {
                            title = Some(val);
                        }
                    }
                    k if k == OKF_KEY_SUMMARY || k == OKF_KEY_DESCRIPTION => {
                        if summary.is_empty() && !val.is_empty() {
                            summary = val;
                        }
                    }
                    k if k == OKF_KEY_STATUS => {
                        if let Some(st) = OkfDocStatus::parse(&val) {
                            status = st;
                        }
                    }
                    k if k == OKF_KEY_OWNER => {
                        if !val.is_empty() {
                            owner = Some(val);
                        }
                    }
                    k if k == OKF_KEY_VERIFIED_BY => {
                        if !val.is_empty() {
                            verified_by = Some(val);
                        }
                    }
                    k if k == OKF_KEY_VERIFIED_AT => {
                        if !val.is_empty() {
                            verified_at = Some(val);
                        }
                    }
                    k if k == OKF_KEY_STALE_AFTER => {
                        if !val.is_empty() {
                            stale_after = Some(val);
                        }
                    }
                    k if k == OKF_KEY_VERIFIED => match val_trimmed.is_empty() {
                        true => stack.push((indent, FrontmatterSection::Verified)),
                        false => {
                            if !val.is_empty() {
                                verified_by = Some(val);
                            }
                        }
                    },
                    k if k == OKF_KEY_TAGS => match val_trimmed.is_empty() {
                        true => stack.push((indent, FrontmatterSection::Tags)),
                        false => {
                            for t in parse_inline_list(val_trimmed, 16) {
                                push_unique_capped(&mut tags, t, 16);
                            }
                        }
                    },
                    k if k == OKF_KEY_SOURCES => match val_trimmed.is_empty() {
                        true => stack.push((indent, FrontmatterSection::Sources)),
                        false => {
                            for s in parse_inline_list(val_trimmed, OKF_MAX_SOURCES_PER_DOC) {
                                push_unique_capped(&mut sources, s, OKF_MAX_SOURCES_PER_DOC);
                            }
                        }
                    },
                    _ => {
                        if val_trimmed.is_empty() {
                            stack.push((indent, FrontmatterSection::Ignored));
                        }
                    }
                }
            }
        }
    }

    if !closed {
        return None;
    }
    let doc_type = doc_type?;
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    let id = concept_id_from_rel_path(rel_path);
    let resolved_title = title
        .or_else(|| extract_first_h1(&body))
        .unwrap_or_else(|| {
            id.rsplit('/')
                .next()
                .unwrap_or(&id)
                .to_string()
        });

    let outbound_links: Vec<String> = extract_markdown_links(&body)
        .into_iter()
        .filter_map(|target| resolve_linked_concept_id(rel_path, &target))
        .filter(|linked_id| linked_id != &id)
        .collect();

    let freshness = evaluate_freshness(stale_after.as_deref(), now);
    let (missing_sources, drifted_sources) =
        evaluate_source_provenance(workspace_dir, &sources, verified_at.as_deref());
    let trust_tier = evaluate_trust_tier(
        status,
        verified_by.as_deref(),
        verified_at.as_deref(),
        &missing_sources,
        &drifted_sources,
    );

    Some(OkfConcept {
        id,
        rel_path: rel_path.replace('\\', "/"),
        doc_type,
        title: resolved_title,
        summary,
        status,
        owner,
        tags,
        verified_by,
        verified_at,
        stale_after,
        sources,
        outbound_links,
        missing_sources,
        drifted_sources,
        freshness,
        trust_tier,
        body,
    })
}

fn collect_markdown_files_bounded(
    workspace_canon: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    cap: usize,
) {
    if out.len() >= cap {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|d| d.path()).collect();
    entries.sort();
    for path in entries {
        if out.len() >= cap {
            break;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with('.') && name != "node_modules" && name != "target" {
                collect_markdown_files_bounded(workspace_canon, &path, out, cap);
            }
            continue;
        }
        if meta.is_file()
            && meta.len() <= OKF_MAX_FILE_BYTES
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            && std::fs::canonicalize(&path).is_ok_and(|c| c.starts_with(workspace_canon))
        {
            out.push(path);
        }
    }
}

/// Returns `true` when any declarative OKF discovery directory exists inside `workspace_dir`.
pub fn has_okf_discovery_dir(workspace_dir: &Path) -> bool {
    OKF_DISCOVERY_DIRS
        .iter()
        .any(|dir| workspace_dir.join(dir).is_dir())
}

/// Scans all OKF knowledge bundles in `workspace_dir` (strictly jailed, bounded, zero regex).
pub fn scan_okf_catalog(workspace_dir: &Path) -> Vec<OkfConcept> {
    if !has_okf_discovery_dir(workspace_dir) {
        return Vec::new();
    }
    let canon_ws = std::fs::canonicalize(workspace_dir).unwrap_or_else(|_| workspace_dir.to_path_buf());
    let mut files: Vec<PathBuf> = Vec::new();
    for rel_dir in OKF_DISCOVERY_DIRS {
        let dir = workspace_dir.join(rel_dir);
        if dir.is_dir() {
            collect_markdown_files_bounded(&canon_ws, &dir, &mut files, OKF_MAX_FILES_SCAN);
        }
    }
    if files.is_empty() {
        return Vec::new();
    }

    let now = Utc::now();
    let mut seen_ids = BTreeSet::new();
    let mut concepts = Vec::new();
    for file in files {
        let Ok(raw) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(workspace_dir)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(concept) = parse_okf_document(workspace_dir, &rel, &raw, now)
            && seen_ids.insert(concept.id.clone())
        {
            concepts.push(concept);
        }
    }
    concepts.sort_by(|a, b| a.id.cmp(&b.id));
    concepts
}

pub fn summarize_okf_concept(c: &OkfConcept) -> OkfConceptSummary {
    let preview_src = match c.summary.trim().is_empty() {
        true => c.body.as_str(),
        false => c.summary.as_str(),
    };
    OkfConceptSummary {
        id: c.id.clone(),
        rel_path: c.rel_path.clone(),
        doc_type: c.doc_type.clone(),
        title: c.title.clone(),
        status: c.status,
        freshness: c.freshness,
        trust_tier: c.trust_tier,
        link_count: c.outbound_links.len(),
        source_count: c.sources.len(),
        preview: compact_preview(&preview_src.replace('\n', " "), OKF_SUMMARY_PREVIEW_CHARS),
    }
}

/// Builds a compact Tier-2 `<knowledge_catalog_index>` summary list for `SnapshotManager`.
/// Returns `(active_concepts_count, formatted_lines)` — strictly empty (0 tokens) when unused.
pub fn build_okf_catalog_index(workspace_dir: &Path) -> (usize, Vec<String>) {
    let catalog: Vec<OkfConcept> = scan_okf_catalog(workspace_dir)
        .into_iter()
        .filter(|c| !matches!(c.status, OkfDocStatus::Deprecated | OkfDocStatus::Superseded))
        .collect();
    if catalog.is_empty() {
        return (0, Vec::new());
    }
    let total = catalog.len();
    let lines = catalog
        .into_iter()
        .take(zenko_core::OKF_MAX_INDEX_CONCEPTS)
        .map(|c| {
            let s = summarize_okf_concept(&c);
            format!(
                "- `{}` [{} · {}/{}]: {}",
                s.id,
                s.doc_type,
                s.trust_tier.as_str(),
                s.freshness.as_str(),
                compact_preview(&s.title, 72)
            )
        })
        .collect();
    (total, lines)
}

/// Reads a single OKF concept by canonical ID (`knowledge/auth`), stem (`auth`), or relative path.
pub fn read_okf_concept(workspace_dir: &Path, id_or_path: &str) -> Option<OkfConcept> {
    let needle = concept_id_from_rel_path(id_or_path.trim());
    if needle.is_empty() {
        return None;
    }
    let catalog = scan_okf_catalog(workspace_dir);
    catalog
        .iter()
        .find(|c| c.id.eq_ignore_ascii_case(&needle))
        .or_else(|| {
            catalog.iter().find(|c| {
                c.id.rsplit('/')
                    .next()
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(&needle))
            })
        })
        .cloned()
}

/// Hybrid OKF search fusing code-aware token overlap + 4-bit TurboQuant cosine + trust/freshness weights.
pub fn search_okf_catalog(
    workspace_dir: &Path,
    query: &str,
    type_filter: Option<&str>,
    include_deprecated: bool,
    limit: usize,
) -> Vec<OkfSearchHit> {
    let catalog = scan_okf_catalog(workspace_dir);
    if catalog.is_empty() {
        return Vec::new();
    }
    let cap = limit.clamp(1, zenko_core::OKF_MAX_SEARCH_LIMIT);
    let trimmed = query.trim();
    let type_needle = type_filter.map(|t| t.trim().to_ascii_lowercase());

    let filtered: Vec<&OkfConcept> = catalog
        .iter()
        .filter(|c| {
            let status_ok = include_deprecated
                || !matches!(c.status, OkfDocStatus::Deprecated | OkfDocStatus::Superseded);
            let type_ok = type_needle
                .as_deref()
                .filter(|s| !s.is_empty())
                .is_none_or(|t| c.doc_type.to_ascii_lowercase() == t);
            status_ok && type_ok
        })
        .collect();

    if filtered.is_empty() {
        return Vec::new();
    }

    if trimmed.is_empty() {
        let mut hits: Vec<OkfSearchHit> = filtered
            .into_iter()
            .map(|c| OkfSearchHit {
                summary: summarize_okf_concept(c),
                score: c.trust_tier.rank_weight() * c.freshness.rank_weight(),
                vector_cosine: 0.0,
                outbound_links: c.outbound_links.clone(),
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.summary.id.cmp(&b.summary.id))
        });
        hits.truncate(cap);
        return hits;
    }

    let q_terms: BTreeSet<String> = tokenize_terms(trimmed)
        .into_iter()
        .filter(|t| !is_stopword(t))
        .collect();
    let codec = TurboQuantCodec::default_memory_codec();
    let prepared_q = codec.prepare_text_query(trimmed);

    let mut lex_scores: Vec<(usize, f64)> = Vec::with_capacity(filtered.len());
    let mut vec_scores: Vec<(usize, f32)> = Vec::with_capacity(filtered.len());
    let mut cos_by_idx: Vec<f32> = vec![0.0; filtered.len()];

    for (idx, c) in filtered.iter().enumerate() {
        let doc_text = format!(
            "{} {} {} {} {}",
            c.doc_type,
            c.title,
            c.tags.join(" "),
            c.summary,
            c.body
        );
        if !q_terms.is_empty() {
            let d_terms: BTreeSet<String> = tokenize_terms(&doc_text)
                .into_iter()
                .filter(|t| !is_stopword(t))
                .collect();
            let title_terms: BTreeSet<String> = tokenize_terms(&format!("{} {}", c.title, c.id))
                .into_iter()
                .filter(|t| !is_stopword(t))
                .collect();
            let overlap = q_terms.intersection(&d_terms).count() as f64;
            let title_hits = q_terms.intersection(&title_terms).count() as f64;
            let lex = (overlap + title_hits * 1.5) / (q_terms.len() as f64);
            if lex > 0.0 {
                lex_scores.push((idx, lex));
            }
        }
        if !prepared_q.is_zero() {
            let enc = codec.encode_text(&doc_text);
            let cos = prepared_q.cosine(&enc);
            cos_by_idx[idx] = cos;
            if cos >= TURBOQUANT_MIN_COSINE {
                vec_scores.push((idx, cos));
            }
        }
    }

    lex_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    vec_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let lex_rank: HashMap<usize, usize> = lex_scores
        .into_iter()
        .enumerate()
        .map(|(r, (i, _))| (i, r))
        .collect();
    let vec_rank: HashMap<usize, usize> = vec_scores
        .into_iter()
        .enumerate()
        .map(|(r, (i, _))| (i, r))
        .collect();

    let mut hits: Vec<OkfSearchHit> = filtered
        .into_iter()
        .enumerate()
        .filter_map(|(idx, c)| {
            let r_lex = lex_rank.get(&idx).copied();
            let r_vec = vec_rank.get(&idx).copied();
            if r_lex.is_none() && r_vec.is_none() {
                return None;
            }
            let rrf_l = r_lex.map_or(0.0, |r| 1.0 / (MEMORY_RRF_K + (r as f64) + 1.0));
            let rrf_v = r_vec.map_or(0.0, |r| {
                TURBOQUANT_RRF_WEIGHT / (MEMORY_RRF_K + (r as f64) + 1.0)
            });
            let score = (rrf_l + rrf_v) * c.trust_tier.rank_weight() * c.freshness.rank_weight();
            Some(OkfSearchHit {
                summary: summarize_okf_concept(c),
                score,
                vector_cosine: cos_by_idx[idx],
                outbound_links: c.outbound_links.clone(),
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(cap);
    hits
}

/// Bounded BFS knowledge-graph neighborhood (`direct_outbound`, `transitive_outbound`, `backlinks`, `broken_links`).
pub fn compute_okf_graph(
    workspace_dir: &Path,
    id_or_path: &str,
    max_hops: usize,
) -> Option<OkfGraphNeighborhood> {
    let catalog = scan_okf_catalog(workspace_dir);
    let needle = concept_id_from_rel_path(id_or_path.trim());
    let root = catalog
        .iter()
        .find(|c| c.id.eq_ignore_ascii_case(&needle))
        .or_else(|| {
            catalog.iter().find(|c| {
                c.id.rsplit('/')
                    .next()
                    .is_some_and(|stem| stem.eq_ignore_ascii_case(&needle))
            })
        })?;

    let by_id: BTreeMap<String, &OkfConcept> = catalog.iter().map(|c| (c.id.clone(), c)).collect();
    let mut backlinks = Vec::new();
    for c in &catalog {
        if c.id != root.id && c.outbound_links.iter().any(|l| l == &root.id) {
            backlinks.push(c.id.clone());
        }
    }

    let mut direct_outbound = Vec::new();
    let mut broken_links = Vec::new();
    for target in &root.outbound_links {
        match by_id.contains_key(target) {
            true => direct_outbound.push(target.clone()),
            false => broken_links.push(target.clone()),
        }
    }

    let hop_cap = max_hops.clamp(1, OKF_MAX_GRAPH_HOPS);
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visited.insert(root.id.clone());
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    for d in &direct_outbound {
        if visited.insert(d.clone()) {
            queue.push_back((d.clone(), 1));
        }
    }

    let mut transitive_outbound = Vec::new();
    while let Some((cur_id, depth)) = queue.pop_front() {
        if depth >= hop_cap {
            continue;
        }
        if let Some(node) = by_id.get(&cur_id) {
            for next in &node.outbound_links {
                if by_id.contains_key(next) && visited.insert(next.clone()) {
                    transitive_outbound.push(next.clone());
                    queue.push_back((next.clone(), depth + 1));
                }
            }
        }
    }

    Some(OkfGraphNeighborhood {
        concept_id: root.id.clone(),
        direct_outbound,
        transitive_outbound,
        backlinks,
        broken_links,
    })
}

/// Audits an entire OKF catalog for expired `stale_after`, drifted/missing `sources`, broken links, and unverified concepts.
pub fn audit_okf_freshness(workspace_dir: &Path) -> OkfFreshnessAuditReport {
    let catalog = scan_okf_catalog(workspace_dir);
    let valid_ids: BTreeSet<&str> = catalog.iter().map(|c| c.id.as_str()).collect();
    let mut report = OkfFreshnessAuditReport {
        total_concepts: catalog.len(),
        ..Default::default()
    };

    for c in &catalog {
        match c.trust_tier {
            OkfTrustTier::Verified => report.verified_count += 1,
            OkfTrustTier::SourceDrifted => report.drifted_count += 1,
            OkfTrustTier::Unverified => report.issues.push(OkfAuditIssue {
                concept_id: c.id.clone(),
                rel_path: c.rel_path.clone(),
                kind: OkfAuditIssueKind::Unverified,
                detail: "Missing verified_by/verified_at provenance".into(),
            }),
            OkfTrustTier::Deprecated => {}
        }
        match c.freshness {
            OkfFreshnessState::Stale => {
                report.stale_count += 1;
                report.issues.push(OkfAuditIssue {
                    concept_id: c.id.clone(),
                    rel_path: c.rel_path.clone(),
                    kind: OkfAuditIssueKind::Stale,
                    detail: format!(
                        "Expired past stale_after={}",
                        c.stale_after.as_deref().unwrap_or("unknown")
                    ),
                });
            }
            OkfFreshnessState::ExpiringSoon => report.issues.push(OkfAuditIssue {
                concept_id: c.id.clone(),
                rel_path: c.rel_path.clone(),
                kind: OkfAuditIssueKind::ExpiringSoon,
                detail: format!(
                    "Expiring soon (stale_after={})",
                    c.stale_after.as_deref().unwrap_or("unknown")
                ),
            }),
            OkfFreshnessState::Fresh | OkfFreshnessState::Unspecified => {}
        }
        for src in &c.missing_sources {
            report.issues.push(OkfAuditIssue {
                concept_id: c.id.clone(),
                rel_path: c.rel_path.clone(),
                kind: OkfAuditIssueKind::MissingSource,
                detail: format!("Provenance source not found: {src}"),
            });
        }
        for src in &c.drifted_sources {
            report.issues.push(OkfAuditIssue {
                concept_id: c.id.clone(),
                rel_path: c.rel_path.clone(),
                kind: OkfAuditIssueKind::SourceDrifted,
                detail: format!("Source modified after verified_at: {src}"),
            });
        }
        for link in &c.outbound_links {
            if !valid_ids.contains(link.as_str()) {
                report.broken_links_count += 1;
                report.issues.push(OkfAuditIssue {
                    concept_id: c.id.clone(),
                    rel_path: c.rel_path.clone(),
                    kind: OkfAuditIssueKind::BrokenLink,
                    detail: format!("Broken Markdown link target: {link}.md"),
                });
            }
        }
    }
    report
}

/// Zero-overhead Turn-1 reconnaissance formatter: emits `(None, [])` when no OKF bundle exists.
pub fn format_okf_recon(workspace_dir: &Path, query: &str) -> (Option<String>, Vec<String>) {
    let catalog = scan_okf_catalog(workspace_dir);
    if catalog.is_empty() {
        return (None, Vec::new());
    }
    let verified = catalog
        .iter()
        .filter(|c| c.trust_tier == OkfTrustTier::Verified)
        .count();
    let stale = catalog
        .iter()
        .filter(|c| c.freshness == OkfFreshnessState::Stale)
        .count();
    let drifted = catalog
        .iter()
        .filter(|c| c.trust_tier == OkfTrustTier::SourceDrifted)
        .count();
    let summary = format!(
        "{} concepts ({} verified, {} stale, {} drifted)",
        catalog.len(),
        verified,
        stale,
        drifted
    );
    let hits = search_okf_catalog(workspace_dir, query, None, false, OKF_RECON_MAX_HITS)
        .into_iter()
        .map(|h| {
            let stale_tag = match h.summary.freshness {
                OkfFreshnessState::Stale => "[STALE]",
                OkfFreshnessState::ExpiringSoon => "[EXPIRING]",
                OkfFreshnessState::Fresh | OkfFreshnessState::Unspecified => "",
            };
            format!(
                "`{}` [{}][{}]{stale_tag} {}: {}",
                h.summary.id,
                h.summary.doc_type,
                h.summary.trust_tier.badge(),
                h.summary.title,
                compact_preview(&h.summary.preview, 84)
            )
        })
        .collect();
    (Some(summary), hits)
}

pub struct OkfUpsertRequest<'a> {
    pub slug: &'a str,
    pub doc_type: &'a str,
    pub title: &'a str,
    pub summary: &'a str,
    pub status: OkfDocStatus,
    pub owner: Option<&'a str>,
    pub tags: &'a [String],
    pub verified_by: Option<&'a str>,
    pub verified_at: Option<&'a str>,
    pub stale_after: Option<&'a str>,
    pub sources: &'a [String],
    pub body: &'a str,
}

fn sanitize_single_line(raw: &str, known_secrets: &[String]) -> String {
    sanitize_memory_text_with_secrets(raw, known_secrets)
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string()
}

/// Atomically authors or updates an OKF concept under `knowledge/<slug>.md` (secret-scrubbed & jailed).
pub fn upsert_okf_concept(
    workspace_dir: &Path,
    req: OkfUpsertRequest<'_>,
    known_secrets: &[String],
) -> Result<OkfConcept> {
    let slug = sanitize_brain_slug(req.slug)?;
    let doc_type = sanitize_single_line(req.doc_type, known_secrets);
    if doc_type.is_empty() {
        return Err(ZenkoError::Tool(
            "OKF concept requires a non-empty `type` field".into(),
        ));
    }
    let title = sanitize_single_line(req.title, known_secrets);
    let summary = sanitize_single_line(req.summary, known_secrets);
    let body = sanitize_memory_text_with_secrets(req.body, known_secrets);

    let root = workspace_dir.join(OKF_DEFAULT_WRITE_DIR);
    std::fs::create_dir_all(&root)?;
    let target = root.join(format!("{slug}.md"));

    let canon_ws = std::fs::canonicalize(workspace_dir).unwrap_or_else(|_| workspace_dir.to_path_buf());
    let canon_root = std::fs::canonicalize(&root)?;
    if !canon_root.starts_with(&canon_ws) {
        return Err(ZenkoError::Tool(
            "OKF write directory escapes workspace root".into(),
        ));
    }

    let mut out = String::with_capacity(512 + body.len());
    out.push_str(OKF_FRONTMATTER_DELIM);
    out.push('\n');
    out.push_str(&format!("{OKF_KEY_TYPE}: {doc_type}\n"));
    if !title.is_empty() {
        out.push_str(&format!("{OKF_KEY_TITLE}: \"{title}\"\n"));
    }
    if !summary.is_empty() {
        out.push_str(&format!("{OKF_KEY_SUMMARY}: \"{summary}\"\n"));
    }
    out.push_str(&format!("{OKF_KEY_STATUS}: {}\n", req.status.as_str()));
    if let Some(owner) = req.owner.map(|s| sanitize_single_line(s, known_secrets)).filter(|s| !s.is_empty()) {
        out.push_str(&format!("{OKF_KEY_OWNER}: \"{owner}\"\n"));
    }
    if !req.tags.is_empty() {
        let clean_tags: Vec<String> = req
            .tags
            .iter()
            .take(16)
            .map(|t| sanitize_single_line(t, known_secrets))
            .filter(|t| !t.is_empty())
            .collect();
        if !clean_tags.is_empty() {
            out.push_str(&format!("{OKF_KEY_TAGS}: [{}]\n", clean_tags.join(", ")));
        }
    }
    if let Some(vb) = req.verified_by.map(|s| sanitize_single_line(s, known_secrets)).filter(|s| !s.is_empty()) {
        out.push_str(&format!("{OKF_KEY_VERIFIED_BY}: \"{vb}\"\n"));
    }
    if let Some(va) = req.verified_at.map(|s| sanitize_single_line(s, known_secrets)).filter(|s| !s.is_empty()) {
        out.push_str(&format!("{OKF_KEY_VERIFIED_AT}: \"{va}\"\n"));
    }
    if let Some(sa) = req.stale_after.map(|s| sanitize_single_line(s, known_secrets)).filter(|s| !s.is_empty()) {
        out.push_str(&format!("{OKF_KEY_STALE_AFTER}: \"{sa}\"\n"));
    }
    if !req.sources.is_empty() {
        let clean_sources: Vec<String> = req
            .sources
            .iter()
            .take(OKF_MAX_SOURCES_PER_DOC)
            .map(|s| sanitize_single_line(s, known_secrets))
            .filter(|s| !s.is_empty())
            .collect();
        if !clean_sources.is_empty() {
            out.push_str(&format!("{OKF_KEY_SOURCES}:\n"));
            for src in clean_sources {
                out.push_str(&format!("  - {OKF_KEY_RESOURCE}: \"{src}\"\n"));
            }
        }
    }
    out.push_str(OKF_FRONTMATTER_DELIM);
    out.push('\n');
    if !body.trim().is_empty() {
        out.push_str(body.trim());
        out.push('\n');
    }

    let tmp_path = root.join(format!(".{slug}.{}.tmp", new_id()));
    std::fs::write(&tmp_path, out.as_bytes())?;
    std::fs::rename(&tmp_path, &target)?;

    let rel_path = format!("{OKF_DEFAULT_WRITE_DIR}/{slug}.md");
    parse_okf_document(workspace_dir, &rel_path, &out, Utc::now()).ok_or_else(|| {
        ZenkoError::Tool("failed to re-parse authored OKF concept".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn okf_parses_trust_freshness_links_and_detects_source_drift() {
        let base = std::env::temp_dir().join(format!("zenko_okf_{}", new_id()));
        let knowledge = base.join("knowledge");
        let src_dir = base.join("src");
        let _ = std::fs::create_dir_all(&knowledge);
        let _ = std::fs::create_dir_all(&src_dir);
        let _ = std::fs::write(src_dir.join("auth.rs"), "pub fn verify() {}\n");

        // Non-OKF markdown without `type:` must be skipped cleanly
        let _ = std::fs::write(knowledge.join("README.md"), "# Random notes\nNo frontmatter.\n");

        // Fresh verified concept linking to billing.md and citing src/auth.rs
        let _ = std::fs::write(
            knowledge.join("auth.md"),
            r#"---
type: Architecture
title: Auth Pipeline
status: active
verified:
  by: security-team
  at: 2099-01-01T00:00:00Z
stale_after: 2099-12-31T00:00:00Z
sources:
  - resource: src/auth.rs
---
See [Billing Flow](billing.md) and ignore code:
```md
[Fake](ignored.md)
```
Also skip `[Inline](nope.md)`.
"#,
        );

        // Stale + drifted concept (verified in 2020 while src/auth.rs was modified today)
        let _ = std::fs::write(
            knowledge.join("billing.md"),
            r#"---
type: Runbook
title: Billing Retries
verified_by: ops
verified_at: 2020-01-01T00:00:00Z
stale_after: 2020-06-01
sources: [src/auth.rs, src/missing_file.rs]
---
Handles payment retries and links to [Broken](missing.md).
"#,
        );

        let catalog = scan_okf_catalog(&base);
        assert_eq!(catalog.len(), 2, "README.md without type: must be skipped");

        let auth = read_okf_concept(&base, "auth").expect("auth found");
        assert_eq!(auth.trust_tier, OkfTrustTier::Verified);
        assert_eq!(auth.freshness, OkfFreshnessState::Fresh);
        assert_eq!(auth.sources, vec!["src/auth.rs".to_string()]);
        assert_eq!(auth.outbound_links, vec!["knowledge/billing".to_string()]);

        let billing = read_okf_concept(&base, "billing").expect("billing found");
        assert_eq!(billing.freshness, OkfFreshnessState::Stale);
        assert_eq!(billing.trust_tier, OkfTrustTier::SourceDrifted);
        assert_eq!(billing.missing_sources, vec!["src/missing_file.rs"]);
        assert_eq!(billing.drifted_sources, vec!["src/auth.rs"]);

        let graph = compute_okf_graph(&base, "knowledge/auth", 2).expect("graph ok");
        assert_eq!(graph.direct_outbound, vec!["knowledge/billing"]);
        let back = compute_okf_graph(&base, "billing", 2).expect("back graph ok");
        assert_eq!(back.backlinks, vec!["knowledge/auth"]);
        assert_eq!(back.broken_links, vec!["knowledge/missing"]);

        let audit = audit_okf_freshness(&base);
        assert_eq!(audit.total_concepts, 2);
        assert_eq!(audit.verified_count, 1);
        assert_eq!(audit.stale_count, 1);
        assert_eq!(audit.drifted_count, 1);
        assert_eq!(audit.broken_links_count, 1);

        // Hybrid search combines TurboQuant cosine + trust weighting
        let hits = search_okf_catalog(&base, "authentication pipeline", None, true, 5);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].summary.id, "knowledge/auth");

        let _ = std::fs::remove_dir_all(&base);
    }
}
