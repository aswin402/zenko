//! Autonomous Event-Driven 2-Stage "Dream" Memory Consolidation Engine.
//!
//! Synthesizes:
//! - Codex (`codex-rs`) transactional time-bounded SQLite leases & watermarks (zero stale lockfiles)
//! - Claude Code 4-phase REM consolidation (signal gathering, ISO date normalization, contradiction resolution)
//! - Prime Agent diff-auditable pre-edit snapshots (`~/.zenko/memories/.history/`)
//!
//! Runs automatically via `tokio::sync::Notify` (zero busy-polling, zero `/dream` command required),
//! works dynamically across all models (`dream_model -> compaction_model -> fast_model -> lead_model`),
//! and executes Stage-0 zero-token hygiene even when offline.

use crate::runtime::AgentRuntime;
use chrono::{DateTime, Duration, Utc};
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use zenko_core::{
    AgentEvent, ChatMessage, ChatRequest, ConciseMode, DREAM_AUTO_COMPRESS_DOCS,
    DREAM_AUTO_COMPRESS_MIN_SAVED_BYTES, DREAM_AUTO_COMPRESS_THRESHOLD_DEN,
    DREAM_AUTO_COMPRESS_THRESHOLD_NUM, DREAM_KV_LAST_RUN_KEY, DREAM_KV_WATERMARK_KEY,
    DREAM_LEASE_SECS, DREAM_MAX_ORIENTATION_MEMORIES, DREAM_MAX_OUTPUT_TOKENS,
    DREAM_MAX_PROPOSED_SKILLS, DREAM_MAX_SESSIONS_PER_RUN, DREAM_MAX_TRANSCRIPT_CHARS,
    DREAM_MIN_SESSION_EVENTS, DreamRunRecord, DreamStatus, EventKind, EventRole, ExecutionTier,
    LlmDelta, LlmProvider, LogChannel, LogDirection, LogStatusTone, MEMORY_SCOPE_GLOBAL,
    MemoryKind, NOTE_KIND_BRAINSTORM_SPEC, NOTE_KIND_CLARIFICATION_DECISION,
    NOTE_KIND_EXECUTION_PLAN, ORCH_STAGE_DREAM, PROJECT_MEMORY_DOC_NAME, Result, Session,
    SessionEvent, SessionRolloutSummary, SkillAuthor, SkillStatus, UsageStats, compact_preview,
    format_hono_entry, new_id, scrub_secrets,
};
use zenko_memory::{
    SnapshotManager, estimate_tokens, find_missing_workspace_paths, project_scope_for_workspace,
};

#[derive(Debug, Deserialize)]
struct DreamExtractionPayload {
    #[serde(default)]
    memories: Vec<ExtractedMemory>,
    #[serde(default)]
    archive_ids: Vec<String>,
    #[serde(default)]
    doc_patches: Vec<ExtractedDocPatch>,
    #[serde(default)]
    proposed_skills: Vec<ProposedSkill>,
}

#[derive(Debug, Deserialize)]
struct ProposedSkill {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    triggers: Vec<String>,
    #[serde(default)]
    allowed_tools: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractedMemory {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
    #[serde(default = "default_salience")]
    salience: f64,
    #[serde(default)]
    supersedes_query: Option<String>,
    #[serde(default)]
    source_session_id: Option<String>,
}

fn default_salience() -> f64 {
    1.0
}

#[derive(Debug, Deserialize)]
struct ExtractedDocPatch {
    #[serde(default)]
    doc: String,
    #[serde(default, alias = "op")]
    action: String,
    #[serde(default)]
    old_text: Option<String>,
    #[serde(default)]
    new_text: String,
    #[serde(default)]
    source_session_id: Option<String>,
}

/// Returns live Dream consolidation telemetry and pending session count.
pub fn dream_status(runtime: &AgentRuntime) -> DreamStatus {
    let cfg = runtime.config();
    let watermark_ts = runtime.store.get_kv(DREAM_KV_WATERMARK_KEY).ok().flatten();
    let last_run_ts = runtime.store.get_kv(DREAM_KV_LAST_RUN_KEY).ok().flatten();
    let pending_sessions = runtime
        .store
        .list_sessions_for_dream(watermark_ts.as_deref(), DREAM_MIN_SESSION_EVENTS, 50)
        .map(|v| v.len())
        .unwrap_or(0);
    let recent_runs = runtime.store.list_dream_runs(10).unwrap_or_default();

    DreamStatus {
        enabled: cfg.memory.dream_enabled,
        running: runtime.store.is_dream_lease_active(),
        watermark_ts,
        last_run_ts,
        pending_sessions,
        recent_runs,
    }
}

/// Event-driven zero-polling daemon background loop woken when sessions finish meaningful work.
pub async fn run_dream_worker(runtime: Arc<AgentRuntime>, cancel: CancellationToken) {
    info!("autonomous dream memory consolidation worker started (zero-polling)");
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = runtime.dream_notify.notified() => {}
        }

