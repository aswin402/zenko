//! Modular Context Assembly & Multi-Source Relevance Engine.
//!
//! Strictly separates two orthogonal concerns:
//! 1. **Instruction Precedence (`InstructionPrecedence`):** Determines which constraint wins when
//!    instructions conflict (`PlatformSafety > ActiveTaskState > ProjectRules > CompiledTruth >
//!    ScopedMemory > RuntimeSignals > UserDefaultsAndPersona`). Numerical relevance scores can
//!    NEVER override security, ExecPolicy, Plan Mode, or higher-precedence constraints.
//! 2. **Context Relevance (`retrieve_ranked_context`):** Multi-factor deterministic scoring across
//!    Repo Project Brain, Personal Hub Wiki, OKF Knowledge Catalog, SQLite Memories, and Progressive
//!    Skills so the agent loads strictly relevant context just-in-time while keeping Tier-1/Tier-2
//!    KV-cache prefixes 100% frozen.

use crate::compaction::estimate_text_tokens_calibrated;
use chrono::Utc;
use std::collections::BTreeSet;
use std::path::Path;
use zenko_core::{
    BrainPageStatus, BrainScope, CONTEXT_RETRIEVAL_DEFAULT_LIMIT, CONTEXT_RETRIEVAL_MAX_LIMIT,
    CONTEXT_SCORE_WEIGHT_AUTHORITY, CONTEXT_SCORE_WEIGHT_CONFIDENCE,
    CONTEXT_SCORE_WEIGHT_FRESHNESS, CONTEXT_SCORE_WEIGHT_RELEVANCE, CONTEXT_SCORE_WEIGHT_SCOPE,
    CONTEXT_TAIL_MAX_TOKENS, ContextRetrievalReport, ContextSourceKind, InstructionPrecedence,
    OkfFreshnessState, OkfTrustTier, RankedContextItem, SkillStatus, TOOL_KNOWLEDGE_CATALOG,
    TOOL_MEMORY, TOOL_PROJECT_BRAIN, TOOL_SKILL_VIEW, compact_preview,
};
use zenko_memory::algorithms::tokenize_terms;
use zenko_memory::brain::{has_hub_wiki, has_project_brain, search_brain_pages_scoped};
use zenko_memory::okf::{parse_okf_timestamp, search_okf_catalog};
use zenko_memory::{SnapshotManager, SqliteStore, project_scope_for_workspace};

/// One volatile Tier-3 context fragment tagged with explicit precedence and drop policy.
#[derive(Debug, Clone)]
pub struct TailContextBlock {
    pub precedence: InstructionPrecedence,
    pub kind: &'static str,
    pub mandatory: bool,
    pub body: String,
    order: usize,
}

#[derive(Debug, Clone, Default)]
pub struct AssembledTailContext {
    pub rendered: String,
    pub estimated_tokens: usize,
    pub included_kinds: Vec<&'static str>,
    pub dropped_kinds: Vec<&'static str>,
}

/// Precedence-ordered, token-budgeted assembler for Tier-3 `<system_reminder>` tails.
/// Preserves Tier-1/Tier-2 KV-cache prefixes while guaranteeing mandatory safety/task blocks
/// are always placed first and never dropped under tail budget pressure.
#[derive(Debug, Default)]
pub struct TailContextBuilder {
    blocks: Vec<TailContextBlock>,
}

