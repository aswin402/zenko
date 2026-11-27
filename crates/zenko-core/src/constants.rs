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
/// Quantized epoch step for Stage-A microcompaction cut boundaries so the KV prefix stays
/// byte-identical across consecutive turns instead of shifting by 1 message every iteration.
pub const MICROCOMPACT_EPOCH_STEP: usize = 4;
pub const COMPACTION_MIN_EVENTS_BETWEEN_CHECKPOINTS: usize = 8;
pub const COMPACTION_MAX_CHECKPOINT_TOKENS: u32 = 2_048;
pub const COMPACTION_MIN_USABLE_INPUT_TOKENS: usize = 4_096;
pub const COMPACTION_MAX_OUTPUT_RESERVE_NUM: u64 = 30;
pub const COMPACTION_MAX_OUTPUT_RESERVE_DEN: u64 = 100;
pub const COMPACTION_MAX_TRANSCRIPT_CHARS: usize = 28_000;
pub const COMPACTION_MAX_DETERMINISTIC_CHARS: usize = 4_800;
pub const COMPACTION_ANCHOR_GOAL_MAX_CHARS: usize = 600;
pub const COMPACTION_LEDGER_MAX_FILES_PER_KIND: usize = 32;
pub const COMPACTION_LEDGER_MAX_ARTIFACTS: usize = 16;
pub const COMPACTION_OVERFLOW_MAX_RETRIES: usize = 2;
pub const COMPACTION_OVERFLOW_BPT_DECAY: f64 = 0.72;
pub const COMPACTION_OVERFLOW_MICRO_TRIGGER: f64 = 0.30;
pub const COMPACTION_OVERFLOW_BUDGET_RATIOS: &[f64] = &[0.58, 0.45];
pub const COMPACTION_EMERGENCY_SINGLE_MSG_MAX_CHARS: usize = 8_000;

// Real-token self-calibration (EMA bytes-per-token from live provider `usage.prompt_tokens`)
pub const TOKEN_CALIBRATION_DEFAULT_BYTES_PER_TOK: f64 = 3.5;
pub const TOKEN_CALIBRATION_MIN_BYTES_PER_TOK: f64 = 1.6;
pub const TOKEN_CALIBRATION_MAX_BYTES_PER_TOK: f64 = 5.0;
pub const TOKEN_CALIBRATION_EMA_ALPHA: f64 = 0.35;
pub const TOKEN_CALIBRATION_MIN_SAMPLE_BYTES: usize = 2_048;

/// Declarative case-insensitive substrings identifying provider HTTP 400 context-window overflow
/// errors for automatic reactive compaction & retry (zero regex).
pub const PROVIDER_CONTEXT_OVERFLOW_SIGNATURES: &[&str] = &[
    "context_length_exceeded",
    "maximum context length",
    "prompt is too long",
    "exceeds the maximum",
    "too many tokens",
    "context window",
    "input is too long",
    "max_tokens_exceeded",
    "request too large",
    "context limit",
];

// Prime-Agent-inspired Compaction-Surviving Session Scratchpad (`session_vars`)
pub const SESSION_VAR_MAX_ITEMS: usize = 32;
pub const SESSION_VAR_MAX_KEY_CHARS: usize = 64;
pub const SESSION_VAR_MAX_VALUE_BYTES: usize = 32 * 1024;
pub const SESSION_VAR_PREVIEW_CHARS: usize = 100;

// Oh-My-Pi-inspired Abandoned-Branch Summarization on Tree Fork / Regenerate
pub const BRANCH_SUMMARY_MAX_EVENTS: usize = 24;
pub const BRANCH_SUMMARY_MAX_CHARS: usize = 1_600;
pub const ORCH_STAGE_OVERFLOW_RECOVERY: &str = "overflow_recovery";
pub const ORCH_STAGE_BRANCH_SUMMARY: &str = "branch_summary";
pub const NOTE_KIND_BRANCH_SUMMARY: &str = "branch_summary";

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
pub const WATCHER_MAX_INOTIFY_DIRS: usize = 512;
pub const WATCHER_INOTIFY_READ_BUF_BYTES: usize = 4_096;
pub const WATCHER_MIN_WAKE_MS: u64 = 200;

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
    "compress_doc",
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
    "original_goal",
    "file_ledger",
    "branch_summary",
    "session_vars",
    "zpowers_state",
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
pub const TOOL_CODE_REFACTOR: &str = "code_refactor";
pub const TOOL_WEB_FETCH: &str = "web_fetch";
pub const TOOL_MEMORY: &str = "memory_tool";
pub const TOOL_PROJECT_BRAIN: &str = "project_brain";
pub const TOOL_KNOWLEDGE_CATALOG: &str = "knowledge_catalog";
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
pub const TOOL_SCRIPT_BLOCK: &str = "script_block";

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