        // Brief settle window to coalesce burst turn completions before checking gates
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
        }

        if let Err(e) = run_dream_cycle(&runtime, "auto", false, cancel.clone()).await {
            warn!(error = %e, "autonomous dream consolidation cycle failed");
        }
    }
}

/// Executes a full 2-stage Dream consolidation pass (Stage-0 zero-token hygiene + Stage-1 LLM
/// extraction + Stage-2 contradiction archival, scoped dedup upsert & reversible doc patching).
pub async fn run_dream_cycle(
    runtime: &Arc<AgentRuntime>,
    trigger: &str,
    force: bool,
    cancel: CancellationToken,
) -> Result<DreamRunRecord> {
    let cfg = runtime.config();
    let now = Utc::now();

    if !force {
        if !cfg.memory.dream_enabled {
            return Ok(skipped_record(trigger, "dream consolidation disabled in config"));
        }
        if runtime.active_sessions_count() > 0 {
            return Ok(skipped_record(trigger, "deferred while active session turn is running"));
        }
        if let Ok(Some(last_raw)) = runtime.store.get_kv(DREAM_KV_LAST_RUN_KEY)
            && let Ok(last_dt) = DateTime::parse_from_rfc3339(&last_raw) {
                let min_gap = Duration::hours(cfg.memory.dream_cooldown_hours.max(1) as i64);
                if now - last_dt.with_timezone(&Utc) < min_gap {
                    return Ok(skipped_record(trigger, "cooldown window active"));
                }
            }
        if cfg.general.daily_spend_limit_usd > 0.0 {
            let today = runtime.store.daily_cost_usd().unwrap_or(0.0);
            if today >= cfg.general.daily_spend_limit_usd * 0.90 {
                return Ok(skipped_record(trigger, "daily spend budget headroom low"));
            }
        }
    }

    let watermark = runtime.store.get_kv(DREAM_KV_WATERMARK_KEY).ok().flatten();
    let candidate_sessions = runtime.store.list_sessions_for_dream(
        watermark.as_deref(),
        DREAM_MIN_SESSION_EVENTS,
        DREAM_MAX_SESSIONS_PER_RUN,
    )?;

    if !force && candidate_sessions.len() < cfg.memory.dream_min_sessions.max(1) {
        return Ok(skipped_record(
            trigger,
            &format!(
                "waiting for {} eligible sessions (currently {})",
                cfg.memory.dream_min_sessions.max(1),
                candidate_sessions.len()
            ),
        ));
    }

    let owner_token = format!("dream_{}", new_id());
    if !runtime
        .store
        .try_claim_dream_lease(&owner_token, DREAM_LEASE_SECS)?
    {
        return Ok(skipped_record(trigger, "another dream worker holds active SQLite lease"));
    }

    let mut rec = DreamRunRecord {
        id: new_id(),
        trigger: trigger.to_string(),
        status: "running".into(),
        model: None,
        sessions_scanned: candidate_sessions.len(),
        memories_added: 0,
        memories_merged: 0,
        memories_archived: 0,
        docs_patched: 0,
        skills_proposed: 0,
        diff_preview: None,
        detail: None,
        started_at: now,
        finished_at: None,
    };
    let _ = runtime.store.insert_dream_run(&rec);

    let outcome = execute_dream_stages(runtime, &cfg, &candidate_sessions, &mut rec, cancel).await;
    let _ = runtime.store.release_dream_lease(&owner_token);

    rec.finished_at = Some(Utc::now());
    match outcome {
        Ok(processed_watermark) => {
            if rec.status == "running" {
                rec.status = "completed".into();
            }
            // Advance cooldown & watermark STRICTLY when Stage-1/2 completed cleanly
            // (zero silent cooldown burn or data loss on transient LLM blips!).
            if rec.status == "completed" {
                let _ = runtime
                    .store
                    .set_kv(DREAM_KV_LAST_RUN_KEY, &Utc::now().to_rfc3339());
                if let Some(latest_ts) = processed_watermark {
                    let _ = runtime.store.set_kv(DREAM_KV_WATERMARK_KEY, &latest_ts);
                }
            }
            let _ = runtime.store.finish_dream_run(&rec);

            let detail = format!(
                "scanned={} added={} merged={} archived={} docs_patched={} skills_proposed={}",
                rec.sessions_scanned,
                rec.memories_added,
                rec.memories_merged,
                rec.memories_archived,
                rec.docs_patched,
                rec.skills_proposed
            );
            if rec.memories_added
                + rec.memories_merged
                + rec.memories_archived
                + rec.docs_patched
                + rec.skills_proposed
                > 0
            {
                runtime.broadcast_memory_delta(format!(
                    "Background Dream consolidation updated persistent memory ({detail})."
                ));
            }
            let log_entry = format_hono_entry(
                LogDirection::Work,
                LogChannel::System,
                &format!("dream ({trigger}) · {detail}"),
                Some((&rec.status, LogStatusTone::Ok)),
                rec.model.as_deref(),
            );
            runtime.logs.push(log_entry);
            runtime.emit(AgentEvent::OrchestratorUpdated {
                session_id: "global".into(),
                stage: ORCH_STAGE_DREAM.into(),
                tier: ExecutionTier::Standard,
                detail,
                model: rec.model.clone(),
            });
            Ok(rec)
        }
        Err(err) => {
            rec.status = "failed".into();
            rec.detail = Some(err.to_string());
            let _ = runtime.store.finish_dream_run(&rec);
            Err(err)
        }
    }
}

