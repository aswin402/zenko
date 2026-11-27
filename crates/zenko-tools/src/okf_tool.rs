//! Deferred `knowledge_catalog` Tool — Open Knowledge Format (OKF v0.2/v0.3) Graph, Trust & Freshness Engine
//! (interoperable with `GoogleCloudPlatform/knowledge-catalog/okf` & `okf.md`; zero regex, zero hardcoding).

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zenko_core::{
    OKF_ACTIONS, OKF_DEFAULT_SEARCH_LIMIT, OKF_MAX_GRAPH_HOPS, OKF_MAX_SEARCH_LIMIT,
    OKF_STATUS_VALUES, OkfAction, OkfDocStatus, Result, RiskLevel, SecretVault,
    TOOL_KNOWLEDGE_CATALOG, Tool, ToolConcurrency, ToolContext, ToolExposure, ToolOutput, ToolSet,
    ToolSpec, ZenkoError, is_okf_tool_args_mutating,
};
use zenko_memory::okf::{
    OkfUpsertRequest, audit_okf_freshness, compute_okf_graph, read_okf_concept, scan_okf_catalog,
    search_okf_catalog, summarize_okf_concept, upsert_okf_concept,
};

pub struct KnowledgeCatalogTool {
    vault: Option<Arc<SecretVault>>,
}

impl KnowledgeCatalogTool {
    pub fn new() -> Self {
        Self { vault: None }
    }

    pub fn new_with_vault(vault: Arc<SecretVault>) -> Self {
        Self { vault: Some(vault) }
    }

    fn known_secrets(&self) -> Vec<String> {
        self.vault
            .as_ref()
            .map(|v| v.all_values())
            .unwrap_or_default()
    }
}

impl Default for KnowledgeCatalogTool {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_string_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn ok_meta(content: String, meta: Value) -> ToolOutput {
    ToolOutput {
        content,
        is_error: false,
        diff: None,
        metadata: Some(meta),
    }
}

#[async_trait]
impl Tool for KnowledgeCatalogTool {
    fn toolset(&self) -> ToolSet {
        ToolSet::Memory
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Deferred
    }