// Native English Concise Mode, Read-Less Structural Compressor & Reusable Script Blocks (v5)
pub const CONCISE_MODES: &[&str] = &["off", "lite", "full", "ultra"];
pub const READ_LESS_DEFAULT_MIN_BYTES: usize = 2_048;
pub const READ_LESS_MIN_LINES: usize = 24;
pub const READ_LESS_MIN_SAVED_BYTES: usize = 256;
pub const READ_LESS_MIN_SAVINGS_PCT: usize = 20;
pub const READ_LESS_MAX_SAMPLE_ROWS: usize = 5;
pub const READ_LESS_TAIL_ROWS: usize = 2;
pub const READ_LESS_MAX_JSON_ARRAY_SAMPLE: usize = 4;
pub const READ_LESS_MAX_JSON_KEYS: usize = 24;
pub const READ_LESS_JSON_SCHEMA_SCAN_ITEMS: usize = 64;
pub const READ_LESS_JSON_FULL_DEPTH: usize = 3;
pub const READ_LESS_MAX_STRING_CHARS: usize = 320;
pub const READ_LESS_MAX_TAIL_LINES: usize = 28;
pub const READ_LESS_DEDUP_MIN_REPEATS: usize = 2;
pub const READ_LESS_TEST_MIN_PASS_WITH_SUMMARY: usize = 8;
pub const READ_LESS_TEST_MIN_PASS_WITHOUT_SUMMARY: usize = 16;
pub const READ_LESS_TEST_MAX_KEPT_OTHER_LINES: usize = 80;
pub const READ_LESS_CSV_SAMPLE_SCAN_LINES: usize = 12;
pub const READ_LESS_CSV_MIN_COLS: usize = 3;
pub const READ_LESS_CSV_MIN_ROWS: usize = 16;
pub const READ_LESS_LOG_MAX_REPEAT_SAMPLES: usize = 2;
pub const READ_LESS_LOG_MAX_KEPT_LINES: usize = 48;
pub const READ_LESS_LOG_HEAD_KEEP: usize = 12;
pub const READ_LESS_LOG_MAX_MIDDLE_DIAGS: usize = 16;
pub const READ_LESS_JSON_PRIORITY_KEYS: &[&str] = &[
    "error", "errors", "message", "status", "code", "ok", "id", "name", "total", "count",
];
pub const READ_LESS_PASS_PREFIXES: &[&str] = &[
    "--- PASS:", "PASS ", "PASS\t", "✓ ", "✔ ", "ok  \t",
];
pub const READ_LESS_FAIL_PREFIXES: &[&str] = &[
    "--- FAIL:", "FAIL", "✗ ", "✖ ", "× ", "error[", "error:", "fatal:", "ERR!", "E   ",
];
pub const READ_LESS_FAIL_SUBSTRS: &[&str] = &[
    "panicked at",
    "assertion ",
    "assertionerror",
    "traceback (most recent",
    "permission denied",
    "error ts",
];
pub const READ_LESS_SUMMARY_PREFIXES: &[&str] = &[
    "test result:",
    "failures:",
    "Test Files",
    "Test Suites:",
    "Tests ",
    "Summary:",
    "=== short test summary",
];
pub const READ_LESS_BUILD_PROGRESS_PREFIXES: &[&str] = &[
    "Compiling ",
    "Checking ",
    "Downloading ",
    "Downloaded ",
    "Updating ",
    "Fresh ",
    "Locking ",
    "Adding ",
    "Removing ",
    "Installing ",
    "Unpacking ",
    "Resolving ",
    "Fetching ",
    "Building ",
    "Doc-tests ",
];
/// Ephemeral/noisy tools eligible for Read-Less structural compression & session dedup
/// (strictly excludes `read_file`, `code_search`, `lsp_tool`, and file editors so exact
/// line numbers and hash anchors are never modified).
pub const READ_LESS_ELIGIBLE_TOOLS: &[&str] = &[
    TOOL_SHELL_EXEC,
    TOOL_JOB_READ,
    TOOL_GIT,
    TOOL_WEB_FETCH,
    TOOL_WEB_SEARCH,
    TOOL_GLOB_FIND,
    TOOL_SCRIPT_BLOCK,
    TOOL_SPAWN_AGENT,
    TOOL_AGENT_WAIT,
];
pub const READ_LESS_SCRUB_MIN_BYTES: usize = 256;
pub const READ_LESS_SCRUB_MIN_SAVED_BYTES: usize = 64;
pub const READ_LESS_GLOB_MIN_LINES: usize = 24;
pub const READ_LESS_GLOB_MAX_PER_DIR: usize = 8;
pub const READ_LESS_DEDUP_MIN_BYTES: usize = 384;
pub const READ_LESS_DEDUP_MAX_ENTRIES_PER_SESSION: usize = 48;
pub const READ_LESS_DEDUP_MAX_CALL_GAP: u64 = 16;
pub const READ_LESS_FAILURE_TEE_MIN_BYTES: usize = 768;
pub const READ_LESS_DIAG_MAX_GROUPS: usize = 18;
pub const READ_LESS_DIAG_SAMPLES_BALANCED: usize = 5;
pub const READ_LESS_DIAG_SAMPLES_HIGH: usize = 3;
pub const READ_LESS_DIAG_SAMPLES_ULTRA: usize = 2;
pub const READ_LESS_GIT_LOG_MAX_ENTRIES: usize = 30;
pub const READ_LESS_PKG_MAX_WARN_LINES: usize = 24;
pub const READ_LESS_LOCKFILE_DIFF_NAMES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "poetry.lock",
    "uv.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "flake.lock",
];
pub const READ_LESS_GENERATED_DIFF_SUFFIXES: &[&str] = &[
    ".min.js",
    ".min.css",
    ".map",
    ".pb.go",
    ".pb.rs",
    ".generated.ts",
];
pub const CUSTOM_FILTERS_FILE_NAME: &str = "filters.toml";
pub const CUSTOM_FILTERS_PROJECT_DIR: &str = ".zenko";
pub const CUSTOM_FILTERS_SCHEMA_VERSION: u32 = 1;
pub const CUSTOM_FILTERS_MAX_FILE_BYTES: usize = 128 * 1024;
pub const CUSTOM_FILTERS_MAX_RULES: usize = 64;
pub const CUSTOM_FILTERS_MAX_ON_EMPTY_CHARS: usize = 160;
pub const ECONOMY_DISCOVER_DEFAULT_LIMIT: usize = 10;
pub const ECONOMY_DISCOVER_MIN_BYTES: usize = 1_200;
pub const ECONOMY_DISCOVER_SCAN_EVENTS: usize = 600;
pub const ARTIFACT_SALVAGE_MAX_LINES: usize = 12;
pub const ARTIFACT_SALVAGE_MAX_LINE_CHARS: usize = 180;
pub const FILE_MODE_OWNER_EXEC: u32 = 0o700;
pub const SCRIPT_BLOCKS_DIR_NAME: &str = "blocks";
pub const SCRIPT_BLOCK_EXT: &str = "sh";
pub const SCRIPT_BLOCK_ACTIONS: &[&str] = &["list", "save", "read", "run", "delete"];
pub const SCRIPT_BLOCK_MAX_BYTES: usize = 64 * 1024;
pub const SCRIPT_BLOCK_MAX_LIST: usize = 64;
pub const SCRIPT_BLOCK_SLUG_MAX_CHARS: usize = 48;
pub const SCRIPT_BLOCK_HEADER_SCAN_LINES: usize = 4;
pub const SCRIPT_BLOCK_MAX_ARGS: usize = 32;
pub const SCRIPT_BLOCK_DESC_PREFIX: &str = "# zenko-block:";
pub const SCRIPT_BLOCK_DEFAULT_DESC: &str = "Reusable script block";
pub const DOC_COMPRESS_MIN_BYTES: usize = 128;
pub const DOC_COMPRESS_PREVIEW_CHARS: usize = 420;
pub const DOC_COMPRESS_PROTECTED_PREFIXES: &[&str] = &[
    "http://", "https://", "./", "../", "~/", "/", "--",
];
/// Declarative English phrase-to-shorthand table for `Lite` / `Full` / `Ultra` Markdown compression.
pub const DOC_COMPRESS_LITE_PHRASES: &[(&str, &str)] = &[
    ("it is important to note that ", ""),
    ("it should be noted that ", ""),
    ("please keep in mind that ", ""),
    ("keep in mind that ", ""),
    ("please note that ", ""),
    ("due to the fact that ", "because "),
    ("at this point in time", "now"),
    ("at the present time", "now"),
    ("in the event that ", "if "),
    ("for the purpose of ", "for "),
    ("in order to ", "to "),
    ("make sure that you ", "ensure "),
    ("please make sure to ", "ensure "),
    ("make sure to ", "ensure "),
    ("please be sure to ", "ensure "),
    ("be sure to ", "ensure "),
    ("as well as ", "and "),
    ("a large number of ", "many "),
    ("in addition to ", "plus "),
    ("with respect to ", "for "),
    ("prior to ", "before "),
    ("subsequent to ", "after "),
    ("in order for ", "for "),
    ("on a regular basis", "regularly"),
    ("at all times", "always"),
    ("has the ability to ", "can "),
    ("is able to ", "can "),
];
/// Additional telegraphic English connector shorthands applied in `Ultra` Markdown compression.
pub const DOC_COMPRESS_ULTRA_PHRASES: &[(&str, &str)] = &[
    ("results in ", "-> "),
    ("leads to ", "-> "),
    ("therefore ", "-> "),
    ("instead of ", "vs "),
    ("rather than ", "vs "),
    ("for example ", "e.g. "),
    ("such as ", "e.g. "),
];
/// Strictly ASCII-sorted English filler words dropped in `Lite` Markdown compression (zero negations).
pub const DOC_COMPRESS_LITE_FILLERS: &[&str] = &[
    "actually",
    "basically",
    "essentially",
    "generally",
    "just",
    "kindly",
    "literally",
    "obviously",
    "please",
    "really",
    "simply",
    "typically",
    "usually",
    "very",
];
/// Strictly ASCII-sorted English filler/article words dropped in `Full`/`Ultra` Markdown compression
/// (never includes negations like `no`, `not`, `never`, `without`, `must`, `only`).
pub const DOC_COMPRESS_TELEGRAPHIC_STOPWORDS: &[&str] = &[
    "a",
    "actually",
    "an",
    "basically",
    "currently",
    "essentially",
    "generally",
    "just",
    "kindly",
    "literally",
    "obviously",
    "please",
    "really",
    "simply",
    "that",
    "the",
    "typically",
    "usually",
    "very",
];
pub const DREAM_AUTO_COMPRESS_THRESHOLD_NUM: usize = 65;
pub const DREAM_AUTO_COMPRESS_THRESHOLD_DEN: usize = 100;
pub const DREAM_AUTO_COMPRESS_MIN_SAVED_BYTES: usize = 64;
pub const DREAM_AUTO_COMPRESS_DOCS: &[(&str, usize)] = &[
    ("USER.md", MAX_USER_DOC_TOKENS),
    ("MEMORY.md", MAX_MEMORY_DOC_TOKENS),
    (PROJECT_MEMORY_DOC_NAME, MAX_PROJECT_MEMORY_DOC_TOKENS),
];

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
pub const NOTE_KIND_CONCISE_MODE: &str = "concise_mode";

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
pub const ORCH_STAGE_CONCISE: &str = "concise";
pub const ORCH_STAGE_READ_LESS: &str = "read_less";
pub const ORCH_STAGE_ZMODE: &str = "zmode";
pub const NOTE_KIND_ZMODE: &str = "zmode";
pub const ZMODES: &[&str] = &["lite", "balanced", "high", "ultra"];

