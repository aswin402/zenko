use crate::algorithms::{
    build_fts5_query_plan, compute_simhash64, extract_searchable_event_text,
    find_missing_workspace_paths, hamming_distance, hybrid_memory_score, relaxed_term_overlap,
    sanitize_memory_text, sanitize_memory_text_with_secrets, shingle_jaccard,
};
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
    SUBAGENT_LIST_DEFAULT_LIMIT, SUBAGENT_MAILBOX_MAX_MESSAGES, ScoredMemoryHit, Session,
    SessionContextTurn, SessionContextWindow, SessionEvent, SessionRolloutSummary,
    SessionSearchFilter, SessionSearchHit, SessionStatus, SessionTask, SessionTaskStatus,
    SubagentRecord, SubagentStatus, SubagentWorkspaceMode, TaskKind, ToolUsageStat,
    UiCustomization, UiProfile, UsageStats, WorkspaceCheckpoint, ZenkoError, new_id,
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

        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            PRAGMA busy_timeout = 5000;
            PRAGMA cache_size = -2000;
            PRAGMA mmap_size = 0;
            PRAGMA temp_store = MEMORY;
            "#,
        )
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
                    ON sessions(parent_session_id, updated_at DESC);",
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

    // ─── Sessions ──────────────────────────────────────────────────────────────

    pub fn create_session(
        &self,
        title: &str,
        channel: &str,
        external_chat_id: Option<&str>,
        external_thread_id: Option<&str>,
        permission_mode: PermissionMode,
    ) -> Result<Session> {
        let now = Utc::now();
        let session = Session {
            id: new_id(),
            title: title.to_string(),
            channel: channel.to_string(),
            external_chat_id: external_chat_id.map(String::from),
            external_thread_id: external_thread_id.map(String::from),
            model_override: None,
            permission_mode,
            status: SessionStatus::Idle,
            head_event_id: None,
            frozen_system_hash: None,
            workspace_dir: None,
            parent_session_id: None,
            agent_role: None,
            agent_depth: 0,
            worktree_path: None,
            worktree_branch: None,
            subagent_count: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
            created_at: now,
            updated_at: now,
        };
        self.insert_session_row(&session)?;
        Ok(session)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_child_session(
        &self,
        parent_session_id: &str,
        role: &str,
        depth: u8,
        title: &str,
        workspace_dir: &str,
        worktree_path: Option<&str>,
        worktree_branch: Option<&str>,
        model_override: Option<&str>,
        permission_mode: PermissionMode,
    ) -> Result<Session> {
        let now = Utc::now();
        let session = Session {
            id: new_id(),
            title: title.to_string(),
            channel: "subagent".to_string(),
            external_chat_id: None,
            external_thread_id: None,
            model_override: model_override.map(String::from),
            permission_mode,
            status: SessionStatus::Running,
            head_event_id: None,
            frozen_system_hash: None,
            workspace_dir: Some(workspace_dir.to_string()),
            parent_session_id: Some(parent_session_id.to_string()),
            agent_role: Some(role.to_string()),
            agent_depth: depth,
            worktree_path: worktree_path.map(String::from),
            worktree_branch: worktree_branch.map(String::from),
            subagent_count: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
            created_at: now,
            updated_at: now,
        };
        self.insert_session_row(&session)?;
        Ok(session)
    }

    fn insert_session_row(&self, session: &Session) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, title, channel, external_chat_id, external_thread_id, model_override, permission_mode, status, head_event_id, frozen_system_hash, workspace_dir, parent_session_id, agent_role, agent_depth, worktree_path, worktree_branch, total_tokens, total_cost_usd, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                params![
                    session.id,
                    session.title,
                    session.channel,
                    session.external_chat_id,
                    session.external_thread_id,
                    session.model_override,
                    session.permission_mode.as_str(),
                    session.status.as_str(),
                    session.head_event_id,
                    session.frozen_system_hash,
                    session.workspace_dir,
                    session.parent_session_id,
                    session.agent_role,
                    session.agent_depth as i64,
                    session.worktree_path,
                    session.worktree_branch,
                    session.total_tokens as i64,
                    session.total_cost_usd,
                    session.created_at.to_rfc3339(),
                    session.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn set_session_workspace(&self, session_id: &str, workspace_dir: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET workspace_dir = ?1 WHERE id = ?2",
                params![workspace_dir, session_id],
            )?;
            Ok(())
        })
    }

    pub fn set_session_worktree(
        &self,
        session_id: &str,
        workspace_dir: &str,
        worktree_path: &str,
        worktree_branch: &str,
    ) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET workspace_dir = ?1, worktree_path = ?2, worktree_branch = ?3 WHERE id = ?4",
                params![workspace_dir, worktree_path, worktree_branch, session_id],
            )?;
            Ok(())
        })
    }

    pub fn find_or_create_channel_session(
        &self,
        channel: &str,
        chat_id: &str,
        thread_id: Option<&str>,
        title: &str,
        permission_mode: PermissionMode,
    ) -> Result<Session> {
        let existing = self.with_conn(|conn| {
            conn.query_row(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id, s.model_override,
                        s.permission_mode, s.status, s.head_event_id, s.frozen_system_hash, s.workspace_dir,
                        s.parent_session_id, s.agent_role, s.agent_depth, s.worktree_path, s.worktree_branch,
                        s.total_tokens, s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents sa WHERE sa.parent_session_id = s.id)
                 FROM sessions s
                 WHERE s.channel = ?1 AND s.external_chat_id = ?2 AND COALESCE(s.external_thread_id, '') = COALESCE(?3, '')
                 ORDER BY s.updated_at DESC LIMIT 1",
                params![channel, chat_id, thread_id],
                row_to_session,
            )
            .optional()
        })?;
        if let Some(s) = existing {
            return Ok(s);
        }
        self.create_session(title, channel, Some(chat_id), thread_id, permission_mode)
    }

    pub fn get_session(&self, id: &str) -> Result<Option<Session>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id, s.model_override,
                        s.permission_mode, s.status, s.head_event_id, s.frozen_system_hash, s.workspace_dir,
                        s.parent_session_id, s.agent_role, s.agent_depth, s.worktree_path, s.worktree_branch,
                        s.total_tokens, s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents sa WHERE sa.parent_session_id = s.id)
                 FROM sessions s WHERE s.id = ?1",
                params![id],
                row_to_session,
            )
            .optional()
        })
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id, s.model_override,
                        s.permission_mode, s.status, s.head_event_id, s.frozen_system_hash, s.workspace_dir,
                        s.parent_session_id, s.agent_role, s.agent_depth, s.worktree_path, s.worktree_branch,
                        s.total_tokens, s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents sa WHERE sa.parent_session_id = s.id)
                 FROM sessions s ORDER BY s.updated_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Lists top-level user/channel root sessions (`parent_session_id IS NULL`) with exact
    /// SQL-aggregated `subagent_count` so high subagent volume never crowds out root chats.
    pub fn list_root_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id, s.model_override,
                        s.permission_mode, s.status, s.head_event_id, s.frozen_system_hash, s.workspace_dir,
                        s.parent_session_id, s.agent_role, s.agent_depth, s.worktree_path, s.worktree_branch,
                        s.total_tokens, s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents sa WHERE sa.parent_session_id = s.id)
                 FROM sessions s
                 WHERE s.parent_session_id IS NULL
                 ORDER BY s.updated_at DESC
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_session(&self, id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute("DELETE FROM session_events_fts WHERE session_id = ?1", params![id])?;
            conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
        })?;
        Ok(n > 0)
    }

    pub fn clear_session_events(&self, id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute("DELETE FROM session_events_fts WHERE session_id = ?1", params![id])?;
            conn.execute("DELETE FROM session_events WHERE session_id = ?1", params![id])?;
            conn.execute(
                "UPDATE sessions SET head_event_id = NULL, total_tokens = 0, total_cost_usd = 0.0, status = 'idle', updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
    }

    pub fn update_session_status(&self, id: &str, status: SessionStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.as_str(), now, id],
            )?;
            Ok(())
        })
    }

    pub fn update_session_meta(
        &self,
        id: &str,
        title: Option<&str>,
        model_override: Option<Option<&str>>,
        permission_mode: Option<PermissionMode>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            if let Some(t) = title {
                conn.execute(
                    "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
                    params![t, now, id],
                )?;
            }
            if let Some(m) = model_override {
                conn.execute(
                    "UPDATE sessions SET model_override = ?1, updated_at = ?2 WHERE id = ?3",
                    params![m, now, id],
                )?;
            }
            if let Some(p) = permission_mode {
                conn.execute(
                    "UPDATE sessions SET permission_mode = ?1, updated_at = ?2 WHERE id = ?3",
                    params![p.as_str(), now, id],
                )?;
            }
            Ok(())
        })
    }

    pub fn set_session_head(&self, session_id: &str, event_id: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET head_event_id = ?1, updated_at = ?2 WHERE id = ?3",
                params![event_id, now, session_id],
            )?;
            Ok(())
        })
    }

    pub fn set_session_frozen_hash(&self, session_id: &str, hash: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sessions SET frozen_system_hash = ?1 WHERE id = ?2",
                params![hash, session_id],
            )?;
            Ok(())
        })
    }

    pub fn recover_interrupted_sessions(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            // Expire orphaned pending approvals whose in-memory oneshot channels were lost on restart.
            conn.execute(
                "UPDATE approvals SET status = 'denied', resolved_by = 'system:restart', resolved_at = ?1 WHERE status = 'pending'",
                params![now],
            )?;
            conn.execute(
                "UPDATE sessions SET status = 'idle', updated_at = ?1 WHERE status IN ('running', 'waiting_approval')",
                params![now],
            )
        })
    }

    // ─── Event Tree (Durable Trajectory + Branching) ───────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn append_event(
        &self,
        session_id: &str,
        parent_id: Option<&str>,
        role: EventRole,
        kind: EventKind,
        content_json: serde_json::Value,
        usage: &UsageStats,
    ) -> Result<SessionEvent> {
        let now = Utc::now();
        let id = new_id();
        let content_str = serde_json::to_string(&content_json)?;

        let branch_index: i64 = self.with_conn(|conn| {
            let sibling_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_events WHERE session_id = ?1 AND COALESCE(parent_id, '') = COALESCE(?2, '')",
                params![session_id, parent_id],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT INTO session_events (id, session_id, parent_id, branch_index, role, kind, content_json, tokens_in, tokens_out, cache_read_tokens, cost_usd, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    session_id,
                    parent_id,
                    sibling_count,
                    role.as_str(),
                    kind.as_str(),
                    content_str,
                    usage.prompt_tokens as i64,
                    usage.completion_tokens as i64,
                    usage.cache_read_tokens as i64,
                    usage.cost_usd,
                    now.to_rfc3339(),
                ],
            )?;
            let searchable = extract_searchable_event_text(role, kind, &content_json);
            if !searchable.is_empty() {
                conn.execute(
                    "INSERT INTO session_events_fts (event_id, session_id, role, body) VALUES (?1, ?2, ?3, ?4)",
                    params![id, session_id, role.as_str(), searchable],
                )?;
            }
            let added_tokens = (usage.prompt_tokens + usage.completion_tokens) as i64;
            conn.execute(
                "UPDATE sessions
                 SET head_event_id = ?1,
                     total_tokens = total_tokens + ?2,
                     total_cost_usd = total_cost_usd + ?3,
                     updated_at = ?4
                 WHERE id = ?5",
                params![id, added_tokens, usage.cost_usd, now.to_rfc3339(), session_id],
            )?;
            Ok(sibling_count)
        })?;

        Ok(SessionEvent {
            id,
            session_id: session_id.to_string(),
            parent_id: parent_id.map(String::from),
            branch_index,
            role,
            kind,
            content_json,
            tokens_in: usage.prompt_tokens,
            tokens_out: usage.completion_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cost_usd: usage.cost_usd,
            created_at: now,
        })
    }

    pub fn get_active_branch(&self, session_id: &str) -> Result<Vec<SessionEvent>> {
        let session = match self.get_session(session_id)? {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };
        let all = self.list_all_session_events(session_id)?;
        let Some(head_id) = session.head_event_id else {
            return Ok(Vec::new());
        };
        let mut by_id = std::collections::HashMap::with_capacity(all.len());
        for ev in all {
            by_id.insert(ev.id.clone(), ev);
        }
        let mut chain = Vec::new();
        let mut cur = Some(head_id);
        let mut guard = 0usize;
        while let Some(id) = cur {
            guard += 1;
            if guard > 10_000 {
                break;
            }
            if let Some(ev) = by_id.remove(&id) {
                cur = ev.parent_id.clone();
                chain.push(ev);
            } else {
                break;
            }
        }
        chain.reverse();
        Ok(chain)
    }

    pub fn list_all_session_events(&self, session_id: &str) -> Result<Vec<SessionEvent>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, parent_id, branch_index, role, kind, content_json, tokens_in, tokens_out, cache_read_tokens, cost_usd, created_at
                 FROM session_events WHERE session_id = ?1 ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map(params![session_id], row_to_event)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn list_sibling_branches(
        &self,
        session_id: &str,
        parent_id: Option<&str>,
    ) -> Result<Vec<SessionEvent>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, parent_id, branch_index, role, kind, content_json, tokens_in, tokens_out, cache_read_tokens, cost_usd, created_at
                 FROM session_events
                 WHERE session_id = ?1 AND COALESCE(parent_id, '') = COALESCE(?2, '')
                 ORDER BY branch_index ASC",
            )?;
            let rows = stmt.query_map(params![session_id, parent_id], row_to_event)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Finds the deepest leaf descending from `event_id` (latest branch tip).
    pub fn find_latest_leaf(&self, session_id: &str, event_id: &str) -> Result<String> {
        let events = self.list_all_session_events(session_id)?;
        let mut cur = event_id.to_string();
        loop {
            let next = events
                .iter()
                .filter(|e| e.parent_id.as_deref() == Some(cur.as_str()))
                .max_by_key(|e| e.created_at)
                .map(|e| e.id.clone());
            match next {
                Some(n) => cur = n,
                None => return Ok(cur),
            }
        }
    }

    // ─── Approvals ─────────────────────────────────────────────────────────────

    pub fn insert_approval(&self, record: &ApprovalRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO approvals (id, session_id, call_id, tool_name, args_preview, risk, status, resolved_by, created_at, resolved_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    record.id,
                    record.session_id,
                    record.call_id,
                    record.tool_name,
                    record.args_preview,
                    record.risk.as_str(),
                    record.status.as_str(),
                    record.resolved_by,
                    record.created_at.to_rfc3339(),
                    record.resolved_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn resolve_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
        resolved_by: &str,
    ) -> Result<Option<ApprovalRecord>> {
        let status = match decision {
            ApprovalDecision::Approve => ApprovalStatus::Approved,
            ApprovalDecision::Deny => ApprovalStatus::Denied,
            ApprovalDecision::AlwaysAllow => ApprovalStatus::AlwaysAllowed,
        };
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE approvals SET status = ?1, resolved_by = ?2, resolved_at = ?3 WHERE id = ?4 AND status = 'pending'",
                params![status.as_str(), resolved_by, now, approval_id],
            )?;
            conn.query_row(
                "SELECT id, session_id, call_id, tool_name, args_preview, risk, status, resolved_by, created_at, resolved_at
                 FROM approvals WHERE id = ?1",
                params![approval_id],
                row_to_approval,
            )
            .optional()
        })
    }

    pub fn list_pending_approvals(&self) -> Result<Vec<ApprovalRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, call_id, tool_name, args_preview, risk, status, resolved_by, created_at, resolved_at
                 FROM approvals WHERE status = 'pending' ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], row_to_approval)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    // ─── Gateways & Pairing ────────────────────────────────────────────────────

    pub fn create_pairing_code(
        &self,
        channel: &str,
        role: PeerRole,
        ttl_mins: i64,
    ) -> Result<PairingCode> {
        let code = zenko_core::SecretVault::generate_pairing_code();
        let expires_at = Utc::now() + Duration::minutes(ttl_mins.max(1));
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO pairing_codes (code, channel, role, expires_at, used_at) VALUES (?1, ?2, ?3, ?4, NULL)",
                params![code, channel, role.as_str(), expires_at.to_rfc3339()],
            )?;
            Ok(())
        })?;
        Ok(PairingCode {
            code,
            channel: channel.to_string(),
            role,
            expires_at,
            used_at: None,
        })
    }

    pub fn list_active_pairing_codes(&self, channel: &str) -> Result<Vec<PairingCode>> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT code, channel, role, expires_at, used_at
                 FROM pairing_codes
                 WHERE channel = ?1 AND used_at IS NULL AND expires_at > ?2
                 ORDER BY expires_at DESC",
            )?;
            let rows = stmt.query_map(params![channel, now], |r| {
                Ok(PairingCode {
                    code: r.get(0)?,
                    channel: r.get(1)?,
                    role: PeerRole::from_str_loose(&r.get::<_, String>(2)?),
                    expires_at: parse_dt(&r.get::<_, String>(3)?),
                    used_at: r.get::<_, Option<String>>(4)?.map(|s| parse_dt(&s)),
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn redeem_pairing_code(
        &self,
        channel: &str,
        code: &str,
        peer_id: &str,
        username: Option<&str>,
        display_name: Option<&str>,
    ) -> Result<Option<GatewayPeer>> {
        let normalized = code.trim().to_ascii_uppercase();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let role_opt: Option<String> = self.with_conn(|conn| {
            conn.query_row(
                "SELECT role FROM pairing_codes WHERE code = ?1 AND channel = ?2 AND used_at IS NULL AND expires_at > ?3",
                params![normalized, channel, now_str],
                |r| r.get(0),
            )
            .optional()
        })?;
        let Some(role_str) = role_opt else {
            return Ok(None);
        };
        let role = PeerRole::from_str_loose(&role_str);
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE pairing_codes SET used_at = ?1 WHERE code = ?2",
                params![now_str, normalized],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO gateway_peers (channel, peer_id, username, display_name, role, paired_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![channel, peer_id, username, display_name, role.as_str(), now_str],
            )?;
            Ok(())
        })?;
        Ok(Some(GatewayPeer {
            channel: channel.to_string(),
            peer_id: peer_id.to_string(),
            username: username.map(String::from),
            display_name: display_name.map(String::from),
            role,
            paired_at: now,
        }))
    }

    pub fn get_peer(&self, channel: &str, peer_id: &str) -> Result<Option<GatewayPeer>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT channel, peer_id, username, display_name, role, paired_at
                 FROM gateway_peers WHERE channel = ?1 AND peer_id = ?2",
                params![channel, peer_id],
                row_to_peer,
            )
            .optional()
        })
    }

    pub fn list_peers(&self, channel: Option<&str>) -> Result<Vec<GatewayPeer>> {
        self.with_conn(|conn| {
            if let Some(ch) = channel {
                let mut stmt = conn.prepare(
                    "SELECT channel, peer_id, username, display_name, role, paired_at
                     FROM gateway_peers WHERE channel = ?1 ORDER BY paired_at DESC",
                )?;
                let rows = stmt.query_map(params![ch], row_to_peer)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            } else {
                let mut stmt = conn.prepare(
                    "SELECT channel, peer_id, username, display_name, role, paired_at
                     FROM gateway_peers ORDER BY paired_at DESC",
                )?;
                let rows = stmt.query_map([], row_to_peer)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            }
        })
    }

    pub fn update_peer_role(&self, channel: &str, peer_id: &str, role: PeerRole) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE gateway_peers SET role = ?1 WHERE channel = ?2 AND peer_id = ?3",
                params![role.as_str(), channel, peer_id],
            )?;
            Ok(())
        })
    }

    pub fn remove_peer(&self, channel: &str, peer_id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM gateway_peers WHERE channel = ?1 AND peer_id = ?2",
                params![channel, peer_id],
            )
        })?;
        Ok(n > 0)
    }

    // ─── Episodic Memory (Scoped FTS5 + SimHash/Jaccard Dedup + RRF Hybrid) ────

    pub fn add_memory(
        &self,
        scope: &str,
        kind: &str,
        title: &str,
        content: &str,
        salience: f64,
    ) -> Result<MemoryEntry> {
        let res = self.upsert_memory_dedup(scope, kind, title, content, salience, None, None, true)?;
        Ok(res.entry)
    }

    /// Inserts a sanitized, SimHash-indexed memory entry — or merges into an existing near-duplicate
    /// in the same scope when `dedup_enabled` and `shingle_jaccard >= MEMORY_DEDUP_JACCARD_THRESHOLD`.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_memory_dedup(
        &self,
        scope: &str,
        kind: &str,
        title: &str,
        content: &str,
        salience: f64,
        source_session_id: Option<&str>,
        source_event_id: Option<&str>,
        dedup_enabled: bool,
    ) -> Result<MemoryUpsertResult> {
        self.upsert_memory_dedup_with_secrets(
            scope,
            kind,
            title,
            content,
            salience,
            source_session_id,
            source_event_id,
            dedup_enabled,
            &[],
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_memory_dedup_with_secrets(
        &self,
        scope: &str,
        kind: &str,
        title: &str,
        content: &str,
        salience: f64,
        source_session_id: Option<&str>,
        source_event_id: Option<&str>,
        dedup_enabled: bool,
        known_secrets: &[String],
    ) -> Result<MemoryUpsertResult> {
        let clean_title = sanitize_memory_text_with_secrets(title.trim(), known_secrets);
        let clean_content = sanitize_memory_text_with_secrets(content.trim(), known_secrets);
        let norm_scope = match scope.trim().is_empty() {
            true => MEMORY_SCOPE_GLOBAL.to_string(),
            false => scope.trim().to_string(),
        };
        let norm_kind = MemoryKind::from_str_loose(kind).as_str().to_string();
        let clamped_salience = salience.clamp(0.1, 3.0);
        let combined = format!("{clean_title} {clean_content}");
        let simhash = compute_simhash64(&combined);
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        if dedup_enabled {
            let existing: Vec<MemoryEntry> = self.with_conn(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at
                     FROM memory_entries
                     WHERE scope = ?1 AND archived = 0
                     ORDER BY created_at DESC LIMIT 200",
                )?;
                let rows = stmt.query_map(params![norm_scope], row_to_memory)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })?;

            let mut best_match: Option<(MemoryEntry, f64)> = None;
            for cand in existing {
                let cand_text = format!("{} {}", cand.title, cand.content);
                let cand_hash = match cand.content_simhash {
                    0 => compute_simhash64(&cand_text),
                    h => h,
                };
                if hamming_distance(simhash, cand_hash) > MEMORY_DEDUP_SIMHASH_MAX_HAMMING
                    && !cand.title.eq_ignore_ascii_case(&clean_title)
                {
                    continue;
                }
                let sim = shingle_jaccard(&combined, &cand_text);
                if sim >= MEMORY_DEDUP_JACCARD_THRESHOLD
                    && best_match.as_ref().map_or(true, |(_, s)| sim > *s)
                {
                    best_match = Some((cand, sim));
                }
            }

            if let Some((mut prev, sim)) = best_match {
                let merged_salience = prev.salience.max(clamped_salience).min(3.0) + 0.05;
                let merged_salience = merged_salience.min(3.0);
                let merged_content = match clean_content.len() >= prev.content.len() {
                    true => clean_content,
                    false => prev.content.clone(),
                };
                let merged_title = match clean_title.is_empty() {
                    true => prev.title.clone(),
                    false => clean_title,
                };
                let merged_hash = compute_simhash64(&format!("{merged_title} {merged_content}"));
                let merged_sid = source_session_id
                    .map(String::from)
                    .or_else(|| prev.source_session_id.clone());
                let merged_eid = source_event_id
                    .map(String::from)
                    .or_else(|| prev.source_event_id.clone());

                self.with_conn(|conn| {
                    conn.execute(
                        "UPDATE memory_entries
                         SET kind = ?1, title = ?2, content = ?3, salience = ?4, updated_at = ?5,
                             content_simhash = ?6, source_session_id = ?7, source_event_id = ?8
                         WHERE id = ?9",
                        params![
                            norm_kind,
                            merged_title,
                            merged_content,
                            merged_salience,
                            now_str,
                            merged_hash as i64,
                            merged_sid,
                            merged_eid,
                            prev.id,
                        ],
                    )?;
                    conn.execute("DELETE FROM memory_fts WHERE entry_id = ?1", params![prev.id])?;
                    conn.execute(
                        "INSERT INTO memory_fts (entry_id, title, content) VALUES (?1, ?2, ?3)",
                        params![prev.id, merged_title, merged_content],
                    )?;
                    Ok(())
                })?;

                prev.kind = norm_kind;
                prev.title = merged_title;
                prev.content = merged_content;
                prev.salience = merged_salience;
                prev.updated_at = Some(now);
                prev.content_simhash = merged_hash;
                prev.source_session_id = merged_sid;
                prev.source_event_id = merged_eid;
                return Ok(MemoryUpsertResult {
                    entry: prev,
                    merged: true,
                    similarity: sim,
                });
            }
        }

        let entry = MemoryEntry {
            id: new_id(),
            scope: norm_scope,
            kind: norm_kind,
            title: clean_title,
            content: clean_content,
            salience: clamped_salience,
            access_count: 0,
            last_accessed_at: None,
            updated_at: Some(now),
            content_simhash: simhash,
            source_session_id: source_session_id.map(String::from),
            source_event_id: source_event_id.map(String::from),
            archived: false,
            created_at: now,
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO memory_entries
                 (id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, ?8, ?9, ?10, 0, ?11)",
                params![
                    entry.id,
                    entry.scope,
                    entry.kind,
                    entry.title,
                    entry.content,
                    entry.salience,
                    now_str,
                    entry.content_simhash as i64,
                    entry.source_session_id,
                    entry.source_event_id,
                    now_str,
                ],
            )?;
            conn.execute(
                "INSERT INTO memory_fts (entry_id, title, content) VALUES (?1, ?2, ?3)",
                params![entry.id, entry.title, entry.content],
            )?;
            Ok(())
        })?;
        Ok(MemoryUpsertResult {
            entry,
            merged: false,
            similarity: 0.0,
        })
    }

    pub fn update_memory(
        &self,
        id: &str,
        title: Option<&str>,
        content: Option<&str>,
        kind: Option<&str>,
        salience: Option<f64>,
        scope: Option<&str>,
    ) -> Result<Option<MemoryEntry>> {
        let Some(mut cur) = self.get_memory(id)? else {
            return Ok(None);
        };
        if let Some(t) = title.map(str::trim).filter(|s| !s.is_empty()) {
            cur.title = sanitize_memory_text(t);
        }
        if let Some(c) = content.map(str::trim).filter(|s| !s.is_empty()) {
            cur.content = sanitize_memory_text(c);
        }
        if let Some(k) = kind.map(str::trim).filter(|s| !s.is_empty()) {
            cur.kind = MemoryKind::from_str_loose(k).as_str().to_string();
        }
        if let Some(s) = salience {
            cur.salience = s.clamp(0.1, 3.0);
        }
        if let Some(sc) = scope.map(str::trim).filter(|s| !s.is_empty()) {
            cur.scope = sc.to_string();
        }
        let now = Utc::now();
        cur.updated_at = Some(now);
        cur.content_simhash = compute_simhash64(&format!("{} {}", cur.title, cur.content));

        self.with_conn(|conn| {
            conn.execute(
                "UPDATE memory_entries
                 SET scope = ?1, kind = ?2, title = ?3, content = ?4, salience = ?5, updated_at = ?6, content_simhash = ?7
                 WHERE id = ?8",
                params![
                    cur.scope,
                    cur.kind,
                    cur.title,
                    cur.content,
                    cur.salience,
                    now.to_rfc3339(),
                    cur.content_simhash as i64,
                    cur.id,
                ],
            )?;
            conn.execute("DELETE FROM memory_fts WHERE entry_id = ?1", params![cur.id])?;
            if !cur.archived {
                conn.execute(
                    "INSERT INTO memory_fts (entry_id, title, content) VALUES (?1, ?2, ?3)",
                    params![cur.id, cur.title, cur.content],
                )?;
            }
            Ok(())
        })?;
        Ok(Some(cur))
    }

    pub fn get_memory(&self, id: &str) -> Result<Option<MemoryEntry>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at
                 FROM memory_entries WHERE id = ?1",
                params![id],
                row_to_memory,
            )
            .optional()
        })
    }

    pub fn archive_memory(&self, id: &str) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let n = self.with_conn(|conn| {
            conn.execute("DELETE FROM memory_fts WHERE entry_id = ?1", params![id])?;
            conn.execute(
                "UPDATE memory_entries SET archived = 1, updated_at = ?1 WHERE id = ?2 AND archived = 0",
                params![now, id],
            )
        })?;
        Ok(n > 0)
    }

    pub fn search_memory_fts(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>> {
        let hits = self.search_memory_hybrid(query, None, true, None, limit)?;
        Ok(hits.into_iter().map(|h| h.entry).collect())
    }

    /// Workspace-scoped pure-Rust Reciprocal Rank Fusion (RRF) memory search:
    /// 1. 2-tier FTS5 query planner (strict AND -> relaxed OR fallback)
    /// 2. Symbol-aware token-shingle Jaccard re-ranking
    /// 3. Ebbinghaus temporal decay + access reinforcement + git-root project boost
    /// 4. Live filesystem path staleness verification (when `workspace_dir` is provided)
    /// 5. Atomic `access_count` + `last_accessed_at` reinforcement on returned hits
    pub fn search_memory_hybrid(
        &self,
        query: &str,
        project_scope: Option<&str>,
        include_global: bool,
        workspace_dir: Option<&Path>,
        limit: usize,
    ) -> Result<Vec<ScoredMemoryHit>> {
        let cap = limit.clamp(1, 50);
        let trimmed = query.trim();

        let scope_matches = |entry_scope: &str| -> bool {
            match (project_scope, include_global) {
                (Some(ps), true) => entry_scope == ps || entry_scope == MEMORY_SCOPE_GLOBAL,
                (Some(ps), false) => entry_scope == ps,
                (None, true) => true,
                (None, false) => entry_scope == MEMORY_SCOPE_GLOBAL,
            }
        };

        if trimmed.is_empty() {
            let rows = self.list_memories_scoped(project_scope, include_global, false, cap)?;
            let now = Utc::now();
            return Ok(rows
                .into_iter()
                .map(|entry| {
                    let stale_paths = workspace_dir
                        .map(|ws| find_missing_workspace_paths(ws, &entry.content))
                        .unwrap_or_default();
                    let age = (now - entry.updated_at.unwrap_or(entry.created_at)).num_hours() as f64
                        / 24.0;
                    let is_proj = project_scope.map_or(false, |ps| entry.scope == ps);
                    let score = hybrid_memory_score(
                        None,
                        None,
                        age,
                        MemoryKind::from_str_loose(&entry.kind),
                        entry.salience,
                        entry.access_count,
                        is_proj,
                        !stale_paths.is_empty(),
                    );
                    ScoredMemoryHit {
                        entry,
                        score,
                        stale_paths,
                    }
                })
                .collect());
        }

        let plan = build_fts5_query_plan(trimmed);
        let candidate_cap = (cap * 4).max(24) as i64;
        let mut bm25_ordered: Vec<MemoryEntry> = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        self.with_conn(|conn| {
            let run_fts = |fts_q: &str,
                           require_overlap: bool,
                           out: &mut Vec<MemoryEntry>,
                           seen: &mut std::collections::HashSet<String>|
             -> rusqlite::Result<()> {
                let mut stmt = conn.prepare(
                    "SELECT m.id, m.scope, m.kind, m.title, m.content, m.salience, m.access_count,
                            m.last_accessed_at, m.updated_at, m.content_simhash,
                            m.source_session_id, m.source_event_id, m.archived, m.created_at
                     FROM memory_fts f
                     JOIN memory_entries m ON m.id = f.entry_id
                     WHERE memory_fts MATCH ?1 AND m.archived = 0
                     ORDER BY bm25(memory_fts) ASC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![fts_q, candidate_cap], row_to_memory)?;
                for row in rows.flatten() {
                    if !scope_matches(&row.scope) {
                        continue;
                    }
                    if require_overlap {
                        let hay = format!("{} {}", row.title, row.content);
                        if relaxed_term_overlap(&plan.signal_terms, &hay) < MEMORY_RELAXED_MIN_OVERLAP
                        {
                            continue;
                        }
                    }
                    if seen.insert(row.id.clone()) {
                        out.push(row);
                    }
                }
                Ok(())
            };

            if let Some(ref eq) = plan.exact_and {
                let _ = run_fts(eq, false, &mut bm25_ordered, &mut seen_ids);
            }
            if bm25_ordered.len() < cap {
                if let Some(ref sq) = plan.split_and {
                    let _ = run_fts(sq, false, &mut bm25_ordered, &mut seen_ids);
                }
            }
            if bm25_ordered.len() < cap {
                if let Some(ref rq) = plan.relaxed_or {
                    let _ = run_fts(rq, true, &mut bm25_ordered, &mut seen_ids);
                }
            }
            Ok(())
        })?;

        if bm25_ordered.is_empty() {
            return Ok(Vec::new());
        }

        // Secondary lexical rank via symbol-aware shingle Jaccard overlap
        let mut jac_pairs: Vec<(usize, f64)> = bm25_ordered
            .iter()
            .enumerate()
            .map(|(idx, e)| {
                let text = format!("{} {}", e.title, e.content);
                (idx, shingle_jaccard(trimmed, &text))
            })
            .collect();
        jac_pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut jac_rank_map: HashMap<usize, usize> = HashMap::with_capacity(jac_pairs.len());
        for (rank, (orig_idx, sim)) in jac_pairs.into_iter().enumerate() {
            if sim > 0.0 {
                jac_rank_map.insert(orig_idx, rank);
            }
        }

        let now = Utc::now();
        let mut scored: Vec<ScoredMemoryHit> = bm25_ordered
            .into_iter()
            .enumerate()
            .map(|(bm25_rank, entry)| {
                let stale_paths = workspace_dir
                    .map(|ws| find_missing_workspace_paths(ws, &entry.content))
                    .unwrap_or_default();
                let ref_ts = entry.updated_at.unwrap_or(entry.created_at);
                let age_days = (now - ref_ts).num_hours().max(0) as f64 / 24.0;
                let is_proj = project_scope.map_or(false, |ps| entry.scope == ps);
                let score = hybrid_memory_score(
                    Some(bm25_rank),
                    jac_rank_map.get(&bm25_rank).copied(),
                    age_days,
                    MemoryKind::from_str_loose(&entry.kind),
                    entry.salience,
                    entry.access_count,
                    is_proj,
                    !stale_paths.is_empty(),
                );
                ScoredMemoryHit {
                    entry,
                    score,
                    stale_paths,
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(cap);

        // ACT-R access reinforcement on returned hits
        let now_str = now.to_rfc3339();
        let hit_ids: Vec<String> = scored.iter().map(|h| h.entry.id.clone()).collect();
        let _ = self.with_conn(|conn| {
            for id in &hit_ids {
                let _ = conn.execute(
                    "UPDATE memory_entries SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
                    params![now_str, id],
                );
            }
            Ok(())
        });
        for hit in &mut scored {
            hit.entry.access_count += 1;
            hit.entry.last_accessed_at = Some(now);
        }

        Ok(scored)
    }

    pub fn list_memories(&self, limit: usize) -> Result<Vec<MemoryEntry>> {
        self.list_memories_scoped(None, true, false, limit)
    }

    pub fn list_memories_scoped(
        &self,
        project_scope: Option<&str>,
        include_global: bool,
        include_archived: bool,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at
                 FROM memory_entries WHERE 1=1",
            );
            let mut bind_vals: Vec<rusqlite::types::Value> = Vec::new();
            if !include_archived {
                sql.push_str(" AND archived = 0");
            }
            match (project_scope, include_global) {
                (Some(ps), true) => {
                    sql.push_str(" AND (scope = ? OR scope = ?)");
                    bind_vals.push(rusqlite::types::Value::Text(ps.to_string()));
                    bind_vals.push(rusqlite::types::Value::Text(MEMORY_SCOPE_GLOBAL.to_string()));
                }
                (Some(ps), false) => {
                    sql.push_str(" AND scope = ?");
                    bind_vals.push(rusqlite::types::Value::Text(ps.to_string()));
                }
                (None, false) => {
                    sql.push_str(" AND scope = ?");
                    bind_vals.push(rusqlite::types::Value::Text(MEMORY_SCOPE_GLOBAL.to_string()));
                }
                (None, true) => {}
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            bind_vals.push(rusqlite::types::Value::Integer(limit.clamp(1, 200) as i64));

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(bind_vals), row_to_memory)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Stage-0 zero-LLM deterministic hygiene:
    /// 1. Oldest-first SQL sweep archiving dormant low-salience entries older than `MEMORY_ARCHIVE_INACTIVE_DAYS`
    ///    (never cut off by recent-200 caps even on multi-month 24/7 databases).
    /// 2. Live workspace path staleness check with bounded salience demotion (`MEMORY_STALE_SALIENCE_DEMOTION`).
    /// 3. SimHash + Jaccard near-duplicate archival within each scope.
    pub fn run_memory_hygiene(&self, workspace_dir: Option<&Path>) -> Result<MemoryHygieneReport> {
        let mut report = MemoryHygieneReport::default();
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let cutoff = (now - Duration::days(MEMORY_ARCHIVE_INACTIVE_DAYS)).to_rfc3339();

        // Pass 1: Dedicated oldest-first archival of dormant low-salience memories
        let dormant_ids: Vec<String> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id FROM memory_entries
                 WHERE archived = 0
                   AND access_count = 0
                   AND salience <= ?1
                   AND COALESCE(last_accessed_at, updated_at, created_at) < ?2
                 ORDER BY created_at ASC
                 LIMIT 500",
            )?;
            let rows = stmt.query_map(params![MEMORY_ARCHIVE_MAX_SALIENCE, cutoff], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        for id in dormant_ids {
            if self.archive_memory(&id).unwrap_or(false) {
                report.entries_archived += 1;
            }
        }

        // Pass 2: Active scan (up to 500 entries) for stale-path demotion + SimHash/Jaccard dedup
        let entries: Vec<MemoryEntry> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, scope, kind, title, content, salience, access_count, last_accessed_at, updated_at, content_simhash, source_session_id, source_event_id, archived, created_at
                 FROM memory_entries
                 WHERE archived = 0
                 ORDER BY created_at DESC LIMIT 500",
            )?;
            let rows = stmt.query_map([], row_to_memory)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        let mut kept: Vec<(String, String, u64, String)> = Vec::new();
        for e in entries {
            if let Some(ws) = workspace_dir {
                if !find_missing_workspace_paths(ws, &e.content).is_empty() {
                    report.stale_flagged += 1;
                    let demoted = (e.salience - MEMORY_STALE_SALIENCE_DEMOTION).max(0.2);
                    if (demoted - e.salience).abs() > f64::EPSILON {
                        let _ = self.with_conn(|conn| {
                            conn.execute(
                                "UPDATE memory_entries SET salience = ?1, updated_at = ?2 WHERE id = ?3",
                                params![demoted, now_str, e.id],
                            )?;
                            Ok(())
                        });
                    }
                }
            }

            let text = format!("{} {}", e.title, e.content);
            let hash = match e.content_simhash {
                0 => compute_simhash64(&text),
                h => h,
            };
            let is_dup = kept.iter().any(|(_, sc, kh, kt)| {
                sc == &e.scope
                    && hamming_distance(hash, *kh) <= MEMORY_DEDUP_SIMHASH_MAX_HAMMING
                    && shingle_jaccard(&text, kt) >= MEMORY_DEDUP_JACCARD_THRESHOLD
            });
            match is_dup {
                true => {
                    if self.archive_memory(&e.id).unwrap_or(false) {
                        report.duplicates_merged += 1;
                    }
                }
                false => kept.push((e.id, e.scope, hash, text)),
            }
        }
        Ok(report)
    }

    pub fn delete_memory(&self, id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute("DELETE FROM memory_fts WHERE entry_id = ?1", params![id])?;
            conn.execute("DELETE FROM memory_entries WHERE id = ?1", params![id])
        })?;
        Ok(n > 0)
    }

    // ─── Cross-Session Recall (`session_events_fts`) ───────────────────────────

    /// Sub-millisecond cross-session FTS5 recall with 3-tier exact->split->relaxed planner and filters.
    pub fn search_sessions_fts(
        &self,
        filter: &SessionSearchFilter,
    ) -> Result<Vec<SessionSearchHit>> {
        let limit = filter
            .limit
            .unwrap_or(SESSION_SEARCH_DEFAULT_LIMIT)
            .clamp(1, SESSION_SEARCH_MAX_LIMIT);
        let plan = build_fts5_query_plan(&filter.query);
        if plan.exact_and.is_none() && plan.split_and.is_none() && plan.relaxed_or.is_none() {
            return Ok(Vec::new());
        }

        let mut results: Vec<SessionSearchHit> = Vec::new();
        let mut seen_events = std::collections::HashSet::new();

        self.with_conn(|conn| {
            let run_query = |fts_q: &str,
                             rem: usize,
                             require_overlap: bool,
                             out: &mut Vec<SessionSearchHit>,
                             seen: &mut std::collections::HashSet<String>|
             -> rusqlite::Result<()> {
                let mut sql = String::from(
                    "SELECT s.id, s.title, s.channel, s.workspace_dir, e.id, e.role,
                            snippet(session_events_fts, 3, '', '', '...', 24) AS snip,
                            bm25(session_events_fts) AS rank,
                            e.created_at
                     FROM session_events_fts f
                     JOIN session_events e ON e.id = f.event_id
                     JOIN sessions s ON s.id = f.session_id
                     WHERE session_events_fts MATCH ?",
                );
                let mut binds: Vec<rusqlite::types::Value> =
                    vec![rusqlite::types::Value::Text(fts_q.to_string())];

                if let Some(since) = filter.since {
                    sql.push_str(" AND e.created_at >= ?");
                    binds.push(rusqlite::types::Value::Text(since.to_rfc3339()));
                }
                if let Some(ref ch) = filter.channel {
                    if !ch.trim().is_empty() && !ch.eq_ignore_ascii_case("all") {
                        sql.push_str(" AND s.channel = ?");
                        binds.push(rusqlite::types::Value::Text(ch.trim().to_string()));
                    }
                }
                if let Some(ref ws) = filter.workspace_dir {
                    if !ws.trim().is_empty() {
                        sql.push_str(" AND s.workspace_dir = ?");
                        binds.push(rusqlite::types::Value::Text(ws.trim().to_string()));
                    }
                }
                if let Some(ref sid) = filter.session_id {
                    if !sid.trim().is_empty() {
                        sql.push_str(" AND s.id = ?");
                        binds.push(rusqlite::types::Value::Text(sid.trim().to_string()));
                    }
                }
                if let Some(ref ex_sid) = filter.exclude_session_id {
                    if !ex_sid.trim().is_empty() {
                        sql.push_str(" AND s.id != ?");
                        binds.push(rusqlite::types::Value::Text(ex_sid.trim().to_string()));
                    }
                }
                if let Some(ref role) = filter.role {
                    if !role.trim().is_empty() && !role.eq_ignore_ascii_case("all") {
                        sql.push_str(" AND e.role = ?");
                        binds.push(rusqlite::types::Value::Text(role.trim().to_string()));
                    }
                }

                sql.push_str(" ORDER BY rank ASC, e.created_at DESC LIMIT ?");
                binds.push(rusqlite::types::Value::Integer((rem * 2).max(8) as i64));

                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(binds), |r| {
                    Ok(SessionSearchHit {
                        session_id: r.get(0)?,
                        session_title: r.get(1)?,
                        channel: r.get(2)?,
                        workspace_dir: r.get(3)?,
                        event_id: r.get(4)?,
                        role: r.get(5)?,
                        snippet: r.get(6)?,
                        rank: r.get(7)?,
                        created_at: parse_dt(&r.get::<_, String>(8)?),
                    })
                })?;
                for hit in rows.flatten() {
                    if require_overlap
                        && relaxed_term_overlap(&plan.signal_terms, &hit.snippet)
                            < MEMORY_RELAXED_MIN_OVERLAP
                    {
                        continue;
                    }
                    if seen.insert(hit.event_id.clone()) {
                        out.push(hit);
                        if out.len() >= limit {
                            break;
                        }
                    }
                }
                Ok(())
            };

            if let Some(ref eq) = plan.exact_and {
                let _ = run_query(eq, limit, false, &mut results, &mut seen_events);
            }
            if results.len() < limit {
                if let Some(ref sq) = plan.split_and {
                    let rem = limit.saturating_sub(results.len());
                    let _ = run_query(sq, rem, false, &mut results, &mut seen_events);
                }
            }
            if results.len() < limit {
                if let Some(ref rq) = plan.relaxed_or {
                    let rem = limit.saturating_sub(results.len());
                    let _ = run_query(rq, rem, true, &mut results, &mut seen_events);
                }
            }
            Ok(())
        })?;

        Ok(results)
    }

    /// Hermes-style lineage traversal: expands ±`window` surrounding events around `event_id`
    /// on that session's active branch (or chronological event log) in sub-millisecond SQLite I/O.
    pub fn get_session_event_context(
        &self,
        event_id: &str,
        window: usize,
    ) -> Result<Option<SessionContextWindow>> {
        let sid_opt: Option<String> = self.with_conn(|conn| {
            conn.query_row(
                "SELECT session_id FROM session_events WHERE id = ?1",
                params![event_id],
                |r| r.get(0),
            )
            .optional()
        })?;
        let Some(sid) = sid_opt else {
            return Ok(None);
        };
        let Some(session) = self.get_session(&sid)? else {
            return Ok(None);
        };
        let mut events = self.get_active_branch(&sid)?;
        if !events.iter().any(|e| e.id == event_id) {
            events = self.list_all_session_events(&sid)?;
        }
        let Some(anchor_idx) = events.iter().position(|e| e.id == event_id) else {
            return Ok(None);
        };
        let rad = window.clamp(1, SESSION_CONTEXT_MAX_WINDOW);
        let start = anchor_idx.saturating_sub(rad);
        let end = (anchor_idx + rad + 1).min(events.len());
        let turns = events[start..end]
            .iter()
            .filter_map(|ev| {
                let text = extract_searchable_event_text(ev.role, ev.kind, &ev.content_json);
                if text.is_empty() && ev.id != event_id {
                    return None;
                }
                Some(SessionContextTurn {
                    event_id: ev.id.clone(),
                    role: ev.role.as_str().to_string(),
                    kind: ev.kind.as_str().to_string(),
                    text,
                    is_anchor: ev.id == event_id,
                    created_at: ev.created_at,
                })
            })
            .collect();

        Ok(Some(SessionContextWindow {
            session_id: session.id,
            session_title: session.title,
            channel: session.channel,
            workspace_dir: session.workspace_dir,
            anchor_event_id: event_id.to_string(),
            events: turns,
        }))
    }

    pub fn get_rollout_summary(
        &self,
        session_id: &str,
        expected_updated_at: &str,
    ) -> Result<Option<SessionRolloutSummary>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT session_id, scope, digest, session_updated_at, created_at
                 FROM session_rollout_summaries
                 WHERE session_id = ?1 AND session_updated_at = ?2",
                params![session_id, expected_updated_at],
                |r| {
                    Ok(SessionRolloutSummary {
                        session_id: r.get(0)?,
                        scope: r.get(1)?,
                        digest: r.get(2)?,
                        session_updated_at: r.get(3)?,
                        created_at: parse_dt(&r.get::<_, String>(4)?),
                    })
                },
            )
            .optional()
        })
    }

    pub fn upsert_rollout_summary(&self, summary: &SessionRolloutSummary) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO session_rollout_summaries
                 (session_id, scope, digest, session_updated_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    summary.session_id,
                    summary.scope,
                    summary.digest,
                    summary.session_updated_at,
                    summary.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    // ─── Autonomous Dream Consolidation Lease, Watermark & Audit Log ───────────

    /// Transactional time-bounded SQLite lease (Codex `codex-rs` pattern — self-heals on crash,
    /// immune to stale lockfiles). Uses an RAII `IMMEDIATE` transaction so any error automatically
    /// rolls back without leaving the shared connection stuck inside an open transaction.
    pub fn try_claim_dream_lease(&self, owner_token: &str, lease_secs: i64) -> Result<bool> {
        let now = Utc::now();
        let clamped_secs = lease_secs.clamp(30, 86_400 * 7);
        let expires = now + Duration::seconds(clamped_secs);
        self.with_conn(|conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            let existing: Option<String> = tx
                .query_row(
                    "SELECT v FROM kv_state WHERE k = ?1",
                    params![DREAM_KV_LEASE_KEY],
                    |r| r.get(0),
                )
                .optional()?;
            let occupied = existing
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .is_some_and(|v| {
                    let owner = v["owner"].as_str().unwrap_or("");
                    let exp = v["expires_at"].as_str().map(parse_dt).unwrap_or(now);
                    owner != owner_token && exp > now
                });
            if occupied {
                tx.rollback()?;
                return Ok(false);
            }
            let payload = serde_json::json!({
                "owner": owner_token,
                "expires_at": expires.to_rfc3339(),
            })
            .to_string();
            tx.execute(
                "INSERT OR REPLACE INTO kv_state (k, v, updated_at) VALUES (?1, ?2, ?3)",
                params![DREAM_KV_LEASE_KEY, payload, now.to_rfc3339()],
            )?;
            tx.commit()?;
            Ok(true)
        })
    }

    pub fn release_dream_lease(&self, owner_token: &str) -> Result<()> {
        self.with_conn(|conn| {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT v FROM kv_state WHERE k = ?1",
                    params![DREAM_KV_LEASE_KEY],
                    |r| r.get(0),
                )
                .optional()?;
            let owns = existing
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .map_or(false, |v| v["owner"].as_str() == Some(owner_token));
            if owns {
                conn.execute("DELETE FROM kv_state WHERE k = ?1", params![DREAM_KV_LEASE_KEY])?;
            }
            Ok(())
        })
    }

    pub fn is_dream_lease_active(&self) -> bool {
        let now = Utc::now();
        self.get_kv(DREAM_KV_LEASE_KEY)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .map_or(false, |v| {
                v["expires_at"].as_str().map(parse_dt).map_or(false, |e| e > now)
            })
    }

    /// Lists idle sessions updated after `since_ts` that have at least `min_events` events.
    pub fn list_sessions_for_dream(
        &self,
        since_ts: Option<&str>,
        min_events: usize,
        limit: usize,
    ) -> Result<Vec<Session>> {
        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id,
                        s.model_override, s.permission_mode, s.status, s.head_event_id,
                        s.frozen_system_hash, s.workspace_dir, s.parent_session_id, s.agent_role,
                        s.agent_depth, s.worktree_path, s.worktree_branch, s.total_tokens,
                        s.total_cost_usd, s.created_at, s.updated_at
                 FROM sessions s
                 WHERE s.status = 'idle'
                   AND s.parent_session_id IS NULL
                   AND (SELECT COUNT(*) FROM session_events e WHERE e.session_id = s.id AND e.role != 'system') >= ?",
            );
            let mut binds: Vec<rusqlite::types::Value> =
                vec![rusqlite::types::Value::Integer(min_events as i64)];
            if let Some(ts) = since_ts.map(str::trim).filter(|s| !s.is_empty()) {
                sql.push_str(" AND s.updated_at > ?");
                binds.push(rusqlite::types::Value::Text(ts.to_string()));
            }
            sql.push_str(" ORDER BY s.updated_at ASC LIMIT ?");
            binds.push(rusqlite::types::Value::Integer(limit.clamp(1, 50) as i64));

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(binds), row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn insert_dream_run(&self, rec: &DreamRunRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO dream_runs
                 (id, trigger, status, model, sessions_scanned, memories_added, memories_merged, memories_archived, docs_patched, skills_proposed, diff_preview, detail, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    rec.id,
                    rec.trigger,
                    rec.status,
                    rec.model,
                    rec.sessions_scanned as i64,
                    rec.memories_added as i64,
                    rec.memories_merged as i64,
                    rec.memories_archived as i64,
                    rec.docs_patched as i64,
                    rec.skills_proposed as i64,
                    rec.diff_preview,
                    rec.detail,
                    rec.started_at.to_rfc3339(),
                    rec.finished_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn finish_dream_run(&self, rec: &DreamRunRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE dream_runs
                 SET status = ?1, model = ?2, sessions_scanned = ?3, memories_added = ?4,
                     memories_merged = ?5, memories_archived = ?6, docs_patched = ?7,
                     skills_proposed = ?8, diff_preview = ?9, detail = ?10, finished_at = ?11
                 WHERE id = ?12",
                params![
                    rec.status,
                    rec.model,
                    rec.sessions_scanned as i64,
                    rec.memories_added as i64,
                    rec.memories_merged as i64,
                    rec.memories_archived as i64,
                    rec.docs_patched as i64,
                    rec.skills_proposed as i64,
                    rec.diff_preview,
                    rec.detail,
                    rec.finished_at.map(|t| t.to_rfc3339()),
                    rec.id,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_dream_runs(&self, limit: usize) -> Result<Vec<DreamRunRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, trigger, status, model, sessions_scanned, memories_added, memories_merged,
                        memories_archived, docs_patched, skills_proposed, diff_preview, detail, started_at, finished_at
                 FROM dream_runs ORDER BY started_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit.clamp(1, 50) as i64], |r| {
                Ok(DreamRunRecord {
                    id: r.get(0)?,
                    trigger: r.get(1)?,
                    status: r.get(2)?,
                    model: r.get(3)?,
                    sessions_scanned: r.get::<_, i64>(4)?.max(0) as usize,
                    memories_added: r.get::<_, i64>(5)?.max(0) as usize,
                    memories_merged: r.get::<_, i64>(6)?.max(0) as usize,
                    memories_archived: r.get::<_, i64>(7)?.max(0) as usize,
                    docs_patched: r.get::<_, i64>(8)?.max(0) as usize,
                    skills_proposed: r.get::<_, i64>(9).unwrap_or(0).max(0) as usize,
                    diff_preview: r.get(10)?,
                    detail: r.get(11)?,
                    started_at: parse_dt(&r.get::<_, String>(12)?),
                    finished_at: r.get::<_, Option<String>>(13)?.map(|s| parse_dt(&s)),
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    // ─── Cron & Heartbeats ─────────────────────────────────────────────────────

    pub fn upsert_cron_job(&self, job: &CronJob) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO cron_jobs
                 (id, name, cron_expr, kind, prompt, script_path, target_channel, target_chat_id, target_thread_id, enabled, next_run_at, last_run_at, last_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    job.id,
                    job.name,
                    job.cron_expr,
                    job.kind.as_str(),
                    job.prompt,
                    job.script_path,
                    job.target_channel,
                    job.target_chat_id,
                    job.target_thread_id,
                    if job.enabled { 1 } else { 0 },
                    job.next_run_at.map(|t| t.to_rfc3339()),
                    job.last_run_at.map(|t| t.to_rfc3339()),
                    job.last_status,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_cron_job(&self, id: &str) -> Result<Option<CronJob>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, name, cron_expr, kind, prompt, script_path, target_channel, target_chat_id, target_thread_id, enabled, next_run_at, last_run_at, last_status
                 FROM cron_jobs WHERE id = ?1",
                params![id],
                row_to_cron,
            )
            .optional()
        })
    }

    pub fn list_cron_jobs(&self) -> Result<Vec<CronJob>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, cron_expr, kind, prompt, script_path, target_channel, target_chat_id, target_thread_id, enabled, next_run_at, last_run_at, last_status
                 FROM cron_jobs ORDER BY name ASC",
            )?;
            let rows = stmt.query_map([], row_to_cron)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_cron_job(&self, id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| conn.execute("DELETE FROM cron_jobs WHERE id = ?1", params![id]))?;
        Ok(n > 0)
    }

    // ─── UI Profiles (Deep Customization Engine) ───────────────────────────────

    pub fn get_active_ui_profile(&self) -> Result<UiProfile> {
        let opt = self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, name, is_active, config_json, updated_at FROM ui_profiles WHERE is_active = 1 LIMIT 1",
                [],
                row_to_ui_profile,
            )
            .optional()
        })?;
        match opt {
            Some(p) => Ok(p),
            None => Ok(UiProfile {
                id: "default".into(),
                name: "Obsidian Zinc (Default)".into(),
                is_active: true,
                config: UiCustomization::default(),
                updated_at: Utc::now(),
            }),
        }
    }

    pub fn list_ui_profiles(&self) -> Result<Vec<UiProfile>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, is_active, config_json, updated_at FROM ui_profiles ORDER BY is_active DESC, updated_at DESC",
            )?;
            let rows = stmt.query_map([], row_to_ui_profile)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn save_ui_profile(&self, profile: &UiProfile) -> Result<()> {
        let json = serde_json::to_string(&profile.config)?;
        self.with_conn(|conn| {
            if profile.is_active {
                conn.execute("UPDATE ui_profiles SET is_active = 0", [])?;
            }
            conn.execute(
                "INSERT OR REPLACE INTO ui_profiles (id, name, is_active, config_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    profile.id,
                    profile.name,
                    if profile.is_active { 1 } else { 0 },
                    json,
                    profile.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn activate_ui_profile(&self, id: &str) -> Result<bool> {
        let now = Utc::now().to_rfc3339();
        let n = self.with_conn(|conn| {
            conn.execute("UPDATE ui_profiles SET is_active = 0", [])?;
            conn.execute(
                "UPDATE ui_profiles SET is_active = 1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )
        })?;
        Ok(n > 0)
    }

    pub fn delete_ui_profile(&self, id: &str) -> Result<bool> {
        if id == "default" {
            return Ok(false);
        }
        let n = self.with_conn(|conn| conn.execute("DELETE FROM ui_profiles WHERE id = ?1", params![id]))?;
        self.ensure_default_ui_profile()?;
        Ok(n > 0)
    }

    // ─── Usage & Cost Ledger ───────────────────────────────────────────────────

    pub fn record_usage(&self, session_id: &str, provider: &str, model: &str, u: &UsageStats) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO usage_ledger (id, session_id, provider, model, prompt_tokens, completion_tokens, cache_read_tokens, cost_usd, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    new_id(),
                    session_id,
                    provider,
                    model,
                    u.prompt_tokens as i64,
                    u.completion_tokens as i64,
                    u.cache_read_tokens as i64,
                    u.cost_usd,
                    now,
                ],
            )?;
            Ok(())
        })
    }

    pub fn daily_cost_usd(&self) -> Result<f64> {
        let day_start = Utc::now().format("%Y-%m-%dT00:00:00").to_string();
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_ledger WHERE created_at >= ?1",
                params![day_start],
                |r| r.get(0),
            )
        })
    }

    pub fn total_usage_summary(&self) -> Result<(f64, f64)> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0), COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(cache_read_tokens), 0) FROM usage_ledger",
                [],
                |r| {
                    let cost: f64 = r.get(0)?;
                    let prompt: i64 = r.get(1)?;
                    let cached: i64 = r.get(2)?;
                    let pct = if prompt > 0 {
                        (cached as f64 / prompt as f64) * 100.0
                    } else {
                        0.0
                    };
                    Ok((cost, pct))
                },
            )
        })
    }

    /// Aggregates live Dashboard analytics in bounded SQL queries (zero full-table memory loads).
    pub fn dashboard_db_metrics(&self, days: i64) -> Result<SqliteDashboardMetrics> {
        let window_days = days.clamp(7, 366);
        let cutoff = (Utc::now() - Duration::days(window_days))
            .format("%Y-%m-%dT00:00:00")
            .to_string();

        self.with_conn(|conn| {
            let mut by_day: BTreeMap<String, DailyActivityPoint> = BTreeMap::new();

            // 1. Daily session events (turns, tool calls, tokens, cost)
            {
                let mut stmt = conn.prepare(
                    "SELECT substr(created_at, 1, 10) AS day,
                            COUNT(*) AS ev_cnt,
                            SUM(CASE WHEN role = 'user' THEN 1 ELSE 0 END) AS turns,
                            SUM(CASE WHEN role = 'tool' THEN 1 ELSE 0 END) AS tools,
                            COALESCE(SUM(tokens_in + tokens_out), 0) AS toks,
                            COALESCE(SUM(cache_read_tokens), 0) AS cached,
                            COALESCE(SUM(cost_usd), 0.0) AS cost
                     FROM session_events
                     WHERE created_at >= ?1
                     GROUP BY day
                     ORDER BY day ASC",
                )?;
                let mut rows = stmt.query(params![&cutoff])?;
                while let Some(r) = rows.next()? {
                    let date: String = r.get(0)?;
                    if date.len() != 10 {
                        continue;
                    }
                    by_day.insert(
                        date.clone(),
                        DailyActivityPoint {
                            date,
                            events: r.get::<_, i64>(1)?.max(0) as u64,
                            turns: r.get::<_, i64>(2)?.max(0) as u64,
                            tool_calls: r.get::<_, i64>(3)?.max(0) as u64,
                            tokens: r.get::<_, i64>(4)?.max(0) as u64,
                            cache_read_tokens: r.get::<_, i64>(5)?.max(0) as u64,
                            cost_usd: r.get::<_, f64>(6)?.max(0.0),
                        },
                    );
                }
            }

            // 2. Merge usage_ledger daily tokens/cache/cost (captures subagents/cron/compaction too)
            {
                let mut stmt = conn.prepare(
                    "SELECT substr(created_at, 1, 10) AS day,
                            COUNT(*) AS calls,
                            COALESCE(SUM(prompt_tokens + completion_tokens), 0) AS toks,
                            COALESCE(SUM(cache_read_tokens), 0) AS cached,
                            COALESCE(SUM(cost_usd), 0.0) AS cost
                     FROM usage_ledger
                     WHERE created_at >= ?1
                     GROUP BY day
                     ORDER BY day ASC",
                )?;
                let mut rows = stmt.query(params![&cutoff])?;
                while let Some(r) = rows.next()? {
                    let date: String = r.get(0)?;
                    if date.len() != 10 {
                        continue;
                    }
                    let calls = r.get::<_, i64>(1)?.max(0) as u64;
                    let toks = r.get::<_, i64>(2)?.max(0) as u64;
                    let cached = r.get::<_, i64>(3)?.max(0) as u64;
                    let cost = r.get::<_, f64>(4)?.max(0.0);
                    let entry = by_day
                        .entry(date.clone())
                        .or_insert_with(|| DailyActivityPoint {
                            date,
                            ..Default::default()
                        });
                    entry.tokens = entry.tokens.max(toks);
                    entry.cache_read_tokens = entry.cache_read_tokens.max(cached);
                    entry.cost_usd = entry.cost_usd.max(cost);
                    if entry.events == 0 {
                        entry.events = calls;
                    }
                }
            }

            // 3. Top tools from tool events via native SQLite json_extract (zero regex)
            let mut top_tools = Vec::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT COALESCE(json_extract(content_json, '$.tool_name'), 'tool') AS tname,
                            COUNT(*) AS cnt,
                            SUM(CASE WHEN COALESCE(json_extract(content_json, '$.is_error'), 0) != 0 THEN 1 ELSE 0 END) AS errs,
                            COALESCE(AVG(CAST(COALESCE(json_extract(content_json, '$.duration_ms'), 0) AS REAL)), 0.0) AS avg_ms
                     FROM session_events
                     WHERE role = 'tool'
                     GROUP BY tname
                     ORDER BY cnt DESC
                     LIMIT 12",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    top_tools.push(ToolUsageStat {
                        name: r.get(0)?,
                        calls: r.get::<_, i64>(1)?.max(0) as u64,
                        errors: r.get::<_, i64>(2)?.max(0) as u64,
                        avg_duration_ms: r.get::<_, f64>(3)?.max(0.0).round() as u64,
                    });
                }
            }

            // 4. Top models from usage_ledger
            let mut top_models = Vec::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT provider, model,
                            COUNT(*) AS calls,
                            COALESCE(SUM(prompt_tokens + completion_tokens), 0) AS toks,
                            COALESCE(SUM(cache_read_tokens), 0) AS cached,
                            COALESCE(SUM(cost_usd), 0.0) AS cost
                     FROM usage_ledger
                     GROUP BY provider, model
                     ORDER BY toks DESC, calls DESC
                     LIMIT 10",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    top_models.push(ModelUsageStat {
                        provider: r.get(0)?,
                        model: r.get(1)?,
                        calls: r.get::<_, i64>(2)?.max(0) as u64,
                        tokens: r.get::<_, i64>(3)?.max(0) as u64,
                        cache_read_tokens: r.get::<_, i64>(4)?.max(0) as u64,
                        cost_usd: r.get::<_, f64>(5)?.max(0.0),
                    });
                }
            }

            // 5. Sessions by channel
            let mut channels = BTreeMap::new();
            {
                let mut stmt = conn.prepare(
                    "SELECT channel, COUNT(*) FROM sessions GROUP BY channel ORDER BY COUNT(*) DESC",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    channels.insert(r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64);
                }
            }

            // 6. Aggregate totals & entity counts
            let count_query = |sql: &str| -> u64 {
                conn.query_row(sql, [], |r| r.get::<_, i64>(0))
                    .unwrap_or(0)
                    .max(0) as u64
            };
            let counts = DashboardCounts {
                sessions: count_query("SELECT COUNT(*) FROM sessions"),
                messages: count_query("SELECT COUNT(*) FROM session_events WHERE role IN ('user', 'assistant')"),
                tool_calls: count_query("SELECT COUNT(*) FROM session_events WHERE role = 'tool'"),
                memories: count_query("SELECT COUNT(*) FROM memory_entries"),
                skills_enabled: 0,
                skills_total: 0,
                cron_jobs: count_query("SELECT COUNT(*) FROM cron_jobs"),
                goal_loops: count_query("SELECT COUNT(*) FROM goal_loops"),
                background_jobs: count_query("SELECT COUNT(*) FROM background_jobs"),
                subagents: count_query("SELECT COUNT(*) FROM subagents"),
                checkpoints: count_query("SELECT COUNT(*) FROM workspace_checkpoints"),
                mcp_servers: 0,
                paired_peers: count_query("SELECT COUNT(*) FROM gateway_peers"),
            };

            let (ledger_tokens, ledger_cached): (u64, u64) = conn
                .query_row(
                    "SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0), COALESCE(SUM(cache_read_tokens), 0) FROM usage_ledger",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, i64>(1)?.max(0) as u64)),
                )
                .unwrap_or((0, 0));
            let (ev_tokens, ev_cached): (u64, u64) = conn
                .query_row(
                    "SELECT COALESCE(SUM(tokens_in + tokens_out), 0), COALESCE(SUM(cache_read_tokens), 0) FROM session_events",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, i64>(1)?.max(0) as u64)),
                )
                .unwrap_or((0, 0));

            // 7. Compute consecutive active day streak ending today or yesterday
            let active_days = by_day.values().filter(|d| d.events > 0 || d.tokens > 0).count() as u32;
            let today = Utc::now().date_naive();
            let mut streak = 0u32;
            let mut cursor = today;
            let today_key = today.format("%Y-%m-%d").to_string();
            if !by_day.get(&today_key).map(|d| d.events > 0 || d.tokens > 0).unwrap_or(false) {
                cursor = today - Duration::days(1);
            }
            for _ in 0..366 {
                let key = cursor.format("%Y-%m-%d").to_string();
                if by_day.get(&key).map(|d| d.events > 0 || d.tokens > 0).unwrap_or(false) {
                    streak += 1;
                    cursor -= Duration::days(1);
                } else {
                    break;
                }
            }

            Ok(SqliteDashboardMetrics {
                counts,
                daily_activity: by_day.into_values().collect(),
                top_tools,
                top_models,
                channels,
                streak_days: streak,
                active_days,
                total_tokens: ledger_tokens.max(ev_tokens),
                total_cache_read_tokens: ledger_cached.max(ev_cached),
            })
        })
    }

    // ─── KV State ──────────────────────────────────────────────────────────────

    pub fn get_kv(&self, key: &str) -> Result<Option<String>> {
        self.with_conn(|conn| {
            conn.query_row("SELECT v FROM kv_state WHERE k = ?1", params![key], |r| r.get(0))
                .optional()
        })
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO kv_state (k, v, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, now],
            )?;
            Ok(())
        })
    }

    // ─── Categorized Persistent Debug Logs (Indexed Ring Table) ────────────────

    pub fn insert_log(&self, entry: &LogEntry) -> Result<()> {
        let id = new_id();
        let payload_str = entry
            .payload
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let should_prune =
            (self.log_inserts.fetch_add(1, Ordering::Relaxed) + 1) % LOG_SQLITE_PRUNE_INTERVAL == 0;

        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO logs (id, ts, level, category, target, session_id, turn_id, call_id, message, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    entry.timestamp.to_rfc3339(),
                    entry.level,
                    entry.category,
                    entry.target,
                    entry.session_id,
                    entry.turn_id,
                    entry.call_id,
                    entry.message,
                    payload_str,
                ],
            )?;
            if should_prune {
                conn.execute(
                    "DELETE FROM logs WHERE id NOT IN (SELECT id FROM logs ORDER BY ts DESC, id DESC LIMIT ?1)",
                    params![LOG_SQLITE_MAX_ROWS as i64],
                )?;
            }
            Ok(())
        })
    }

    pub fn query_logs(&self, filter: &LogFilter) -> Result<Vec<LogEntry>> {
        let limit = filter.limit.unwrap_or(120).clamp(1, 1000) as i64;
        let category = filter
            .category
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let channel = filter
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let level = filter
            .level
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
            .map(str::to_ascii_uppercase);
        let session_id = filter
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let search_like = filter
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|q| format!("%{}%", q.to_ascii_lowercase()));

        let mut rows = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT ts, level, category, target, session_id, turn_id, call_id, message, payload_json
                 FROM logs
                 WHERE (?1 IS NULL OR category = ?1)
                   AND (?2 IS NULL OR target = ?2)
                   AND (?3 IS NULL OR level = ?3)
                   AND (?4 IS NULL OR session_id = ?4)
                   AND (?5 IS NULL OR LOWER(message) LIKE ?5 OR LOWER(COALESCE(payload_json, '')) LIKE ?5)
                 ORDER BY ts DESC, id DESC
                 LIMIT ?6",
            )?;
            let mapped = stmt.query_map(
                params![category, channel, level, session_id, search_like, limit],
                row_to_log_entry,
            )?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        rows.reverse();
        Ok(rows)
    }

    pub fn log_category_counts(&self) -> Result<BTreeMap<String, u64>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT category, COUNT(*) FROM logs GROUP BY category ORDER BY category ASC")?;
            let rows = stmt.query_map([], |r| {
                let cat: String = r.get(0)?;
                let cnt: i64 = r.get(1)?;
                Ok((cat, cnt.max(0) as u64))
            })?;
            let mut map = BTreeMap::new();
            for item in rows {
                let (k, v) = item?;
                map.insert(k, v);
            }
            Ok(map)
        })
    }

    // ─── Async Background Jobs ─────────────────────────────────────────────────

    pub fn upsert_job(&self, job: &BackgroundJobRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO background_jobs
                 (id, session_id, kind, label, command, workspace_dir, pid, status, exit_code, log_path, bytes_written, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    job.id,
                    job.session_id,
                    job.kind.as_str(),
                    job.label,
                    job.command,
                    job.workspace_dir,
                    job.pid.map(|p| p as i64),
                    job.status.as_str(),
                    job.exit_code,
                    job.log_path,
                    job.bytes_written as i64,
                    job.started_at.to_rfc3339(),
                    job.finished_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_job(&self, id: &str) -> Result<Option<BackgroundJobRecord>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, session_id, kind, label, command, workspace_dir, pid, status, exit_code, log_path, bytes_written, started_at, finished_at
                 FROM background_jobs WHERE id = ?1",
                params![id],
                row_to_job,
            )
            .optional()
        })
    }

    pub fn list_jobs(
        &self,
        session_id: Option<&str>,
        kind: Option<TaskKind>,
        limit: usize,
    ) -> Result<Vec<BackgroundJobRecord>> {
        let cap = limit.clamp(1, JOB_MAX_HISTORY) as i64;
        let kind_str = kind.map(TaskKind::as_str);
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, kind, label, command, workspace_dir, pid, status, exit_code, log_path, bytes_written, started_at, finished_at
                 FROM background_jobs
                 WHERE (?1 IS NULL OR session_id = ?1)
                   AND (?2 IS NULL OR kind = ?2)
                 ORDER BY started_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![session_id, kind_str, cap], row_to_job)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn update_job_meta(
        &self,
        id: &str,
        label: Option<String>,
        kind: Option<TaskKind>,
    ) -> Result<Option<BackgroundJobRecord>> {
        let Some(mut rec) = self.get_job(id)? else {
            return Ok(None);
        };
        if let Some(lbl) = label {
            let trimmed = lbl.trim();
            rec.label = match trimmed.is_empty() {
                true => None,
                false => Some(trimmed.to_string()),
            };
        }
        if let Some(k) = kind {
            rec.kind = k;
        }
        self.upsert_job(&rec)?;
        Ok(Some(rec))
    }

    pub fn delete_job(&self, id: &str) -> Result<Option<BackgroundJobRecord>> {
        let existing = self.get_job(id)?;
        if existing.is_some() {
            self.with_conn(|conn| {
                conn.execute("DELETE FROM background_jobs WHERE id = ?1", params![id])
            })?;
        }
        Ok(existing)
    }

    pub fn prune_finished_jobs(&self, kind: Option<TaskKind>) -> Result<Vec<BackgroundJobRecord>> {
        let running_str = BackgroundJobStatus::Running.as_str();
        let kind_str = kind.map(TaskKind::as_str);
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, kind, label, command, workspace_dir, pid, status, exit_code, log_path, bytes_written, started_at, finished_at
                 FROM background_jobs
                 WHERE status != ?1 AND (?2 IS NULL OR kind = ?2)",
            )?;
            let removed = stmt
                .query_map(params![running_str, kind_str], row_to_job)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            conn.execute(
                "DELETE FROM background_jobs WHERE status != ?1 AND (?2 IS NULL OR kind = ?2)",
                params![running_str, kind_str],
            )?;
            Ok(removed)
        })
    }

    pub fn recover_interrupted_jobs(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let interrupted_str = BackgroundJobStatus::Interrupted.as_str();
        let running_str = BackgroundJobStatus::Running.as_str();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE background_jobs
                 SET status = ?1, finished_at = COALESCE(finished_at, ?2)
                 WHERE status = ?3",
                params![interrupted_str, now, running_str],
            )
        })
    }

    // ─── Durable SQLite Follow-Up Prompt Queue ─────────────────────────────────

    pub fn enqueue_followup(&self, session_id: &str, text: &str) -> Result<(QueuedPrompt, usize)> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        let item = QueuedPrompt {
            id: new_id(),
            session_id: session_id.to_string(),
            text: text.to_string(),
            status: zenko_core::QueuedPromptStatus::Queued,
            created_at: Utc::now(),
        };
        let inner = self.with_conn(|conn| {
            let existing: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_queue WHERE session_id = ?1 AND status = ?2",
                params![session_id, queued_str],
                |r| r.get(0),
            )?;
            if existing as usize >= SESSION_QUEUE_MAX_ITEMS {
                return Ok(Err(ZenkoError::PolicyDenied(format!(
                    "Session follow-up queue is full (max {SESSION_QUEUE_MAX_ITEMS})."
                ))));
            }
            conn.execute(
                "INSERT INTO session_queue (id, session_id, text, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    item.id,
                    item.session_id,
                    item.text,
                    item.status.as_str(),
                    item.created_at.to_rfc3339(),
                ],
            )?;
            Ok(Ok((existing as usize) + 1))
        })?;
        let new_count = inner?;
        Ok((item, new_count))
    }

    pub fn list_queued_followups(&self, session_id: &str) -> Result<Vec<QueuedPrompt>> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, text, status, created_at
                 FROM session_queue
                 WHERE session_id = ?1 AND status = ?2
                 ORDER BY created_at ASC, id ASC",
            )?;
            let rows = stmt.query_map(params![session_id, queued_str], row_to_queued_prompt)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn pop_next_followup(&self, session_id: &str) -> Result<Option<QueuedPrompt>> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        let dispatched_str = zenko_core::QueuedPromptStatus::Dispatched.as_str();
        self.with_conn(|conn| {
            let next: Option<QueuedPrompt> = conn
                .query_row(
                    "SELECT id, session_id, text, status, created_at
                     FROM session_queue
                     WHERE session_id = ?1 AND status = ?2
                     ORDER BY created_at ASC, id ASC
                     LIMIT 1",
                    params![session_id, queued_str],
                    row_to_queued_prompt,
                )
                .optional()?;
            if let Some(ref item) = next {
                conn.execute(
                    "UPDATE session_queue SET status = ?1 WHERE id = ?2",
                    params![dispatched_str, item.id],
                )?;
            }
            Ok(next)
        })
    }

    pub fn cancel_queued_followup(&self, session_id: &str, queue_id: &str) -> Result<bool> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        let cancelled_str = zenko_core::QueuedPromptStatus::Cancelled.as_str();
        let n = self.with_conn(|conn| {
            conn.execute(
                "UPDATE session_queue SET status = ?1
                 WHERE id = ?2 AND session_id = ?3 AND status = ?4",
                params![cancelled_str, queue_id, session_id, queued_str],
            )
        })?;
        Ok(n > 0)
    }

    pub fn pop_queued_followup_by_id(
        &self,
        session_id: &str,
        queue_id: &str,
    ) -> Result<Option<QueuedPrompt>> {
        let queued_str = zenko_core::QueuedPromptStatus::Queued.as_str();
        let dispatched_str = zenko_core::QueuedPromptStatus::Dispatched.as_str();
        self.with_conn(|conn| {
            let found: Option<QueuedPrompt> = conn
                .query_row(
                    "SELECT id, session_id, text, status, created_at
                     FROM session_queue
                     WHERE id = ?1 AND session_id = ?2 AND status = ?3",
                    params![queue_id, session_id, queued_str],
                    row_to_queued_prompt,
                )
                .optional()?;
            if let Some(mut item) = found {
                let updated = conn.execute(
                    "UPDATE session_queue SET status = ?1
                     WHERE id = ?2 AND session_id = ?3 AND status = ?4",
                    params![dispatched_str, queue_id, session_id, queued_str],
                )?;
                if updated > 0 {
                    item.status = zenko_core::QueuedPromptStatus::Dispatched;
                    return Ok(Some(item));
                }
            }
            Ok(None)
        })
    }

    // ─── Phase 5: Async Subagents & Inter-Agent Mailbox ────────────────────────

    pub fn upsert_subagent(&self, rec: &SubagentRecord) -> Result<()> {
        let output_schema_json = rec
            .output_schema
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let structured_json = rec
            .structured_output
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let files_json = serde_json::to_string(&rec.changed_files)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO subagents
                 (session_id, parent_session_id, role, depth, goal, status, workspace_mode, workspace_dir,
                  worktree_branch, worktree_base_commit, worktree_dirty_baseline, model, summary,
                  output_schema_json, structured_output_json, diff_stat, changed_files_json,
                  tokens_used, cost_usd, resume_count, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                params![
                    rec.id,
                    rec.parent_session_id,
                    rec.role,
                    rec.depth as i64,
                    rec.goal,
                    rec.status.as_str(),
                    rec.workspace_mode.as_str(),
                    rec.workspace_dir,
                    rec.worktree_branch,
                    rec.worktree_base_commit,
                    match rec.worktree_dirty_baseline {
                        true => 1i64,
                        false => 0i64,
                    },
                    rec.model,
                    rec.summary,
                    output_schema_json,
                    structured_json,
                    rec.diff_stat,
                    files_json,
                    rec.tokens_used as i64,
                    rec.cost_usd,
                    rec.resume_count as i64,
                    rec.started_at.to_rfc3339(),
                    rec.finished_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_subagent(&self, session_id: &str) -> Result<Option<SubagentRecord>> {
        let mut rec = self.with_conn(|conn| {
            conn.query_row(
                "SELECT session_id, parent_session_id, role, depth, goal, status, workspace_mode, workspace_dir,
                        worktree_branch, worktree_base_commit, worktree_dirty_baseline, model, summary,
                        output_schema_json, structured_output_json, diff_stat, changed_files_json,
                        tokens_used, cost_usd, resume_count, started_at, finished_at
                 FROM subagents WHERE session_id = ?1",
                params![session_id],
                row_to_subagent,
            )
            .optional()
        })?;
        if let Some(ref mut single) = rec {
            self.hydrate_subagent_tree_rollups(std::slice::from_mut(single))?;
        }
        Ok(rec)
    }

    pub fn list_subagents(
        &self,
        parent_session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SubagentRecord>> {
        let cap = limit.clamp(1, SUBAGENT_LIST_DEFAULT_LIMIT.max(100)) as i64;
        let mut rows = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id, parent_session_id, role, depth, goal, status, workspace_mode, workspace_dir,
                        worktree_branch, worktree_base_commit, worktree_dirty_baseline, model, summary,
                        output_schema_json, structured_output_json, diff_stat, changed_files_json,
                        tokens_used, cost_usd, resume_count, started_at, finished_at
                 FROM subagents
                 WHERE (?1 IS NULL OR parent_session_id = ?1)
                 ORDER BY started_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![parent_session_id, cap], row_to_subagent)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;
        if !rows.is_empty() {
            self.hydrate_subagent_tree_rollups(&mut rows)?;
        }
        Ok(rows)
    }

    /// Computes Prime-Agent hierarchical subtree rollups (`tree_tokens`, `tree_cost_usd`,
    /// `descendant_count`) across all subagent rows in a single flat query + BFS (zero N+1 SQL).
    fn hydrate_subagent_tree_rollups(&self, records: &mut [SubagentRecord]) -> Result<()> {
        let edges: Vec<(String, String, u64, f64)> = self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id, parent_session_id, tokens_used, cost_usd FROM subagents",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?.max(0) as u64,
                    r.get::<_, f64>(3)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })?;

        let mut children_by_parent: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut stats_by_id: HashMap<&str, (u64, f64)> = HashMap::with_capacity(edges.len());
        for (sid, pid, toks, cost) in &edges {
            children_by_parent
                .entry(pid.as_str())
                .or_default()
                .push(sid.as_str());
            stats_by_id.insert(sid.as_str(), (*toks, *cost));
        }

        for rec in records.iter_mut() {
            let mut total_tok = rec.tokens_used;
            let mut total_cost = rec.cost_usd;
            let mut desc_count = 0usize;
            let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
            let mut queue: std::collections::VecDeque<&str> = std::collections::VecDeque::new();
            visited.insert(rec.id.as_str());
            queue.push_back(rec.id.as_str());
            while let Some(cur) = queue.pop_front() {
                if let Some(kids) = children_by_parent.get(cur) {
                    for &kid in kids {
                        if visited.insert(kid) {
                            desc_count += 1;
                            if let Some(&(kt, kc)) = stats_by_id.get(kid) {
                                total_tok = total_tok.saturating_add(kt);
                                total_cost += kc;
                            }
                            queue.push_back(kid);
                        }
                    }
                }
            }
            rec.tree_tokens = total_tok;
            rec.tree_cost_usd = total_cost;
            rec.descendant_count = desc_count;
        }
        Ok(())
    }

    /// Returns directory names under `<data_dir>/worktrees/` that belong to live or reviewable
    /// subagent records (excludes typed `SubagentStatus::Interrupted` crash orphans — zero string sniffing).
    pub fn list_preserved_worktree_ids(&self) -> Result<std::collections::HashSet<String>> {
        let interrupted_str = SubagentStatus::Interrupted.as_str();
        let wt_mode = SubagentWorkspaceMode::Worktree.as_str();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT session_id FROM subagents WHERE workspace_mode = ?1 AND status != ?2",
            )?;
            let rows = stmt.query_map(params![wt_mode, interrupted_str], |r| r.get::<_, String>(0))?;
            Ok(rows.flatten().collect())
        })
    }

    /// Breadth-first traversal collecting all descendant subagent session IDs rooted under `root_session_id`.
    pub fn list_descendant_session_ids(&self, root_session_id: &str) -> Result<Vec<String>> {
        self.with_conn(|conn| {
            let mut out = Vec::new();
            let mut frontier = std::collections::VecDeque::new();
            frontier.push_back(root_session_id.to_string());
            let mut visited = std::collections::HashSet::new();
            visited.insert(root_session_id.to_string());
            let mut stmt = conn.prepare(
                "SELECT session_id FROM subagents WHERE parent_session_id = ?1",
            )?;
            while let Some(cur) = frontier.pop_front() {
                let children: Vec<String> = stmt
                    .query_map(params![cur], |r| r.get(0))?
                    .filter_map(std::result::Result::ok)
                    .collect();
                for child in children {
                    if visited.insert(child.clone()) {
                        out.push(child.clone());
                        frontier.push_back(child);
                    }
                }
            }
            Ok(out)
        })
    }

    pub fn recover_interrupted_subagents(&self) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let interrupted_str = SubagentStatus::Interrupted.as_str();
        let running_str = SubagentStatus::Running.as_str();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE subagents
                 SET status = ?1,
                     summary = COALESCE(summary, 'Subagent interrupted by daemon restart.'),
                     finished_at = COALESCE(finished_at, ?2)
                 WHERE status = ?3",
                params![interrupted_str, now, running_str],
            )
        })
    }

    pub fn insert_agent_message(
        &self,
        from_session_id: &str,
        to_session_id: &str,
        body: &str,
    ) -> Result<AgentMailMessage> {
        let now = Utc::now();
        let msg = AgentMailMessage {
            id: new_id(),
            from_session_id: from_session_id.to_string(),
            to_session_id: to_session_id.to_string(),
            body: body.to_string(),
            read_at: None,
            created_at: now,
            delivery_action: None,
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_messages (id, from_session_id, to_session_id, body, read_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                params![
                    msg.id,
                    msg.from_session_id,
                    msg.to_session_id,
                    msg.body,
                    msg.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })?;
        Ok(msg)
    }

    /// Marks a specific mailbox message as read (used when directly resuming an idle child with that message).
    pub fn mark_agent_message_read(&self, id: &str) -> Result<()> {
        let now_str = Utc::now().to_rfc3339();
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE agent_messages SET read_at = COALESCE(read_at, ?1) WHERE id = ?2",
                params![now_str, id],
            )?;
            Ok(())
        })
    }

    /// Atomically reads and marks unread mailbox messages delivered to `to_session_id`.
    pub fn drain_unread_agent_messages(
        &self,
        to_session_id: &str,
    ) -> Result<Vec<AgentMailMessage>> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let cap = SUBAGENT_MAILBOX_MAX_MESSAGES as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, from_session_id, to_session_id, body, read_at, created_at
                 FROM agent_messages
                 WHERE to_session_id = ?1 AND read_at IS NULL
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?2",
            )?;
            let mut msgs = stmt
                .query_map(params![to_session_id, cap], row_to_agent_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            for msg in &mut msgs {
                conn.execute(
                    "UPDATE agent_messages SET read_at = ?1 WHERE id = ?2",
                    params![now_str, msg.id],
                )?;
                msg.read_at = Some(now);
            }
            Ok(msgs)
        })
    }

    pub fn list_agent_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<AgentMailMessage>> {
        let cap = limit.clamp(1, 100) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, from_session_id, to_session_id, body, read_at, created_at
                 FROM agent_messages
                 WHERE from_session_id = ?1 OR to_session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let mut rows = stmt
                .query_map(params![session_id, cap], row_to_agent_message)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.reverse();
            Ok(rows)
        })
    }

    // ─── Phase 6: Durable SQLite Session Task DAG ──────────────────────────────

    pub fn upsert_session_task(&self, task: &SessionTask) -> Result<()> {
        let deps_json = serde_json::to_string(&task.depends_on)?;
        self.with_conn(|conn| {
            let existing: i64 = conn.query_row(
                "SELECT COUNT(*) FROM session_tasks WHERE session_id = ?1 AND id != ?2",
                params![task.session_id, task.id],
                |r| r.get(0),
            )?;
            if existing as usize >= SESSION_TASK_MAX_ITEMS {
                return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
                    std::io::Error::other(format!(
                        "session task DAG limit reached (max {SESSION_TASK_MAX_ITEMS})"
                    )),
                )));
            }
            conn.execute(
                "INSERT OR REPLACE INTO session_tasks
                 (id, session_id, branch_event_id, title, description, active_label, status,
                  depends_on_json, owner_agent, verification_cmd, verification_evidence, notes,
                  priority, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    task.id,
                    task.session_id,
                    task.branch_event_id,
                    task.title,
                    task.description,
                    task.active_label,
                    task.status.as_str(),
                    deps_json,
                    task.owner_agent,
                    task.verification_cmd,
                    task.verification_evidence,
                    task.notes,
                    task.priority,
                    task.created_at.to_rfc3339(),
                    task.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn replace_session_tasks(&self, session_id: &str, tasks: &[SessionTask]) -> Result<()> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM session_tasks WHERE session_id = ?1",
                params![session_id],
            )?;
            for task in tasks.iter().take(SESSION_TASK_MAX_ITEMS) {
                let deps_json = serde_json::to_string(&task.depends_on).unwrap_or_else(|_| "[]".into());
                tx.execute(
                    "INSERT INTO session_tasks
                     (id, session_id, branch_event_id, title, description, active_label, status,
                      depends_on_json, owner_agent, verification_cmd, verification_evidence, notes,
                      priority, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        task.id,
                        session_id,
                        task.branch_event_id,
                        task.title,
                        task.description,
                        task.active_label,
                        task.status.as_str(),
                        deps_json,
                        task.owner_agent,
                        task.verification_cmd,
                        task.verification_evidence,
                        task.notes,
                        task.priority,
                        task.created_at.to_rfc3339(),
                        task.updated_at.to_rfc3339(),
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn get_session_task(&self, session_id: &str, task_id: &str) -> Result<Option<SessionTask>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, session_id, branch_event_id, title, description, active_label, status,
                        depends_on_json, owner_agent, verification_cmd, verification_evidence, notes,
                        priority, created_at, updated_at
                 FROM session_tasks
                 WHERE session_id = ?1 AND id = ?2",
                params![session_id, task_id],
                row_to_session_task,
            )
            .optional()
        })
    }

    pub fn list_session_tasks(&self, session_id: &str) -> Result<Vec<SessionTask>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, branch_event_id, title, description, active_label, status,
                        depends_on_json, owner_agent, verification_cmd, verification_evidence, notes,
                        priority, created_at, updated_at
                 FROM session_tasks
                 WHERE session_id = ?1
                 ORDER BY priority DESC, created_at ASC, id ASC",
            )?;
            let rows = stmt.query_map(params![session_id], row_to_session_task)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// ACID atomic task claim for multi-agent coordination: succeeds only when unclaimed or already
    /// owned by `owner_agent` and not in a terminal/blocked state.
    pub fn claim_session_task(
        &self,
        session_id: &str,
        task_id: &str,
        owner_agent: &str,
    ) -> Result<Option<SessionTask>> {
        let now = Utc::now().to_rfc3339();
        let in_prog = SessionTaskStatus::InProgress.as_str();
        let pending = SessionTaskStatus::Pending.as_str();
        let updated = self.with_conn(|conn| {
            conn.execute(
                "UPDATE session_tasks
                 SET owner_agent = ?1, status = ?2, updated_at = ?3
                 WHERE session_id = ?4 AND id = ?5
                   AND status IN (?6, ?2)
                   AND (owner_agent IS NULL OR owner_agent = '' OR owner_agent = ?1)",
                params![owner_agent, in_prog, now, session_id, task_id, pending],
            )
        })?;
        match updated > 0 {
            true => self.get_session_task(session_id, task_id),
            false => Ok(None),
        }
    }

    pub fn delete_session_task(&self, session_id: &str, task_id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM session_tasks WHERE session_id = ?1 AND id = ?2",
                params![session_id, task_id],
            )
        })?;
        Ok(n > 0)
    }

    pub fn clear_session_tasks(&self, session_id: &str) -> Result<usize> {
        self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM session_tasks WHERE session_id = ?1",
                params![session_id],
            )
        })
    }

    // ─── Phase 6: Workspace Checkpoints & Rewind Metadata ──────────────────────

    pub fn insert_checkpoint(&self, cp: &WorkspaceCheckpoint) -> Result<()> {
        let files_json = serde_json::to_string(&cp.changed_files)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO workspace_checkpoints
                 (id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref, post_ref,
                  trigger_tool, label, changed_files_json, is_safety_backup, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    cp.id,
                    cp.session_id,
                    cp.turn_id,
                    cp.event_id,
                    cp.workspace_dir,
                    cp.backend.as_str(),
                    cp.pre_ref,
                    cp.post_ref,
                    cp.trigger_tool,
                    cp.label,
                    files_json,
                    if cp.is_safety_backup { 1i64 } else { 0i64 },
                    cp.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_checkpoint_post_state(
        &self,
        checkpoint_id: &str,
        post_ref: &str,
        changed_files: &[String],
    ) -> Result<()> {
        let files_json = serde_json::to_string(changed_files)?;
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE workspace_checkpoints
                 SET post_ref = ?1, changed_files_json = ?2
                 WHERE id = ?3",
                params![post_ref, files_json, checkpoint_id],
            )?;
            Ok(())
        })
    }

    pub fn get_checkpoint(&self, checkpoint_id: &str) -> Result<Option<WorkspaceCheckpoint>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE id = ?1",
                params![checkpoint_id],
                row_to_checkpoint,
            )
            .optional()
        })
    }

    pub fn latest_checkpoint_for_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<WorkspaceCheckpoint>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE workspace_dir = ?1 AND post_ref IS NOT NULL AND is_safety_backup = 0
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT 1",
                params![workspace_dir],
                row_to_checkpoint,
            )
            .optional()
        })
    }

    /// Returns recent finalized checkpoints (including safety backups) newest-first for exact
    /// per-file Zenko provenance & manual hand-edit detection across chained rewinds.
    pub fn recent_finalized_checkpoints_for_workspace(
        &self,
        workspace_dir: &str,
        limit: usize,
    ) -> Result<Vec<WorkspaceCheckpoint>> {
        let cap = limit.clamp(1, 64) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE workspace_dir = ?1 AND post_ref IS NOT NULL
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![workspace_dir, cap], row_to_checkpoint)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Collects all content-addressed manifest IDs still referenced by surviving `content_blobs`
    /// checkpoints so pruning never deletes shared manifests and can safely mark-and-sweep blobs.
    pub fn list_live_blob_manifest_refs(&self) -> Result<std::collections::HashSet<String>> {
        let backend_blobs = CheckpointBackend::ContentBlobs.as_str();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT pre_ref, post_ref FROM workspace_checkpoints WHERE backend = ?1",
            )?;
            let mut live = std::collections::HashSet::new();
            let rows = stmt.query_map(params![backend_blobs], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })?;
            for row in rows.flatten() {
                if !row.0.is_empty() {
                    live.insert(row.0);
                }
                if let Some(post) = row.1.filter(|s| !s.is_empty()) {
                    live.insert(post);
                }
            }
            Ok(live)
        })
    }

    pub fn list_checkpoints(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<WorkspaceCheckpoint>> {
        let cap = limit.clamp(1, 200) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE (?1 IS NULL OR session_id = ?1)
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![session_id, cap], row_to_checkpoint)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_checkpoint(&self, checkpoint_id: &str) -> Result<bool> {
        let n = self.with_conn(|conn| {
            conn.execute(
                "DELETE FROM workspace_checkpoints WHERE id = ?1",
                params![checkpoint_id],
            )
        })?;
        Ok(n > 0)
    }

    /// Prunes excess checkpoints beyond `max_per_session` for a session and returns removed rows.
    pub fn prune_session_checkpoints(
        &self,
        session_id: &str,
        max_per_session: usize,
    ) -> Result<Vec<WorkspaceCheckpoint>> {
        let keep = max_per_session.max(1) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE session_id = ?1
                 ORDER BY created_at DESC
                 LIMIT -1 OFFSET ?2",
            )?;
            let stale = stmt
                .query_map(params![session_id, keep], row_to_checkpoint)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            for cp in &stale {
                conn.execute(
                    "DELETE FROM workspace_checkpoints WHERE id = ?1",
                    params![cp.id],
                )?;
            }
            Ok(stale)
        })
    }

    /// Prunes checkpoints older than `retention_days` across all sessions and returns removed rows.
    pub fn prune_expired_checkpoints(
        &self,
        retention_days: u64,
    ) -> Result<Vec<WorkspaceCheckpoint>> {
        let cutoff = (Utc::now() - Duration::days(retention_days.max(1) as i64)).to_rfc3339();
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, turn_id, event_id, workspace_dir, backend, pre_ref,
                        post_ref, trigger_tool, label, changed_files_json, is_safety_backup,
                        created_at
                 FROM workspace_checkpoints
                 WHERE created_at < ?1",
            )?;
            let expired = stmt
                .query_map(params![cutoff], row_to_checkpoint)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            for cp in &expired {
                conn.execute(
                    "DELETE FROM workspace_checkpoints WHERE id = ?1",
                    params![cp.id],
                )?;
            }
            Ok(expired)
        })
    }

    // ─── Phase 8: Persistent 24/7 Bounded Goal Loops & Export Queries ─────────

    pub fn upsert_goal_loop(&self, goal: &GoalLoopRecord) -> Result<()> {
        let cmd_json = serde_json::to_string(&goal.verify_cmd).unwrap_or_else(|_| "[]".into());
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO goal_loops
                 (id, name, session_id, workspace_dir, goal_text, verify_cmd_json, interval_secs,
                  max_iterations, max_budget_usd, max_consecutive_stalls, checkpoint_each_iter,
                  auto_rewind_on_stall, model, notify_channel, status, iterations_done,
                  consecutive_stalls, cost_spent_usd, last_verify_exit, last_verify_hash,
                  last_verify_output, next_run_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
                params![
                    goal.id,
                    goal.name,
                    goal.session_id,
                    goal.workspace_dir,
                    goal.goal_text,
                    cmd_json,
                    goal.interval_secs as i64,
                    goal.max_iterations as i64,
                    goal.max_budget_usd,
                    goal.max_consecutive_stalls as i64,
                    if goal.checkpoint_each_iter { 1i64 } else { 0i64 },
                    if goal.auto_rewind_on_stall { 1i64 } else { 0i64 },
                    goal.model,
                    goal.notify_channel,
                    goal.status.as_str(),
                    goal.iterations_done as i64,
                    goal.consecutive_stalls as i64,
                    goal.cost_spent_usd,
                    goal.last_verify_exit,
                    goal.last_verify_hash,
                    goal.last_verify_output,
                    goal.next_run_at.map(|t| t.to_rfc3339()),
                    goal.created_at.to_rfc3339(),
                    goal.updated_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_goal_loop(&self, id: &str) -> Result<Option<GoalLoopRecord>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, name, session_id, workspace_dir, goal_text, verify_cmd_json, interval_secs,
                        max_iterations, max_budget_usd, max_consecutive_stalls, checkpoint_each_iter,
                        auto_rewind_on_stall, model, notify_channel, status, iterations_done,
                        consecutive_stalls, cost_spent_usd, last_verify_exit, last_verify_hash,
                        last_verify_output, next_run_at, created_at, updated_at
                 FROM goal_loops WHERE id = ?1",
                params![id],
                row_to_goal_loop,
            )
            .optional()
        })
    }

    pub fn list_goal_loops(&self, limit: usize) -> Result<Vec<GoalLoopRecord>> {
        let cap = limit.clamp(1, 200.max(GOAL_LOOP_LIST_DEFAULT_LIMIT)) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, session_id, workspace_dir, goal_text, verify_cmd_json, interval_secs,
                        max_iterations, max_budget_usd, max_consecutive_stalls, checkpoint_each_iter,
                        auto_rewind_on_stall, model, notify_channel, status, iterations_done,
                        consecutive_stalls, cost_spent_usd, last_verify_exit, last_verify_hash,
                        last_verify_output, next_run_at, created_at, updated_at
                 FROM goal_loops ORDER BY updated_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![cap], row_to_goal_loop)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn delete_goal_loop(&self, id: &str) -> Result<bool> {
        self.with_conn(|conn| {
            let n = conn.execute("DELETE FROM goal_loops WHERE id = ?1", params![id])?;
            Ok(n > 0)
        })
    }

    pub fn insert_goal_loop_run(&self, run: &GoalLoopRun) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO goal_loop_runs
                 (id, goal_id, iteration, checkpoint_id, verify_exit_code, verify_output_excerpt,
                  verify_hash, tokens_used, cost_usd, duration_ms, outcome, reward, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    run.id,
                    run.goal_id,
                    run.iteration as i64,
                    run.checkpoint_id,
                    run.verify_exit_code,
                    run.verify_output_excerpt,
                    run.verify_hash,
                    run.tokens_used as i64,
                    run.cost_usd,
                    run.duration_ms as i64,
                    run.outcome,
                    run.reward,
                    run.created_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_goal_loop_runs(&self, goal_id: &str, limit: usize) -> Result<Vec<GoalLoopRun>> {
        let cap = limit.clamp(1, 200.max(GOAL_LOOP_RUNS_DEFAULT_LIMIT)) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, goal_id, iteration, checkpoint_id, verify_exit_code, verify_output_excerpt,
                        verify_hash, tokens_used, cost_usd, duration_ms, outcome, reward, created_at
                 FROM goal_loop_runs WHERE goal_id = ?1 ORDER BY iteration DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![goal_id, cap], row_to_goal_loop_run)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Returns the most recent verification outcome for `session_id` (joins `goal_loops` + `goal_loop_runs`).
    pub fn latest_goal_run_for_session(&self, session_id: &str) -> Result<Option<GoalLoopRun>> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT r.id, r.goal_id, r.iteration, r.checkpoint_id, r.verify_exit_code,
                        r.verify_output_excerpt, r.verify_hash, r.tokens_used, r.cost_usd,
                        r.duration_ms, r.outcome, r.reward, r.created_at
                 FROM goal_loop_runs r
                 INNER JOIN goal_loops g ON g.id = r.goal_id
                 WHERE g.session_id = ?1
                 ORDER BY r.created_at DESC
                 LIMIT 1",
                params![session_id],
                row_to_goal_loop_run,
            )
            .optional()
        })
    }

    /// Lists all events for a session chronologically (used by `--all-branches` trajectory export).
    pub fn get_all_session_events(&self, session_id: &str) -> Result<Vec<SessionEvent>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, parent_id, branch_index, role, kind, content_json,
                        tokens_in, tokens_out, cache_read_tokens, cost_usd, created_at
                 FROM session_events WHERE session_id = ?1 ORDER BY created_at ASC",
            )?;
            let rows = stmt.query_map(params![session_id], row_to_event)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// Lists sessions filtered by optional `since` RFC3339 timestamp and `channel` for batch export.
    pub fn list_sessions_for_export(
        &self,
        since: Option<DateTime<Utc>>,
        channel: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Session>> {
        let cap = limit.clamp(1, EXPORT_MAX_BATCH_LIMIT) as i64;
        let since_iso = since.map(|dt| dt.to_rfc3339());
        let chan = channel
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.title, s.channel, s.external_chat_id, s.external_thread_id,
                        s.model_override, s.permission_mode, s.status, s.head_event_id,
                        s.frozen_system_hash, s.workspace_dir, s.parent_session_id, s.agent_role,
                        s.agent_depth, s.worktree_path, s.worktree_branch, s.total_tokens,
                        s.total_cost_usd, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM subagents a WHERE a.parent_session_id = s.id)
                 FROM sessions s
                 WHERE (?1 IS NULL OR s.updated_at >= ?1)
                   AND (?2 IS NULL OR s.channel = ?2)
                 ORDER BY s.updated_at DESC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![since_iso, chan, cap], row_to_session)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_event_tree_and_fts_search() {
        let dir = std::env::temp_dir().join(format!("zenko_mem_test_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let s = store
            .create_session("Test", "webui", None, None, PermissionMode::Balanced)
            .expect("create session");
        let e1 = store
            .append_event(
                &s.id,
                None,
                EventRole::User,
                EventKind::Message,
                serde_json::json!({"text": "hello"}),
                &UsageStats::default(),
            )
            .expect("e1");
        let e2 = store
            .append_event(
                &s.id,
                Some(&e1.id),
                EventRole::Assistant,
                EventKind::Message,
                serde_json::json!({"text": "world"}),
                &UsageStats::default(),
            )
            .expect("e2");
        let _e2_alt = store
            .append_event(
                &s.id,
                Some(&e1.id),
                EventRole::Assistant,
                EventKind::Message,
                serde_json::json!({"text": "alternate branch"}),
                &UsageStats::default(),
            )
            .expect("e2_alt");

        let branch = store.get_active_branch(&s.id).expect("branch");
        assert_eq!(branch.len(), 2);
        assert_eq!(branch[1].content_json["text"], "alternate branch");

        store.set_session_head(&s.id, Some(&e2.id)).expect("switch");
        let branch1 = store.get_active_branch(&s.id).expect("branch1");
        assert_eq!(branch1[1].content_json["text"], "world");

        store
            .add_memory("global", "fact", "Rust Toolchain", "Uses rustls and mimalloc", 1.0)
            .expect("add mem");
        let found = store.search_memory_fts("mimalloc", 5).expect("search");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Rust Toolchain");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scoped_hybrid_memory_dedup_session_search_and_dream_lease() {
        let dir = std::env::temp_dir().join(format!("zenko_p4_sql_{}", new_id()));
        let ws_a = dir.join("repo_a");
        let ws_b = dir.join("repo_b");
        std::fs::create_dir_all(ws_a.join("src")).expect("ws_a");
        std::fs::create_dir_all(&ws_b).expect("ws_b");
        std::fs::write(ws_a.join("src/auth.rs"), "pub fn verify() {}").expect("auth");

        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let scope_a = crate::project_scope_for_workspace(&ws_a);
        let scope_b = crate::project_scope_for_workspace(&ws_b);
        assert_ne!(scope_a, scope_b);

        // 1. SimHash + Jaccard deduplication merges near-identical facts in the same scope
        let ins1 = store
            .upsert_memory_dedup(
                &scope_a,
                "project",
                "Auth Middleware",
                "JWT verification lives in src/auth.rs and never uses OpenSSL",
                1.0,
                None,
                None,
                true,
            )
            .expect("ins1");
        assert!(!ins1.merged);
        let ins2 = store
            .upsert_memory_dedup(
                &scope_a,
                "project",
                "Auth Middleware",
                "JWT verification lives in src/auth.rs and never uses OpenSSL crates",
                1.1,
                None,
                None,
                true,
            )
            .expect("ins2");
        assert!(ins2.merged, "near-duplicate must merge instead of duplicating");
        assert_eq!(ins1.entry.id, ins2.entry.id);

        // 2. Workspace scoping isolates Repo A from Repo B + 2-tier AND->OR recalls multi-word queries
        store
            .upsert_memory_dedup(
                &scope_b,
                "project",
                "Repo B Secret",
                "Repo B uses GraphQL resolvers exclusively",
                1.0,
                None,
                None,
                true,
            )
            .expect("ins_b");
        let hits_a = store
            .search_memory_hybrid(
                "How does JWT authentication verification work with nonexistentword?",
                Some(&scope_a),
                true,
                Some(&ws_a),
                5,
            )
            .expect("hits_a");
        assert_eq!(hits_a.len(), 1);
        assert_eq!(hits_a[0].entry.id, ins1.entry.id);
        assert!(hits_a[0].stale_paths.is_empty());
        assert_eq!(hits_a[0].entry.access_count, 1);

        // Delete src/auth.rs -> live staleness verifier flags it immediately!
        std::fs::remove_file(ws_a.join("src/auth.rs")).expect("rm");
        let hits_stale = store
            .search_memory_hybrid("JWT auth", Some(&scope_a), true, Some(&ws_a), 5)
            .expect("hits_stale");
        assert_eq!(hits_stale[0].stale_paths, vec!["src/auth.rs"]);

        // 3. Cross-session FTS5 `session_search` + Hermes ±N context window expansion
        let s1 = store
            .create_session("Auth Refactor", "cli", None, None, PermissionMode::Balanced)
            .expect("s1");
        store
            .set_session_workspace(&s1.id, &ws_a.display().to_string())
            .expect("set ws");
        let ev1 = store
            .append_event(
                &s1.id,
                None,
                EventRole::User,
                EventKind::Message,
                serde_json::json!({"text": "Fix PASETO token rotation clock skew in auth adapter"}),
                &UsageStats::default(),
            )
            .expect("ev1");
        let _ev2 = store
            .append_event(
                &s1.id,
                Some(&ev1.id),
                EventRole::Assistant,
                EventKind::Message,
                serde_json::json!({"text": "Added 30-second leeway to PASETO verifier."}),
                &UsageStats::default(),
            )
            .expect("ev2");
        let s_hits = store
            .search_sessions_fts(&SessionSearchFilter {
                query: "PASETO rotation skew".into(),
                workspace_dir: Some(ws_a.display().to_string()),
                ..Default::default()
            })
            .expect("s_hits");
        assert_eq!(s_hits.len(), 1);
        assert_eq!(s_hits[0].session_id, s1.id);

        let ctx_win = store
            .get_session_event_context(&s_hits[0].event_id, 2)
            .expect("ctx_win")
            .expect("some ctx");
        assert_eq!(ctx_win.events.len(), 2);
        assert!(ctx_win.events[0].is_anchor);
        assert!(ctx_win.events[1].text.contains("30-second leeway"));

        // 4. Codex Stage-1 rollout summary cache round-trip + Stage-0 hygiene stale salience demotion
        let rsum = SessionRolloutSummary {
            session_id: s1.id.clone(),
            scope: scope_a.clone(),
            digest: "### Session cached digest".into(),
            session_updated_at: "2026-11-22T00:00:00Z".into(),
            created_at: Utc::now(),
        };
        store.upsert_rollout_summary(&rsum).expect("upsert rollout");
        assert!(
            store
                .get_rollout_summary(&s1.id, "2026-11-22T00:00:00Z")
                .expect("get rollout")
                .is_some()
        );
        assert!(
            store
                .get_rollout_summary(&s1.id, "2026-11-22T01:00:00Z")
                .expect("stale rollout miss")
                .is_none()
        );
        let before_sal = store.get_memory(&ins1.entry.id).expect("get").expect("e").salience;
        let hyg = store.run_memory_hygiene(Some(&ws_a)).expect("hygiene");
        assert!(hyg.stale_flagged >= 1);
        let after_sal = store.get_memory(&ins1.entry.id).expect("get").expect("e").salience;
        assert!(after_sal < before_sal, "stale-path memory salience must demote");

        // 5. Transactional Dream SQLite lease blocks concurrent claim until released
        assert!(store.try_claim_dream_lease("owner_1", 120).expect("claim 1"));
        assert!(!store.try_claim_dream_lease("owner_2", 120).expect("claim 2 blocked"));
        store.release_dream_lease("owner_1").expect("release 1");
        assert!(store.try_claim_dream_lease("owner_2", 120).expect("claim 2 ok"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn subagent_hierarchy_and_mailbox_persistence() {
        let dir = std::env::temp_dir().join(format!("zenko_p5_sql_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let parent = store
            .create_session("Lead", "webui", None, None, PermissionMode::Balanced)
            .expect("parent");
        let child = store
            .create_child_session(
                &parent.id,
                "architect",
                1,
                "Arch Child",
                "/tmp/repo",
                None,
                None,
                None,
                PermissionMode::Balanced,
            )
            .expect("child");
        let grandchild = store
            .create_child_session(
                &child.id,
                "explorer",
                2,
                "Explorer Leaf",
                "/tmp/repo",
                None,
                None,
                None,
                PermissionMode::Balanced,
            )
            .expect("grandchild");

        let now = Utc::now();
        store
            .upsert_subagent(&SubagentRecord {
                id: child.id.clone(),
                parent_session_id: parent.id.clone(),
                role: "architect".into(),
                depth: 1,
                goal: "Design auth".into(),
                status: SubagentStatus::Running,
                workspace_mode: SubagentWorkspaceMode::Inherit,
                workspace_dir: "/tmp/repo".into(),
                worktree_branch: None,
                worktree_base_commit: None,
                worktree_dirty_baseline: false,
                model: Some("test-model".into()),
                summary: None,
                output_schema: None,
                structured_output: None,
                diff_stat: None,
                changed_files: Vec::new(),
                tokens_used: 100,
                cost_usd: 0.002,
                tree_tokens: 0,
                tree_cost_usd: 0.0,
                descendant_count: 0,
                resume_count: 0,
                started_at: now,
                finished_at: None,
            })
            .expect("upsert child");
        store
            .upsert_subagent(&SubagentRecord {
                id: grandchild.id.clone(),
                parent_session_id: child.id.clone(),
                role: "explorer".into(),
                depth: 2,
                goal: "Scan auth files".into(),
                status: SubagentStatus::Completed,
                workspace_mode: SubagentWorkspaceMode::Inherit,
                workspace_dir: "/tmp/repo".into(),
                worktree_branch: None,
                worktree_base_commit: None,
                worktree_dirty_baseline: false,
                model: Some("fast-model".into()),
                summary: Some("Found src/auth.rs".into()),
                output_schema: None,
                structured_output: Some(serde_json::json!({"files": ["src/auth.rs"]})),
                diff_stat: None,
                changed_files: Vec::new(),
                tokens_used: 420,
                cost_usd: 0.001,
                tree_tokens: 0,
                tree_cost_usd: 0.0,
                descendant_count: 0,
                resume_count: 0,
                started_at: now,
                finished_at: Some(now),
            })
            .expect("upsert grandchild");

        let desc = store.list_descendant_session_ids(&parent.id).expect("desc");
        assert_eq!(desc.len(), 2);
        assert!(desc.contains(&child.id));
        assert!(desc.contains(&grandchild.id));

        let child_loaded = store.get_subagent(&child.id).expect("get child").expect("some");
        assert_eq!(child_loaded.tree_tokens, 520);
        assert_eq!(child_loaded.descendant_count, 1);
        assert!((child_loaded.tree_cost_usd - 0.003).abs() < 1e-9);

        let loaded = store.get_subagent(&grandchild.id).expect("get").expect("some");
        assert_eq!(loaded.structured_output.unwrap()["files"][0], "src/auth.rs");

        // Inter-agent mailbox atomic drain
        store
            .insert_agent_message(&parent.id, &child.id, "Focus on OAuth2 PKCE")
            .expect("mail");
        let drained = store.drain_unread_agent_messages(&child.id).expect("drain 1");
        assert_eq!(drained.len(), 1);
        assert!(drained[0].read_at.is_some());
        assert_eq!(drained[0].body, "Focus on OAuth2 PKCE");
        assert!(store.drain_unread_agent_messages(&child.id).expect("drain 2").is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_tasks_dag_atomic_claim_and_checkpoints_persistence() {
        let dir = std::env::temp_dir().join(format!("zenko_p6_sql_{}", new_id()));
        let store = SqliteStore::open(dir.join("zenko.db")).expect("open db");
        let s = store
            .create_session("Phase 6 Test", "cli", None, None, PermissionMode::Plan)
            .expect("session");
        assert_eq!(s.permission_mode, PermissionMode::Plan);

        let now = Utc::now();
        let t1 = SessionTask {
            id: "t1".into(),
            session_id: s.id.clone(),
            branch_event_id: None,
            title: "Design schema".into(),
            description: "Write SQLite DDL".into(),
            active_label: Some("Designing schema".into()),
            status: SessionTaskStatus::Pending,
            depends_on: vec![],
            owner_agent: None,
            verification_cmd: Some("cargo test".into()),
            verification_evidence: None,
            notes: None,
            priority: 10,
            created_at: now,
            updated_at: now,
        };
        store.upsert_session_task(&t1).expect("upsert t1");

        // Atomic claim succeeds for first worker and rejects competing second worker
        let claimed = store
            .claim_session_task(&s.id, "t1", "coder_a")
            .expect("claim 1")
            .expect("claimed");
        assert_eq!(claimed.owner_agent.as_deref(), Some("coder_a"));
        assert_eq!(claimed.status, SessionTaskStatus::InProgress);
        assert!(
            store
                .claim_session_task(&s.id, "t1", "coder_b")
                .expect("claim 2")
                .is_none(),
            "competing worker must not steal claimed task"
        );

        // Workspace checkpoint insert, post-state update, and retention pruning
        let cp1 = WorkspaceCheckpoint {
            id: "cp_1".into(),
            session_id: s.id.clone(),
            turn_id: "turn_1".into(),
            event_id: None,
            workspace_dir: "/tmp/ws".into(),
            backend: CheckpointBackend::ShadowGit,
            pre_ref: "aaa111".into(),
            post_ref: None,
            trigger_tool: "write_file".into(),
            label: "Before write_file".into(),
            changed_files: vec![],
            is_safety_backup: false,
            created_at: now - Duration::seconds(10),
        };
        let cp2 = WorkspaceCheckpoint {
            id: "cp_2".into(),
            session_id: s.id.clone(),
            turn_id: "turn_2".into(),
            event_id: None,
            workspace_dir: "/tmp/ws".into(),
            backend: CheckpointBackend::ShadowGit,
            pre_ref: "bbb222".into(),
            post_ref: Some("ccc333".into()),
            trigger_tool: "shell_exec".into(),
            label: "Before shell_exec".into(),
            changed_files: vec!["src/main.rs".into()],
            is_safety_backup: false,
            created_at: now,
        };
        store.insert_checkpoint(&cp1).expect("insert cp1");
        store.insert_checkpoint(&cp2).expect("insert cp2");
        store
            .update_checkpoint_post_state("cp_1", "bbb222", &["README.md".into()])
            .expect("update post");
        let loaded = store.get_checkpoint("cp_1").expect("get").expect("some");
        assert_eq!(loaded.post_ref.as_deref(), Some("bbb222"));
        assert_eq!(loaded.changed_files, vec!["README.md"]);

        let pruned = store.prune_session_checkpoints(&s.id, 1).expect("prune");
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].id, "cp_1");
        assert_eq!(store.list_checkpoints(Some(&s.id), 10).expect("list").len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }
}