/// Zero-token Stage-0/post-patch memory doc hygiene: automatically compresses any persistent
/// memory document (`USER.md`, `MEMORY.md`, `PROJECT_MEMORY.md`) once it reaches 65% of its token
/// budget, writing an atomic `.history/` rollback backup first and validating headings/code/paths.
pub(crate) fn auto_compress_bloated_memory_docs(
    mgr: &SnapshotManager,
    configured_mode: ConciseMode,
) -> (usize, usize) {
    let effective_mode = match configured_mode {
        ConciseMode::Off => ConciseMode::Lite,
        other => other,
    };
    let mut docs_compressed = 0usize;
    let mut total_bytes_saved = 0usize;
    for &(doc_name, max_tokens) in DREAM_AUTO_COMPRESS_DOCS {
        let Ok(current) = mgr.read_doc(doc_name) else {
            continue;
        };
        if current.trim().is_empty() {
            continue;
        }
        let est_tokens = estimate_tokens(&current);
        if est_tokens.saturating_mul(DREAM_AUTO_COMPRESS_THRESHOLD_DEN)
            < max_tokens.saturating_mul(DREAM_AUTO_COMPRESS_THRESHOLD_NUM)
        {
            continue;
        }
        let Ok(preview) = mgr.compress_doc(doc_name, effective_mode, true) else {
            continue;
        };
        if preview.bytes_saved < DREAM_AUTO_COMPRESS_MIN_SAVED_BYTES {
            continue;
        }
        if let Ok(applied) = mgr.compress_doc(doc_name, effective_mode, false)
            && applied.bytes_saved >= DREAM_AUTO_COMPRESS_MIN_SAVED_BYTES
        {
            docs_compressed = docs_compressed.saturating_add(1);
            total_bytes_saved = total_bytes_saved.saturating_add(applied.bytes_saved);
        }
    }
    (docs_compressed, total_bytes_saved)
}