// Declarative ZMode context/efficiency profile bounds (single source of truth; zero hardcoding)
pub const ZMODE_LITE_READ_LESS_MIN_BYTES: usize = 4_096;
pub const ZMODE_BALANCED_READ_LESS_MIN_BYTES: usize = READ_LESS_DEFAULT_MIN_BYTES;
pub const ZMODE_HIGH_READ_LESS_MIN_BYTES: usize = 900;
pub const ZMODE_ULTRA_READ_LESS_MIN_BYTES: usize = 512;

pub const ZMODE_LITE_MICROCOMPACT_RATIO: f64 = 0.78;
pub const ZMODE_BALANCED_MICROCOMPACT_RATIO: f64 = DEFAULT_MICROCOMPACT_TRIGGER_RATIO;
pub const ZMODE_HIGH_MICROCOMPACT_RATIO: f64 = 0.55;
pub const ZMODE_ULTRA_MICROCOMPACT_RATIO: f64 = 0.45;

pub const ZMODE_LITE_COMPACTION_RATIO: f64 = 0.90;
pub const ZMODE_BALANCED_COMPACTION_RATIO: f64 = DEFAULT_COMPACTION_TRIGGER_RATIO;
pub const ZMODE_HIGH_COMPACTION_RATIO: f64 = 0.78;
pub const ZMODE_ULTRA_COMPACTION_RATIO: f64 = 0.70;

pub const ZMODE_LITE_P90_MULTIPLIER: f64 = 1.40;
pub const ZMODE_BALANCED_P90_MULTIPLIER: f64 = ECONOMY_LOOP_P90_MULTIPLIER;
pub const ZMODE_HIGH_P90_MULTIPLIER: f64 = 1.15;
pub const ZMODE_ULTRA_P90_MULTIPLIER: f64 = 1.10;

pub const ZMODE_LITE_PRUNE_MIN_PROMPTS: u32 = ECONOMY_TOOL_PRUNE_MIN_PROMPTS;
pub const ZMODE_BALANCED_PRUNE_MIN_PROMPTS: u32 = ECONOMY_TOOL_PRUNE_MIN_PROMPTS;
pub const ZMODE_HIGH_PRUNE_MIN_PROMPTS: u32 = 6;
pub const ZMODE_ULTRA_PRUNE_MIN_PROMPTS: u32 = 4;

pub const ZMODE_LITE_PRUNE_MIN_SESSIONS: u32 = ECONOMY_TOOL_PRUNE_MIN_SESSIONS;
pub const ZMODE_BALANCED_PRUNE_MIN_SESSIONS: u32 = ECONOMY_TOOL_PRUNE_MIN_SESSIONS;
pub const ZMODE_HIGH_PRUNE_MIN_SESSIONS: u32 = 2;
pub const ZMODE_ULTRA_PRUNE_MIN_SESSIONS: u32 = 2;

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
    TOOL_CODE_MINIMALISM,
];
pub const PLANNING_TOOLS: &[&str] = &[
    TOOL_RECORD_BRAINSTORM,
    TOOL_ASK_CLARIFICATION,
    TOOL_PLAN_UPDATE,
    TOOL_SESSION_TASK,
    TOOL_ORCHESTRATE_CONTROL,
    TOOL_ZPOWERS,
];
/// Tools that are conditionally permitted in Read-Only Plan Mode (enforced per-argument by ExecPolicy):
/// read-only shell commands, read-only memory actions, read-only subagent roles, worktree diff,
/// read-only git actions, read-only LSP actions (plus `rename` with `dry_run: true`), read-only
/// `skill_manage` queries, read-only `goal_loop` queries, read-only `token_economy` inspections,
/// `zpowers` planning/status actions (excluding workspace-mutating `promote`),
/// and `batch_tools` (each inner call individually policy-enforced).
pub const PLAN_MODE_SCOPED_TOOLS: &[&str] = &[
    TOOL_SHELL_EXEC,
    TOOL_MEMORY,
    TOOL_PROJECT_BRAIN,
    TOOL_KNOWLEDGE_CATALOG,
    TOOL_SPAWN_AGENT,
    TOOL_AGENT_WORKTREE,
    TOOL_GIT,
    TOOL_LSP,
    TOOL_CODE_REFACTOR,
    TOOL_SKILL_MANAGE,
    TOOL_GOAL_LOOP,
    TOOL_BATCH_TOOLS,
    TOOL_INVOKE_TOOL,
    TOOL_SCRIPT_BLOCK,
    TOOL_TOKEN_ECONOMY,
    TOOL_ZPOWERS,
];
pub const MUTATING_FILE_TOOLS: &[&str] = &[TOOL_WRITE_FILE, TOOL_HASH_EDIT, TOOL_APPLY_PATCH];
pub const VERIFICATION_SHELL_TOOLS: &[&str] = &[TOOL_SHELL_EXEC, TOOL_JOB_WAIT];

