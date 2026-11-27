use crate::algorithms::{
    build_fts5_query_plan, compute_simhash64, extract_searchable_event_text,
    find_missing_workspace_paths, hamming_distance, hybrid_memory_score,
    hybrid_memory_score_with_vector, relaxed_term_overlap, sanitize_memory_text_with_secrets,
    shingle_jaccard,
};
use crate::turboquant::{TurboQuantCodec, TurboQuantVector};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use zenko_core::{
    AgentMailMessage, ApprovalDecision, ApprovalRecord, ApprovalStatus, BackgroundJobRecord,
    BackgroundJobStatus, CheckpointBackend, CronJob, CronKind, DREAM_KV_FTS_BACKFILLED_KEY,
    DREAM_KV_LEASE_KEY, DailyActivityPoint, DashboardCounts, DreamRunRecord,
    EXPORT_MAX_BATCH_LIMIT, EventKind, EventRole, GOAL_LOOP_LIST_DEFAULT_LIMIT,
    GOAL_LOOP_RUNS_DEFAULT_LIMIT, GatewayPeer, GoalLoopRecord, GoalLoopRun, GoalLoopStatus,
    JOB_MAX_HISTORY, LOG_SQLITE_MAX_ROWS, LOG_SQLITE_PRUNE_INTERVAL, LogEntry, LogFilter,
    MEMORY_ARCHIVE_INACTIVE_DAYS, MEMORY_ARCHIVE_MAX_SALIENCE, MEMORY_DEDUP_JACCARD_THRESHOLD,
    MEMORY_DEDUP_SIMHASH_MAX_HAMMING, MEMORY_RELAXED_MIN_OVERLAP, MEMORY_SCOPE_GLOBAL,
    MEMORY_STALE_SALIENCE_DEMOTION, MemoryEntry, MemoryHygieneReport, MemoryKind,
    MemoryUpsertResult, ModelUsageStat, PairingCode, PeerRole, PermissionMode, QueuedPrompt,
    Result, RiskLevel, SESSION_CONTEXT_MAX_WINDOW, SESSION_QUEUE_MAX_ITEMS,
    SESSION_SEARCH_DEFAULT_LIMIT, SESSION_SEARCH_MAX_LIMIT, SESSION_TASK_MAX_ITEMS,
    SESSION_VAR_MAX_ITEMS, SESSION_VAR_MAX_KEY_CHARS, SESSION_VAR_MAX_VALUE_BYTES,
    SESSION_VAR_PREVIEW_CHARS, SUBAGENT_LIST_DEFAULT_LIMIT, SUBAGENT_MAILBOX_MAX_MESSAGES,
    SQLITE_JOURNAL_SIZE_LIMIT_BYTES, SQLITE_VACUUM_FREELIST_MIN_BYTES, ScoredMemoryHit, Session,
    SessionContextTurn, SessionContextWindow, SessionEvent, SessionRolloutSummary,
    SessionSearchFilter, SessionSearchHit, SessionStatus, SessionTask, SessionTaskStatus,
    SessionVarEntry, SubagentRecord, SubagentStatus, SubagentWorkspaceMode, TURBOQUANT_MIN_COSINE,
    TURBOQUANT_SQLITE_SCAN_LIMIT, TaskKind, ToolUsageStat, UiCustomization, UiProfile, UsageStats,
    WorkspaceCheckpoint, ZenkoError, compact_preview, new_id,
};

#[derive(Debug, Clone, Default)]
pub struct SqliteDashboardMetrics {
    pub counts: DashboardCounts,
    pub daily_activity: Vec<DailyActivityPoint>,
    pub top_tools: Vec<ToolUsageStat>,
    pub top_models: Vec<ModelUsageStat>,
    pub channels: BTreeMap<String, u64>,
    pub streak_days: u32,
    pub active_days: u32,
    pub total_tokens: u64,
    pub total_cache_read_tokens: u64,
}