async fn execute_dream_stages(
    runtime: &Arc<AgentRuntime>,
    cfg: &zenko_core::ZenkoConfig,
    sessions: &[Session],
    rec: &mut DreamRunRecord,
    cancel: CancellationToken,
) -> Result<Option<String>> {
    // Stage 0: Zero-token deterministic hygiene (always runs even offline)
    let ws = cfg.resolved_workspace_dir();
    let hygiene = runtime.store.run_memory_hygiene(Some(&ws))?;
    rec.memories_merged += hygiene.duplicates_merged;
    rec.memories_archived += hygiene.entries_archived;

    let stage0_mgr = SnapshotManager::new(cfg.resolved_data_dir(), &ws)
        .with_hub_dir(cfg.resolved_hub_dir());
    let (stage0_compressed, stage0_bytes_saved) = match cfg.memory.dream_auto_patch_docs {
        true => auto_compress_bloated_memory_docs(&stage0_mgr, cfg.context.concise_mode),
        false => (0, 0),
    };
    rec.docs_patched += stage0_compressed;

    if sessions.is_empty() {
        rec.status = "hygiene_only".into();
        rec.detail = Some(format!(
            "Stage-0 hygiene complete (dedup={}, archived={}, stale_paths={}, docs_compressed={}, bytes_saved={})",
            hygiene.duplicates_merged,
            hygiene.entries_archived,
            hygiene.stale_flagged,
            stage0_compressed,
            stage0_bytes_saved,
        ));
        return Ok(None);
    }

    // Resolve dynamic model chain: dream_model -> compaction_model -> fast_model -> lead_model
    let dream_model = [
        cfg.provider.dream_model.as_str(),
        cfg.provider.compaction_model.as_str(),
        cfg.provider.fast_model.as_str(),
        cfg.provider.lead_model.as_str(),
    ]
    .into_iter()
    .map(str::trim)
    .find(|m| !m.is_empty())
    .map(String::from);

    let Some(model) = dream_model else {
        rec.status = "hygiene_only".into();
        rec.detail = Some("No model configured; completed Stage-0 deterministic hygiene".into());
        return Ok(None);
    };
    rec.model = Some(model.clone());

    let known_secrets = runtime.vault.all_values();

    // Stage 1: Build compact, secret-scrubbed multi-session transcript bundle with Codex rollout cache
    let mut session_scope_map: HashMap<String, String> = HashMap::with_capacity(sessions.len());
    let mut session_ws_map: HashMap<String, PathBuf> = HashMap::with_capacity(sessions.len());
    let mut processed_latest_ts: Option<DateTime<Utc>> = None;
    let mut processed_count = 0usize;
    let mut transcript = String::with_capacity(4096);
    for sess in sessions {
        let sess_ws = sess
            .workspace_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| ws.clone());
        let proj_scope = project_scope_for_workspace(&sess_ws);
        let sess_updated_iso = sess.updated_at.to_rfc3339();
        let digest = match runtime
            .store
            .get_rollout_summary(&sess.id, &sess_updated_iso)
            .ok()
            .flatten()
        {
            Some(cached) => cached.digest,
            None => {
                let events = runtime.store.get_active_branch(&sess.id).unwrap_or_default();
                let built = build_session_digest(sess, &events, &proj_scope, &known_secrets);
                let _ = runtime.store.upsert_rollout_summary(&SessionRolloutSummary {
                    session_id: sess.id.clone(),
                    scope: proj_scope.clone(),
                    digest: built.clone(),
                    session_updated_at: sess_updated_iso,
                    created_at: Utc::now(),
                });
                built
            }
        };
        if processed_count > 0 && transcript.len() + digest.len() + 32 > DREAM_MAX_TRANSCRIPT_CHARS
        {
            break;
        }
        session_scope_map.insert(sess.id.clone(), proj_scope);
        session_ws_map.insert(sess.id.clone(), sess_ws);
        processed_count += 1;
        processed_latest_ts = Some(match processed_latest_ts {
            Some(prev) => prev.max(sess.updated_at),
            None => sess.updated_at,
        });
        transcript.push_str(&digest);
        transcript.push('\n');
    }
    rec.sessions_scanned = processed_count;

    // Phase-1 Orientation (Claude Code AutoDream + Codex Stage-2): show existing scoped memories
    // with IDs and live `[stale-path]` warnings so Dream can prune/resolve contradictions accurately.
    let default_proj_scope = project_scope_for_workspace(&ws);
    let orientation_entries = runtime
        .store
        .list_memories_scoped(Some(&default_proj_scope), true, false, DREAM_MAX_ORIENTATION_MEMORIES)
        .unwrap_or_default();
    let orientation_block = orientation_entries
        .iter()
        .map(|m| {
            let stale = find_missing_workspace_paths(&ws, &m.content);
            let stale_tag = match stale.is_empty() {
                true => String::new(),
                false => format!(" [stale-path: {}]", stale.join(", ")),
            };
            format!(
                "- [id={}] [{}][{}] (sal={:.1}) {}: {}{}",
                m.id,
                m.scope,
                m.kind,
                m.salience,
                compact_preview(&m.title, 60),
                compact_preview(&m.content, 140),
                stale_tag
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let snapshot = runtime.snapshots.load_snapshot()?;
    let existing_skill_slugs: Vec<String> = snapshot
        .skills
        .iter()
        .map(|s| format!("`{}` ({})", s.slug, s.name))
        .collect();
    let today_iso = Utc::now().format("%Y-%m-%d").to_string();
    let sys_prompt = format!(
        "You are Zenko's autonomous memory consolidation engine (today={today_iso}).\n\
         Orient against `<existing_memories>`, `<existing_skills>`, `<current_user_md>`, `<current_memory_md>`, and `<current_project_memory_md>`, then analyze `<recent_sessions>`.\n\
         Extract ONLY durable, non-obvious, high-signal facts worth remembering across future sessions:\n\
         - `user`: stable developer habits, communication style, toolchain preferences (scope=`global`).\n\
         - `feedback`: explicit user corrections, banned patterns, testing standards (scope=`global` or `project`).\n\
         - `project`: architectural decisions, subtle invariants, bug root causes (scope=`project`).\n\
         - `reference`: external URLs, dashboards, deployment pointers.\n\
         STRICT RULES:\n\
         1. Never repeat facts already captured accurately in `<existing_memories>` or obvious repo layouts.\n\
         2. Never include secrets, tokens, or relative dates like 'yesterday' — convert dates to absolute ISO `{today_iso}`.\n\
         3. For contradicted or dead `[stale-path]` entries visible in `<existing_memories>`, list their exact IDs in `archive_ids` (and/or populate `supersedes_query`).\n\
         4. Only propose `doc_patches` for high-value curated updates: global habits -> `USER.md`, cross-repo notes -> `MEMORY.md`, repo-specific architecture -> `PROJECT_MEMORY.md`.\n\
         5. Only populate `proposed_skills` (max {DREAM_MAX_PROPOSED_SKILLS}) when a session resolved a genuinely reusable multi-step debugging/engineering workflow not already in `<existing_skills>` (saved safely as `draft` for human review).\n\
         Return STRICT JSON ONLY (no prose outside JSON):\n\
         {{\"memories\":[{{\"kind\":\"project\",\"scope\":\"project\",\"title\":\"...\",\"content\":\"...\",\"salience\":1.2,\"supersedes_query\":null,\"source_session_id\":\"...\"}}],\"archive_ids\":[],\"doc_patches\":[{{\"doc\":\"PROJECT_MEMORY.md\",\"action\":\"append\",\"old_text\":null,\"new_text\":\"...\"}}],\"proposed_skills\":[{{\"slug\":\"my-workflow\",\"name\":\"My Workflow\",\"description\":\"When to use this recipe\",\"content\":\"1. Step one...\",\"triggers\":[\"keyword\"],\"allowed_tools\":[\"read_file\"]}}]}}"
    );

    let user_prompt = format!(
        "<existing_memories>\n{}\n</existing_memories>\n\n\
         <existing_skills>\n{}\n</existing_skills>\n\n\
         <current_user_md>\n{}\n</current_user_md>\n\n\
         <current_memory_md>\n{}\n</current_memory_md>\n\n\
         <current_project_memory_md>\n{}\n</current_project_memory_md>\n\n\
         <recent_sessions>\n{}\n</recent_sessions>",
        match orientation_block.is_empty() {
            true => "(none)",
            false => orientation_block.as_str(),
        },
        match existing_skill_slugs.is_empty() {
            true => "(none)".to_string(),
            false => existing_skill_slugs.join(", "),
        },
        compact_preview(&snapshot.user_md, 1_500),
        compact_preview(&snapshot.memory_md, 2_000),
        compact_preview(&snapshot.project_memory_md, 2_000),
        transcript
    );

    let req = ChatRequest {
        session_id: format!("dream_{}", rec.id),
        model: model.clone(),
        fallbacks: Vec::new(),
        system_prompt: sys_prompt,
        messages: vec![ChatMessage {
            role: EventRole::User,
            content: user_prompt,
            reasoning_content: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }],
        tools: Vec::new(),
        temperature: 0.1,
        max_tokens: DREAM_MAX_OUTPUT_TOKENS,
        enable_streaming: cfg.provider.enable_streaming,
        enable_prompt_caching: false,
        sticky_session_routing: false,
        provider_sort: cfg.provider.provider_sort.clone(),
        reasoning_effort: None,
    };

    let llm_out = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let mut stream = runtime.provider.stream_chat(req, cancel.clone()).await?;
        let mut text = String::new();
        let mut usage = UsageStats::default();
        while let Some(item) = stream.next().await {
            match item? {
                LlmDelta::Text(t) => text.push_str(&t),
                LlmDelta::Usage(u) => usage = u,
                LlmDelta::Thinking(_) | LlmDelta::ToolCallComplete(_) | LlmDelta::Done { .. } => {}
            }
        }
        Ok::<(String, UsageStats), zenko_core::ZenkoError>((text, usage))
    })
    .await;

    let (raw_text, usage) = match llm_out {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            rec.status = "hygiene_only".into();
            rec.detail = Some(format!("Dream LLM skipped ({e}); Stage-0 hygiene persisted"));
            return Ok(None);
        }
        Err(_) => {
            rec.status = "hygiene_only".into();
            rec.detail = Some("Dream LLM timed out; Stage-0 hygiene persisted".into());
            return Ok(None);
        }
    };

    if usage.prompt_tokens > 0 || usage.completion_tokens > 0 || usage.cost_usd > 0.0 {
        let overhead_toks = usage.prompt_tokens.saturating_add(usage.completion_tokens) as i64;
        if overhead_toks > 0 && cfg.economy.enabled {
            let _ = runtime.store.record_economy_ledger_event(
                &default_proj_scope,
                zenko_core::EconomyLedgerKind::OverheadDream,
                overhead_toks,
                -usage.cost_usd.abs(),
                &format!("Dream cycle ({model})"),
            );
        }
        let _ = runtime
            .store
            .record_usage("dream", &cfg.provider.kind, &model, &usage);
    }

    let Some(parsed) = parse_dream_json(&raw_text) else {
        rec.status = "hygiene_only".into();
        rec.detail = Some("Dream model emitted non-JSON output; Stage-0 hygiene persisted".into());
        return Ok(None);
    };

    // Stage 2: Deterministic ID archival + contradiction resolution + scoped upsert + doc patching
    for obsolete_id in parsed.archive_ids.into_iter().take(16) {
        let trimmed = obsolete_id.trim();
        if !trimmed.is_empty() && runtime.store.archive_memory(trimmed).unwrap_or(false) {
            rec.memories_archived += 1;
        }
    }

    for mem in parsed.memories.into_iter().take(16) {
        if mem.title.trim().is_empty() || mem.content.trim().is_empty() {
            continue;
        }
        let kind = MemoryKind::from_str_loose(&mem.kind);
        let scope = match mem.scope.trim().to_ascii_lowercase().as_str() {
            "global" => MEMORY_SCOPE_GLOBAL.to_string(),
            "project" => mem
                .source_session_id
                .as_deref()
                .and_then(|sid| session_scope_map.get(sid))
                .cloned()
                .unwrap_or_else(|| default_proj_scope.clone()),
            _ => match kind {
                MemoryKind::User => MEMORY_SCOPE_GLOBAL.to_string(),
                _ => default_proj_scope.clone(),
            },
        };

        // Contradiction resolution: archive superseded older facts in the same scope
        if let Some(sup_q) = mem
            .supersedes_query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let scope_arg = match scope == MEMORY_SCOPE_GLOBAL {
                true => None,
                false => Some(scope.as_str()),
            };
            let include_global = scope == MEMORY_SCOPE_GLOBAL;
            if let Ok(old_hits) =
                runtime
                    .store
                    .search_memory_hybrid(sup_q, scope_arg, include_global, None, 2)
            {
                for old in old_hits {
                    if old.entry.scope == scope
                        && runtime.store.archive_memory(&old.entry.id).unwrap_or(false)
                    {
                        rec.memories_archived += 1;
                    }
                }
            }
        }

        if let Ok(up) = runtime.store.upsert_memory_dedup_with_secrets(
            &scope,
            kind.as_str(),
            &mem.title,
            &mem.content,
            mem.salience,
            mem.source_session_id.as_deref(),
            None,
            cfg.memory.dedup_enabled,
            &known_secrets,
        ) {
            match up.merged {
                true => rec.memories_merged += 1,
                false => rec.memories_added += 1,
            }
        }
    }

    if cfg.memory.dream_auto_patch_docs {
        let mut diffs = Vec::new();
        for patch in parsed.doc_patches.into_iter().take(4) {
            let doc_name = match patch.doc.trim() {
                "USER.md" => "USER.md",
                "MEMORY.md" => "MEMORY.md",
                PROJECT_MEMORY_DOC_NAME => PROJECT_MEMORY_DOC_NAME,
                _ => continue,
            };
            if patch.new_text.trim().is_empty() {
                continue;
            }
            let target_ws = patch
                .source_session_id
                .as_deref()
                .and_then(|sid| session_ws_map.get(sid))
                .unwrap_or(&ws);
            let doc_mgr = SnapshotManager::new(cfg.resolved_data_dir(), target_ws)
                .with_hub_dir(cfg.resolved_hub_dir());
            let res = match patch.action.trim().to_ascii_lowercase().as_str() {
                "patch" => match patch.old_text.as_deref().filter(|s| !s.trim().is_empty()) {
                    Some(old) => doc_mgr
                        .patch_doc_with_secrets(doc_name, old, &patch.new_text, "dream", &known_secrets)
                        .map(|(d, _)| (d, true)),
                    None => continue,
                },
                _ => doc_mgr
                    .append_doc_guarded_with_secrets(doc_name, &patch.new_text, "dream", &known_secrets)
                    .map(|(d, _, added)| (d, added)),
            };
            if let Ok((diff, changed)) = res
                && changed {
                    rec.docs_patched += 1;
                    if !diff.is_empty() {
                        diffs.push(diff);
                    }
                }
        }
        if !diffs.is_empty() {
            rec.diff_preview = Some(compact_preview(&diffs.join("\n"), 2_000));
        }
        // Re-check doc compression right after Stage-1 doc patches in case new bullets pushed a doc over threshold.
        let (post_compressed, _) =
            auto_compress_bloated_memory_docs(&stage0_mgr, cfg.context.concise_mode);
        rec.docs_patched += post_compressed;
    }

    if cfg.skills.dream_propose_skills {
        let existing_slugs: std::collections::HashSet<String> = snapshot
            .skills
            .iter()
            .map(|s| s.slug.to_ascii_lowercase())
            .collect();
        for prop in parsed
            .proposed_skills
            .into_iter()
            .take(DREAM_MAX_PROPOSED_SKILLS)
        {
            let slug = prop.slug.trim();
            if slug.is_empty()
                || prop.content.trim().is_empty()
                || existing_slugs.contains(&slug.to_ascii_lowercase())
            {
                continue;
            }
            let (status, enabled) = match cfg.skills.require_approval {
                true => (SkillStatus::Draft, false),
                false => (SkillStatus::Active, true),
            };
            if let Ok((saved, _)) = runtime.snapshots.create_skill_structured(
                slug,
                match prop.name.trim().is_empty() {
                    true => None,
                    false => Some(prop.name.trim()),
                },
                prop.description.trim(),
                prop.content.trim(),
                enabled,
                status,
                SkillAuthor::Dream,
                &prop.triggers,
                &prop.allowed_tools,
                "dream_propose",
                &known_secrets,
                cfg.skills.max_history_snapshots,
            ) {
                rec.skills_proposed += 1;
                runtime.emit(AgentEvent::SkillUpdated {
                    slug: saved.slug,
                    action: "proposed".into(),
                    status: saved.status.as_str().into(),
                });
            }
        }
    }

    rec.detail = Some(format!(
        "Consolidated {} session(s): +{} added, {} merged, {} archived, {} doc patch(es), {} skill draft(s)",
        rec.sessions_scanned,
        rec.memories_added,
        rec.memories_merged,
        rec.memories_archived,
        rec.docs_patched,
        rec.skills_proposed
    ));
    Ok(processed_latest_ts.map(|dt| dt.to_rfc3339()))
}

