//! Centralized global constants and configuration defaults for Zenko.
//! All crates reference these symbols instead of hardcoding literals.

pub const APP_NAME: &str = "Zenko";
pub const DEFAULT_DATA_DIR: &str = "~/.zenko";
pub const DEFAULT_HUB_DIR: &str = "~/zenko_hub";
pub const DEFAULT_CONFIG_FILE: &str = "zenko.toml";
pub const DEFAULT_WORKSPACE_DIR: &str = ".";
pub const HUB_PATH_PREFIX: &str = "hub:";
pub const HUB_SUBDIR_PROJECTS: &str = "projects";
pub const HUB_SUBDIR_SCRIPTS: &str = "scripts";
pub const HUB_SUBDIR_MEDIA: &str = "media";
pub const HUB_SUBDIR_MEDIA_IMAGES: &str = "media/images";
pub const HUB_SUBDIR_MEDIA_VIDEOS: &str = "media/videos";
pub const HUB_SUBDIR_MEDIA_AUDIO: &str = "media/audio";
pub const HUB_SUBDIR_ARTIFACTS: &str = "artifacts";
pub const HUB_REQUIRED_SUBDIRS: &[&str] = &[
    HUB_SUBDIR_PROJECTS,
    HUB_SUBDIR_SCRIPTS,
    HUB_SUBDIR_MEDIA_IMAGES,
    HUB_SUBDIR_MEDIA_VIDEOS,
    HUB_SUBDIR_MEDIA_AUDIO,
    HUB_SUBDIR_ARTIFACTS,
];
pub const DEFAULT_LOG_LEVEL: &str = "info";
pub const DEFAULT_MAX_TOOL_ITERATIONS: usize = 24;
pub const DEFAULT_DAILY_SPEND_LIMIT_USD: f64 = 5.0;

pub const DEFAULT_WEBUI_BIND: &str = "127.0.0.1";
pub const DEFAULT_WEBUI_PORT: u16 = 7878;

pub const DEFAULT_THEME_PRESET: &str = "zenko_vermilion";
pub const DEFAULT_THEME_DARK_MODE: &str = "dark";
pub const DEFAULT_THEME_LIGHT_MODE: &str = "light";

pub const DEFAULT_PROVIDER_ID: &str = "openrouter";
pub const DEFAULT_PROVIDER_SORT: &str = "price";
pub const DEFAULT_TEMPERATURE: f32 = 0.2;
pub const DEFAULT_MAX_TOKENS: u32 = 16_384;

pub const DEFAULT_TELEGRAM_MODE: &str = "polling";
pub const DEFAULT_TELEGRAM_POLL_TIMEOUT_SECS: u64 = 25;
pub const DEFAULT_PAIRING_TTL_MINS: i64 = 15;

pub const DEFAULT_SHELL_TIMEOUT_SECS: u64 = 60;
pub const SHELL_MAX_TIMEOUT_SECS: u64 = 300;
pub const SHELL_MIN_OUTPUT_BYTES: usize = 1_024;
pub const SHELL_BIN_UNIX: &str = "/bin/sh";
pub const KILL_BIN_UNIX: &str = "/bin/kill";
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 16_384;
pub const DEFAULT_HOOK_TIMEOUT_SECS: u64 = 5;
pub const DEFAULT_APPROVAL_TIMEOUT_SECS: u64 = 300;
pub const APPROVAL_RESOLVER_STEERED: &str = "system:steered";
pub const APPROVAL_RESOLVER_CANCELLED: &str = "system:cancelled";
pub const APPROVAL_RESOLVER_TIMEOUT: &str = "system:timeout";
pub const MAX_PARALLEL_TOOLS: usize = 6;
pub const JOBS_DIR_NAME: &str = "tasks";
pub const TASKS_DIR_NAME: &str = "tasks";
pub const JOB_MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
pub const JOB_MIN_READ_BYTES: usize = 256;
pub const JOB_DEFAULT_READ_BYTES: usize = 8_192;
pub const JOB_MAX_READ_BYTES: usize = 65_536;
pub const JOB_DEFAULT_LIST_LIMIT: usize = 25;
pub const JOB_STREAM_BUF_BYTES: usize = 4_096;
pub const JOB_LIVE_DELTA_MAX_BYTES: usize = 2_048;
pub const JOB_LIVE_DELTA_COALESCE_MS: u64 = 60;
pub const JOB_MAX_AUTO_BACKGROUND_SECS: u64 = 120;
pub const JOB_STOP_GRACE_MS: u64 = 600;
pub const JOB_PUMP_DRAIN_MS: u64 = 500;
pub const JOB_SIGKILL_WAIT_MS: u64 = 400;
pub const JOB_STOP_WAIT_SECS: u64 = 2;
pub const JOB_MAX_HISTORY: usize = 100;
pub const JOB_WAKE_COALESCE_MS: u64 = 400;
pub const JOB_DEFAULT_WAIT_SECS: u64 = 30;
pub const JOB_MAX_WAIT_SECS: u64 = 120;
pub const JOB_NOTICE_CMD_PREVIEW_CHARS: usize = 80;
pub const DEFAULT_AUTO_BACKGROUND_SECS: u64 = 12;
pub const SESSION_MAX_JOB_NOTICES: usize = 16;
pub const SESSION_QUEUE_MAX_ITEMS: usize = 32;
pub const STEERING_PREVIEW_CHARS: usize = 120;
pub const ATTACH_DEFAULT_REPLAY_EVENTS: usize = 10;
pub const IPC_DAEMON_PROBE_TIMEOUT_MS: u64 = 400;
pub const IPC_WAIT_TIMEOUT_SLACK_SECS: u64 = 5;
pub const IPC_FRAME_SNAPSHOT: &str = "snapshot";
pub const IPC_FRAME_EVENT: &str = "event";
pub const IPC_FRAME_CHUNK: &str = "chunk";
pub const IPC_FRAME_DONE: &str = "done";

/// Sanitized environment allowlist forwarded to child shell processes (zero Vault secrets).
pub const SAFE_SHELL_ENV_KEYS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "TERM",
    "LANG",
    "LC_ALL",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "TMPDIR",
];
pub const DEFAULT_WEB_FETCH_TIMEOUT_SECS: u64 = 20;
pub const DEFAULT_WEB_FETCH_MAX_BYTES: usize = 1_048_576;
pub const DEFAULT_WEB_FETCH_MAX_REDIRECTS: usize = 5;
pub const DEFAULT_WEB_FETCH_USER_AGENT: &str = "Zenko-Agent/0.1";
pub const DEFAULT_READ_FILE_MAX_BYTES: u64 = 2_097_152;
pub const DEFAULT_READ_FILE_LINE_WINDOW: usize = 200;
pub const DEFAULT_SEARCH_MAX_FILE_BYTES: u64 = 2_097_152;
pub const DEFAULT_SEARCH_SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".next",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    ".cache",
];

pub const DEFAULT_COMPACTION_TRIGGER_RATIO: f64 = 0.78;
pub const DEFAULT_MICROCOMPACT_TRIGGER_RATIO: f64 = 0.55;
pub const DEFAULT_CONTEXT_WINDOW_FALLBACK: u64 = 64_000;
pub const DEFAULT_COMPACTION_KEEP_RECENT: usize = 8;
pub const DEFAULT_COMPACTION_SNIPPET_CHARS: usize = 220;
pub const MICROCOMPACT_PROTECT_RECENT_EXCHANGES: usize = 4;
pub const MICROCOMPACT_MIN_TOOL_CHARS: usize = 1_200;
pub const COMPACTION_MIN_EVENTS_BETWEEN_CHECKPOINTS: usize = 8;
pub const COMPACTION_MAX_CHECKPOINT_TOKENS: u32 = 2_048;

// Phase 3 Context Engineering: 3-Tier Prompting, Artifact Spilling, Progressive Skills & Deferred Tools
pub const ARTIFACTS_DIR_NAME: &str = "artifacts";
pub const ARTIFACT_PATH_PREFIX: &str = "artifact:";
pub const ARTIFACT_HEAD_RATIO_NUM: usize = 35;
pub const ARTIFACT_HEAD_RATIO_DEN: usize = 100;
pub const ARTIFACT_MAX_AGE_SECS: u64 = 7 * 24 * 3_600;
pub const ARTIFACT_MAX_PER_SESSION: usize = 64;

pub const REPO_RULE_FILES: &[&str] = &["ZENKO.md", "AGENTS.md", "CLAUDE.md", "RULES.md"];
pub const WORKSPACE_SKILL_DIRS: &[&str] = &[".zenko/skills", ".agents/skills"];
pub const MAX_REPO_RULES_BYTES: usize = 24 * 1024;
pub const MAX_SKILL_SUBFILE_BYTES: u64 = 256 * 1024;
pub const TOOL_SEARCH_DEFAULT_LIMIT: usize = 5;
pub const TOOL_SEARCH_MAX_LIMIT: usize = 12;