#[derive(Clone)]
pub struct SqliteStore {
    path: PathBuf,
    conn: Arc<Mutex<Connection>>,
    log_inserts: Arc<AtomicU64>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path_buf)
            .map_err(|e| ZenkoError::Storage(format!("failed to open sqlite: {e}")))?;

        conn.execute_batch(&format!(
            "PRAGMA journal_mode = WAL;\n\
             PRAGMA synchronous = NORMAL;\n\
             PRAGMA foreign_keys = ON;\n\
             PRAGMA busy_timeout = 5000;\n\
             PRAGMA cache_size = -2000;\n\
             PRAGMA mmap_size = 0;\n\
             PRAGMA temp_store = MEMORY;\n\
             PRAGMA journal_size_limit = {SQLITE_JOURNAL_SIZE_LIMIT_BYTES};"
        ))
        .map_err(|e| ZenkoError::Storage(format!("failed to apply pragmas: {e}")))?;

        let store = Self {
            path: path_buf,
            conn: Arc::new(Mutex::new(conn)),
            log_inserts: Arc::new(AtomicU64::new(0)),
        };
        store.migrate()?;
        store.ensure_default_ui_profile()?;
        Ok(store)
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| ZenkoError::Storage("sqlite mutex poisoned".into()))?;
        f(&guard).map_err(|e| ZenkoError::Storage(e.to_string()))
    }

    pub fn db_size_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    pub fn db_path(&self) -> &Path {
        &self.path
    }

    /// Truncates the WAL file back to disk and asks SQLite to release idle page-cache memory.
    pub fn compact_wal(&self) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize; PRAGMA shrink_memory;",
            )
        })
    }

    /// Reclaims deleted SQLite pages via `VACUUM` only when the freelist exceeds
    /// `SQLITE_VACUUM_FREELIST_MIN_BYTES`, followed by WAL truncation & page-cache shrink.
    pub fn vacuum_if_bloated(&self) -> Result<bool> {
        let reclaimable_bytes: u64 = self.with_conn(|conn| {
            let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
            let freelist: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
            Ok((page_size.max(0) as u64).saturating_mul(freelist.max(0) as u64))
        })?;
        if reclaimable_bytes < SQLITE_VACUUM_FREELIST_MIN_BYTES {
            let _ = self.compact_wal();
            return Ok(false);
        }
        self.with_conn(|conn| {
            conn.execute_batch(
                "VACUUM; PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize; PRAGMA shrink_memory;",
            )
        })?;
        Ok(true)
    }

    /// Total disk footprint of SQLite DB + WAL + SHM files.
    pub fn db_total_size_bytes(&self) -> (u64, u64) {
        let base = self.path.to_string_lossy();
        let candidates = [
            self.path.clone(),
            PathBuf::from(format!("{base}-wal")),
            PathBuf::from(format!("{base}-shm")),
        ];
        let mut bytes = 0u64;
        let mut files = 0u64;
        for p in candidates {
            if let Ok(meta) = std::fs::metadata(&p) {
                bytes = bytes.saturating_add(meta.len());
                files += 1;
            }
        }
        (bytes, files)
    }

    fn migrate(&self) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS sessions (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    channel TEXT NOT NULL,
                    external_chat_id TEXT,
                    external_thread_id TEXT,
                    model_override TEXT,
                    permission_mode TEXT NOT NULL DEFAULT 'balanced',
                    status TEXT NOT NULL DEFAULT 'idle',
                    head_event_id TEXT,
                    frozen_system_hash TEXT,
                    total_tokens INTEGER NOT NULL DEFAULT 0,
                    total_cost_usd REAL NOT NULL DEFAULT 0.0,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_sessions_channel_ext
                    ON sessions(channel, external_chat_id, external_thread_id);

                CREATE TABLE IF NOT EXISTS session_events (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    parent_id TEXT,
                    branch_index INTEGER NOT NULL DEFAULT 0,
                    role TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    content_json TEXT NOT NULL,
                    tokens_in INTEGER NOT NULL DEFAULT 0,
                    tokens_out INTEGER NOT NULL DEFAULT 0,
                    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                    cost_usd REAL NOT NULL DEFAULT 0.0,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_events_session_created
                    ON session_events(session_id, created_at ASC);
                CREATE INDEX IF NOT EXISTS idx_events_parent
                    ON session_events(session_id, parent_id);

                CREATE TABLE IF NOT EXISTS approvals (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    call_id TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    args_preview TEXT NOT NULL,
                    risk TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'pending',
                    resolved_by TEXT,
                    created_at TEXT NOT NULL,
                    resolved_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_approvals_status ON approvals(status, created_at DESC);

                CREATE TABLE IF NOT EXISTS gateway_peers (
                    channel TEXT NOT NULL,
                    peer_id TEXT NOT NULL,
                    username TEXT,
                    display_name TEXT,
                    role TEXT NOT NULL DEFAULT 'owner',
                    paired_at TEXT NOT NULL,
                    PRIMARY KEY (channel, peer_id)
                );

                CREATE TABLE IF NOT EXISTS pairing_codes (
                    code TEXT PRIMARY KEY,
                    channel TEXT NOT NULL,
                    role TEXT NOT NULL DEFAULT 'owner',
                    expires_at TEXT NOT NULL,
                    used_at TEXT
                );

                CREATE TABLE IF NOT EXISTS memory_entries (
                    id TEXT PRIMARY KEY,
                    scope TEXT NOT NULL DEFAULT 'global',
                    kind TEXT NOT NULL DEFAULT 'fact',
                    title TEXT NOT NULL,
                    content TEXT NOT NULL,
                    salience REAL NOT NULL DEFAULT 1.0,
                    created_at TEXT NOT NULL
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                    entry_id UNINDEXED,
                    title,
                    content
                );

                CREATE TABLE IF NOT EXISTS memory_vectors (
                    entry_id TEXT PRIMARY KEY REFERENCES memory_entries(id) ON DELETE CASCADE,
                    dim INTEGER NOT NULL,
                    bits INTEGER NOT NULL,
                    vec_blob BLOB NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS cron_jobs (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    cron_expr TEXT NOT NULL,
                    kind TEXT NOT NULL DEFAULT 'cron',
                    prompt TEXT NOT NULL,
                    script_path TEXT,
                    target_channel TEXT NOT NULL DEFAULT 'webui',
                    target_chat_id TEXT,
                    target_thread_id TEXT,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    next_run_at TEXT,
                    last_run_at TEXT,
                    last_status TEXT
                );

                CREATE TABLE IF NOT EXISTS ui_profiles (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    is_active INTEGER NOT NULL DEFAULT 0,
                    config_json TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS usage_ledger (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    prompt_tokens INTEGER NOT NULL DEFAULT 0,
                    completion_tokens INTEGER NOT NULL DEFAULT 0,
                    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                    cost_usd REAL NOT NULL DEFAULT 0.0,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_usage_created ON usage_ledger(created_at DESC);

                CREATE TABLE IF NOT EXISTS kv_state (
                    k TEXT PRIMARY KEY,
                    v TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS logs (
                    id TEXT PRIMARY KEY,
                    ts TEXT NOT NULL,
                    level TEXT NOT NULL,
                    category TEXT NOT NULL,
                    target TEXT NOT NULL,
                    session_id TEXT,
                    turn_id TEXT,
                    call_id TEXT,
                    message TEXT NOT NULL,
                    payload_json TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_logs_ts ON logs(ts DESC);
                CREATE INDEX IF NOT EXISTS idx_logs_category_ts ON logs(category, ts DESC);
                CREATE INDEX IF NOT EXISTS idx_logs_session_ts ON logs(session_id, ts DESC);
                CREATE INDEX IF NOT EXISTS idx_logs_level_ts ON logs(level, ts DESC);

                CREATE TABLE IF NOT EXISTS background_jobs (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL,
                    kind TEXT NOT NULL DEFAULT 'shell',
                    label TEXT,
                    command TEXT NOT NULL,
                    workspace_dir TEXT NOT NULL,
                    pid INTEGER,
                    status TEXT NOT NULL,
                    exit_code INTEGER,
                    log_path TEXT NOT NULL,
                    bytes_written INTEGER NOT NULL DEFAULT 0,
                    started_at TEXT NOT NULL,
                    finished_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_jobs_session_started
                    ON background_jobs(session_id, started_at DESC);
                CREATE INDEX IF NOT EXISTS idx_jobs_status_started
                    ON background_jobs(status, started_at DESC);

                CREATE TABLE IF NOT EXISTS session_queue (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    text TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'queued',
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_session_queue_lookup
                    ON session_queue(session_id, status, created_at ASC);

                CREATE VIRTUAL TABLE IF NOT EXISTS session_events_fts USING fts5(
                    event_id UNINDEXED,
                    session_id UNINDEXED,
                    role UNINDEXED,
                    body
                );

                CREATE TABLE IF NOT EXISTS dream_runs (
                    id TEXT PRIMARY KEY,
                    trigger TEXT NOT NULL,
                    status TEXT NOT NULL,
                    model TEXT,
                    sessions_scanned INTEGER NOT NULL DEFAULT 0,
                    memories_added INTEGER NOT NULL DEFAULT 0,
                    memories_merged INTEGER NOT NULL DEFAULT 0,
                    memories_archived INTEGER NOT NULL DEFAULT 0,
                    docs_patched INTEGER NOT NULL DEFAULT 0,
                    diff_preview TEXT,
                    detail TEXT,
                    started_at TEXT NOT NULL,
                    finished_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_dream_runs_started
                    ON dream_runs(started_at DESC);

                CREATE TABLE IF NOT EXISTS session_rollout_summaries (
                    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                    scope TEXT NOT NULL,
                    digest TEXT NOT NULL,
                    session_updated_at TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS subagents (
                    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
                    parent_session_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    depth INTEGER NOT NULL DEFAULT 1,
                    goal TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'running',
                    workspace_mode TEXT NOT NULL DEFAULT 'inherit',
                    workspace_dir TEXT NOT NULL,
                    worktree_branch TEXT,
                    worktree_base_commit TEXT,
                    worktree_dirty_baseline INTEGER NOT NULL DEFAULT 0,
                    model TEXT,
                    summary TEXT,
                    output_schema_json TEXT,
                    structured_output_json TEXT,
                    diff_stat TEXT,
                    changed_files_json TEXT,
                    tokens_used INTEGER NOT NULL DEFAULT 0,
                    cost_usd REAL NOT NULL DEFAULT 0.0,
                    resume_count INTEGER NOT NULL DEFAULT 0,
                    started_at TEXT NOT NULL,
                    finished_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_subagents_parent
                    ON subagents(parent_session_id, started_at DESC);
                CREATE INDEX IF NOT EXISTS idx_subagents_status
                    ON subagents(status, started_at DESC);

                CREATE TABLE IF NOT EXISTS agent_messages (
                    id TEXT PRIMARY KEY,
                    from_session_id TEXT NOT NULL,
                    to_session_id TEXT NOT NULL,
                    body TEXT NOT NULL,
                    read_at TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_agent_messages_to
                    ON agent_messages(to_session_id, read_at, created_at ASC);

                CREATE TABLE IF NOT EXISTS session_tasks (
                    id TEXT NOT NULL,
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    branch_event_id TEXT,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    active_label TEXT,
                    status TEXT NOT NULL DEFAULT 'pending',
                    depends_on_json TEXT NOT NULL DEFAULT '[]',
                    owner_agent TEXT,
                    verification_cmd TEXT,
                    verification_evidence TEXT,
                    notes TEXT,
                    priority INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (session_id, id)
                );
                CREATE INDEX IF NOT EXISTS idx_session_tasks_lookup
                    ON session_tasks(session_id, status, priority DESC, created_at ASC);

                CREATE TABLE IF NOT EXISTS session_vars (
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    var_key TEXT NOT NULL,
                    summary TEXT NOT NULL DEFAULT '',
                    value TEXT NOT NULL,
                    byte_len INTEGER NOT NULL DEFAULT 0,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (session_id, var_key)
                );

                CREATE TABLE IF NOT EXISTS workspace_checkpoints (
                    id TEXT PRIMARY KEY,
                    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                    turn_id TEXT NOT NULL,
                    event_id TEXT,
                    workspace_dir TEXT NOT NULL,
                    backend TEXT NOT NULL DEFAULT 'shadow_git',
                    pre_ref TEXT NOT NULL,
                    post_ref TEXT,
                    trigger_tool TEXT NOT NULL,
                    label TEXT NOT NULL,
                    changed_files_json TEXT NOT NULL DEFAULT '[]',
                    is_safety_backup INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_workspace_checkpoints_session
                    ON workspace_checkpoints(session_id, created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_workspace_checkpoints_workspace
                    ON workspace_checkpoints(workspace_dir, created_at DESC);

                CREATE TABLE IF NOT EXISTS goal_loops (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    workspace_dir TEXT NOT NULL,
                    goal_text TEXT NOT NULL,
                    verify_cmd_json TEXT NOT NULL,
                    interval_secs INTEGER NOT NULL DEFAULT 0,
                    max_iterations INTEGER NOT NULL DEFAULT 10,
                    max_budget_usd REAL NOT NULL DEFAULT 2.0,
                    max_consecutive_stalls INTEGER NOT NULL DEFAULT 3,
                    checkpoint_each_iter INTEGER NOT NULL DEFAULT 1,
                    auto_rewind_on_stall INTEGER NOT NULL DEFAULT 0,
                    model TEXT,
                    notify_channel TEXT NOT NULL DEFAULT 'webui',
                    status TEXT NOT NULL DEFAULT 'active',
                    iterations_done INTEGER NOT NULL DEFAULT 0,
                    consecutive_stalls INTEGER NOT NULL DEFAULT 0,
                    cost_spent_usd REAL NOT NULL DEFAULT 0.0,
                    last_verify_exit INTEGER,
                    last_verify_hash TEXT,
                    last_verify_output TEXT,
                    next_run_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_goal_loops_status_next
                    ON goal_loops(status, next_run_at ASC);

                CREATE TABLE IF NOT EXISTS goal_loop_runs (
                    id TEXT PRIMARY KEY,
                    goal_id TEXT NOT NULL REFERENCES goal_loops(id) ON DELETE CASCADE,
                    iteration INTEGER NOT NULL,
                    checkpoint_id TEXT,
                    verify_exit_code INTEGER NOT NULL,
                    verify_output_excerpt TEXT NOT NULL DEFAULT '',
                    verify_hash TEXT NOT NULL DEFAULT '',
                    tokens_used INTEGER NOT NULL DEFAULT 0,
                    cost_usd REAL NOT NULL DEFAULT 0.0,
                    duration_ms INTEGER NOT NULL DEFAULT 0,
                    outcome TEXT NOT NULL,
                    reward REAL NOT NULL DEFAULT 0.0,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_goal_loop_runs_goal
                    ON goal_loop_runs(goal_id, iteration DESC);

                CREATE TABLE IF NOT EXISTS repo_micro_skills (
                    id TEXT PRIMARY KEY,
                    repo_scope TEXT NOT NULL,
                    signature TEXT NOT NULL,
                    fact TEXT NOT NULL,
                    failed_cmd TEXT NOT NULL DEFAULT '',
                    fixed_cmd TEXT NOT NULL DEFAULT '',
                    corroborations INTEGER NOT NULL DEFAULT 1,
                    session_ids_json TEXT NOT NULL DEFAULT '[]',
                    need_hits INTEGER NOT NULL DEFAULT 0,
                    exposures INTEGER NOT NULL DEFAULT 0,
                    contradictions INTEGER NOT NULL DEFAULT 0,
                    status TEXT NOT NULL DEFAULT 'candidate',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    UNIQUE(repo_scope, signature)
                );
                CREATE INDEX IF NOT EXISTS idx_micro_skills_scope_status
                    ON repo_micro_skills(repo_scope, status, corroborations DESC);

                CREATE TABLE IF NOT EXISTS repo_tool_usage (
                    repo_scope TEXT NOT NULL,
                    tool_name TEXT NOT NULL,
                    call_count INTEGER NOT NULL DEFAULT 0,
                    prompts_since_used INTEGER NOT NULL DEFAULT 0,
                    sessions_observed INTEGER NOT NULL DEFAULT 0,
                    last_session_id TEXT NOT NULL DEFAULT '',
                    estimated_schema_tokens INTEGER NOT NULL DEFAULT 0,
                    pruned_to_deferred INTEGER NOT NULL DEFAULT 0,
                    manually_pinned INTEGER NOT NULL DEFAULT 0,
                    last_used_at TEXT,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY(repo_scope, tool_name)
                );
                CREATE INDEX IF NOT EXISTS idx_tool_usage_scope_pruned
                    ON repo_tool_usage(repo_scope, pruned_to_deferred);

                CREATE TABLE IF NOT EXISTS repo_turn_baselines (
                    id TEXT PRIMARY KEY,
                    repo_scope TEXT NOT NULL,
                    tool_calls INTEGER NOT NULL,
                    iterations INTEGER NOT NULL,
                    succeeded INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_turn_baselines_scope
                    ON repo_turn_baselines(repo_scope, succeeded, created_at DESC);

                CREATE TABLE IF NOT EXISTS economy_ledger (
                    id TEXT PRIMARY KEY,
                    repo_scope TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    tokens_delta INTEGER NOT NULL DEFAULT 0,
                    cost_usd_delta REAL NOT NULL DEFAULT 0.0,
                    command_family TEXT NOT NULL DEFAULT '',
                    detail TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_economy_ledger_scope
                    ON economy_ledger(repo_scope, created_at DESC);

                CREATE TABLE IF NOT EXISTS trusted_filter_files (
                    repo_scope TEXT PRIMARY KEY,
                    file_path TEXT NOT NULL,
                    sha256_hex TEXT NOT NULL,
                    trusted_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS repo_code_metrics (
                    repo_scope TEXT PRIMARY KEY,
                    lines_added INTEGER NOT NULL DEFAULT 0,
                    lines_deleted INTEGER NOT NULL DEFAULT 0,
                    turns_with_edits INTEGER NOT NULL DEFAULT 0,
                    new_files_created INTEGER NOT NULL DEFAULT 0,
                    bloat_hints_issued INTEGER NOT NULL DEFAULT 0,
                    debt_markers_last_seen INTEGER NOT NULL DEFAULT 0,
                    updated_at TEXT NOT NULL
                );
                "#,
            )?;

            // Safe idempotent column migrations for existing ~/.zenko/zenko.db databases.
            for (table, col, def) in [
                ("background_jobs", "kind", "TEXT NOT NULL DEFAULT 'shell'"),
                ("background_jobs", "label", "TEXT"),
                ("cron_jobs", "script_path", "TEXT"),
                ("sessions", "workspace_dir", "TEXT"),
                ("sessions", "parent_session_id", "TEXT"),
                ("sessions", "agent_role", "TEXT"),
                ("sessions", "agent_depth", "INTEGER NOT NULL DEFAULT 0"),
                ("sessions", "worktree_path", "TEXT"),
                ("sessions", "worktree_branch", "TEXT"),
                ("memory_entries", "access_count", "INTEGER NOT NULL DEFAULT 0"),
                ("memory_entries", "last_accessed_at", "TEXT"),
                ("memory_entries", "updated_at", "TEXT"),
                ("memory_entries", "content_simhash", "INTEGER NOT NULL DEFAULT 0"),
                ("memory_entries", "source_session_id", "TEXT"),
                ("memory_entries", "source_event_id", "TEXT"),
                ("memory_entries", "archived", "INTEGER NOT NULL DEFAULT 0"),
                ("subagents", "resume_count", "INTEGER NOT NULL DEFAULT 0"),
                ("subagents", "output_schema_json", "TEXT"),
                ("subagents", "worktree_base_commit", "TEXT"),
                ("subagents", "worktree_dirty_baseline", "INTEGER NOT NULL DEFAULT 0"),
                ("dream_runs", "skills_proposed", "INTEGER NOT NULL DEFAULT 0"),
                ("economy_ledger", "command_family", "TEXT NOT NULL DEFAULT ''"),
            ] {
                ensure_column(conn, table, col, def)?;
            }
            conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_jobs_kind_started
                    ON background_jobs(kind, started_at DESC);
                 CREATE INDEX IF NOT EXISTS idx_memory_scope_archived
                    ON memory_entries(scope, archived, created_at DESC);
                 CREATE INDEX IF NOT EXISTS idx_sessions_workspace
                    ON sessions(workspace_dir, updated_at DESC);
                 CREATE INDEX IF NOT EXISTS idx_sessions_parent
                    ON sessions(parent_session_id, updated_at DESC);
                 CREATE INDEX IF NOT EXISTS idx_economy_ledger_cmd
                    ON economy_ledger(repo_scope, command_family);",
            )?;

            // One-time idempotent backfill of `session_events_fts` for pre-Phase-4 databases.
            let already_backfilled: Option<String> = conn
                .query_row(
                    "SELECT v FROM kv_state WHERE k = ?1",
                    params![DREAM_KV_FTS_BACKFILLED_KEY],
                    |r| r.get(0),
                )
                .optional()?;
            if already_backfilled.is_none() {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, role, kind, content_json FROM session_events",
                )?;
                let rows: Vec<(String, String, String, String, String)> = stmt
                    .query_map([], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                    })?
                    .filter_map(std::result::Result::ok)
                    .collect();
                drop(stmt);
                for (eid, sid, role_s, kind_s, raw_json) in rows {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw_json) {
                        let body = extract_searchable_event_text(
                            EventRole::from_str_loose(&role_s),
                            EventKind::from_str_loose(&kind_s),
                            &val,
                        );
                        if !body.is_empty() {
                            let _ = conn.execute(
                                "INSERT INTO session_events_fts (event_id, session_id, role, body) VALUES (?1, ?2, ?3, ?4)",
                                params![eid, sid, role_s, body],
                            );
                        }
                    }
                }
                let now = Utc::now().to_rfc3339();
                conn.execute(
                    "INSERT OR REPLACE INTO kv_state (k, v, updated_at) VALUES (?1, '1', ?2)",
                    params![DREAM_KV_FTS_BACKFILLED_KEY, now],
                )?;
            }
            Ok(())
        })
    }

    fn ensure_default_ui_profile(&self) -> Result<()> {
        let count: i64 = self.with_conn(|conn| {
            conn.query_row("SELECT COUNT(*) FROM ui_profiles", [], |row| row.get(0))
        })?;
        if count == 0 {
            let profile = UiProfile {
                id: "default".to_string(),
                name: "Zenko Vermilion (Default)".to_string(),
                is_active: true,
                config: UiCustomization::default(),
                updated_at: Utc::now(),
            };
            self.save_ui_profile(&profile)?;
        }
        Ok(())
    }

}

fn row_to_goal_loop(r: &rusqlite::Row<'_>) -> rusqlite::Result<GoalLoopRecord> {
    let cmd_raw: String = r.get(5)?;
    Ok(GoalLoopRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        session_id: r.get(2)?,
        workspace_dir: r.get(3)?,
        goal_text: r.get(4)?,
        verify_cmd: serde_json::from_str(&cmd_raw).unwrap_or_default(),
        interval_secs: r.get::<_, i64>(6)?.max(0) as u64,
        max_iterations: r.get::<_, i64>(7)?.max(1) as usize,
        max_budget_usd: r.get(8)?,
        max_consecutive_stalls: r.get::<_, i64>(9)?.max(1) as usize,
        checkpoint_each_iter: r.get::<_, i64>(10)? != 0,
        auto_rewind_on_stall: r.get::<_, i64>(11)? != 0,
        model: r.get(12)?,
        notify_channel: r.get(13)?,
        status: GoalLoopStatus::from_str_loose(&r.get::<_, String>(14)?),
        iterations_done: r.get::<_, i64>(15)?.max(0) as usize,
        consecutive_stalls: r.get::<_, i64>(16)?.max(0) as usize,
        cost_spent_usd: r.get(17)?,
        last_verify_exit: r.get(18)?,
        last_verify_hash: r.get(19)?,
        last_verify_output: r.get(20)?,
        next_run_at: r.get::<_, Option<String>>(21)?.map(|s| parse_dt(&s)),
        created_at: parse_dt(&r.get::<_, String>(22)?),
        updated_at: parse_dt(&r.get::<_, String>(23)?),
    })
}

fn row_to_goal_loop_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<GoalLoopRun> {
    Ok(GoalLoopRun {
        id: r.get(0)?,
        goal_id: r.get(1)?,
        iteration: r.get::<_, i64>(2)?.max(0) as usize,
        checkpoint_id: r.get(3)?,
        verify_exit_code: r.get(4)?,
        verify_output_excerpt: r.get(5)?,
        verify_hash: r.get(6)?,
        tokens_used: r.get::<_, i64>(7)?.max(0) as u64,
        cost_usd: r.get(8)?,
        duration_ms: r.get::<_, i64>(9)?.max(0) as u64,
        outcome: r.get(10)?,
        reward: r.get(11)?,
        created_at: parse_dt(&r.get::<_, String>(12)?),
    })
}

fn row_to_session_task(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionTask> {
    let deps_raw: String = r.get(7)?;
    Ok(SessionTask {
        id: r.get(0)?,
        session_id: r.get(1)?,
        branch_event_id: r.get(2)?,
        title: r.get(3)?,
        description: r.get(4)?,
        active_label: r.get(5)?,
        status: SessionTaskStatus::from_str_loose(&r.get::<_, String>(6)?),
        depends_on: serde_json::from_str(&deps_raw).unwrap_or_default(),
        owner_agent: r.get(8)?,
        verification_cmd: r.get(9)?,
        verification_evidence: r.get(10)?,
        notes: r.get(11)?,
        priority: r.get(12)?,
        created_at: parse_dt(&r.get::<_, String>(13)?),
        updated_at: parse_dt(&r.get::<_, String>(14)?),
    })
}

fn row_to_checkpoint(r: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceCheckpoint> {
    let files_raw: String = r.get(10)?;
    Ok(WorkspaceCheckpoint {
        id: r.get(0)?,
        session_id: r.get(1)?,
        turn_id: r.get(2)?,
        event_id: r.get(3)?,
        workspace_dir: r.get(4)?,
        backend: CheckpointBackend::from_str_loose(&r.get::<_, String>(5)?),
        pre_ref: r.get(6)?,
        post_ref: r.get(7)?,
        trigger_tool: r.get(8)?,
        label: r.get(9)?,
        changed_files: serde_json::from_str(&files_raw).unwrap_or_default(),
        is_safety_backup: r.get::<_, i64>(11)? != 0,
        created_at: parse_dt(&r.get::<_, String>(12)?),
    })
}

fn row_to_subagent(r: &rusqlite::Row<'_>) -> rusqlite::Result<SubagentRecord> {
    let output_schema_raw: Option<String> = r.get(13)?;
    let structured_raw: Option<String> = r.get(14)?;
    let files_raw: Option<String> = r.get(16)?;
    let tokens_used = r.get::<_, i64>(17)?.max(0) as u64;
    let cost_usd: f64 = r.get(18)?;
    Ok(SubagentRecord {
        id: r.get(0)?,
        parent_session_id: r.get(1)?,
        role: r.get(2)?,
        depth: r.get::<_, i64>(3)?.clamp(0, u8::MAX as i64) as u8,
        goal: r.get(4)?,
        status: SubagentStatus::from_str_loose(&r.get::<_, String>(5)?),
        workspace_mode: SubagentWorkspaceMode::from_str_loose(&r.get::<_, String>(6)?),
        workspace_dir: r.get(7)?,
        worktree_branch: r.get(8)?,
        worktree_base_commit: r.get(9)?,
        worktree_dirty_baseline: r.get::<_, i64>(10)? != 0,
        model: r.get(11)?,
        summary: r.get(12)?,
        output_schema: output_schema_raw.and_then(|s| serde_json::from_str(&s).ok()),
        structured_output: structured_raw.and_then(|s| serde_json::from_str(&s).ok()),
        diff_stat: r.get(15)?,
        changed_files: files_raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        tokens_used,
        cost_usd,
        tree_tokens: tokens_used,
        tree_cost_usd: cost_usd,
        descendant_count: 0,
        resume_count: r.get::<_, i64>(19)?.max(0) as u32,
        started_at: parse_dt(&r.get::<_, String>(20)?),
        finished_at: r.get::<_, Option<String>>(21)?.map(|s| parse_dt(&s)),
    })
}

fn row_to_agent_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentMailMessage> {
    Ok(AgentMailMessage {
        id: r.get(0)?,
        from_session_id: r.get(1)?,
        to_session_id: r.get(2)?,
        body: r.get(3)?,
        read_at: r.get::<_, Option<String>>(4)?.map(|s| parse_dt(&s)),
        created_at: parse_dt(&r.get::<_, String>(5)?),
        delivery_action: None,
    })
}

fn row_to_queued_prompt(r: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedPrompt> {
    Ok(QueuedPrompt {
        id: r.get(0)?,
        session_id: r.get(1)?,
        text: r.get(2)?,
        status: zenko_core::QueuedPromptStatus::from_str_loose(&r.get::<_, String>(3)?),
        created_at: parse_dt(&r.get::<_, String>(4)?),
    })
}

fn row_to_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<BackgroundJobRecord> {
    Ok(BackgroundJobRecord {
        id: r.get(0)?,
        session_id: r.get(1)?,
        kind: TaskKind::from_str_loose(&r.get::<_, String>(2)?),
        label: r.get(3)?,
        command: r.get(4)?,
        workspace_dir: r.get(5)?,
        pid: r.get::<_, Option<i64>>(6)?.and_then(|v| u32::try_from(v).ok()),
        status: BackgroundJobStatus::from_str_loose(&r.get::<_, String>(7)?),
        exit_code: r.get(8)?,
        log_path: r.get(9)?,
        bytes_written: r.get::<_, i64>(10)?.max(0) as u64,
        started_at: parse_dt(&r.get::<_, String>(11)?),
        finished_at: r.get::<_, Option<String>>(12)?.map(|s| parse_dt(&s)),
    })
}

fn row_to_log_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<LogEntry> {
    let payload_raw: Option<String> = r.get(8)?;
    let payload = payload_raw.and_then(|raw| serde_json::from_str(&raw).ok());
    Ok(LogEntry {
        timestamp: parse_dt(&r.get::<_, String>(0)?),
        level: r.get(1)?,
        category: r.get(2)?,
        target: r.get(3)?,
        session_id: r.get(4)?,
        turn_id: r.get(5)?,
        call_id: r.get(6)?,
        message: r.get(7)?,
        payload,
    })
}

fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn ensure_column(conn: &Connection, table: &str, col: &str, col_def: &str) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let exists = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(std::result::Result::ok)
        .any(|name| name == col);
    if !exists {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {col_def}"), [])?;
    }
    Ok(())
}

fn row_to_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: r.get(0)?,
        title: r.get(1)?,
        channel: r.get(2)?,
        external_chat_id: r.get(3)?,
        external_thread_id: r.get(4)?,
        model_override: r.get(5)?,
        permission_mode: PermissionMode::from_str_loose(&r.get::<_, String>(6)?),
        status: SessionStatus::from_str_loose(&r.get::<_, String>(7)?),
        head_event_id: r.get(8)?,
        frozen_system_hash: r.get(9)?,
        workspace_dir: r.get(10)?,
        parent_session_id: r.get(11)?,
        agent_role: r.get(12)?,
        agent_depth: r.get::<_, Option<i64>>(13)?.unwrap_or(0).clamp(0, u8::MAX as i64) as u8,
        worktree_path: r.get(14)?,
        worktree_branch: r.get(15)?,
        total_tokens: r.get::<_, i64>(16)?.max(0) as u64,
        total_cost_usd: r.get(17)?,
        created_at: parse_dt(&r.get::<_, String>(18)?),
        updated_at: parse_dt(&r.get::<_, String>(19)?),
        subagent_count: r.get::<_, i64>(20).unwrap_or(0).max(0) as usize,
    })
}

fn row_to_event(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionEvent> {
    let raw_json: String = r.get(6)?;
    let content_json = serde_json::from_str(&raw_json).unwrap_or(serde_json::Value::Null);
    Ok(SessionEvent {
        id: r.get(0)?,
        session_id: r.get(1)?,
        parent_id: r.get(2)?,
        branch_index: r.get(3)?,
        role: EventRole::from_str_loose(&r.get::<_, String>(4)?),
        kind: EventKind::from_str_loose(&r.get::<_, String>(5)?),
        content_json,
        tokens_in: r.get::<_, i64>(7)?.max(0) as u64,
        tokens_out: r.get::<_, i64>(8)?.max(0) as u64,
        cache_read_tokens: r.get::<_, i64>(9)?.max(0) as u64,
        cost_usd: r.get(10)?,
        created_at: parse_dt(&r.get::<_, String>(11)?),
    })
}

fn row_to_approval(r: &rusqlite::Row<'_>) -> rusqlite::Result<ApprovalRecord> {
    let risk_s: String = r.get(5)?;
    let risk = match risk_s.as_str() {
        "safe" => RiskLevel::Safe,
        "dangerous" => RiskLevel::Dangerous,
        _ => RiskLevel::Moderate,
    };
    Ok(ApprovalRecord {
        id: r.get(0)?,
        session_id: r.get(1)?,
        call_id: r.get(2)?,
        tool_name: r.get(3)?,
        args_preview: r.get(4)?,
        risk,
        status: ApprovalStatus::from_str_loose(&r.get::<_, String>(6)?),
        resolved_by: r.get(7)?,
        created_at: parse_dt(&r.get::<_, String>(8)?),
        resolved_at: r.get::<_, Option<String>>(9)?.map(|s| parse_dt(&s)),
    })
}

fn row_to_peer(r: &rusqlite::Row<'_>) -> rusqlite::Result<GatewayPeer> {
    Ok(GatewayPeer {
        channel: r.get(0)?,
        peer_id: r.get(1)?,
        username: r.get(2)?,
        display_name: r.get(3)?,
        role: PeerRole::from_str_loose(&r.get::<_, String>(4)?),
        paired_at: parse_dt(&r.get::<_, String>(5)?),
    })
}

fn row_to_memory(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryEntry> {
    Ok(MemoryEntry {
        id: r.get(0)?,
        scope: r.get(1)?,
        kind: r.get(2)?,
        title: r.get(3)?,
        content: r.get(4)?,
        salience: r.get(5)?,
        access_count: r.get::<_, i64>(6)?.max(0) as u64,
        last_accessed_at: r.get::<_, Option<String>>(7)?.map(|s| parse_dt(&s)),
        updated_at: r.get::<_, Option<String>>(8)?.map(|s| parse_dt(&s)),
        content_simhash: r.get::<_, i64>(9)? as u64,
        source_session_id: r.get(10)?,
        source_event_id: r.get(11)?,
        archived: r.get::<_, i64>(12)? != 0,
        created_at: parse_dt(&r.get::<_, String>(13)?),
    })
}

fn row_to_cron(r: &rusqlite::Row<'_>) -> rusqlite::Result<CronJob> {
    Ok(CronJob {
        id: r.get(0)?,
        name: r.get(1)?,
        cron_expr: r.get(2)?,
        kind: CronKind::from_str_loose(&r.get::<_, String>(3)?),
        prompt: r.get(4)?,
        script_path: r.get(5)?,
        target_channel: r.get(6)?,
        target_chat_id: r.get(7)?,
        target_thread_id: r.get(8)?,
        enabled: r.get::<_, i64>(9)? != 0,
        next_run_at: r.get::<_, Option<String>>(10)?.map(|s| parse_dt(&s)),
        last_run_at: r.get::<_, Option<String>>(11)?.map(|s| parse_dt(&s)),
        last_status: r.get(12)?,
    })
}

fn row_to_ui_profile(r: &rusqlite::Row<'_>) -> rusqlite::Result<UiProfile> {
    let raw: String = r.get(3)?;
    let config = serde_json::from_str(&raw).unwrap_or_default();
    Ok(UiProfile {
        id: r.get(0)?,
        name: r.get(1)?,
        is_active: r.get::<_, i64>(2)? != 0,
        config,
        updated_at: parse_dt(&r.get::<_, String>(4)?),
    })
}


mod approvals_gateways;
mod cron_ui_logs;
mod economy;
mod jobs_agents;
mod memories;
mod sessions;
mod tasks_checkpoints_goals;
#[cfg(test)]
mod tests;