impl TailContextBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(
        &mut self,
        precedence: InstructionPrecedence,
        kind: &'static str,
        mandatory: bool,
        body: String,
    ) {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return;
        }
        let order = self.blocks.len();
        self.blocks.push(TailContextBlock {
            precedence,
            kind,
            mandatory,
            body: trimmed.to_string(),
            order,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Sorts deterministically by `(precedence_rank, insertion_order)` and enforces `max_tokens`.
    /// Mandatory blocks (e.g. `PlatformSafety`, `ActiveTaskState`, orchestrator hints) are NEVER
    /// dropped; optional lower-precedence blocks are dropped first when tail budget is tight.
    pub fn build(mut self, max_tokens: usize, bytes_per_token: f64) -> AssembledTailContext {
        if self.blocks.is_empty() {
            return AssembledTailContext::default();
        }
        self.blocks
            .sort_by_key(|b| (b.precedence.rank(), b.order));

        let budget = match max_tokens {
            0 => CONTEXT_TAIL_MAX_TOKENS,
            n => n,
        };
        let mut kept: Vec<TailContextBlock> = Vec::with_capacity(self.blocks.len());
        let mut dropped_kinds: Vec<&'static str> = Vec::new();
        let mut used_tokens = 0usize;

        for block in self.blocks {
            let tok = estimate_text_tokens_calibrated(&block.body, bytes_per_token).max(1);
            match block.mandatory || used_tokens.saturating_add(tok) <= budget {
                true => {
                    used_tokens = used_tokens.saturating_add(tok);
                    kept.push(block);
                }
                false => {
                    dropped_kinds.push(block.kind);
                }
            }
        }

        let included_kinds: Vec<&'static str> = kept.iter().map(|b| b.kind).collect();
        let rendered = kept
            .into_iter()
            .map(|b| b.body)
            .collect::<Vec<_>>()
            .join("\n\n");

        AssembledTailContext {
            rendered,
            estimated_tokens: used_tokens,
            included_kinds,
            dropped_kinds,
        }
    }
}

/// Computes the transparent weighted relevance score in `[0.0, 1.0]` using declarative weights.
pub fn compute_context_score(
    relevance: f64,
    scope_match: f64,
    authority: f64,
    freshness: f64,
    confidence: f64,
) -> f64 {
    let clamp01 = |v: f64| v.clamp(0.0, 1.0);
    (CONTEXT_SCORE_WEIGHT_RELEVANCE * clamp01(relevance))
        + (CONTEXT_SCORE_WEIGHT_SCOPE * clamp01(scope_match))
        + (CONTEXT_SCORE_WEIGHT_AUTHORITY * clamp01(authority))
        + (CONTEXT_SCORE_WEIGHT_FRESHNESS * clamp01(freshness))
        + (CONTEXT_SCORE_WEIGHT_CONFIDENCE * clamp01(confidence))
}

fn trust_authority(trust: OkfTrustTier) -> f64 {
    match trust {
        OkfTrustTier::Verified => 1.0,
        OkfTrustTier::Unverified => 0.78,
        OkfTrustTier::SourceDrifted => 0.45,
        OkfTrustTier::Deprecated => 0.20,
    }
}

fn freshness_factor(freshness: OkfFreshnessState, updated_iso: Option<&str>) -> f64 {
    match freshness {
        OkfFreshnessState::Fresh => 1.0,
        OkfFreshnessState::Unspecified => updated_iso
            .and_then(parse_okf_timestamp)
            .map(|ts| memory_age_freshness((Utc::now() - ts).num_days().max(0)))
            .unwrap_or(0.80),
        OkfFreshnessState::ExpiringSoon => 0.65,
        OkfFreshnessState::Stale => 0.35,
    }
}

/// Blends per-source normalized rank score with true query-term lexical coverage so a single
/// weak hit inside a small source cannot self-normalize to `relevance = 1.0`.
fn blended_relevance(raw_norm: f64, q_terms: &BTreeSet<String>, doc_text: &str) -> f64 {
    if q_terms.is_empty() {
        return raw_norm.clamp(0.15, 1.0);
    }
    let d_terms: BTreeSet<String> = tokenize_terms(doc_text).into_iter().collect();
    let coverage = (q_terms.intersection(&d_terms).count() as f64) / (q_terms.len() as f64);
    (0.55 * coverage + 0.45 * raw_norm.clamp(0.0, 1.0)).clamp(0.15, 1.0)
}

fn memory_age_freshness(age_days: i64) -> f64 {
    match age_days {
        d if d <= 7 => 1.0,
        d if d <= 30 => 0.85,
        d if d <= 90 => 0.65,
        _ => 0.45,
    }
}

/// Unified multi-source context retrieval across:
/// 1. Repo Project Brain (`brain/`)
/// 2. Personal Hub Wiki (`hub:wiki/`)
/// 3. Repo OKF Knowledge Catalog (`knowledge/` / `docs/`)
/// 4. Scoped & Global SQLite Memories (`memory_entries`)
/// 5. Progressive Skills (`~/.zenko/skills` + repo `.zenko/skills`)
///
/// Pure deterministic Rust (zero LLM calls, zero regex) with explicit precedence tags and
/// ready-to-call `load_hint` instructions for just-in-time progressive disclosure.
pub fn retrieve_ranked_context(
    store: &SqliteStore,
    snapshots: &SnapshotManager,
    workspace_dir: &Path,
    hub_dir: &Path,
    query: &str,
    limit: usize,
) -> ContextRetrievalReport {
    let trimmed = query.trim();
    let project_scope = project_scope_for_workspace(workspace_dir);
    let cap = match limit {
        0 => CONTEXT_RETRIEVAL_DEFAULT_LIMIT,
        n => n.clamp(1, CONTEXT_RETRIEVAL_MAX_LIMIT),
    };
    if trimmed.is_empty() {
        return ContextRetrievalReport {
            query: String::new(),
            workspace_scope: project_scope,
            total_candidates: 0,
            items: Vec::new(),
        };
    }

    let mut items: Vec<RankedContextItem> = Vec::new();
    let per_source_cap = cap.max(6);
    let q_terms: BTreeSet<String> = tokenize_terms(trimmed).into_iter().collect();

    // 1. Repo Project Brain (`brain/`)
    if has_project_brain(workspace_dir) {
        let hits = search_brain_pages_scoped(
            workspace_dir,
            hub_dir,
            BrainScope::Workspace,
            trimmed,
            true,
            per_source_cap,
        );
        let max_raw = hits.first().map(|h| h.score).unwrap_or(1.0).max(1.0);
        for h in hits {
            let doc_text = format!(
                "{} {} {} {}",
                h.summary.id,
                h.summary.title,
                h.summary.tags.join(" "),
                h.summary.truth_preview
            );
            let rel = blended_relevance(h.score / max_raw, &q_terms, &doc_text);
            let scope_match = 1.0;
            let auth = match h.summary.status {
                BrainPageStatus::Active => trust_authority(h.summary.trust_tier).max(0.95),
                BrainPageStatus::Draft => 0.72,
                BrainPageStatus::Archived => 0.45,
            };
            let fresh = freshness_factor(h.summary.freshness, Some(&h.summary.updated));
            let conf = match h.summary.has_reversals {
                true => 0.95,
                false => 0.88,
            };
            let score = compute_context_score(rel, scope_match, auth, fresh, conf);
            items.push(RankedContextItem {
                source: ContextSourceKind::ProjectBrain,
                precedence: ContextSourceKind::ProjectBrain.precedence(),
                id: h.summary.id.clone(),
                title: h.summary.title,
                summary: h.summary.truth_preview,
                load_hint: format!(
                    "invoke_tool(tool=\"{TOOL_PROJECT_BRAIN}\", args={{\"scope\":\"workspace\",\"action\":\"read_page\",\"id\":\"{}\"}})",
                    h.summary.id
                ),
                score,
                relevance: rel,
                scope_match,
                authority: auth,
                freshness: fresh,
                confidence: conf,
                tags: h.summary.tags,
            });
        }
    }

    // 2. Personal Hub Wiki (`hub:wiki/`)
    if has_hub_wiki(hub_dir) {
        let hits = search_brain_pages_scoped(
            workspace_dir,
            hub_dir,
            BrainScope::Hub,
            trimmed,
            false,
            per_source_cap,
        );
        let max_raw = hits.first().map(|h| h.score).unwrap_or(1.0).max(1.0);
        for h in hits {
            let doc_text = format!(
                "{} {} {} {}",
                h.summary.id,
                h.summary.title,
                h.summary.tags.join(" "),
                h.summary.truth_preview
            );
            let rel = blended_relevance(h.score / max_raw, &q_terms, &doc_text);
            let scope_match = 0.70;
            let auth = trust_authority(h.summary.trust_tier).max(0.82);
            let fresh = freshness_factor(h.summary.freshness, Some(&h.summary.updated));
            let conf = 0.85;
            let score = compute_context_score(rel, scope_match, auth, fresh, conf);
            items.push(RankedContextItem {
                source: ContextSourceKind::HubWiki,
                precedence: ContextSourceKind::HubWiki.precedence(),
                id: h.summary.id.clone(),
                title: h.summary.title,
                summary: h.summary.truth_preview,
                load_hint: format!(
                    "invoke_tool(tool=\"{TOOL_PROJECT_BRAIN}\", args={{\"scope\":\"hub\",\"action\":\"read_page\",\"id\":\"{}\"}})",
                    h.summary.id
                ),
                score,
                relevance: rel,
                scope_match,
                authority: auth,
                freshness: fresh,
                confidence: conf,
                tags: h.summary.tags,
            });
        }
    }

    // 3. Repo OKF Knowledge Catalog
    let okf_hits = search_okf_catalog(workspace_dir, trimmed, None, false, per_source_cap);
    let okf_max = okf_hits.first().map(|h| h.score).unwrap_or(1.0).max(0.001);
    for h in okf_hits {
        let doc_text = format!("{} {} {}", h.summary.id, h.summary.title, h.summary.preview);
        let rel = blended_relevance(h.score / okf_max, &q_terms, &doc_text);
        let scope_match = 0.95;
        let auth = trust_authority(h.summary.trust_tier);
        let fresh = freshness_factor(h.summary.freshness, None);
        let conf = match h.summary.source_count > 0 {
            true => 0.92,
            false => 0.78,
        };
        let score = compute_context_score(rel, scope_match, auth, fresh, conf);
        items.push(RankedContextItem {
            source: ContextSourceKind::OkfCatalog,
            precedence: ContextSourceKind::OkfCatalog.precedence(),
            id: h.summary.id.clone(),
            title: h.summary.title,
            summary: h.summary.preview,
            load_hint: format!(
                "invoke_tool(tool=\"{TOOL_KNOWLEDGE_CATALOG}\", args={{\"action\":\"read\",\"id\":\"{}\"}})",
                h.summary.id
            ),
            score,
            relevance: rel,
            scope_match,
            authority: auth,
            freshness: fresh,
            confidence: conf,
            tags: vec![h.summary.doc_type],
        });
    }

    // 4. Scoped & Global SQLite Memories
    if let Ok(mem_hits) = store.search_memory_hybrid(
        trimmed,
        Some(&project_scope),
        true,
        Some(workspace_dir),
        per_source_cap,
    ) {
        let now = Utc::now();
        let max_mem = mem_hits.first().map(|h| h.score).unwrap_or(1.0).max(0.001);
        for h in mem_hits {
            let is_project = h.entry.scope == project_scope;
            let source = match is_project {
                true => ContextSourceKind::ProjectMemory,
                false => ContextSourceKind::GlobalMemory,
            };
            let doc_text = format!("{} {}", h.entry.title, h.entry.content);
            let rel = blended_relevance(h.score / max_mem, &q_terms, &doc_text);
            let scope_match = match is_project {
                true => 1.0,
                false => 0.55,
            };
            let auth = match is_project {
                true => 0.78,
                false => 0.62,
            };
            let age_days = h
                .entry
                .updated_at
                .or(h.entry.last_accessed_at)
                .map(|ts| (now - ts).num_days().max(0))
                .unwrap_or(0);
            let fresh = memory_age_freshness(age_days);
            let conf = (h.entry.salience / 2.0).clamp(0.4, 1.0);
            let score = compute_context_score(rel, scope_match, auth, fresh, conf);
            items.push(RankedContextItem {
                source,
                precedence: source.precedence(),
                id: h.entry.id.clone(),
                title: h.entry.title.clone(),
                summary: compact_preview(&h.entry.content, 120),
                load_hint: format!(
                    "invoke_tool(tool=\"{TOOL_MEMORY}\", args={{\"action\":\"search\",\"query\":\"{}\"}})",
                    compact_preview(&h.entry.title, 48).replace('"', "'")
                ),
                score,
                relevance: rel,
                scope_match,
                authority: auth,
                freshness: fresh,
                confidence: conf,
                tags: vec![h.entry.kind.to_string(), h.entry.scope],
            });
        }
    }

    // 5. Progressive Skills (token overlap across slug, name, description, triggers)
    if !q_terms.is_empty()
        && let Ok(skills) = snapshots.list_skills()
    {
        for sk in skills
            .into_iter()
            .filter(|s| s.enabled && s.status == SkillStatus::Active)
        {
            let text = format!(
                "{} {} {} {}",
                sk.slug,
                sk.name,
                sk.description,
                sk.triggers.join(" ")
            );
            let s_terms: BTreeSet<String> = tokenize_terms(&text).into_iter().collect();
            let overlap = q_terms.intersection(&s_terms).count();
            if overlap == 0 {
                continue;
            }
            let rel = ((overlap as f64) / (q_terms.len() as f64)).clamp(0.15, 1.0);
            let scope_match = 0.80;
            let auth = 0.72;
            let fresh = 0.85;
            let conf = 0.85;
            let score = compute_context_score(rel, scope_match, auth, fresh, conf);
            items.push(RankedContextItem {
                source: ContextSourceKind::Skill,
                precedence: ContextSourceKind::Skill.precedence(),
                id: sk.slug.clone(),
                title: sk.name,
                summary: compact_preview(&sk.description, 120),
                load_hint: format!("{TOOL_SKILL_VIEW}(slug=\"{}\")", sk.slug),
                score,
                relevance: rel,
                scope_match,
                authority: auth,
                freshness: fresh,
                confidence: conf,
                tags: sk.triggers,
            });
        }
    }

    let total_candidates = items.len();
    items.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.precedence.rank().cmp(&b.precedence.rank()))
            .then_with(|| a.id.cmp(&b.id))
    });
    items.truncate(cap);

    ContextRetrievalReport {
        query: trimmed.to_string(),
        workspace_scope: project_scope,
        total_candidates,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenko_core::{BrainPageCategory, MemoryKind};
    use zenko_memory::brain::{create_brain_page_scoped, ingest_raw_source, lint_brain};

    #[test]
    fn tail_context_builder_enforces_precedence_order_and_never_drops_mandatory_blocks() {
        let mut builder = TailContextBuilder::new();
        builder.push(
            InstructionPrecedence::RuntimeSignals,
            "mailbox",
            false,
            "optional low priority chatter ".repeat(300),
        );
        builder.push(
            InstructionPrecedence::ActiveTaskState,
            "task_dag",
            true,
            "<active_task_dag>Keep step 1 done</active_task_dag>".into(),
        );
        builder.push(
            InstructionPrecedence::PlatformSafety,
            "plan_mode",
            true,
            "<plan_mode>STRICT READ-ONLY</plan_mode>".into(),
        );

        let assembled = builder.build(120, 4.0);
        assert_eq!(assembled.included_kinds, vec!["plan_mode", "task_dag"]);
        assert_eq!(assembled.dropped_kinds, vec!["mailbox"]);
        let idx_plan = assembled
            .rendered
            .find("<plan_mode>")
            .expect("plan_mode present");
        let idx_dag = assembled
            .rendered
            .find("<active_task_dag>")
            .expect("task_dag present");
        assert!(
            idx_plan < idx_dag,
            "PlatformSafety must always precede ActiveTaskState"
        );
    }

    #[test]
    fn unified_context_retrieval_ranks_project_brain_hub_wiki_and_scoped_memory() {
        let base = std::env::temp_dir().join(format!("zenko_ctx_eng_{}", zenko_core::new_id()));
        let ws = base.join("ws");
        let hub = base.join("hub");
        let data = base.join("data");
        std::fs::create_dir_all(&ws).expect("ws");
        std::fs::create_dir_all(&hub).expect("hub");
        std::fs::create_dir_all(&data).expect("data");

        // Workspace Project Brain page
        let _ = create_brain_page_scoped(
            &ws,
            &hub,
            BrainScope::Workspace,
            "postgres-migrations",
            "PostgreSQL Migration Rules",
            BrainPageCategory::Decision,
            BrainPageStatus::Active,
            &["postgres".into(), "database".into()],
            "All PostgreSQL schema migrations use transactional SQL files.",
            "Adopted transactional migrations",
            "migrations/001.sql",
            &["auth-schema".into()],
            &[],
        )
        .expect("workspace brain");

        // Personal Hub Wiki raw drop + compiled page
        let raw_rel = ingest_raw_source(
            &ws,
            &hub,
            BrainScope::Hub,
            "karpathy-wiki-notes",
            "Karpathy LLM Wiki Notes",
            "https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f",
            "Compile raw sources into interlinked wiki pages with index.md and log.md.",
            &[],
        )
        .expect("ingest raw");
        let _ = create_brain_page_scoped(
            &ws,
            &hub,
            BrainScope::Hub,
            "llm-wiki-pattern",
            "LLM Wiki Architecture",
            BrainPageCategory::Concept,
            BrainPageStatus::Active,
            &["wiki".into(), "postgres".into()],
            "Use raw/, pages/, index.md, log.md, and lint for compounding knowledge.",
            "Compiled from Karpathy gist",
            &raw_rel,
            &[],
            &[],
        )
        .expect("hub wiki");

        // Lint confirms compiled raw source is recognized & missing workspace file is caught
        let hub_lint = lint_brain(&ws, &hub, BrainScope::Hub);
        assert_eq!(hub_lint.raw_sources_count, 1);
        assert!(hub_lint.uncompiled_raw_sources.is_empty());
        let ws_lint = lint_brain(&ws, &hub, BrainScope::Workspace);
        assert!(
            ws_lint
                .broken_links
                .iter()
                .any(|e| e.contains("auth-schema")),
            "lint must flag dangling affects target: {:?}",
            ws_lint.broken_links
        );
        assert!(
            ws_lint
                .missing_source_files
                .iter()
                .any(|f| f == "migrations/001.sql"),
            "lint must flag missing workspace source file"
        );

        let store = SqliteStore::open(&data.join("zenko.db")).expect("db");
        let proj_scope = project_scope_for_workspace(&ws);
        let _ = store.add_memory(
            &proj_scope,
            MemoryKind::Project.as_str(),
            "PostgreSQL Connection Pool",
            "Bounded pool size 16 for PostgreSQL queries.",
            1.4,
        );
        let snapshots = SnapshotManager::new(&data, &ws).with_hub_dir(&hub);

        let report = retrieve_ranked_context(&store, &snapshots, &ws, &hub, "PostgreSQL migrations", 8);
        assert!(report.total_candidates >= 2);
        assert_eq!(
            report.items[0].source,
            ContextSourceKind::ProjectBrain,
            "active repo Project Brain compiled truth must outrank hub/global items: {:?}",
            report.items
        );
        assert_eq!(report.items[0].precedence, InstructionPrecedence::CompiledTruth);
        assert!(report.items[0].load_hint.contains("read_page"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