// Phase 4: Surgical Memory, Workspace Scoping, Hybrid RRF Recall, `session_search` & Autonomous Dream
pub const MAX_USER_DOC_TOKENS: usize = 600;
pub const MAX_MEMORY_DOC_TOKENS: usize = 1_200;
pub const MAX_PROJECT_MEMORY_DOC_TOKENS: usize = 1_200;
pub const MAX_ZENKO_DOC_TOKENS: usize = 2_400;
pub const MEMORY_HISTORY_DIR_NAME: &str = ".history";
pub const MEMORY_PROJECTS_DIR_NAME: &str = "projects";
pub const PROJECT_MEMORY_DOC_NAME: &str = "PROJECT_MEMORY.md";
pub const MEMORY_MAX_HISTORY_SNAPSHOTS: usize = 24;
pub const MEMORY_SCOPE_GLOBAL: &str = "global";
pub const MEMORY_SCOPE_PROJECT_PREFIX: &str = "project:";

pub const MEMORY_DEDUP_SIMHASH_MAX_HAMMING: u32 = 10;
pub const MEMORY_DEDUP_JACCARD_THRESHOLD: f64 = 0.68;
pub const MEMORY_PROJECT_SCOPE_BOOST: f64 = 1.35;
pub const MEMORY_ACCESS_BOOST_WEIGHT: f64 = 0.18;
pub const MEMORY_RRF_K: f64 = 40.0;
pub const MEMORY_STALE_PATH_PENALTY: f64 = 0.45;
pub const MEMORY_STALE_SALIENCE_DEMOTION: f64 = 0.15;
pub const MEMORY_ARCHIVE_INACTIVE_DAYS: i64 = 90;
pub const MEMORY_ARCHIVE_MAX_SALIENCE: f64 = 0.75;
pub const MEMORY_DEFAULT_SEARCH_LIMIT: usize = 10;
pub const MEMORY_RELAXED_MIN_OVERLAP: f64 = 0.35;
pub const MEMORY_AUTO_RECALL_LIMIT: usize = 4;
pub const MEMORY_AUTO_RECALL_MIN_SCORE: f64 = 0.015;
pub const SESSION_SEARCH_DEFAULT_LIMIT: usize = 8;
pub const SESSION_SEARCH_MAX_LIMIT: usize = 25;
pub const SESSION_CONTEXT_DEFAULT_WINDOW: usize = 3;
pub const SESSION_CONTEXT_MAX_WINDOW: usize = 8;
pub const ORCH_RECON_MAX_SESSION_HITS: usize = 2;

pub const DREAM_DEFAULT_MIN_SESSIONS: usize = 3;
pub const DREAM_DEFAULT_COOLDOWN_HOURS: u64 = 6;
pub const DREAM_MIN_SESSION_EVENTS: usize = 4;
pub const DREAM_LEASE_SECS: i64 = 300;
pub const DREAM_MAX_SESSIONS_PER_RUN: usize = 6;
pub const DREAM_MAX_ORIENTATION_MEMORIES: usize = 24;
pub const DREAM_MAX_TRANSCRIPT_CHARS: usize = 14_000;
pub const DREAM_MAX_OUTPUT_TOKENS: u32 = 2_048;
pub const DREAM_KV_WATERMARK_KEY: &str = "dream_watermark_ts";
pub const DREAM_KV_LAST_RUN_KEY: &str = "dream_last_run_ts";
pub const DREAM_KV_LEASE_KEY: &str = "dream_lease_json";
pub const DREAM_KV_FTS_BACKFILLED_KEY: &str = "session_events_fts_v1";

// Phase 5 / 5.6: Async Subagents, Git Worktree Isolation, Budgets & Inter-Agent Mailbox
pub const WORKTREES_DIR_NAME: &str = "worktrees";
pub const WORKTREE_BRANCH_PREFIX: &str = "zenko/agent-";
pub const WORKTREE_BRANCH_SUFFIX_MAX_CHARS: usize = 14;
pub const WORKTREE_DIRTY_MAX_UNTRACKED_FILES: usize = 64;
pub const WORKTREE_DIRTY_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const WORKTREE_UNTRACKED_PREVIEW_MAX_FILES: usize = 8;
pub const WORKTREE_UNTRACKED_PREVIEW_MAX_CHARS: usize = 600;
pub const WORKTREE_GIT_AUTHOR_NAME: &str = "Zenko";
pub const WORKTREE_GIT_AUTHOR_EMAIL: &str = "zenko@local";
pub const MAX_SUBAGENT_DEPTH: u8 = 2;
pub const MAX_CONCURRENT_SUBAGENTS: usize = 4;
pub const SUBAGENT_DEFAULT_MAX_ITERS: usize = 12;
pub const SUBAGENT_DEFAULT_WAIT_SECS: u64 = 60;
pub const SUBAGENT_MAX_WAIT_SECS: u64 = 300;
pub const SUBAGENT_DEFAULT_MAX_DURATION_SECS: u64 = 300;
pub const SUBAGENT_MAX_DURATION_SECS: u64 = 1_800;
pub const SUBAGENT_DEFAULT_MAX_COST_USD: f64 = 0.50;
pub const SUBAGENT_FORK_MAX_CHARS: usize = 6_000;
pub const SUBAGENT_FORK_RECENT_EVENTS: usize = 12;
pub const SUBAGENT_FORK_TURN_PREVIEW_CHARS: usize = 420;
pub const SUBAGENT_FORK_TOOL_PREVIEW_CHARS: usize = 220;
pub const SUBAGENT_TITLE_PREVIEW_CHARS: usize = 42;
pub const SUBAGENT_MAIL_REPORT_PREVIEW_CHARS: usize = 800;
pub const SUBAGENT_CANCEL_GRACE_SECS: u64 = 2;
pub const SUBAGENT_TIMEOUT_HARD_GRACE_SECS: u64 = 5;
pub const SUBAGENT_SUMMARY_MAX_CHARS: usize = 2_400;
pub const SUBAGENT_DIFF_PREVIEW_MAX_CHARS: usize = 8_000;
pub const SUBAGENT_MAILBOX_MAX_MESSAGES: usize = 32;
pub const SUBAGENT_LIST_DEFAULT_LIMIT: usize = 30;
pub const SUBAGENT_SCHEMA_MAX_RETRIES: usize = 1;
pub const SUBAGENT_SCHEMA_MAX_DEPTH: usize = 32;
pub const SUBAGENT_ROLE_CUSTOM: &str = "custom";
pub const SUBAGENT_CUSTOM_DIRECTIVE_MAX_CHARS: usize = 2_400;

// Phase 6: Durable SQLite Session Task DAG, Read-Only Plan Mode & Shadow-Git Workspace Checkpoints
pub const SESSION_TASK_ID_PREFIX: &str = "task_";
pub const SESSION_TASK_DAG_LOCK_KEY: &str = "session_dag";
pub const SESSION_TASK_MAX_ITEMS: usize = 64;
pub const SESSION_TASK_PROMPT_MAX_ITEMS: usize = 16;
pub const SESSION_TASK_EVIDENCE_MAX_CHARS: usize = 500;
pub const CHECKPOINTS_DIR_NAME: &str = "checkpoints";
pub const CHECKPOINTS_GIT_SUBDIR: &str = "git";
pub const CHECKPOINTS_BLOBS_SUBDIR: &str = "blobs";
pub const CHECKPOINTS_BLOBS_OBJECTS_SUBDIR: &str = "objects";
pub const CHECKPOINTS_BLOBS_MANIFESTS_SUBDIR: &str = "manifests";
pub const CHECKPOINT_TRIGGER_SAFETY: &str = "rewind_safety";
pub const CHECKPOINT_LIST_DEFAULT_LIMIT: usize = 30;
pub const CHECKPOINT_LABEL_MAX_CHARS: usize = 64;
pub const CHECKPOINT_MAX_PER_SESSION: usize = 50;
pub const CHECKPOINT_RETENTION_DAYS: u64 = 30;
pub const CHECKPOINT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
pub const CHECKPOINT_MAX_TRACKED_FILES: usize = 4_000;
pub const CHECKPOINT_DIFF_PREVIEW_MAX_CHARS: usize = 12_000;
pub const CHECKPOINT_GIT_TIMEOUT_SECS: u64 = 15;
pub const CHECKPOINT_GIT_AUTHOR_NAME: &str = "Zenko Checkpoint";
pub const CHECKPOINT_GIT_AUTHOR_EMAIL: &str = "checkpoint@zenko.local";
/// Empty Git tree SHA produced when a workspace contains zero indexable files.
pub const GIT_EMPTY_TREE_SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
pub const CHECKPOINT_SKIP_DIRS: &[&str] = &[
    ".git",
    ".zenko",
    "target",
    "node_modules",
    ".next",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    ".cache",
    ".turbo",
    ".parcel-cache",
];
pub const REWIND_SUMMARY_MAX_EVENTS: usize = 10;
pub const REWIND_SUMMARY_EVENT_PREVIEW_CHARS: usize = 180;