/// Bulky ephemeral tools eligible for Stage-A lossless microcompaction once outside the recent tail.
pub const MICROCOMPACTABLE_TOOLS: &[&str] = &[
    TOOL_READ_FILE,
    TOOL_CODE_SEARCH,
    TOOL_CODE_REFACTOR,
    TOOL_GLOB_FIND,
    TOOL_LIST_DIR,
    TOOL_GIT,
    TOOL_SHELL_EXEC,
    TOOL_WEB_FETCH,
    TOOL_WEB_SEARCH,
    TOOL_MCP_RESOURCE,
    TOOL_JOB_READ,
    TOOL_SCRIPT_BLOCK,
    TOOL_KNOWLEDGE_CATALOG,
];
/// High-signal state/planning tools strictly exempt from microcompaction (prevents working-memory amnesia).
pub const COMPACTION_PROTECTED_TOOLS: &[&str] = &[
    TOOL_RECORD_BRAINSTORM,
    TOOL_PLAN_UPDATE,
    TOOL_SESSION_TASK,
    TOOL_ASK_CLARIFICATION,
    TOOL_ORCHESTRATE_CONTROL,
    TOOL_ZPOWERS,
    TOOL_MEMORY,
    TOOL_PROJECT_BRAIN,
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

// Shared deterministic 64-bit FNV-1a hash constants (reused across Read-Less, Guard, and Token Economy)
pub const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
pub const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

// Native Token Economy Engine constants (inspired by Microsoft TokenEconomizer; 100% native Rust, zero regex)
pub const TOOL_TOKEN_ECONOMY: &str = "token_economy";
pub const ORCH_STAGE_ECONOMY: &str = "economy";
pub const ECONOMY_MICRO_SKILL_MIN_SESSIONS: u32 = 2;
pub const ECONOMY_MICRO_SKILL_MIN_NEED_RATE: f64 = 0.40;
pub const ECONOMY_MICRO_SKILL_MIN_EXPOSURES_FOR_RATE: u32 = 8;
pub const ECONOMY_MICRO_SKILL_MAX_INJECT: usize = 8;
pub const ECONOMY_MICRO_SKILL_MAX_CHARS: usize = 240;
pub const ECONOMY_MICRO_SKILL_MAX_CONTRADICTIONS: u32 = 1;
pub const ECONOMY_TOOL_PRUNE_MIN_PROMPTS: u32 = 10;
pub const ECONOMY_TOOL_PRUNE_MIN_SESSIONS: u32 = 2;
pub const ECONOMY_TOOL_PRUNE_REAUDIT_TASKS: u32 = 5;
pub const ECONOMY_LOOP_P90_MIN_SAMPLES: usize = 5;
pub const ECONOMY_LOOP_P90_MULTIPLIER: f64 = 1.25;
pub const ECONOMY_LOOP_MAX_BASELINE_SAMPLES: usize = 64;
pub const ECONOMY_LOOP_MIN_ENVELOPE_CALLS: usize = 8;
pub const ECONOMY_LOOP_ERROR_DENSITY_RATIO: f64 = 0.45;
pub const ECONOMY_LOOP_OSCILLATION_MIN_ERRORS: usize = 3;
pub const ECONOMY_LOOP_MAX_REDIRECTS_PER_TURN: usize = 1;
pub const ECONOMY_LEDGER_MAX_EVENTS: usize = 500;
pub const ECONOMY_ESTIMATED_USD_PER_1M_TOKENS: f64 = 3.0;
pub const ECONOMY_FAST_JUDGE_MAX_TOKENS: u32 = 220;
pub const ECONOMY_FAST_JUDGE_MIN_CONFIDENCE: f64 = 0.70;
pub const ECONOMY_CMD_PREVIEW_CHARS: usize = 120;
pub const ECONOMY_SHELL_OUTPUT_PREVIEW_CHARS: usize = 280;

/// Core operational tools permanently protected from experience-driven tool pruning.
pub const ECONOMY_PROTECTED_CORE_TOOLS: &[&str] = &[
    TOOL_READ_FILE,
    TOOL_HASH_EDIT,
    TOOL_APPLY_PATCH,
    TOOL_WRITE_FILE,
    TOOL_LIST_DIR,
    TOOL_CODE_SEARCH,
    TOOL_GLOB_FIND,
    TOOL_SHELL_EXEC,
    TOOL_TOOL_SEARCH,
    TOOL_INVOKE_TOOL,
    TOOL_BATCH_TOOLS,
    TOOL_ASK_CLARIFICATION,
    TOOL_ORCHESTRATE_CONTROL,
    TOOL_TOKEN_ECONOMY,
    TOOL_CODE_MINIMALISM,
];

/// Declarative shell wrapper prefixes skipped when extracting the primary executable command.
pub const ECONOMY_SHELL_WRAPPERS: &[&str] = &["env", "time", "nice", "timeout", "command", "nohup"];

/// Declarative environment/invocation error markers signaling a learnable command failure.
pub const ECONOMY_LEARNABLE_FAIL_MARKERS: &[&str] = &[
    "command not found",
    "not found",
    "no such file or directory",
    "unknown option",
    "unrecognized option",
    "unexpected argument",
    "invalid option",
    "unknown subcommand",
    "no such subcommand",
    "is not recognized",
    "exit_code: 127",
    "exit_code: 126",
    "exit_code: 2",
];

/// Declarative markers indicating a shell command failed because the invoked tool itself is broken/missing
/// (triggers 1-strike contradiction retirement when matching an active micro-skill's recommended command).
pub const ECONOMY_CONTRADICTION_MARKERS: &[&str] = &[
    "command not found",
    "no such file or directory",
    "unknown option",
    "unrecognized option",
    "unknown subcommand",
    "no such subcommand",
    "exit_code: 127",
];

// Native Rust Code Minimalism Engine constants (inspired by DietrichGebert/ponytail; zero /commands, zero regex)
pub const TOOL_CODE_MINIMALISM: &str = "code_minimalism";
pub const ORCH_STAGE_MINIMALISM: &str = "minimalism";
pub const NOTE_KIND_MINIMALISM_MODE: &str = "minimalism_mode";
pub const CODE_MINIMALISM_MODES: &[&str] = &["off", "lite", "full", "ultra"];
pub const MINIMALISM_AUDIT_MAX_FILES: usize = 600;
pub const MINIMALISM_AUDIT_MAX_FILE_BYTES: u64 = 262_144;
pub const MINIMALISM_LARGE_FILE_LINES: usize = 450;
pub const MINIMALISM_LONG_FN_LINES: usize = 80;
pub const MINIMALISM_AUDIT_TOP_N: usize = 12;
pub const MINIMALISM_DEBT_MAX_ITEMS: usize = 80;
pub const MINIMALISM_DEBT_TEXT_MAX_CHARS: usize = 160;
pub const MINIMALISM_DIFF_LARGE_HUNK_LOC: usize = 60;
pub const MINIMALISM_WRAPPER_MAX_LINES: usize = 14;
pub const MINIMALISM_DUPLICATE_SYMBOL_MIN_CHARS: usize = 6;
pub const MINIMALISM_ABSTRACTION_MIN_NAME_CHARS: usize = 5;
pub const MINIMALISM_COMMENT_BLOAT_MIN_LINES: usize = 40;
pub const MINIMALISM_COMMENT_BLOAT_MIN_RATIO_PCT: usize = 35;
pub const MINIMALISM_COMMENT_BLOCK_MAX_LINES: usize = 14;
pub const MINIMALISM_DIFF_COMMENT_BLOAT_MIN_ADDED: u64 = 30;
pub const MINIMALISM_DIFF_COMMENT_BLOAT_RATIO_PCT: u64 = 35;
pub const MINIMALISM_BLOAT_HINT_NET_LOC_LITE: u64 = 300;
pub const MINIMALISM_BLOAT_HINT_NET_LOC_FULL: u64 = 140;
pub const MINIMALISM_BLOAT_HINT_NET_LOC_ULTRA: u64 = 70;
pub const MINIMALISM_BLOAT_HINT_NEW_FILES_LITE: usize = 4;
pub const MINIMALISM_BLOAT_HINT_NEW_FILES_FULL: usize = 3;
pub const MINIMALISM_BLOAT_HINT_NEW_FILES_ULTRA: usize = 2;

/// Declarative shortcut/debt comment markers scanned strictly inside comment spans (zero regex).
pub const MINIMALISM_DEBT_MARKERS: &[&str] = &[
    "DEBT:",
    "SHORTCUT:",
    "HACK:",
    "FIXME(debt):",
    "TODO(debt):",
    "PONYTAIL:",
];

/// Declarative source file extensions included in minimalism audits and debt scans.
pub const MINIMALISM_SOURCE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "pyi", "go", "c", "h", "cc",
    "cpp", "cxx", "hpp", "java", "kt", "kts", "swift", "cs",
];

/// Dependency manifests flagged during `audit_diff` so new dependencies pass the Simplicity Ladder.
pub const MINIMALISM_MANIFEST_FILES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "requirements.txt",
];

/// Suffixes signaling potential speculative thin wrapper types when span <= `MINIMALISM_WRAPPER_MAX_LINES`.
pub const MINIMALISM_WRAPPER_SUFFIXES: &[&str] = &[
    "Wrapper", "Adapter", "Facade", "Proxy", "Factory", "Helper", "Delegate",
];