fn build_session_digest(
    sess: &Session,
    events: &[SessionEvent],
    proj_scope: &str,
    known_secrets: &[String],
) -> String {
    let mut out = format!(
        "### Session `{}` (title=\"{}\", channel={}, scope={}, updated={})\n",
        sess.id,
        compact_preview(&sess.title, 60),
        sess.channel,
        proj_scope,
        sess.updated_at.format("%Y-%m-%d")
    );
    // Prefer compacted checkpoints + user turns + brainstorm/plan notes + final assistant answers
    for ev in events.iter().rev().take(24).rev() {
        let line = match (ev.role, ev.kind) {
            (EventRole::System, EventKind::CompactionCheckpoint) => ev
                .content_json
                .get("summary")
                .and_then(|v| v.as_str())
                .map(|s| format!("- [CHECKPOINT] {}", compact_preview(s, 480))),
            (EventRole::System, EventKind::SystemNote) => {
                match ev.content_json.get("note_kind").and_then(|v| v.as_str()) {
                    Some(NOTE_KIND_BRAINSTORM_SPEC) => Some(format!(
                        "- [BRAINSTORM] {} -> {}",
                        compact_preview(
                            ev.content_json["spec"]["problem"].as_str().unwrap_or(""),
                            120
                        ),
                        ev.content_json["spec"]["selected_option_id"]
                            .as_str()
                            .unwrap_or("")
                    )),
                    Some(NOTE_KIND_EXECUTION_PLAN) => Some(format!(
                        "- [PLAN] {}",
                        compact_preview(
                            ev.content_json["plan"]["goal"].as_str().unwrap_or(""),
                            140
                        )
                    )),
                    Some(NOTE_KIND_CLARIFICATION_DECISION) => Some(format!(
                        "- [DECISION] {} -> {}",
                        compact_preview(
                            ev.content_json["decision"]["question"].as_str().unwrap_or(""),
                            120
                        ),
                        ev.content_json["decision"]["selected_option_id"]
                            .as_str()
                            .unwrap_or("")
                    )),
                    _ => None,
                }
            }
            (EventRole::User, _) => ev
                .content_json
                .get("text")
                .and_then(|v| v.as_str())
                .map(|s| format!("- [USER] {}", compact_preview(s, 260))),
            (EventRole::Assistant, EventKind::Message) => ev
                .content_json
                .get("text")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| format!("- [ASSISTANT] {}", compact_preview(s, 280))),
            _ => None,
        };
        if let Some(l) = line {
            out.push_str(&scrub_secrets(&l, known_secrets));
            out.push('\n');
            if out.len() > 2_600 {
                break;
            }
        }
    }
    out
}