// Phase 7: Deep Engineering Tools (`glob_find`, `git_tool`, `lsp_tool`), Pluggable `web_search` & Universal MCP Client
pub const TOOL_GLOB_FIND: &str = "glob_find";
pub const TOOL_GIT: &str = "git_tool";
pub const TOOL_WEB_SEARCH: &str = "web_search";
pub const TOOL_LSP: &str = "lsp_tool";
pub const TOOL_MCP_RESOURCE: &str = "mcp_resource";
pub const MCP_TOOL_PREFIX: &str = "mcp__";

pub const GLOB_DEFAULT_MAX_RESULTS: usize = 100;
pub const GLOB_MAX_RESULTS_CAP: usize = 500;
pub const GLOB_MAX_BRACE_EXPANSIONS: usize = 32;

pub const GIT_TOOL_TIMEOUT_SECS: u64 = 20;
pub const GIT_TOOL_DEFAULT_LOG_LIMIT: usize = 20;
pub const GIT_TOOL_MAX_LOG_LIMIT: usize = 100;
pub const GIT_TOOL_DEFAULT_BLAME_LINES: u64 = 120;
pub const GIT_TOOL_MAX_OUTPUT_CHARS: usize = 24_000;
pub const GIT_SENSITIVE_FILE_PATTERNS: &[&str] = &[
    ".env",
    ".env.",
    ".pem",
    ".key",
    ".p12",
    ".pfx",
    "id_rsa",
    "id_ed25519",
    "vault.enc",
    "vault.key",
    "credentials.json",
    ".npmrc",
    ".netrc",
];

pub const WEB_SEARCH_DEFAULT_PROVIDER: &str = "auto";
pub const WEB_SEARCH_DEFAULT_LIMIT: usize = 8;
pub const WEB_SEARCH_MAX_LIMIT: usize = 20;
pub const WEB_SEARCH_TIMEOUT_SECS: u64 = 15;
pub const WEB_SEARCH_MAX_BYTES: usize = 512 * 1024;
pub const WEB_SEARCH_DDG_URL: &str = "https://html.duckduckgo.com/html/";
pub const WEB_SEARCH_BRAVE_URL: &str = "https://api.search.brave.com/res/v1/web/search";
pub const WEB_SEARCH_TAVILY_URL: &str = "https://api.tavily.com/search";

pub const LSP_REQUEST_TIMEOUT_SECS: u64 = 15;
pub const LSP_STARTUP_TIMEOUT_SECS: u64 = 20;
pub const LSP_DIAGNOSTICS_WAIT_MS: u64 = 600;
pub const LSP_AUTO_DIAG_WAIT_MS: u64 = 450;
pub const LSP_AUTO_DIAG_MAX_ITEMS: usize = 12;
pub const LSP_IDLE_TIMEOUT_SECS: u64 = 300;
pub const LSP_MAX_RESULTS: usize = 80;
pub const LSP_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const LSP_STDERR_RING_BYTES: usize = 4 * 1024;

pub const MCP_CLIENT_PROTOCOL_VERSION: &str = "2025-03-26";
pub const MCP_CLIENT_NAME: &str = "zenko";
pub const MCP_STARTUP_TIMEOUT_SECS: u64 = 15;
pub const MCP_TOOL_TIMEOUT_SECS: u64 = 60;
pub const MCP_IDLE_TIMEOUT_SECS: u64 = 300;
pub const MCP_MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MCP_MAX_CATALOG_PAGES: usize = 16;
pub const MCP_STDERR_RING_BYTES: usize = 4 * 1024;
pub const MCP_KV_CATALOG_PREFIX: &str = "mcp_catalog_v1:";
pub const MCP_MAX_DESCRIPTION_CHARS: usize = 400;

// Phase 8: Continual Self-Evolution, Procedural Skills (`skill_manage`), 24/7 Bounded Goal Loops,
// Declarative Event Triggers & Scrubbed Trajectory Export (`zenko export`)
pub const TOOL_SKILL_MANAGE: &str = "skill_manage";
pub const TOOL_GOAL_LOOP: &str = "goal_loop";

pub const SKILL_HISTORY_DIR_NAME: &str = ".history";
pub const SKILL_MAX_HISTORY_SNAPSHOTS: usize = 24;
pub const SKILL_MAX_CONTENT_BYTES: usize = 128 * 1024;
pub const SKILL_SLUG_MAX_CHARS: usize = 64;
pub const SKILL_REASON_MAX_CHARS: usize = 24;
pub const SKILL_HISTORY_LIST_DEFAULT_LIMIT: usize = 25;
pub const SKILL_DEFAULT_DESCRIPTION: &str = "Reusable Zenko procedural skill";
pub const DREAM_MAX_PROPOSED_SKILLS: usize = 2;

pub const GOAL_LOOP_ID_PREFIX: &str = "goal_";
pub const GOAL_LOOP_DEFAULT_MAX_ITERATIONS: usize = 10;
pub const GOAL_LOOP_MAX_ITERATIONS_CAP: usize = 100;
pub const GOAL_LOOP_DEFAULT_MAX_BUDGET_USD: f64 = 2.0;
pub const GOAL_LOOP_MIN_REMAINING_BUDGET_USD: f64 = 0.01;
pub const GOAL_LOOP_DEFAULT_STALL_LIMIT: usize = 3;
pub const GOAL_LOOP_MAX_STALL_CAP: usize = 20;
pub const GOAL_LOOP_DEFAULT_INTERVAL_SECS: u64 = 0;
pub const GOAL_LOOP_MIN_BACKOFF_SECS: u64 = 3;
pub const GOAL_LOOP_DEFAULT_NOTIFY_CHANNEL: &str = "webui";
pub const GOAL_LOOP_LABEL_PREVIEW_CHARS: usize = 32;
pub const GOAL_LOOP_VERIFY_TIMEOUT_SECS: u64 = 120;
pub const GOAL_LOOP_VERIFY_TIMEOUT_EXIT_CODE: i32 = 124;
pub const GOAL_LOOP_VERIFY_MAX_OUTPUT_BYTES: usize = 8_192;
pub const GOAL_LOOP_VERIFY_CAPTURE_CAP_BYTES: usize = 64 * 1024;
pub const GOAL_LOOP_FEEDBACK_TAIL_BYTES: usize = 2_400;
pub const GOAL_LOOP_HASH_BYTES: usize = 12;
pub const GOAL_LOOP_LIST_DEFAULT_LIMIT: usize = 30;
pub const GOAL_LOOP_RUNS_DEFAULT_LIMIT: usize = 25;
pub const GOAL_LOOP_IPC_TIMEOUT_SECS: u64 = 300;
/// Shell interpreter basenames whose `-c` script payload is recursively inspected by ExecPolicy.
pub const VERIFY_SHELL_INTERPRETERS: &[&str] = &["sh", "bash", "zsh", "dash", "fish", "ash"];

pub const WEBHOOK_DEFAULT_COOLDOWN_SECS: u64 = 10;
pub const WEBHOOK_DEFAULT_MAX_PER_HOUR: usize = 60;
pub const WATCHER_MIN_POLL_INTERVAL_SECS: u64 = 3;
pub const WATCHER_DEFAULT_POLL_INTERVAL_SECS: u64 = 5;
pub const WATCHER_MAX_SLEEP_SECS: u64 = 30;
pub const WATCHER_DEFAULT_DEBOUNCE_SECS: u64 = 2;
pub const WATCHER_DEFAULT_COOLDOWN_SECS: u64 = 15;
pub const WATCHER_DEFAULT_MAX_PER_HOUR: usize = 30;
pub const WATCHER_SELF_EDIT_SUPPRESS_SECS: u64 = 8;
pub const WATCHER_MAX_FILES_PER_SCAN: usize = 4_000;
pub const WATCHER_PREVIEW_MAX_FILES: usize = 12;

pub const TRAJECTORY_SCHEMA_VERSION: &str = "zenko.trajectory.v1";
pub const EXPORT_DEFAULT_BATCH_LIMIT: usize = 200;
pub const EXPORT_MAX_BATCH_LIMIT: usize = 500;
pub const EXPORT_ANON_WORKSPACE: &str = "<WORKSPACE>";
pub const EXPORT_ANON_HOME: &str = "<HOME>";