/// Strictly ASCII-sorted common lifecycle/trait method names excluded from duplicate-symbol findings
/// (queried via O(log K) `binary_search`).
pub const MINIMALISM_IGNORED_DUP_SYMBOLS: &[&str] = &[
    "as_str",
    "build",
    "cleanup",
    "close",
    "default",
    "default_true",
    "delete",
    "execute",
    "exposure",
    "format",
    "from_str_loose",
    "handle",
    "is_available",
    "is_empty",
    "is_mutating",
    "parse",
    "render",
    "reset",
    "resolve",
    "shutdown",
    "status",
    "toolset",
    "update",
    "upgrade",
    "validate",
];

/// Strictly ASCII-sorted build/tooling/CSS package names excluded from dormant-dependency findings
/// (queried via O(log K) `binary_search`).
pub const MINIMALISM_IGNORED_DORMANT_DEPS: &[&str] = &[
    "@tailwindcss/vite",
    "@vitejs/plugin-react",
    "autoprefixer",
    "postcss",
    "tailwindcss",
    "tslib",
    "typescript",
    "vite",
];

// ── SmartCrusher 2-Tier JSON / TOON Compressor (Headroom + TOON, zero ML/regex)
pub const READ_LESS_EXIT_CODE_PREFIX: &str = "exit_code:";
pub const SMART_CRUSHER_TABULAR_MAX_ITEMS: usize = 40;
pub const SMART_CRUSHER_TABULAR_MIN_SAVINGS_PCT: usize = 30;
pub const SMART_CRUSHER_TABULAR_MAX_COLS: usize = 20;
pub const SMART_CRUSHER_TABULAR_MAX_CELL_CHARS: usize = 120;
pub const SMART_CRUSHER_TOON_TABULAR_MIN_ITEMS: usize = 2;
pub const SMART_CRUSHER_TOON_INIT_CAP_BYTES: usize = 2_048;
pub const SMART_CRUSHER_TOON_MAX_BYTES: usize = 3_200;
pub const SMART_CRUSHER_TOON_MIN_SAVINGS_VS_MINIFIED_PCT: usize = 20;
pub const SMART_CRUSHER_TOON_KEYED_MAP_MIN_ENTRIES: usize = 4;
pub const SMART_CRUSHER_TOON_MAX_DEPTH: usize = 8;
pub const SMART_CRUSHER_TOON_INLINE_MAX_ITEMS: usize = 24;
pub const SMART_CRUSHER_TOON_INLINE_MAX_CHARS: usize = 160;
pub const SMART_CRUSHER_ERROR_HTTP_MIN_CODE: i64 = 400;
pub const SMART_CRUSHER_NULL_ERROR_VALUES: &[&str] = &["none", "null", "nil", "ok"];
pub const SMART_CRUSHER_OUTLIER_SKIP_KEYS: &[&str] = &[
    "id",
    "index",
    "timestamp",
    "ts",
    "created_at",
    "updated_at",
];
pub const SMART_CRUSHER_HEAD_KEEP: usize = 2;
pub const SMART_CRUSHER_TAIL_KEEP: usize = 2;
pub const SMART_CRUSHER_MAX_KEPT_BALANCED: usize = 10;
pub const SMART_CRUSHER_MAX_KEPT_HIGH: usize = 8;
pub const SMART_CRUSHER_MAX_KEPT_ULTRA: usize = 6;
pub const SMART_CRUSHER_MAX_ERROR_ITEMS: usize = 6;
pub const SMART_CRUSHER_MAX_OUTLIER_ITEMS: usize = 4;
pub const SMART_CRUSHER_OUTLIER_SIGMA: f64 = 2.0;
pub const SMART_CRUSHER_MIN_NUMERIC_SAMPLES: usize = 5;
pub const SMART_CRUSHER_MIN_QUERY_TOKEN_LEN: usize = 3;
pub const SMART_CRUSHER_MAX_QUERY_TOKENS: usize = 12;
pub const SMART_CRUSHER_ERROR_KEYS: &[&str] = &[
    "error",
    "err",
    "exception",
    "failure",
    "failed",
    "fatal",
    "panic",
    "traceback",
    "stack",
    "stack_trace",
];
pub const SMART_CRUSHER_STATUS_KEYS: &[&str] = &[
    "status",
    "status_code",
    "statusCode",
    "http_status",
    "code",
    "level",
    "severity",
    "state",
    "result",
    "outcome",
];
pub const SMART_CRUSHER_ERROR_STATUS_VALUES: &[&str] = &[
    "error",
    "err",
    "fail",
    "failed",
    "failure",
    "fatal",
    "critical",
    "panic",
    "crash",
    "crashed",
    "timeout",
    "timed_out",
    "rejected",
    "denied",
    "unhealthy",
    "degraded",
    "broken",
];
pub const SMART_CRUSHER_OK_KEYS: &[&str] = &["ok", "success", "passed", "healthy", "valid"];
pub const SMART_CRUSHER_QUERY_ARG_KEYS: &[&str] = &[
    "query", "q", "filter", "search", "pattern", "command", "cmd", "url", "path", "topic",
    "keyword", "grep", "jq",
];
pub const SMART_CRUSHER_QUERY_STOPWORDS: &[&str] = &[
    "and", "arg", "cat", "cmd", "curl", "for", "from", "get", "http", "https", "json", "list",
    "post", "run", "set", "the", "this", "true", "false", "with",
];

// ── CacheAligner Prefix Stability Guard (zero regex) ─────────────────────────
pub const CACHE_VOLATILE_MAX_WARNINGS: usize = 6;
pub const CACHE_VOLATILE_LABEL_KEYS: &[&str] = &[
    "current_time",
    "current_date",
    "timestamp",
    "generated_at",
    "updated_at",
    "request_id",
    "session_id",
    "trace_id",
];

// ── DOX Hierarchical Scoped Repo Rules & Drift Guard ─────────────────────────
pub const DOX_MAX_TREE_DEPTH: usize = 4;
pub const DOX_MAX_TREE_DOCS: usize = 24;
pub const DOX_PURPOSE_MAX_CHARS: usize = 120;
pub const DOX_MAX_SCOPED_RULE_BYTES: usize = 4 * 1024;
pub const DOX_MAX_SCOPED_RULES_PER_SESSION: usize = 16;
pub const DOX_SCOPED_RULES_TAG_OPEN: &str = "<scoped_repo_rules ";
pub const DOX_SCOPED_RULES_TAG_PREFIX: &str = "<scoped_repo_rules path=\"";
pub const DOX_SCOPED_RULES_TAG_CLOSE: &str = "</scoped_repo_rules>";
pub const DOX_SCOPED_RULES_META_KEY: &str = "scoped_rules_injected";
pub const DOX_DRIFT_MIN_ADDED_LINES: u64 = 12;
pub const DOX_DRIFT_MAX_FINDINGS: usize = 6;