    fn concurrency_for_args(&self, args: &Value) -> ToolConcurrency {
        match is_okf_tool_args_mutating(args) {
            false => ToolConcurrency::Shared,
            true => ToolConcurrency::Global,
        }
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_KNOWLEDGE_CATALOG.into(),
            description: "Query, traverse, audit, and author Git-native Open Knowledge Format (OKF v0.2/v0.3) Markdown bundles (`knowledge/`, `okf/`, `docs/knowledge/`, `catalog/`): 4-bit TurboQuant + lexical hybrid search weighted by verification trust & calendar freshness (`stale_after`, `verified_by`, `verified_at`, live `sources:` drift checks) and zero-DB Markdown link-graph BFS.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": OKF_ACTIONS,
                        "description": "Read-only (`list`, `search`, `read`, `graph`, `audit_freshness`) or atomic write (`upsert`)."
                    },
                    "id": { "type": "string", "description": "Concept ID (e.g. `knowledge/auth` or stem `auth`) for `read`, `graph`, or `upsert`." },
                    "query": { "type": "string", "description": "Search query for `search`." },
                    "type": { "type": "string", "description": "OKF concept `type` (required for `upsert`, optional filter for `list`/`search`)." },
                    "title": { "type": "string", "description": "Concept title (for `upsert`)." },
                    "summary": { "type": "string", "description": "Short 1-line summary (for `upsert`)." },
                    "status": { "type": "string", "enum": OKF_STATUS_VALUES, "description": "Lifecycle status (default: `active`)." },
                    "owner": { "type": "string", "description": "Owning team/actor (for `upsert`)." },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "verified_by": { "type": "string", "description": "Verification actor/pipeline (for `upsert`)." },
                    "verified_at": { "type": "string", "description": "RFC 3339 or YYYY-MM-DD verification timestamp (for `upsert`)." },
                    "stale_after": { "type": "string", "description": "RFC 3339 or YYYY-MM-DD expiry timestamp (for `upsert`)." },
                    "sources": { "type": "array", "items": { "type": "string" }, "description": "Workspace paths or upstream URIs this concept depends on (for drift tracking)." },
                    "body": { "type": "string", "description": "Markdown body with standard `[Title](other.md)` links (for `upsert`)." },
                    "include_deprecated": { "type": "boolean", "description": "Include deprecated/superseded concepts in `search` (default: false)." },
                    "max_hops": { "type": "integer", "description": "BFS traversal depth for `graph` (1..3, default: 2)." },
                    "limit": { "type": "integer", "description": "Max results for `search`/`list` (default: 8, cap: 32)." }
                },
                "required": ["action"]
            }),
            risk: RiskLevel::Safe,
        }
    }

    async fn execute(
        &self,
        ctx: &ToolContext,
        args: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput> {
        let raw_action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("list");
        let Some(action) = OkfAction::parse(raw_action) else {
            return Err(ZenkoError::Tool(format!(
                "invalid `knowledge_catalog` action `{raw_action}` (expected one of: {})",
                OKF_ACTIONS.join(", ")
            )));
        };

        let ws = ctx.workspace_dir.clone();
        match action {
            OkfAction::List => {
                let type_filter = args
                    .get("type")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty());
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|v| (v as usize).clamp(1, OKF_MAX_SEARCH_LIMIT))
                    .unwrap_or(OKF_MAX_SEARCH_LIMIT);

                let catalog = tokio::task::spawn_blocking(move || scan_okf_catalog(&ws))
                    .await
                    .map_err(|e| ZenkoError::Tool(format!("okf scan join failed: {e}")))?;

                let summaries: Vec<_> = catalog
                    .iter()
                    .filter(|c| {
                        type_filter
                            .as_deref()
                            .is_none_or(|t| c.doc_type.to_ascii_lowercase() == t)
                    })
                    .take(limit)
                    .map(summarize_okf_concept)
                    .collect();

                let body = match summaries.is_empty() {
                    true => "No OKF concepts found in workspace (`knowledge/`, `okf/`, `docs/knowledge/`, `catalog/`).".into(),
                    false => summaries
                        .iter()
                        .map(|s| {
                            format!(
                                "- `{}` [{}][{}][{}] {} ({} links, {} sources) — {}",
                                s.id,
                                s.doc_type,
                                s.trust_tier.badge(),
                                s.freshness.as_str(),
                                s.title,
                                s.link_count,
                                s.source_count,
                                s.preview
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                Ok(ok_meta(
                    format!("OKF Knowledge Catalog ({} concepts):\n{body}", summaries.len()),
                    json!({ "action": "list", "count": summaries.len(), "concepts": summaries }),
                ))
            }
            OkfAction::Search => {
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let type_filter = args.get("type").and_then(Value::as_str).map(String::from);
                let include_deprecated = args
                    .get("include_deprecated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|v| (v as usize).clamp(1, OKF_MAX_SEARCH_LIMIT))
                    .unwrap_or(OKF_DEFAULT_SEARCH_LIMIT);

                let hits = tokio::task::spawn_blocking(move || {
                    search_okf_catalog(
                        &ws,
                        &query,
                        type_filter.as_deref(),
                        include_deprecated,
                        limit,
                    )
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("okf search join failed: {e}")))?;

                let body = match hits.is_empty() {
                    true => "No matching OKF concepts found.".into(),
                    false => hits
                        .iter()
                        .map(|h| {
                            format!(
                                "- `{}` [{}][{}][{}] score={:.4} cos={:.2} — {}: {}",
                                h.summary.id,
                                h.summary.doc_type,
                                h.summary.trust_tier.badge(),
                                h.summary.freshness.as_str(),
                                h.score,
                                h.vector_cosine,
                                h.summary.title,
                                h.summary.preview
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                Ok(ok_meta(
                    format!("OKF Search ({} hits):\n{body}", hits.len()),
                    json!({ "action": "search", "count": hits.len(), "hits": hits }),
                ))
            }
            OkfAction::Read => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("`knowledge_catalog` read requires `id`".into())
                    })?
                    .to_string();
                let concept = tokio::task::spawn_blocking(move || read_okf_concept(&ws, &id))
                    .await
                    .map_err(|e| ZenkoError::Tool(format!("okf read join failed: {e}")))?
                    .ok_or_else(|| ZenkoError::NotFound("OKF concept not found".into()))?;

                let mut warnings = Vec::new();
                if !concept.missing_sources.is_empty() {
                    warnings.push(format!(
                        "missing_sources: {}",
                        concept.missing_sources.join(", ")
                    ));
                }
                if !concept.drifted_sources.is_empty() {
                    warnings.push(format!(
                        "drifted_sources (modified after verified_at): {}",
                        concept.drifted_sources.join(", ")
                    ));
                }
                let warn_line = match warnings.is_empty() {
                    true => String::new(),
                    false => format!("\nWARNINGS: {}", warnings.join(" | ")),
                };
                let links_line = match concept.outbound_links.is_empty() {
                    true => String::new(),
                    false => format!("\nLinks: {}", concept.outbound_links.join(", ")),
                };
                let text = format!(
                    "# {} (`{}`)\nType: {} · Status: {} · Trust: {} · Freshness: {}{warn_line}{links_line}\n\n{}",
                    concept.title,
                    concept.rel_path,
                    concept.doc_type,
                    concept.status.as_str(),
                    concept.trust_tier.badge(),
                    concept.freshness.as_str(),
                    concept.body
                );
                Ok(ok_meta(
                    text,
                    json!({ "action": "read", "concept": concept }),
                ))
            }
            OkfAction::Graph => {
                let id = args
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("`knowledge_catalog` graph requires `id`".into())
                    })?
                    .to_string();
                let max_hops = args
                    .get("max_hops")
                    .and_then(Value::as_u64)
                    .map(|v| (v as usize).clamp(1, OKF_MAX_GRAPH_HOPS))
                    .unwrap_or(2);
                let graph =
                    tokio::task::spawn_blocking(move || compute_okf_graph(&ws, &id, max_hops))
                        .await
                        .map_err(|e| ZenkoError::Tool(format!("okf graph join failed: {e}")))?
                        .ok_or_else(|| ZenkoError::NotFound("OKF concept not found".into()))?;

                let text = format!(
                    "OKF Graph for `{}` (hops <= {max_hops}):\n\
                     - direct_outbound ({}): {}\n\
                     - transitive_outbound ({}): {}\n\
                     - backlinks ({}): {}\n\
                     - broken_links ({}): {}",
                    graph.concept_id,
                    graph.direct_outbound.len(),
                    graph.direct_outbound.join(", "),
                    graph.transitive_outbound.len(),
                    graph.transitive_outbound.join(", "),
                    graph.backlinks.len(),
                    graph.backlinks.join(", "),
                    graph.broken_links.len(),
                    graph.broken_links.join(", "),
                );
                Ok(ok_meta(text, json!({ "action": "graph", "graph": graph })))
            }
            OkfAction::AuditFreshness => {
                let report = tokio::task::spawn_blocking(move || audit_okf_freshness(&ws))
                    .await
                    .map_err(|e| ZenkoError::Tool(format!("okf audit join failed: {e}")))?;

                let details = match report.issues.is_empty() {
                    true => "All OKF concepts are fresh, verified, and link-consistent.".into(),
                    false => report
                        .issues
                        .iter()
                        .take(24)
                        .map(|i| format!("- [{}] `{}`: {}", i.kind.as_str(), i.concept_id, i.detail))
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                let text = format!(
                    "OKF Freshness & Trust Audit: {} concepts ({} verified, {} stale, {} source_drifted, {} broken links)\n{details}",
                    report.total_concepts,
                    report.verified_count,
                    report.stale_count,
                    report.drifted_count,
                    report.broken_links_count
                );
                Ok(ok_meta(
                    text,
                    json!({ "action": "audit_freshness", "report": report }),
                ))
            }
            OkfAction::Upsert => {
                let slug = args
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("`knowledge_catalog` upsert requires `id`".into())
                    })?
                    .to_string();
                let doc_type = args
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        ZenkoError::Tool("`knowledge_catalog` upsert requires `type`".into())
                    })?
                    .to_string();
                let title = args
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let summary = args
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let status = args
                    .get("status")
                    .and_then(Value::as_str)
                    .and_then(OkfDocStatus::parse)
                    .unwrap_or(OkfDocStatus::Active);
                let owner = args.get("owner").and_then(Value::as_str).map(String::from);
                let tags = parse_string_list(&args, "tags");
                let verified_by = args
                    .get("verified_by")
                    .and_then(Value::as_str)
                    .map(String::from);
                let verified_at = args
                    .get("verified_at")
                    .and_then(Value::as_str)
                    .map(String::from);
                let stale_after = args
                    .get("stale_after")
                    .and_then(Value::as_str)
                    .map(String::from);
                let sources = parse_string_list(&args, "sources");
                let body = args
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let secrets = self.known_secrets();

                let concept = tokio::task::spawn_blocking(move || {
                    upsert_okf_concept(
                        &ws,
                        OkfUpsertRequest {
                            slug: &slug,
                            doc_type: &doc_type,
                            title: &title,
                            summary: &summary,
                            status,
                            owner: owner.as_deref(),
                            tags: &tags,
                            verified_by: verified_by.as_deref(),
                            verified_at: verified_at.as_deref(),
                            stale_after: stale_after.as_deref(),
                            sources: &sources,
                            body: &body,
                        },
                        &secrets,
                    )
                })
                .await
                .map_err(|e| ZenkoError::Tool(format!("okf upsert join failed: {e}")))??;

                Ok(ok_meta(
                    format!(
                        "Wrote OKF concept `{}` (`{}`) [{}][{}][{}].",
                        concept.id,
                        concept.rel_path,
                        concept.doc_type,
                        concept.trust_tier.badge(),
                        concept.freshness.as_str()
                    ),
                    json!({ "action": "upsert", "concept": concept }),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_ctx(ws: PathBuf) -> ToolContext {
        ToolContext {
            session_id: "test_sess".into(),
            workspace_dir: ws.clone(),
            data_dir: ws.join(".data"),
            hub_dir: ws.join(".hub"),
            shell_timeout_secs: 10,
            auto_background_after_secs: 30,
            max_output_bytes: 65_536,
            headless_no_wait: true,
            demote_bg: Arc::new(tokio::sync::Notify::new()),
            file_state: None,
        }
    }

    #[tokio::test]
    async fn okf_tool_roundtrips_upsert_search_graph_audit_and_plan_guard() {
        assert!(!is_okf_tool_args_mutating(&json!({ "action": "search", "query": "auth" })));
        assert!(!is_okf_tool_args_mutating(&json!({ "action": "audit_freshness" })));
        assert!(is_okf_tool_args_mutating(&json!({ "action": "upsert", "id": "auth" })));

        let base = std::env::temp_dir().join(format!("zenko_okf_tool_{}", zenko_core::new_id()));
        let _ = std::fs::create_dir_all(&base);
        let ctx = test_ctx(base.clone());
        let tool = KnowledgeCatalogTool::new();
        let cancel = CancellationToken::new();

        let up = tool
            .execute(
                &ctx,
                json!({
                    "action": "upsert",
                    "id": "rate-limits",
                    "type": "Runbook",
                    "title": "Rate Limit Runbook",
                    "summary": "Handling 429 bursts with backoff",
                    "verified_by": "sre",
                    "verified_at": "2099-01-01",
                    "stale_after": "2099-12-31",
                    "body": "Redacts sk-secret-12345678901234567890 and links [Auth](auth.md)."
                }),
                cancel.clone(),
            )
            .await
            .expect("upsert");
        assert!(!up.is_error);
        assert!(up.content.contains("[VERIFIED]"));

        let read = tool
            .execute(&ctx, json!({ "action": "read", "id": "rate-limits" }), cancel.clone())
            .await
            .expect("read");
        assert!(!read.content.contains("sk-secret-12345678901234567890"));
        assert!(read.content.contains("knowledge/auth"));

        let audit = tool
            .execute(&ctx, json!({ "action": "audit_freshness" }), cancel)
            .await
            .expect("audit");
        assert!(audit.content.contains("1 broken links"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