/// Privileged operator caller IDs allowed to administer any subagent across CLI, HTTP, and WebUI.
pub const OPERATOR_SESSION_IDS: &[&str] = &["user", "cli", "webui", "operator"];

/// Deterministic operator caller check (single source of truth across runtime, API, and CLI).
pub fn is_operator_session(session_id: &str) -> bool {
    OPERATOR_SESSION_IDS.contains(&session_id)
}

/// Mutating actions on `memory_tool` (blocked for all subagent roles; surfaced as `<memory_delta>` for root sessions).
pub const MEMORY_MUTATING_ACTIONS: &[&str] = &[
    "add_entry",
    "update_entry",
    "delete_entry",
    "patch_doc",
    "append_doc",
    "append_user",
    "append_memory",
    "rollback_doc",
];

pub fn is_memory_mutating_action(action: &str) -> bool {
    MEMORY_MUTATING_ACTIONS.contains(&action)
}

/// Strictly ASCII-sorted English stopword table for O(log n) binary-search filtering in FTS5 & Jaccard.
pub const MEMORY_STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be", "been",
    "before", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had", "has",
    "have", "how", "in", "into", "is", "it", "its", "just", "may", "more", "not", "now", "of",
    "on", "or", "our", "out", "over", "please", "should", "so", "some", "than", "that", "the",
    "their", "them", "then", "there", "these", "they", "this", "to", "too", "up", "us", "use",
    "was", "we", "were", "what", "when", "where", "which", "who", "will", "with", "would", "you",
    "your",
];

/// Strictly ASCII-sorted generic coding terms excluded from Tier-3 relaxed-OR FTS5 fallbacks
/// so compound identifiers like `huge_test_log` never match unrelated sessions via `"test"`.
pub const MEMORY_GENERIC_CODE_TERMS: &[&str] = &[
    "build", "check", "code", "config", "data", "error", "file", "files", "fix", "help", "info",
    "init", "line", "list", "log", "main", "make", "mode", "name", "new", "next", "note", "path",
    "run", "save", "show", "src", "step", "task", "test", "tests", "text", "time", "tool", "type",
    "update", "used", "user", "work",
];

/// Internal control XML tags stripped from persisted memory entries to block prompt-injection smuggling.
pub const MEMORY_INJECTION_BLOCKED_TAGS: &[&str] = &[
    "system",
    "instructions",
    "system_reminder",
    "situational_awareness",
    "repo_rules",
    "user_profile",
    "memory_snapshot",
    "project_memory_snapshot",
    "memory_delta",
    "recalled_memories",
    "project_memory_invariants",
    "orchestrator_plan",
    "orchestrator_hint",
    "compacted_checkpoint",
    "compacted_history",
    "active_execution_plan",
    "active_task_dag",
    "plan_mode",
    "rewind_summary",
    "lsp_diagnostics",
    "goal_loop_context",
    "background_job_updates",
    "deferred_tools",
    "agent_mailbox",
    "subagent_directive",
    "forked_parent_context",
    "subagent_hook_context",
];

pub const DEFAULT_RATE_LIMITER_MAX_ENTRIES: usize = 1024;
pub const DEFAULT_RATE_LIMITER_TTL_SECS: u64 = 300;

pub const FILE_MODE_OWNER_ONLY: u32 = 0o600;
pub const WEBHOOK_SIGNATURE_HEADER: &str = "x-zenko-signature";

/// Protected internal Zenko files/paths that automated `shell_exec` allowlists must never read.
pub const PROTECTED_DATA_SEGMENTS: &[&str] = &[
    "vault.key",
    "vault.enc",
    "zenko.sock",
    ".zenko/vault",
];

// Local Unix IPC JSON-RPC method registry
pub const IPC_METHOD_STATUS: &str = "status";
pub const IPC_METHOD_ASK: &str = "ask";
pub const IPC_METHOD_STOP: &str = "stop";
pub const IPC_METHOD_PAIR: &str = "pair";
pub const IPC_METHOD_SESSIONS_LIST: &str = "sessions_list";
pub const IPC_METHOD_SESSION_GET: &str = "session_get";
pub const IPC_METHOD_SESSION_CLEAR: &str = "session_clear";
pub const IPC_METHOD_SESSION_DELETE: &str = "session_delete";
pub const IPC_METHOD_APPROVALS_LIST: &str = "approvals_list";
pub const IPC_METHOD_APPROVAL_RESOLVE: &str = "approval_resolve";
pub const IPC_METHOD_MEMORY_SEARCH: &str = "memory_search";
pub const IPC_METHOD_MEMORY_ADD: &str = "memory_add";
pub const IPC_METHOD_CRON_LIST: &str = "cron_list";
pub const IPC_METHOD_CRON_CREATE: &str = "cron_create";
pub const IPC_METHOD_CRON_UPDATE: &str = "cron_update";
pub const IPC_METHOD_CRON_RUN_NOW: &str = "cron_run_now";
pub const IPC_METHOD_CRON_DELETE: &str = "cron_delete";
pub const IPC_METHOD_LOGS_RECENT: &str = "logs_recent";
pub const IPC_METHOD_LOGS_QUERY: &str = "logs_query";
pub const IPC_METHOD_LOGS_FOLLOW: &str = "logs_follow";
pub const IPC_METHOD_LOGS_STATS: &str = "logs_stats";
pub const IPC_METHOD_JOBS_LIST: &str = "jobs_list";
pub const IPC_METHOD_JOBS_START: &str = "jobs_start";
pub const IPC_METHOD_JOBS_READ: &str = "jobs_read";
pub const IPC_METHOD_JOBS_INPUT: &str = "jobs_input";
pub const IPC_METHOD_JOBS_UPDATE: &str = "jobs_update";
pub const IPC_METHOD_JOBS_RESTART: &str = "jobs_restart";
pub const IPC_METHOD_JOBS_STOP: &str = "jobs_stop";
pub const IPC_METHOD_JOBS_DELETE: &str = "jobs_delete";
pub const IPC_METHOD_JOBS_PRUNE: &str = "jobs_prune";
pub const IPC_METHOD_JOBS_WAIT: &str = "jobs_wait";
pub const IPC_METHOD_JOBS_FOLLOW: &str = "jobs_follow";
pub const IPC_METHOD_HUB_LIST: &str = "hub_list";
pub const IPC_METHOD_SESSION_STEER: &str = "session_steer";
pub const IPC_METHOD_SESSION_BACKGROUND: &str = "session_background";
pub const IPC_METHOD_SESSION_QUEUE_LIST: &str = "session_queue_list";
pub const IPC_METHOD_SESSION_QUEUE_CANCEL: &str = "session_queue_cancel";
pub const IPC_METHOD_SESSION_ATTACH: &str = "session_attach";
pub const IPC_METHOD_CLARIFICATIONS_LIST: &str = "clarifications_list";
pub const IPC_METHOD_CLARIFICATION_RESOLVE: &str = "clarification_resolve";
pub const IPC_METHOD_ORCHESTRATOR_TRIAGE: &str = "orchestrator_triage";
pub const IPC_METHOD_SESSION_CONTEXT: &str = "session_context";
pub const IPC_METHOD_SESSIONS_SEARCH: &str = "sessions_search";
pub const IPC_METHOD_MEMORY_UPDATE: &str = "memory_update";
pub const IPC_METHOD_MEMORY_DELETE: &str = "memory_delete";
pub const IPC_METHOD_MEMORY_DREAM: &str = "memory_dream";
pub const IPC_METHOD_MEMORY_HISTORY: &str = "memory_history";
pub const IPC_METHOD_MEMORY_ROLLBACK: &str = "memory_rollback";
pub const IPC_METHOD_AGENTS_LIST: &str = "agents_list";
pub const IPC_METHOD_AGENTS_SPAWN: &str = "agents_spawn";
pub const IPC_METHOD_AGENTS_GET: &str = "agents_get";
pub const IPC_METHOD_AGENTS_MESSAGE: &str = "agents_message";
pub const IPC_METHOD_AGENTS_WAIT: &str = "agents_wait";
pub const IPC_METHOD_AGENTS_KILL: &str = "agents_kill";
pub const IPC_METHOD_AGENTS_DIFF: &str = "agents_diff";
pub const IPC_METHOD_AGENTS_MERGE: &str = "agents_merge";
pub const IPC_METHOD_AGENTS_DISCARD: &str = "agents_discard";
pub const IPC_METHOD_AGENTS_PRUNE: &str = "agents_prune";
pub const IPC_METHOD_TASKS_DAG_LIST: &str = "tasks_dag_list";
pub const IPC_METHOD_TASKS_DAG_CREATE: &str = "tasks_dag_create";
pub const IPC_METHOD_TASKS_DAG_UPDATE: &str = "tasks_dag_update";
pub const IPC_METHOD_TASKS_DAG_DELETE: &str = "tasks_dag_delete";
pub const IPC_METHOD_CHECKPOINTS_LIST: &str = "checkpoints_list";
pub const IPC_METHOD_CHECKPOINTS_DIFF: &str = "checkpoints_diff";
pub const IPC_METHOD_CHECKPOINTS_REWIND: &str = "checkpoints_rewind";
pub const IPC_METHOD_CHECKPOINTS_PRUNE: &str = "checkpoints_prune";
pub const IPC_METHOD_SESSION_APPROVE_PLAN: &str = "session_approve_plan";
pub const IPC_METHOD_MCP_SERVERS_LIST: &str = "mcp_servers_list";
pub const IPC_METHOD_MCP_SERVER_UPSERT: &str = "mcp_server_upsert";
pub const IPC_METHOD_MCP_SERVER_DELETE: &str = "mcp_server_delete";
pub const IPC_METHOD_MCP_SERVER_SYNC: &str = "mcp_server_sync";
pub const IPC_METHOD_LSP_STATUS: &str = "lsp_status";
pub const IPC_METHOD_LSP_STOP: &str = "lsp_stop";
pub const IPC_METHOD_SKILLS_LIST: &str = "skills_list";
pub const IPC_METHOD_SKILL_GET: &str = "skill_get";
pub const IPC_METHOD_SKILL_MANAGE: &str = "skill_manage";
pub const IPC_METHOD_SKILL_APPROVE: &str = "skill_approve";
pub const IPC_METHOD_SKILL_HISTORY: &str = "skill_history";
pub const IPC_METHOD_SKILL_ROLLBACK: &str = "skill_rollback";
pub const IPC_METHOD_GOALS_LIST: &str = "goals_list";
pub const IPC_METHOD_GOAL_GET: &str = "goal_get";
pub const IPC_METHOD_GOAL_CREATE: &str = "goal_create";
pub const IPC_METHOD_GOAL_UPDATE: &str = "goal_update";
pub const IPC_METHOD_GOAL_RUN_NOW: &str = "goal_run_now";
pub const IPC_METHOD_GOAL_DELETE: &str = "goal_delete";
pub const IPC_METHOD_TRIGGERS_LIST: &str = "triggers_list";
pub const IPC_METHOD_EXPORT_SESSION: &str = "export_session";