// ── Repo-Native Project Brain (`mindmuxai/brain.md` open standard, zero regex)
pub const BRAIN_DIR_NAME: &str = "brain";
pub const BRAIN_PAGES_DIR_NAME: &str = "pages";
pub const BRAIN_PROTOCOL_FILE: &str = "BRAIN.md";
pub const BRAIN_PREFS_DIR_NAME: &str = ".mindmux";
pub const BRAIN_PREFS_FILE_NAME: &str = "preferences.json";
pub const BRAIN_PREFS_ROOT_KEY: &str = "brainRoot";
pub const BRAIN_COMPILED_TRUTH_MARKER: &str = "<!-- compiled_truth -->";
pub const BRAIN_TIMELINE_HEADING: &str = "## Timeline";
pub const BRAIN_MAX_SLUG_CHARS: usize = 64;
pub const BRAIN_MAX_TITLE_CHARS: usize = 160;
pub const BRAIN_MAX_PAGE_BYTES: usize = 32 * 1024;
pub const BRAIN_MAX_ROOT_BYTES: usize = 32 * 1024;
pub const BRAIN_MAX_INDEX_PAGES: usize = 32;
pub const BRAIN_MAX_TIMELINE_ENTRIES: usize = 128;
pub const BRAIN_MAX_TAGS: usize = 12;
pub const BRAIN_MAX_AFFECTS: usize = 16;
pub const BRAIN_IMPACT_MAX_HOPS: usize = 3;
pub const BRAIN_RECON_MAX_HITS: usize = 4;
pub const BRAIN_DEFAULT_SEARCH_LIMIT: usize = 8;
pub const BRAIN_MAX_SEARCH_LIMIT: usize = 24;
pub const BRAIN_TRUTH_PREVIEW_CHARS: usize = 110;
pub const BRAIN_PAGE_CATEGORIES: &[&str] = &[
    "project",
    "concept",
    "decision",
    "person",
    "organization",
    "reference",
];
pub const BRAIN_PAGE_STATUSES: &[&str] = &["active", "draft", "archived"];
pub const BRAIN_TIMELINE_KINDS: &[&str] = &[
    "decision",
    "evidence",
    "update",
    "reversal",
    "archived",
];
pub const BRAIN_ROOT_SLOT_SLUGS: &[&str] = &[
    "background",
    "architecture",
    "flow",
    "mindmap",
    "stack",
    "roadmap",
];
pub const BRAIN_ACTIONS: &[&str] = &[
    "init",
    "list_pages",
    "read_page",
    "create_page",
    "update_truth",
    "append_timeline",
    "archive_page",
    "read_root",
    "update_root",
    "search",
    "impact",
];

// ── Chisel / Chisle Precision File I/O, Context Diet & Prose Yield ───────────
pub const JAIL_DIR_CREATE_MODE: u32 = 0o755;
pub const JAIL_TMP_FILE_MODE: u32 = 0o600;
pub const JAIL_TMP_FILE_PREFIX: &str = ".zenko-tmp-";
pub const READ_FILE_MODE_LINES: &str = "lines";
pub const READ_FILE_MODE_OUTLINE: &str = "outline";
pub const READ_FILE_MODES: &[&str] = &[READ_FILE_MODE_LINES, READ_FILE_MODE_OUTLINE];
pub const READ_FILE_CONTEXT_DIET_HINT_MIN_LINES: usize = 300;
pub const READ_FILE_MAX_SYMBOL_QUERIES: usize = 8;
pub const READ_FILE_SYMBOL_NOT_FOUND_PREVIEW: usize = 20;
pub const SHORT_TASK_MAX_PROMPT_CHARS: usize = 160;
pub const SHORT_TASK_MAX_NONEMPTY_LINES: usize = 2;
pub const MEM_DOC_SOUL: &str = "SOUL.md";
pub const MEM_DOC_USER: &str = "USER.md";
pub const MEM_DOC_REPO_RULES_LABEL: &str = "repo_rules";
pub const CODEX_PATCH_BEGIN: &str = "*** Begin Patch";
pub const CODEX_PATCH_END: &str = "*** End Patch";
pub const CODEX_PATCH_ADD_FILE: &str = "*** Add File:";
pub const CODEX_PATCH_DELETE_FILE: &str = "*** Delete File:";
pub const CODEX_PATCH_UPDATE_FILE: &str = "*** Update File:";
pub const CODEX_PATCH_MOVE_TO: &str = "*** Move to:";
pub const CODEX_PATCH_END_OF_FILE: &str = "*** End of File";
pub const UNIFIED_DIFF_DEV_NULL: &str = "/dev/null";
pub const UNIFIED_DIFF_GIT_PREFIX: &str = "diff --git ";
pub const UNIFIED_DIFF_OLD_PREFIX: &str = "--- ";
pub const UNIFIED_DIFF_NEW_PREFIX: &str = "+++ ";
pub const UNIFIED_DIFF_RENAME_FROM: &str = "rename from ";
pub const UNIFIED_DIFF_RENAME_TO: &str = "rename to ";
pub const UNIFIED_DIFF_HUNK_MARKER: &str = "@@";
pub const UNIFIED_DIFF_NO_NEWLINE: &str = "\\ No newline at end of file";
pub const UNIFIED_DIFF_GIT_META_PREFIXES: &[&str] = &[
    "index ",
    "new file mode ",
    "deleted file mode ",
    "old mode ",
    "new mode ",
    "similarity index ",
    "dissimilarity index ",
];
pub const CUSTOM_STYLE_MARKER_HEADERS: &[&str] = &[
    "output style",
    "prose style",
    "writing style",
    "communication style",
    "tone & style",
    "response style",
    "formatting style",
];
pub const CUSTOM_STYLE_DIRECTIVE_TOKENS: &[&str] = &[
    "output_style:",
    "prose_style:",
    "writing_style:",
    "response_style:",
    "custom_output_style",
];

// ── Structural Code Intelligence (tokei, tgrep, ast-grep, sd, ripide, difftastic)
pub const CODE_REFACTOR_ACTIONS: &[&str] = &[
    "struct_search",
    "struct_rewrite",
    "replace",
    "move_imports",
    "stats",
    "struct_diff",
];
pub const TRIGRAM_MIN_REPO_FILES: usize = 64;
pub const TRIGRAM_MAX_INDEXED_FILES: usize = 16_000;
pub const TRIGRAM_MAX_FILE_BYTES: u64 = 512 * 1024;
pub const TRIGRAM_MAX_WORKSPACES_CACHED: usize = 4;
pub const TRIGRAM_QUERY_MIN_RUN_BYTES: usize = 3;
pub const STRUCT_SEARCH_DEFAULT_MAX_MATCHES: usize = 30;
pub const STRUCT_SEARCH_MAX_MATCHES: usize = 80;
pub const STRUCT_DIFF_MAX_TOKENS_DIJKSTRA: usize = 1_600;
pub const STRUCT_DIFF_MAX_EDITS_RENDER: usize = 60;
pub const STRUCT_DIFF_MAX_GIT_FILES: usize = 24;
pub const CODE_REFACTOR_MAX_FILES_APPLY: usize = 64;
pub const CODE_STATS_MAX_FILES: usize = 20_000;
pub const CODE_STATS_RECON_MAX_FILES: usize = 2_500;
pub const CODE_STATS_RECON_TOP_LANGS: usize = 4;
pub const STRUCT_META_SINGLE_PREFIX: &str = "$";
pub const STRUCT_META_VARIADIC_PREFIX: &str = "$$$";
pub const STRUCT_META_WILDCARD: &str = "$_";