/// Deterministic zero-regex JSON object extractor supporting raw JSON or fenced ```json blocks.
fn parse_dream_json(raw: &str) -> Option<DreamExtractionPayload> {
    let trimmed = raw.trim();
    if let Ok(p) = serde_json::from_str::<DreamExtractionPayload>(trimmed) {
        return Some(p);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<DreamExtractionPayload>(&trimmed[start..=end]).ok()
}

fn skipped_record(trigger: &str, reason: &str) -> DreamRunRecord {
    let now = Utc::now();
    DreamRunRecord {
        id: new_id(),
        trigger: trigger.to_string(),
        status: "skipped".into(),
        model: None,
        sessions_scanned: 0,
        memories_added: 0,
        memories_merged: 0,
        memories_archived: 0,
        docs_patched: 0,
        skills_proposed: 0,
        diff_preview: None,
        detail: Some(reason.to_string()),
        started_at: now,
        finished_at: Some(now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_and_raw_dream_extraction_json_without_regex() {
        let fenced = "Here is the consolidation:\n```json\n{\"memories\":[{\"kind\":\"feedback\",\"scope\":\"global\",\"title\":\"No Mocks\",\"content\":\"Use real SQLite DBs in integration tests (2026-11-21)\",\"salience\":1.4,\"supersedes_query\":\"mock database\"}],\"doc_patches\":[{\"doc\":\"MEMORY.md\",\"action\":\"append\",\"new_text\":\"Prefer real SQLite tests\"}]}\n```\n";
        let parsed = parse_dream_json(fenced).expect("must parse fenced json");
        assert_eq!(parsed.memories.len(), 1);
        assert_eq!(parsed.memories[0].kind, "feedback");
        assert_eq!(
            parsed.memories[0].supersedes_query.as_deref(),
            Some("mock database")
        );
        assert_eq!(parsed.doc_patches.len(), 1);
        assert_eq!(parsed.doc_patches[0].doc, "MEMORY.md");
    }

    #[test]
    fn dream_auto_compresses_bloated_memory_doc_and_preserves_short_doc() {
        let dir = std::env::temp_dir().join(format!("zenko_dream_cmp_{}", zenko_core::new_id()));
        let ws = dir.join("ws");
        std::fs::create_dir_all(&ws).expect("ws");
        let mgr = SnapshotManager::new(dir.join("data"), &ws);
        mgr.ensure_defaults().expect("defaults");

        // Short default docs stay untouched.
        let (count0, saved0) = auto_compress_bloated_memory_docs(&mgr, ConciseMode::Full);
        assert_eq!(count0, 0);
        assert_eq!(saved0, 0);

        // Bloated MEMORY.md (>= 65% of 1200-token cap) auto-compresses with a rollback snapshot!
        let mut bloated = String::from("# Long-Term Memory\n\n");
        for i in 0..36 {
            bloated.push_str(&format!(
                "- It is important to note that you should please make sure to run `cargo test --workspace` in order to verify rule {i} at `./crates/zenko-core/src/config.rs` prior to committing.\n"
            ));
        }
        bloated.push_str("\n```rust\nfn keep_verbatim() { let _ = \"please note that\"; }\n```\n");
        mgr.write_doc("MEMORY.md", &bloated).expect("write bloated");

        let (count1, saved1) = auto_compress_bloated_memory_docs(&mgr, ConciseMode::Full);
        assert_eq!(count1, 1);
        assert!(saved1 >= DREAM_AUTO_COMPRESS_MIN_SAVED_BYTES, "saved={saved1}");
        let compressed = mgr.read_doc("MEMORY.md").expect("read compressed");
        assert!(compressed.contains("# Long-Term Memory"));
        assert!(compressed.contains("`cargo test --workspace`"));
        assert!(compressed.contains("./crates/zenko-core/src/config.rs"));
        assert!(compressed.contains("let _ = \"please note that\";"));
        assert!(!mgr.list_doc_history(Some("MEMORY.md")).expect("hist").is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