// Autonomous Orchestrator, Brainstorming, DAG Plan & Core Tool Registry constants (zero hardcoding)
pub const TOOL_READ_FILE: &str = "read_file";
pub const TOOL_WRITE_FILE: &str = "write_file";
pub const TOOL_HASH_EDIT: &str = "hash_edit";
pub const TOOL_APPLY_PATCH: &str = "apply_patch";
pub const TOOL_LIST_DIR: &str = "list_dir";
pub const TOOL_CODE_SEARCH: &str = "code_search";
pub const TOOL_WEB_FETCH: &str = "web_fetch";
pub const TOOL_MEMORY: &str = "memory_tool";
pub const TOOL_CRON: &str = "cron_tool";
pub const TOOL_RECORD_BRAINSTORM: &str = "record_brainstorm";
pub const TOOL_ASK_CLARIFICATION: &str = "ask_clarification";
pub const TOOL_PLAN_UPDATE: &str = "plan_update";
pub const TOOL_SESSION_TASK: &str = "session_task";
pub const TOOL_ORCHESTRATE_CONTROL: &str = "orchestrate_control";
pub const TOOL_SHELL_EXEC: &str = "shell_exec";
pub const TOOL_JOB_LIST: &str = "job_list";
pub const TOOL_JOB_READ: &str = "job_read";
pub const TOOL_JOB_WAIT: &str = "job_wait";
pub const TOOL_JOB_INPUT: &str = "job_input";
pub const TOOL_JOB_STOP: &str = "job_stop";
pub const TOOL_TASK_MANAGE: &str = "task_manage";
pub const TOOL_SKILL_VIEW: &str = "skill_view";
pub const TOOL_TOOL_SEARCH: &str = "tool_search";
pub const TOOL_CONTEXT_INSPECT: &str = "context_inspect";
pub const TOOL_SESSION_SEARCH: &str = "session_search";
pub const TOOL_SPAWN_AGENT: &str = "spawn_agent";
pub const TOOL_AGENT_LIST: &str = "agent_list";
pub const TOOL_AGENT_MESSAGE: &str = "agent_message";
pub const TOOL_AGENT_WAIT: &str = "agent_wait";
pub const TOOL_AGENT_KILL: &str = "agent_kill";
pub const TOOL_AGENT_WORKTREE: &str = "agent_worktree";
pub const TOOL_BATCH_TOOLS: &str = "batch_tools";
pub const TOOL_INVOKE_TOOL: &str = "invoke_tool";

// Minicode Tool System v2 constants (zero hardcoding)
pub const BATCH_TOOLS_MAX_CALLS: usize = 12;
pub const BATCH_TOOLS_MIN_STEP_CHARS: usize = 256;
pub const BATCH_TOOLS_DEFAULT_STEP_CHARS: usize = 4_000;
pub const BATCH_TOOLS_MAX_STEP_CHARS: usize = 16_000;
pub const READ_FILE_MAX_RANGES: usize = 8;
pub const FS_MAX_EDIT_FILE_BYTES: u64 = 4 * 1024 * 1024;
pub const FS_MAX_SINGLE_READ_LINE_BYTES: usize = 128 * 1024;
pub const JAIL_MAX_SYMLINK_HOPS: usize = 32;
pub const LSP_LOCATION_SNIPPET_MAX_BYTES: u64 = 256 * 1024;
pub const HASH_EDIT_MAX_HUNKS: usize = 32;
pub const HASH_EDIT_RECOVERY_RADIUS: usize = 4;
pub const APPLY_PATCH_MAX_EDITS: usize = 32;
pub const APPLY_PATCH_AMBIGUITY_MAX_LOCS: usize = 12;
pub const OUTLINE_MAX_ENTRIES: usize = 250;
pub const OUTLINE_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const OUTLINE_MAX_SIGNATURE_CHARS: usize = 120;
pub const OUTLINE_MAX_BRACE_DEPTH: i32 = 2;
pub const CODE_SEARCH_DEFAULT_MAX_RESULTS: usize = 60;
pub const CODE_SEARCH_MAX_RESULTS_CAP: usize = 200;
pub const CODE_SEARCH_MAX_CONTEXT_LINES: usize = 5;
pub const TOOL_SEARCH_FUZZY_MAX_SUGGESTIONS: usize = 3;
pub const TOOL_SCHEMA_HINT_MAX_PROPS: usize = 10;
pub const TOOL_COERCE_MAX_DEPTH: usize = 8;
pub const TOOL_COERCE_MAX_JSON_REPAIR_BYTES: usize = 256 * 1024;
pub const COERCE_SAFE_INT_MAX_F64: f64 = 9_007_199_254_740_992.0;
pub const TOOL_BM25_K1: f64 = 1.2;
pub const TOOL_BM25_B: f64 = 0.75;
pub const TOOL_BM25_PREFIX_TF_WEIGHT: f64 = 0.5;
pub const TOOL_BM25_EXACT_NAME_BOOST: f64 = 6.0;
pub const TOOL_BM25_SUBSTR_NAME_BOOST: f64 = 2.5;
pub const TOOL_FUZZY_EDIT_WEIGHT: f64 = 0.55;
pub const TOOL_FUZZY_TOKEN_WEIGHT: f64 = 0.45;
pub const TOOL_FUZZY_SUBSTR_BONUS: f64 = 0.35;
pub const TOOL_FUZZY_MIN_SCORE: f64 = 0.30;

// Minicode Tool System v3 constants (Claude Code + Codex + Oh My Pi synthesis)
pub const APPLY_PATCH_MAX_FILES: usize = 24;
pub const SESSION_FILE_TRACKER_MAX_SESSIONS: usize = 64;
pub const SESSION_FILE_TRACKER_MAX_FILES_PER_SESSION: usize = 512;
pub const FILE_FRESHNESS_PREVIEW_LINES: usize = 20;
pub const WEB_FETCH_DEFAULT_MAX_CHARS: usize = 10_000;
pub const WEB_FETCH_MIN_MAX_CHARS: usize = 500;
pub const WEB_FETCH_HARD_MAX_CHARS: usize = 32_000;
pub const WEB_FETCH_CACHE_MAX_ENTRIES: usize = 64;
pub const WEB_FETCH_CACHE_TTL_SECS: u64 = 900;
pub const LSP_RENAME_MAX_FILES: usize = 64;
pub const LSP_RENAME_MAX_EDITS: usize = 512;
pub const HASH_EDIT_OPS: &[&str] = &["replace", "insert_after", "insert_before", "delete"];
pub const PATCH_FILE_ACTIONS: &[&str] = &["update", "create", "delete", "move"];
pub const WEB_FETCH_FORMATS: &[&str] = &["markdown", "text", "raw"];