// ── Tier-Aware Shunt Context Gate & Repo-Native Service Briefing (zero regex) ─
pub const ORCH_STAGE_SHUNT: &str = "shunt";
pub const SUBAGENT_ROLE_EXPLORER: &str = "explorer";
pub const SUBAGENT_ROLE_DRAFTER: &str = "drafter";
pub const SHUNT_META_KEY: &str = "shunt";
pub const SHUNT_META_FIELD_ACTION: &str = "action";
pub const SHUNT_META_FIELD_SOURCE_KIND: &str = "source_kind";
pub const SHUNT_META_FIELD_TOKENS_SAVED: &str = "tokens_saved";
pub const SHUNT_BATCH_STEP_TOKENS_SAVED_KEY: &str = "shunt_tokens_saved";
pub const SHUNT_DEFAULT_MIN_LINES: usize = 350;
pub const SHUNT_MAX_SCAN_BYTES: u64 = 16 * 1024 * 1024;
pub const SHUNT_SCAN_BUF_BYTES: usize = 16 * 1024;
pub const SHUNT_OUTLINE_MAX_SYMBOLS: usize = 48;
pub const SHUNT_MODES: &[&str] = &["off", "advise", "redirect"];
pub const SHUNT_SHELL_DEFAULT_SLICE_LINES: usize = 10;
pub const SHUNT_SHELL_LINES_SHORT_FLAG: &str = "-n";
pub const SHUNT_SHELL_LINES_LONG_FLAG: &str = "--lines";
pub const SHUNT_SHELL_LINES_LONG_EQ: &str = "--lines=";
pub const SHUNT_SHELL_END_OF_OPTIONS: &str = "--";
pub const SHUNT_SHELL_STDIN_OPERAND: &str = "-";
pub const SHUNT_SHELL_GLOB_CHARS: &[char] = &['*', '?', '['];
pub const SHUNT_SHELL_VALUE_FLAGS: &[&str] = &[
    "-n", "--lines", "-c", "--bytes", "-b", "-i", "-l", "-s", "-v", "-w",
];
pub const SHUNT_LINE_PREFIX_ESTIMATE_BYTES: usize = 10;
pub const SHUNT_BYTES_PER_TOKEN_ESTIMATE: usize = 4;
pub const SHUNT_MIN_OUTPUT_CAP_BYTES: usize = 4_096;
pub const SHUNT_FALLBACK_READ_WINDOW_LINES: usize = 120;
pub const SERVICE_CATALOG_FILES: &[&str] = &["catalog-info.yaml", "catalog-info.yml"];
pub const SERVICE_CODEOWNERS_PATHS: &[&str] = &[
    ".github/CODEOWNERS",
    "CODEOWNERS",
    "docs/CODEOWNERS",
    ".gitlab/CODEOWNERS",
];
pub const SERVICE_BRIEF_MAX_FILE_BYTES: u64 = 64 * 1024;
pub const SERVICE_BRIEF_MAX_DEPS: usize = 6;
pub const SERVICE_BRIEF_MAX_APIS: usize = 4;
pub const SERVICE_BRIEF_MAX_OWNERS: usize = 4;
pub const SERVICE_BRIEF_MAX_LINKS: usize = 3;
pub const SERVICE_BRIEF_MAX_SCOPED_RULES: usize = 3;
pub const SERVICE_BRIEF_ANNOTATION_MAX_CHARS: usize = 48;
pub const SERVICE_YAML_DOC_START: &str = "---";
pub const SERVICE_YAML_DOC_END: &str = "...";
pub const SERVICE_YAML_KEY_KIND: &str = "kind";
pub const SERVICE_YAML_KEY_METADATA: &str = "metadata";
pub const SERVICE_YAML_KEY_SPEC: &str = "spec";
pub const SERVICE_YAML_KEY_NAME: &str = "name";
pub const SERVICE_YAML_KEY_ANNOTATIONS: &str = "annotations";
pub const SERVICE_YAML_KEY_OWNER: &str = "owner";
pub const SERVICE_YAML_KEY_LIFECYCLE: &str = "lifecycle";
pub const SERVICE_YAML_KEY_SYSTEM: &str = "system";
pub const SERVICE_YAML_KEY_TYPE: &str = "type";
pub const SERVICE_YAML_KEY_DEPENDS_ON: &str = "dependsOn";
pub const SERVICE_YAML_KEY_PROVIDES_APIS: &str = "providesApis";
pub const SERVICE_CATALOG_ANNOTATION_KEYS: &[(&str, &str)] = &[
    ("backstage.io/techdocs-ref", "docs"),
    ("pagerduty.com/service-id", "pagerduty"),
    ("opsgenie.com/component-id", "opsgenie"),
    ("github.com/project-slug", "repo"),
    ("backstage.io/source-location", "src"),
    ("runbook", "runbook"),
];

// ── TurboQuant Data-Oblivious Vector Quantization (FWHT + Lloyd-Max + QJL) ──
pub const TURBOQUANT_DEFAULT_DIM: usize = 256;
pub const TURBOQUANT_MIN_DIM: usize = 16;
pub const TURBOQUANT_MAX_DIM: usize = 4_096;
pub const TURBOQUANT_MAGIC: [u8; 4] = *b"ZTQ1";
pub const TURBOQUANT_FLAG_QJL: u8 = 0x01;
pub const TURBOQUANT_SEED_MAIN: u64 = 0x5A45_4E4B_4F54_5131;
pub const TURBOQUANT_SEED_QJL: u64 = 0x5A45_4E4B_4F51_4A4C;
pub const TURBOQUANT_RRF_WEIGHT: f64 = 0.90;
pub const TURBOQUANT_MIN_COSINE: f32 = 0.08;
pub const TURBOQUANT_SQLITE_SCAN_LIMIT: usize = 512;
pub const TURBOQUANT_CHAR_NGRAM_MIN: usize = 3;
pub const TURBOQUANT_CHAR_NGRAM_MAX: usize = 4;
pub const TURBOQUANT_TOKEN_WEIGHT: f32 = 1.60;
pub const TURBOQUANT_BIGRAM_WEIGHT: f32 = 1.30;
pub const TURBOQUANT_CHAR_NGRAM_WEIGHT: f32 = 0.55;
/// Optimal standard-Gaussian N(0, 1) Lloyd-Max centroids for 2-bit scalar quantization (4 levels).
pub const TURBOQUANT_LLOYD_MAX_2BIT_CENTROIDS: [f32; 4] =
    [-1.510_417_6, -0.452_780_04, 0.452_780_04, 1.510_417_6];
/// Optimal standard-Gaussian N(0, 1) Lloyd-Max decision boundaries for 2-bit scalar quantization.
pub const TURBOQUANT_LLOYD_MAX_2BIT_THRESHOLDS: [f32; 3] = [-0.981_598_8, 0.0, 0.981_598_8];
/// Optimal standard-Gaussian N(0, 1) Lloyd-Max centroids for 4-bit scalar quantization (16 levels).
pub const TURBOQUANT_LLOYD_MAX_4BIT_CENTROIDS: [f32; 16] = [
    -2.732_578_5,
    -2.069_017_2,
    -1.618_049_6,
    -1.256_185_5,
    -0.942_343_8,
    -0.656_756_5,
    -0.388_051_5,
    -0.128_404_9,
    0.128_404_9,
    0.388_051_5,
    0.656_756_5,
    0.942_343_8,
    1.256_185_5,
    1.618_049_6,
    2.069_017_2,
    2.732_578_5,
];
/// Optimal standard-Gaussian N(0, 1) Lloyd-Max decision boundaries for 4-bit scalar quantization.
pub const TURBOQUANT_LLOYD_MAX_4BIT_THRESHOLDS: [f32; 15] = [
    -2.400_797_8,
    -1.843_533_4,
    -1.437_117_6,
    -1.099_264_7,
    -0.799_550_2,
    -0.522_404_0,
    -0.258_228_2,
    0.0,
    0.258_228_2,
    0.522_404_0,
    0.799_550_2,
    1.099_264_7,
    1.437_117_6,
    1.843_533_4,
    2.400_797_8,
];