// Minicode Tool System v4 constants (KV-cache frozen tools, Landlock sandbox, PTY sessions, dataflow batching)
pub const PTY_MAX_SESSIONS_PER_CHAT: usize = 4;
pub const PTY_MAX_TOTAL_SESSIONS: usize = 32;
pub const PTY_IDLE_TIMEOUT_SECS: u64 = 900;
pub const PTY_DEFAULT_COLS: u16 = 120;
pub const PTY_DEFAULT_ROWS: u16 = 36;
pub const PTY_READ_BUF_BYTES: usize = 4_096;
pub const PTY_MAX_OUTPUT_BYTES: usize = 256 * 1024;
pub const PTY_SEND_DEFAULT_WAIT_MS: u64 = 350;
pub const PTY_SEND_MAX_WAIT_MS: u64 = 10_000;
pub const PTY_ACTIONS: &[&str] = &["exec", "send", "close", "list"];
pub const BATCH_EXTRACTS: &[&str] = &[
    "raw",
    "lines",
    "first_line",
    "paths",
    "contains:<text>",
    "head:<n>",
    "tail:<n>",
];
pub const BATCH_MAX_VAR_BYTES: usize = 64 * 1024;

/// Declarative table of tools representing background-task or subagent orchestration/monitoring.
/// When a running session has active jobs, active subagents, or any of these tools in flight,
/// `SendMode::Auto` deterministically routes incoming user guidance as live `Steer` instead of `Queue`.
pub const ORCH_MONITORING_TOOLS: &[&str] = &[
    TOOL_AGENT_KILL,
    TOOL_AGENT_LIST,
    TOOL_AGENT_MESSAGE,
    TOOL_AGENT_WAIT,
    TOOL_AGENT_WORKTREE,
    TOOL_JOB_INPUT,
    TOOL_JOB_LIST,
    TOOL_JOB_READ,
    TOOL_JOB_STOP,
    TOOL_JOB_WAIT,
    TOOL_SPAWN_AGENT,
    TOOL_TASK_MANAGE,
];

pub const ESCALATION_OPT_PIVOT: &str = "pivot_approach";
pub const ESCALATION_OPT_STOP: &str = "stop_turn";

pub const NOTE_KIND_BRAINSTORM_SPEC: &str = "brainstorm_spec";
pub const NOTE_KIND_EXECUTION_PLAN: &str = "execution_plan";
pub const NOTE_KIND_CLARIFICATION_DECISION: &str = "clarification_decision";
pub const NOTE_KIND_DEFERRED_TOOLS: &str = "deferred_tools_activated";
pub const NOTE_KIND_MEMORY_RECALL: &str = "memory_recall";
pub const NOTE_KIND_SUBAGENT_REPORT: &str = "subagent_report";
pub const NOTE_KIND_REWIND_SUMMARY: &str = "rewind_summary";
pub const NOTE_KIND_PLAN_APPROVED: &str = "plan_approved";
pub const NOTE_KIND_ORCHESTRATE_CONTROL: &str = "orchestrate_control";

pub const ORCH_STAGE_TRIAGED: &str = "triaged";
pub const ORCH_STAGE_POSTURE: &str = "posture";
pub const ORCH_STAGE_GEAR_SHIFT: &str = "gear_shift";
pub const ORCH_STAGE_RECON: &str = "recon";
pub const ORCH_STAGE_INTENT_GATE: &str = "intent_gate";
pub const ORCH_STAGE_PHASE_GATE: &str = "phase_gate";
pub const ORCH_STAGE_PLAN_UPDATED: &str = "plan_updated";
pub const ORCH_STAGE_CIRCUIT_BREAKER: &str = "circuit_breaker";
pub const ORCH_STAGE_ESCALATED: &str = "escalated";
pub const ORCH_STAGE_VERIFICATION_GATE: &str = "verification_gate";
pub const ORCH_STAGE_BUDGET_GUARD: &str = "budget_guard";
pub const ORCH_STAGE_PACING: &str = "pacing";
pub const ORCH_STAGE_SYNTHESIS: &str = "synthesis";
pub const ORCH_STAGE_TRUNCATION_RECOVERY: &str = "truncation_recovery";
pub const ORCH_STAGE_MICROCOMPACT: &str = "microcompact";
pub const ORCH_STAGE_COMPACTION: &str = "compaction";
pub const ORCH_STAGE_TOOL_ACTIVATED: &str = "tool_activated";
pub const ORCH_STAGE_DREAM: &str = "dream";
pub const ORCH_STAGE_SUBAGENT: &str = "subagent";
pub const ORCH_STAGE_PLAN_MODE: &str = "plan_mode";
pub const ORCH_STAGE_TASK_DAG: &str = "task_dag";
pub const ORCH_STAGE_CHECKPOINT: &str = "checkpoint";
pub const ORCH_STAGE_REWIND: &str = "rewind";
pub const ORCH_STAGE_MCP: &str = "mcp";
pub const ORCH_STAGE_SKILL: &str = "skill";
pub const ORCH_STAGE_GOAL_LOOP: &str = "goal_loop";
pub const ORCH_STAGE_TRIGGER: &str = "trigger";

pub const TELEGRAM_CB_APPROVAL_PREFIX: &str = "apr";
pub const TELEGRAM_CB_CLARIFY_PREFIX: &str = "clr";
pub const TELEGRAM_CB_SKILL_PREFIX: &str = "ska";

pub const ORCH_INSTANT_MAX_SCORE: i32 = 1;
pub const ORCH_ORCHESTRATED_MIN_SCORE: i32 = 5;
pub const ORCH_AMBIGUITY_ASK_THRESHOLD: i32 = 4;
pub const ORCH_RECON_MAX_FILES: usize = 24;
pub const ORCH_RECON_MAX_MEMORIES: usize = 6;
pub const ORCH_RECON_MAX_TASKS: usize = 6;
pub const ORCH_RECON_MAX_GIT_FILES: usize = 12;
pub const ORCH_RECON_MAX_MENTIONED_FILES: usize = 6;
pub const ORCH_RECON_MAX_PROBE_BYTES: u64 = 262_144;
pub const ORCH_RECON_GIT_TIMEOUT_MS: u64 = 1_200;
pub const ORCH_LOOP_WINDOW_SIZE: usize = 14;
pub const ORCH_LOOP_REPEAT_THRESHOLD: usize = 3;
pub const ORCH_NON_ERROR_REPEAT_THRESHOLD: usize = 3;
pub const ORCH_OSCILLATION_THRESHOLD: usize = 3;
pub const ORCH_MAX_CONSECUTIVE_ERRORS: usize = 4;
pub const ORCH_MAX_PIVOTS_BEFORE_ESCALATE: usize = 2;
pub const ORCH_MAX_VERIFY_NUDGES: usize = 2;
pub const ORCH_MAX_PLAN_CONTINUATION_NUDGES: usize = 2;
pub const ORCH_MAX_TOTAL_CONTINUATION_NUDGES: usize = 4;
pub const ORCH_MAX_PHASE_GATE_NUDGES: usize = 1;
pub const ORCH_RESEARCH_EXPLORE_SOFT_LIMIT: usize = 4;
pub const ORCH_ADVISORY_EXPLORE_SOFT_LIMIT: usize = 3;
pub const ORCH_IMPLEMENT_PRE_PLAN_SOFT_LIMIT: usize = 3;
pub const ORCH_ITER_WRAPUP_REMAINING: usize = 3;
pub const ORCH_REASONING_MIN_MAX_TOKENS: u32 = 16_384;
pub const ORCH_REASONING_MAX_OUTPUT_CAP: u32 = 32_768;
pub const ORCH_MAX_TRUNCATION_RETRIES: usize = 2;
pub const ORCH_HIGH_RISK_MIN_SCORE: i32 = 3;
pub const ORCH_DELIBERATE_MIN_OPTIONS: usize = 2;
pub const ORCH_DELIBERATE_MIN_PREMORTEM: usize = 2;
pub const ORCH_TEXT_DOOM_MIN_LINES: usize = 6;
pub const ORCH_TEXT_DOOM_REPEAT_THRESHOLD: usize = 4;
pub const ORCH_TEXT_DOOM_MIN_LINE_CHARS: usize = 18;
pub const ORCH_CLARIFICATION_TIMEOUT_SECS: u64 = 180;
pub const ORCH_BRAINSTORM_MAX_OPTIONS: usize = 6;
pub const ORCH_PLAN_MAX_STEPS: usize = 16;

// Evidence-weighted brainstorm scoring constants (anti-confirmation-bias)
pub const ORCH_DEFAULT_OPTION_SCORE: u8 = 7;
pub const ORCH_SCORE_SAFETY_MULT: u32 = 3;
pub const ORCH_SCORE_TESTABILITY_MULT: u32 = 2;
pub const ORCH_SCORE_REV_HIGH: u32 = 15;
pub const ORCH_SCORE_REV_MED: u32 = 8;
pub const ORCH_SCORE_REV_LOW: u32 = 2;
pub const ORCH_SCORE_BLAST_LOW: u32 = 12;
pub const ORCH_SCORE_BLAST_MED: u32 = 6;
pub const ORCH_SCORE_BLAST_HIGH: u32 = 1;
pub const ORCH_SCORE_REC_BONUS: u32 = 4;
pub const ORCH_SCORE_EVIDENCE_ROLLBACK_BONUS: u32 = 6;
pub const ORCH_SCORE_EVIDENCE_RISKS_BONUS: u32 = 3;
pub const ORCH_SCORE_PREMORTEM_BONUS: u32 = 4;
pub const ORCH_SCORE_INFLATED_SAFETY_PENALTY: u32 = 10;
pub const ORCH_SCORE_WIDE_UNPROTECTED_PENALTY: u32 = 8;
pub const ORCH_SCORE_WIDE_FILES_THRESHOLD: usize = 4;

pub const CLARIFICATION_RESOLVER_HEADLESS: &str = "orchestrator:headless_auto";
pub const CLARIFICATION_RESOLVER_TIMEOUT: &str = "orchestrator:timeout_default";
pub const CLARIFICATION_RESOLVER_STEERED: &str = "orchestrator:steered";
pub const CLARIFICATION_RESOLVER_ESCALATED: &str = "orchestrator:escalated_auto";

pub const READ_ONLY_TOOLS: &[&str] = &[
    TOOL_READ_FILE,
    TOOL_LIST_DIR,
    TOOL_CODE_SEARCH,
    TOOL_GLOB_FIND,
    TOOL_WEB_FETCH,
    TOOL_WEB_SEARCH,
    TOOL_MCP_RESOURCE,
    TOOL_JOB_LIST,
    TOOL_JOB_READ,
    TOOL_SKILL_VIEW,
    TOOL_TOOL_SEARCH,
    TOOL_CONTEXT_INSPECT,
    TOOL_SESSION_SEARCH,
    TOOL_AGENT_LIST,
    TOOL_AGENT_WAIT,
];
pub const PLANNING_TOOLS: &[&str] = &[
    TOOL_RECORD_BRAINSTORM,
    TOOL_ASK_CLARIFICATION,
    TOOL_PLAN_UPDATE,
    TOOL_SESSION_TASK,
    TOOL_ORCHESTRATE_CONTROL,
];
/// Tools that are conditionally permitted in Read-Only Plan Mode (enforced per-argument by ExecPolicy):
/// read-only shell commands, read-only memory actions, read-only subagent roles, worktree diff,
/// read-only git actions, read-only LSP actions (plus `rename` with `dry_run: true`), read-only
/// `skill_manage` queries, read-only `goal_loop` queries, and `batch_tools` (each inner call
/// individually policy-enforced).
pub const PLAN_MODE_SCOPED_TOOLS: &[&str] = &[
    TOOL_SHELL_EXEC,
    TOOL_MEMORY,
    TOOL_SPAWN_AGENT,
    TOOL_AGENT_WORKTREE,
    TOOL_GIT,
    TOOL_LSP,
    TOOL_SKILL_MANAGE,
    TOOL_GOAL_LOOP,
    TOOL_BATCH_TOOLS,
    TOOL_INVOKE_TOOL,
];
pub const MUTATING_FILE_TOOLS: &[&str] = &[TOOL_WRITE_FILE, TOOL_HASH_EDIT, TOOL_APPLY_PATCH];
pub const VERIFICATION_SHELL_TOOLS: &[&str] = &[TOOL_SHELL_EXEC, TOOL_JOB_WAIT];

/// Bulky ephemeral tools eligible for Stage-A lossless microcompaction once outside the recent tail.
pub const MICROCOMPACTABLE_TOOLS: &[&str] = &[
    TOOL_READ_FILE,
    TOOL_CODE_SEARCH,
    TOOL_GLOB_FIND,
    TOOL_LIST_DIR,
    TOOL_GIT,
    TOOL_SHELL_EXEC,
    TOOL_WEB_FETCH,
    TOOL_WEB_SEARCH,
    TOOL_MCP_RESOURCE,
    TOOL_JOB_READ,
];
/// High-signal state/planning tools strictly exempt from microcompaction (prevents working-memory amnesia).
pub const COMPACTION_PROTECTED_TOOLS: &[&str] = &[
    TOOL_RECORD_BRAINSTORM,
    TOOL_PLAN_UPDATE,
    TOOL_SESSION_TASK,
    TOOL_ASK_CLARIFICATION,
    TOOL_ORCHESTRATE_CONTROL,
    TOOL_MEMORY,
    TOOL_SKILL_VIEW,
    TOOL_SKILL_MANAGE,
    TOOL_GOAL_LOOP,
    TOOL_TOOL_SEARCH,
    TOOL_CONTEXT_INSPECT,
    TOOL_SESSION_SEARCH,
    TOOL_SPAWN_AGENT,
    TOOL_AGENT_LIST,
    TOOL_AGENT_MESSAGE,
    TOOL_AGENT_WAIT,
    TOOL_AGENT_KILL,
];

pub const REPO_MARKER_FILES: &[(&str, &str)] = &[
    ("Cargo.toml", "rust/cargo"),
    ("package.json", "node/npm"),
    ("pyproject.toml", "python/pyproject"),
    ("requirements.txt", "python/pip"),
    ("go.mod", "go/module"),
    ("Makefile", "make"),
    ("docker-compose.yml", "docker-compose"),
    ("Dockerfile", "docker"),
    ("ZENKO.md", "zenko-rules"),
    ("AGENTS.md", "agents-rules"),
    ("CLAUDE.md", "claude-rules"),
];

pub const VERIFICATION_CMD_PREFIXES: &[&[&str]] = &[
    &["cargo", "pretty", "check"],
    &["cargo", "pretty", "test"],
    &["cargo", "pretty", "build"],
    &["cargo", "pretty", "clippy"],
    &["cargo", "check"],
    &["cargo", "test"],
    &["cargo", "build"],
    &["cargo", "clippy"],
    &["rustc"],
    &["npm", "test"],
    &["npm", "run", "test"],
    &["npm", "run", "build"],
    &["npm", "run", "lint"],
    &["npm", "run", "typecheck"],
    &["pnpm", "test"],
    &["pnpm", "run", "build"],
    &["pnpm", "run", "test"],
    &["bun", "test"],
    &["bun", "run", "build"],
    &["npx", "tsc"],
    &["tsc"],
    &["eslint"],
    &["pytest"],
    &["uv", "run", "pytest"],
    &["python3", "-m", "pytest"],
    &["python3", "-m", "unittest"],
    &["ruff", "check"],
    &["mypy"],
    &["go", "test"],
    &["go", "build"],
    &["make", "test"],
    &["make", "check"],
    &["vitest"],
    &["jest"],
];

/// Case-insensitive CLI flags/subcommands that only print version/help text (never real verification).
pub const VERIFICATION_SPOOF_FLAGS_CI: &[&str] = &["--version", "--help", "help"];
/// Case-sensitive short flags that print version/help (`-V` = version, `-h` = help; lowercase `-v` is verbose!).
pub const VERIFICATION_SPOOF_FLAGS_EXACT: &[&str] = &["-V", "-h"];

/// Declarative strictly read-only shell command prefixes permitted during read-only Intent Gate turns.
pub const READ_ONLY_SHELL_CMD_PREFIXES: &[&[&str]] = &[
    &["ls"],
    &["pwd"],
    &["cd"],
    &["echo"],
    &["cat"],
    &["head"],
    &["tail"],
    &["wc"],
    &["rg"],
    &["grep"],
    &["stat"],
    &["file"],
    &["tree"],
    &["which"],
    &["whoami"],
    &["uname"],
    &["printenv"],
    &["ps"],
    &["ss"],
    &["lsof"],
    &["df"],
    &["du"],
    &["free"],
    &["uptime"],
    &["id"],
    &["date"],
    &["realpath"],
    &["readlink"],
    &["git", "status"],
    &["git", "diff"],
    &["git", "log"],
    &["git", "show"],
    &["git", "ls-files"],
    &["git", "blame"],
    &["git", "rev-parse"],
    &["git", "branch", "--show-current"],
    &["git", "branch", "--list"],
    &["cargo", "metadata"],
    &["cargo", "tree"],
    &["cargo", "check"],
    &["cargo", "pretty", "check"],
    &["find"],
];