// ── Open Knowledge Format (OKF v0.2/v0.3) Graph, Trust & Freshness Engine ───
pub const ORCH_STAGE_OKF: &str = "okf";
pub const OKF_DISCOVERY_DIRS: &[&str] = &["knowledge", "okf", ".okf", "docs/knowledge", "catalog"];
pub const OKF_DEFAULT_WRITE_DIR: &str = "knowledge";
pub const OKF_MAX_FILES_SCAN: usize = 256;
pub const OKF_MAX_FILE_BYTES: u64 = 128 * 1024;
pub const OKF_MAX_LINKS_PER_DOC: usize = 48;
pub const OKF_MAX_SOURCES_PER_DOC: usize = 24;
pub const OKF_MAX_GRAPH_HOPS: usize = 3;
pub const OKF_EXPIRING_SOON_DAYS: i64 = 14;
pub const OKF_RECON_MAX_HITS: usize = 4;
pub const OKF_DEFAULT_SEARCH_LIMIT: usize = 8;
pub const OKF_MAX_SEARCH_LIMIT: usize = 32;
pub const OKF_SUMMARY_PREVIEW_CHARS: usize = 120;
pub const OKF_FRONTMATTER_DELIM: &str = "---";
pub const OKF_KEY_TYPE: &str = "type";
pub const OKF_KEY_TITLE: &str = "title";
pub const OKF_KEY_SUMMARY: &str = "summary";
pub const OKF_KEY_DESCRIPTION: &str = "description";
pub const OKF_KEY_STATUS: &str = "status";
pub const OKF_KEY_OWNER: &str = "owner";
pub const OKF_KEY_TAGS: &str = "tags";
pub const OKF_KEY_VERIFIED: &str = "verified";
pub const OKF_KEY_VERIFIED_BY: &str = "verified_by";
pub const OKF_KEY_VERIFIED_AT: &str = "verified_at";
pub const OKF_KEY_BY: &str = "by";
pub const OKF_KEY_AT: &str = "at";
pub const OKF_KEY_STALE_AFTER: &str = "stale_after";
pub const OKF_KEY_SOURCES: &str = "sources";
pub const OKF_KEY_RESOURCE: &str = "resource";
pub const OKF_ACTIONS: &[&str] = &[
    "list",
    "search",
    "read",
    "graph",
    "audit_freshness",
    "upsert",
];
pub const OKF_STATUS_VALUES: &[&str] = &["active", "draft", "deprecated", "superseded"];
pub const OKF_TRUST_WEIGHT_VERIFIED: f64 = 1.25;
pub const OKF_TRUST_WEIGHT_UNVERIFIED: f64 = 1.00;
pub const OKF_TRUST_WEIGHT_DRIFTED: f64 = 0.72;
pub const OKF_TRUST_WEIGHT_DEPRECATED: f64 = 0.45;
pub const OKF_FRESHNESS_WEIGHT_FRESH: f64 = 1.15;
pub const OKF_FRESHNESS_WEIGHT_EXPIRING: f64 = 0.95;
pub const OKF_FRESHNESS_WEIGHT_STALE: f64 = 0.65;
pub const OKF_FRESHNESS_WEIGHT_UNSPECIFIED: f64 = 1.00;

// ── LMCache-Inspired Chained Prefix-Chunk & Reference Cache Engine ──────────
pub const ORCH_STAGE_PREFIX_CACHE: &str = "prefix_cache";
pub const PREFIX_CACHE_MAX_SESSIONS: usize = 64;
pub const PREFIX_CACHE_KEY_PREFIX: &str = "zk-pfx-";
pub const PREFIX_CACHE_KEY_HEX_LEN: usize = 24;
pub const PREFIX_CACHE_CHUNK_REF_MIN_BYTES: usize = 1_536;
pub const PREFIX_CACHE_CHUNK_REF_MAX_ENTRIES: usize = 64;
pub const PREFIX_CACHE_CHUNK_REF_META_KEY: &str = "prefix_chunk_ref";
pub const PREFIX_CACHE_CHUNK_REF_TOKENS_SAVED_KEY: &str = "prefix_chunk_ref_tokens_saved";
pub const PREFIX_CACHE_TAIL_BLOCK_TAG_OPEN: &str = "<session_runtime_context>";
pub const PREFIX_CACHE_TAIL_BLOCK_TAG_CLOSE: &str = "</session_runtime_context>";

// ── ZPowers Native Plan-First Orchestration & Per-Session Workspace (.zenko/sessions/) ──
pub const TOOL_ZPOWERS: &str = "zpowers";
pub const ORCH_STAGE_ZPOWERS: &str = "zpowers";
pub const ZENKO_PROJECT_DIR: &str = ".zenko";
pub const ZENKO_PROJECT_CONFIG_FILE: &str = "config.toml";
pub const ZENKO_PROJECT_FILTERS_DIR: &str = "filters";
pub const ZENKO_PROJECT_SESSIONS_DIR: &str = "sessions";
pub const ZENKO_PROJECT_LATEST_SESSION_FILE: &str = "latest.json";
pub const ZENKO_SESSION_MANIFEST_FILE: &str = "session.json";
pub const ZENKO_SESSION_ZPOWERS_DIR: &str = "zpowers";
pub const ZENKO_SESSION_ARTIFACTS_DIR: &str = "artifacts";
pub const ZENKO_SESSION_SCRATCH_DIR: &str = "scratch";
pub const ZENKO_GIT_EXCLUDE_REL_PATH: &str = ".git/info/exclude";
pub const ZENKO_GIT_EXCLUDE_ENTRY: &str = ".zenko/sessions/";
/// Project-level `.zenko/` configuration paths kept read-only to autonomous agent writes (Codex pattern).
pub const ZENKO_PROTECTED_PROJECT_SUBPATHS: &[&str] = &[
    ".zenko/config.toml",
    ".zenko/filters",
];
pub const ZPOWERS_STATE_SCHEMA_VERSION: u8 = 1;
pub const ZPOWERS_STATE_FILE: &str = "state.json";
pub const ZPOWERS_SPEC_FILE: &str = "spec.md";
pub const ZPOWERS_PLAN_FILE: &str = "plan.md";
pub const ZPOWERS_DEBUG_FILE: &str = "debug.md";
pub const ZPOWERS_WALKTHROUGH_FILE: &str = "walkthrough.md";
pub const ZPOWERS_REVIEWS_DIR: &str = "reviews";
pub const ZPOWERS_ACTIONS: &[&str] = &[
    "status",
    "write_spec",
    "write_plan",
    "log_debug",
    "record_review",
    "write_walkthrough",
    "fast_track",
    "promote",
];
pub const ZPOWERS_MODES: &[&str] = &["off", "advisory", "guided", "strict"];
pub const ZPOWERS_PHASES: &[&str] = &[
    "idle",
    "recon",
    "spec",
    "plan",
    "execute",
    "debug",
    "verify",
    "review",
    "done",
];
pub const ZPOWERS_REVIEW_STAGES: &[&str] = &["spec", "quality"];
pub const ZPOWERS_REVIEW_VERDICTS: &[&str] = &["approved", "changes_requested", "blocked"];
pub const ZPOWERS_PROMOTE_ALLOWED_ROOTS: &[&str] = &["docs", "knowledge"];
pub const ZPOWERS_DEFAULT_MAX_SESSIONS_PER_WORKSPACE: usize = 25;
pub const ZPOWERS_DEFAULT_RETENTION_DAYS: u64 = 30;
pub const ZPOWERS_DEFAULT_DEBUG_STRIKE_LIMIT: usize = 3;
pub const ZPOWERS_MAX_DOC_BYTES: usize = 64 * 1024;
pub const ZPOWERS_MAX_REVIEWS: usize = 16;
pub const ZPOWERS_MAX_DEBUG_ENTRIES: usize = 24;
pub const ZPOWERS_FAST_TRACK_REASON_MAX_CHARS: usize = 200;

/// Returns `true` only for `zpowers` actions that mutate tracked workspace files outside
/// `.zenko/sessions/<id>/` (currently `promote`). Planning/spec/review writes inside the
/// active session's isolated `zpowers/` folder remain permitted in Read-Only Plan Mode.
pub fn is_zpowers_args_mutating(args: &serde_json::Value) -> bool {
    args.get("action")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|a| a.trim().eq_ignore_ascii_case("promote"))
}