/// Mutating flags disallowed when `find` is invoked during a read-only turn.
pub const READ_ONLY_FIND_MUTATING_FLAGS: &[&str] = &[
    "-delete",
    "-exec",
    "-execdir",
    "-ok",
    "-okdir",
    "-fprint",
    "-fprint0",
    "-fprintf",
    "-fls",
];

// Categorized persistent log storage & flight-recorder constants
pub const LOG_DIR_NAME: &str = "logs";
pub const LOG_SESSIONS_SUBDIR: &str = "sessions";
pub const LOG_WIRE_SUBDIR: &str = "wire";
pub const LOG_CRASH_FILE: &str = "crash.log";
pub const LOG_ALL_FILE: &str = "all.jsonl";
pub const LOG_SESSION_INDEX_FILE: &str = "index.jsonl";
pub const LOG_GATEWAY_FILE: &str = "gateway.jsonl";
pub const LOG_WEBUI_FILE: &str = "webui.jsonl";
pub const LOG_TUI_FILE: &str = "tui.jsonl";
pub const LOG_CRON_FILE: &str = "cron.jsonl";
pub const LOG_SYSTEM_FILE: &str = "system.jsonl";

pub const LOG_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
pub const LOG_MAX_ROTATED_FILES: usize = 4;
pub const LOG_MAX_SESSION_FILES: usize = 100;
pub const LOG_SQLITE_MAX_ROWS: usize = 10_000;
pub const LOG_SQLITE_PRUNE_INTERVAL: u64 = 128;
pub const LOG_WRITER_CHANNEL_CAP: usize = 1024;
pub const LOG_WIRE_MAX_FILES: usize = 40;

pub const ENV_DEBUG_WIRE: &str = "ZENKO_DEBUG_WIRE";
pub const REDACTED_PLACEHOLDER: &str = "[REDACTED]";
pub const SECRET_TOKEN_PREFIXES: &[&str] = &[
    "sk-or-",
    "sk-ant-",
    "sk-proj-",
    "sk-",
    "ghp_",
    "gho_",
    "xoxb-",
    "xoxp-",
];

pub const SYSTEMD_UNIT_NAME: &str = "zenko.service";

// MCP stdio server protocol metadata
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
pub const MCP_SERVER_NAME: &str = "zenko";
pub const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// Deterministic CLI exit codes for scripts & agent-to-agent callers
pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_DAEMON_STOPPED: i32 = 3;

pub const DEFAULT_TOKIO_WORKERS: usize = 2;
pub const DEFAULT_TOKIO_BLOCKING_THREADS: usize = 4;
pub const DEFAULT_SHUTDOWN_GRACE_SECS: u64 = 2;
pub const DEFAULT_SSE_KEEPALIVE_SECS: u64 = 15;

// Vault key registry
pub const VAULT_KEY_OPENROUTER: &str = "OPENROUTER_API_KEY";
pub const VAULT_KEY_MINIMAX: &str = "MINIMAX_API_KEY";
pub const VAULT_KEY_OPENAI: &str = "OPENAI_API_KEY";
pub const VAULT_KEY_DEEPSEEK: &str = "DEEPSEEK_API_KEY";
pub const VAULT_KEY_GEMINI: &str = "GEMINI_API_KEY";
pub const VAULT_KEY_OLLAMA: &str = "OLLAMA_API_KEY";
pub const VAULT_KEY_CUSTOM_PROVIDER: &str = "CUSTOM_PROVIDER_API_KEY";
pub const VAULT_KEY_TELEGRAM: &str = "TELEGRAM_BOT_TOKEN";
pub const VAULT_KEY_WEBUI_ADMIN: &str = "WEBUI_ADMIN_TOKEN";
pub const VAULT_KEY_WEBHOOK_SECRET: &str = "WEBHOOK_HMAC_SECRET";
pub const VAULT_KEY_BRAVE_SEARCH: &str = "BRAVE_API_KEY";
pub const VAULT_KEY_TAVILY_SEARCH: &str = "TAVILY_API_KEY";

// Provider protocol constants
pub const PROVIDER_HTTP_REFERER: &str = "https://github.com/aswin402/zenko";
pub const PROVIDER_X_TITLE: &str = "Zenko Autonomous Agent";
pub const MINIMAX_TEMP_MIN: f32 = 0.01;
pub const MINIMAX_TEMP_MAX: f32 = 1.0;

pub const DEFAULT_ALLOWED_SHELL_PREFIXES: &[&str] = &[
    "ls",
    "pwd",
    "cat",
    "head",
    "tail",
    "wc",
    "git status",
    "git diff",
    "git log",
    "cargo check",
    "cargo test",
    "rg",
    "find",
];

pub const DEFAULT_DENIED_SHELL_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "mkfs",
    "dd if=",
    ":(){ :|:& };:",
    "chmod -R 777 /",
];

/// Declarative binaries that ALWAYS require interactive user approval in default Autonomous mode.
pub const DANGEROUS_APPROVAL_BINS: &[&str] = &[
    "sudo",
    "su",
    "doas",
    "pkexec",
    "shutdown",
    "reboot",
    "poweroff",
    "halt",
    "fdisk",
    "parted",
    "wipefs",
    "shred",
    "dd",
    "killall",
    "dropdb",
];

/// Shell interpreters that require approval when executed as a pipe sink (e.g. `curl ... | sh`).
pub const PIPE_TO_SHELL_BINS: &[&str] = &["sh", "bash", "zsh", "dash", "fish", "ksh"];

// Dynamic live model discovery & metadata hydration constants (zero hardcoded model catalogs)
pub const MODEL_METADATA_REGISTRY_URL: &str = "https://models.dev/api.json";
pub const MODEL_CACHE_TTL_SECS: u64 = 3_600;
pub const MODEL_KV_REGISTRY_KEY: &str = "models_dev_registry_v1";
pub const MODEL_KV_COMPACT_REGISTRY_KEY: &str = "models_dev_compact_v2";
pub const MODEL_KV_LEGACY_COMPACT_THRESHOLD_BYTES: usize = 512 * 1024;
pub const MODEL_KV_PROVIDER_PREFIX: &str = "models_cache_v1:";
pub const OLLAMA_SHOW_TIMEOUT_SECS: u64 = 4;
pub const OLLAMA_SHOW_CONCURRENCY: usize = 6;

// Low-RAM runtime, ring-buffer, SQLite WAL & idle compaction budgets
pub const DEFAULT_EVENT_BROADCAST_CAPACITY: usize = 128;
pub const DEFAULT_LOG_BUFFER_CAPACITY: usize = 200;
pub const SQLITE_JOURNAL_SIZE_LIMIT_BYTES: u64 = 4 * 1024 * 1024;
pub const SQLITE_VACUUM_FREELIST_MIN_BYTES: u64 = 2 * 1024 * 1024;
pub const EXTERNAL_TOOL_REAPER_INTERVAL_SECS: u64 = 60;
pub const IDLE_DB_COMPACT_EVERY_TICKS: u64 = 5;

pub const DEPRECATED_MODEL_STATUSES: &[&str] = &[
    "deprecated",
    "archived",
    "sunset",
    "disabled",
    "offline",
];

/// Declarative non-chat modality segments filtered out when providers omit modality arrays.
pub const NON_CHAT_MODEL_SEGMENTS: &[&str] = &[
    "embedding",
    "embed-",
    "tts-",
    "whisper",
    "dall-e",
    "moderation",
    "audio-preview",
    "realtime",
    "transcribe",
    "speech-",
    "music-",
    "video-",
    "hailuo",
    "image-01",
    "babbage",
    "davinci",
];

// Unified Hono-style structured logger constants
pub const LOG_FIELD_DELIMITER: char = '\t';
pub const LOG_SSE_SKIP_PATH: &str = "/api/events";
pub const LOG_PREVIEW_INPUT_CHARS: usize = 72;
pub const LOG_PREVIEW_ARGS_CHARS: usize = 68;
pub const LOG_PREVIEW_APPROVAL_ARGS_CHARS: usize = 54;
pub const LOG_PREVIEW_ERR_CHARS: usize = 84;
pub const LOG_PREVIEW_ARG_VAL_CHARS: usize = 36;
pub const LOG_PREVIEW_COMPLEX_VAL_CHARS: usize = 28;
pub const LOG_MAX_ARG_KEYS: usize = 3;
