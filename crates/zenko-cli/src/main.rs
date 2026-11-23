mod commands;
mod debug;
mod ipc;
mod mcp;
mod service;
mod setup;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use commands::{AskParams, LogsParams};
use ipc::OutputMode;
use mimalloc::MiMalloc;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};
use zenko_core::{
    ATTACH_DEFAULT_REPLAY_EVENTS, ApprovalDecision, DEFAULT_SHUTDOWN_GRACE_SECS,
    DEFAULT_TOKIO_BLOCKING_THREADS, DEFAULT_TOKIO_WORKERS, EXIT_OK, JobSupervisor, LogChannel,
    LogDirection, LogStatusTone, SecretVault, VAULT_KEY_WEBUI_ADMIN, ZenkoConfig, emit_hono_line,
    find_provider,
};
use zenko_gateways::TelegramGateway;
use zenko_runtime::{
    AgentRuntime, LogBroadcastLayer, run_activity_logger, run_dream_worker,
    run_goal_loop_scheduler, run_workspace_watchers,
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser)]
#[command(
    name = "zenko",
    version,
    about = "Zenko — 100% Native Rust 24/7 Autonomous AI Agent for Humans & AI Agents (Terminal CLI, MCP, WebUI, Telegram)"
)]
struct Cli {
    /// Path to zenko.toml config file
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Emit machine-readable JSON output (ideal for AI agents & scripts)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive first-run setup TUI (Providers, Streaming, Gateway)
    Setup,

    /// Start Zenko WebUI & 24/7 autonomous daemon (WebUI + OpenAI API + Telegram + Cron + Unix IPC)
    Web {
        /// Override WebUI port
        #[arg(short, long)]
        port: Option<u16>,
        /// Override WebUI bind address
        #[arg(short, long)]
        bind: Option<String>,
        /// Print WebUI URL and admin token without starting the server
        #[arg(long)]
        info: bool,
    },

    /// Check running daemon status, RAM RSS, gateways, and daily spend
    Status,

    /// Run a prompt/task (reads piped stdin or args; uses running daemon or standalone)
    Ask {
        /// Prompt text (pass `-` or pipe stdin for multi-line prompts)
        prompt: Vec<String>,
        /// Resume an existing session ID
        #[arg(short, long)]
        session: Option<String>,
        /// Override target workspace directory for this run
        #[arg(short, long)]
        workspace: Option<PathBuf>,
        /// Override model ID for this run
        #[arg(short, long)]
        model: Option<String>,
        /// Override tool policy mode (plan | strict | balanced | autonomous)
        #[arg(long)]
        mode: Option<String>,
        /// Maximum seconds before cancelling this run
        #[arg(long)]
        timeout: Option<u64>,
        /// Force standalone inline run (bypass running daemon IPC)
        #[arg(long)]
        standalone: bool,
        /// Wait for HITL approvals instead of failing fast in headless runs
        #[arg(long)]
        wait_approval: bool,
        /// Stream live NDJSON events (thinking, text, tool_start, tool_done, result)
        #[arg(long)]
        stream_json: bool,
    },

    /// Interactive terminal chat REPL (auto-attaches to running `zenko web` daemon)
    Chat {
        /// Resume an existing session ID
        #[arg(short, long)]
        session: Option<String>,
        /// Override workspace directory
        #[arg(short, long)]
        workspace: Option<PathBuf>,
        /// Override model ID
        #[arg(short, long)]
        model: Option<String>,
        /// Tool execution policy (strict | balanced | autonomous)
        #[arg(long)]
        mode: Option<String>,
        /// Force standalone inline runtime (do not attach to running `zenko web` daemon)
        #[arg(long)]
        standalone: bool,
    },

    /// Attach a terminal to a live session on the running `zenko web` daemon (Ctrl+C detaches safely)
    Attach {
        /// Session ID to observe (or `*` for all sessions)
        session_id: String,
        /// Recent messages to replay on attach
        #[arg(long, default_value_t = ATTACH_DEFAULT_REPLAY_EVENTS)]
        replay: usize,
    },

    /// Demote an active foreground shell command in a session to an async background job
    #[command(alias = "bg")]
    Background {
        /// Active session ID
        session_id: String,
    },

    /// Manage conversation sessions (list, show, clear, delete)
    Sessions {
        #[command(subcommand)]
        action: SessionsCmd,
    },

    /// Inspect and resolve pending tool approvals from terminal
    Approvals {
        #[command(subcommand)]
        action: ApprovalsCmd,
    },

    /// Search or add entries in persistent SQLite FTS5 long-term memory
    Memory {
        #[command(subcommand)]
        action: MemoryCmd,
    },

    /// Manage 24/7 recurring cron jobs, heartbeats, and zero-token hub scripts
    Cron {
        #[command(subcommand)]
        action: CronCmd,
    },

    /// Unified ZenkoTasks manager (CRUD, stream, input, restart, delete, prune across shell/build/dev/debug/script/cron/browser/subagent)
    #[command(alias = "jobs")]
    Tasks {
        #[command(subcommand)]
        action: JobsCmd,
    },

    /// Manage Phase 5 async subagents, git worktrees, and inter-agent mailbox
    Agents {
        #[command(subcommand)]
        action: AgentsCmd,
    },

    /// Inspect ~/zenko_hub/ categorized creations (projects, scripts, media, artifacts)
    Hub {
        #[command(subcommand)]
        action: HubCmd,
    },

    /// Inspect or cancel durable SQLite follow-up queue prompts for a session
    Queue {
        #[command(subcommand)]
        action: QueueCmd,
    },

    /// Inspect or answer pending interactive orchestrator clarifications
    Clarify {
        #[command(subcommand)]
        action: ClarifyCmd,
    },

    /// Inject non-destructive mid-flight steering guidance into a running session turn
    Steer {
        /// Active session ID
        session_id: String,
        /// Steering guidance text
        message: Vec<String>,
    },

    /// Query or live-follow categorized logs (session, gateway, webui, tui, cron, system)
    Logs {
        /// Number of recent log lines
        #[arg(short = 'n', long, default_value_t = 50)]
        limit: usize,
        /// Filter category (session | gateway | webui | tui | cron | system)
        #[arg(short = 'c', long)]
        category: Option<String>,
        /// Filter specific channel (agent | tool | input | approve | http | gateway | tui | cron | system)
        #[arg(long)]
        channel: Option<String>,
        /// Filter log level (INFO | WARN | ERROR)
        #[arg(short = 'l', long)]
        level: Option<String>,
        /// Filter to a specific session ID
        #[arg(short = 's', long)]
        session: Option<String>,
        /// Substring search across message, target, and structured JSON payload
        #[arg(short = 'g', long = "grep")]
        search: Option<String>,
        /// Stream live matching logs from the running daemon (Ctrl+C to stop)
        #[arg(short = 'f', long)]
        follow: bool,
    },

    /// Inspect turn-by-turn execution waterfall (LLM tokens, cache, tools, diffs, logs) for a session
    Trace {
        /// Session ID to trace
        session_id: String,
    },

    /// Inspect live per-tier context token budget, KV-cache hits, deferred tools, artifacts & checkpoints
    Context {
        /// Session ID to inspect
        session_id: String,
    },

    /// One-step safe rewind to a workspace checkpoint (preserves manual hand-edits by default)
    Undo {
        /// Session ID to rewind
        session_id: String,
        /// Optional explicit checkpoint ID (defaults to latest non-safety checkpoint)
        #[arg(long)]
        checkpoint: Option<String>,
        /// Rewind mode (`code_and_conversation` | `code_only` | `conversation_only`)
        #[arg(long, default_value = "code_and_conversation")]
        mode: String,
        /// Optional specific relative file path(s) to restore
        #[arg(long = "file")]
        files: Vec<String>,
        /// Overwrite manual hand-edits made after the last agent turn
        #[arg(long)]
        force: bool,
        /// Skip writing an abandoned-branch lesson summary note
        #[arg(long)]
        no_summary: bool,
    },

    /// Manage shadow-git workspace checkpoints (list, diff preview, rewind, prune)
    Checkpoints {
        #[command(subcommand)]
        action: CheckpointsCmd,
    },

    /// Manage durable SQLite Session Task DAGs (list, ready, create, update, claim, delete, clear)
    Dag {
        #[command(subcommand)]
        action: DagCmd,
    },

    /// Plan Mode operations (approve a session's read-only plan to unlock mutations)
    Plan {
        #[command(subcommand)]
        action: PlanCmd,
    },

    /// Developer diagnostics: dry-run tools, inspect ExecPolicy lexer, prompt snapshot, logs dir, or export a scrubbed bundle
    Debug {
        #[command(subcommand)]
        action: DebugCmd,
    },

    /// Manage Linux systemd --user 24/7 background service unit
    Service {
        #[command(subcommand)]
        action: ServiceCmd,
    },

    /// Gateway management (pairing codes, allowlists)
    Gateway {
        #[command(subcommand)]
        action: GatewayCmd,
    },

    /// Run health, vault, provider, and Telegram diagnostics
    Doctor,

    /// Start Model Context Protocol (MCP) stdio server for AI-agent integration
    Mcp,

    /// Manage external MCP client servers (`stdio` & Streamable HTTP) and live tool discovery
    McpClient {
        #[command(subcommand)]
        action: McpClientCmd,
    },

    /// Inspect or stop pooled Language Server Protocol (LSP) servers powering `lsp_tool`
    Lsp {
        #[command(subcommand)]
        action: LspCmd,
    },

    /// Manage procedural skills (`~/.zenko/skills/`), draft approval, and `.history/` rollback
    Skills {
        #[command(subcommand)]
        action: SkillsCmd,
    },

    /// Manage 24/7 bounded autonomous verify-until-green Goal Loops
    Goals {
        #[command(subcommand)]
        action: GoalsCmd,
    },

    /// Inspect declarative webhook routing rules and workspace file-change watchers
    Triggers {
        #[command(subcommand)]
        action: TriggersCmd,
    },

    /// Export scrubbed multi-turn trajectories (`jsonl`, `sharegpt`, or `markdown`)
    Export {
        /// Optional single session ID to export (omit with `--all` for filtered batch export)
        session_id: Option<String>,
        /// Export all matching sessions in batch mode
        #[arg(long)]
        all: bool,
        /// Output format: `jsonl` | `sharegpt` | `markdown`
        #[arg(short, long, default_value = "jsonl")]
        format: String,
        /// Optional time window (`24h`, `7d`, `2w`, or ISO date) for batch export
        #[arg(long)]
        since: Option<String>,
        /// Optional channel filter (`web`, `cli`, `telegram`, `goal`, `webhook`)
        #[arg(long)]
        channel: Option<String>,
        /// Export only sessions linked to a passing Goal Loop verification (`exit_code == 0`)
        #[arg(long)]
        verified_only: bool,
        /// Minimum user turns required per session
        #[arg(long, default_value_t = 0)]
        min_turns: usize,
        /// Include assistant reasoning/thinking blocks (`<think>` in ShareGPT)
        #[arg(long)]
        include_thinking: bool,
        /// Replace home & workspace paths with `<HOME>` / `<WORKSPACE>`
        #[arg(long)]
        anonymize_paths: bool,
        /// Export every event in the session tree instead of only the active branch
        #[arg(long)]
        all_branches: bool,
        /// Optional output file path (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SessionsCmd {
    /// List recent sessions
    List {
        #[arg(short = 'n', long, default_value_t = 25)]
        limit: usize,
    },
    /// Full-text search across past session transcripts (FTS5)
    Search {
        /// Search query keywords
        query: String,
        /// Optional time window (`24h`, `7d`, `2w`, or ISO date)
        #[arg(long)]
        since: Option<String>,
        /// Filter by channel (`web`, `cli`, `telegram`, `cron`, `api`)
        #[arg(long)]
        channel: Option<String>,
        /// Workspace filter (`all` | `current` | explicit path)
        #[arg(long, default_value = "all")]
        workspace: String,
        /// Filter by speaker role (`user` | `assistant` | `tool`)
        #[arg(long)]
        role: Option<String>,
        #[arg(short = 'n', long, default_value_t = 12)]
        limit: usize,
    },
    /// Show active branch transcript for a session
    Show { id: String },
    /// Clear events in a session
    Clear { id: String },
    /// Delete a session
    Delete { id: String },
}

#[derive(Subcommand)]
enum ApprovalsCmd {
    /// List pending tool approvals
    List,
    /// Approve a pending tool execution once
    Approve { id: String },
    /// Approve and always allow this tool for the daemon lifetime
    Always { id: String },
    /// Deny a pending tool execution
    Deny { id: String },
}

#[derive(Subcommand)]
enum MemoryCmd {
    /// Search hybrid scoped memory (empty query lists recent entries)
    Search {
        #[arg(default_value = "")]
        query: String,
        /// Scope filter (`all` | `current` | `project` | `global`)
        #[arg(long, default_value = "all")]
        scope: String,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Add (or near-duplicate merge) a persistent memory entry
    Add {
        title: String,
        content: String,
        #[arg(long, default_value = "fact")]
        kind: String,
        /// Scope (`global` | `project`)
        #[arg(long, default_value = "global")]
        scope: String,
    },
    /// Update an existing memory entry by ID
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        scope: Option<String>,
    },
    /// Delete a memory entry by ID
    Delete { id: String },
    /// Run background 2-stage memory consolidation ("Dream") now
    Dream {
        /// Ignore session count and cooldown gates
        #[arg(long)]
        force: bool,
    },
    /// List reversible `.history/` snapshots of memory Markdown docs
    History {
        /// Optional doc filter (`USER.md`, `MEMORY.md`, `SOUL.md`, `ZENKO.md`)
        #[arg(long)]
        doc: Option<String>,
    },
    /// Restore a memory doc from a `.history/` snapshot ID
    Rollback { snapshot_id: String },
}

#[derive(Subcommand)]
enum CronCmd {
    /// List scheduled 24/7 jobs
    List,
    /// Schedule a new recurring cron job, heartbeat, or zero-token script job
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        expr: String,
        #[arg(long)]
        prompt: Option<String>,
        /// Script filename/subpath inside ~/zenko_hub/scripts/ (for --kind script)
        #[arg(long)]
        script: Option<String>,
        #[arg(long, default_value = "cron")]
        kind: String,
    },
    /// Edit an existing cron job
    Edit {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        expr: Option<String>,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        script: Option<String>,
    },
    /// Pause a scheduled cron job
    Pause { id: String },
    /// Resume a paused cron job
    Resume { id: String },
    /// Trigger a cron job immediately (LLM or zero-token script)
    Run { id: String },
    /// Delete a scheduled job by ID
    Delete { id: String },
}

#[derive(Subcommand)]
enum JobsCmd {
    /// List running and recent ZenkoTasks (filterable by --kind or --session)
    List {
        /// Optional filter by session ID
        #[arg(short, long)]
        session: Option<String>,
        /// Optional category filter (shell|build|dev|debug|script|cron|browser|subagent)
        #[arg(short, long)]
        kind: Option<String>,
        #[arg(short = 'n', long, default_value_t = 25)]
        limit: usize,
    },
    /// Start a categorized background task directly
    Start {
        /// Category (auto-inferred from command if omitted)
        #[arg(short, long)]
        kind: Option<String>,
        /// Optional human-readable label
        #[arg(short, long)]
        label: Option<String>,
        /// Optional workspace directory override
        #[arg(short, long)]
        workspace: Option<PathBuf>,
        /// Command to run
        command: Vec<String>,
    },
    /// Read streamed output from a task log
    Read {
        id: String,
        /// Byte offset (omit for latest tail)
        #[arg(long)]
        offset: Option<u64>,
        #[arg(long)]
        max_bytes: Option<usize>,
    },
    /// Send stdin text to a running task
    Input {
        id: String,
        text: String,
    },
    /// Update a task's label or category kind
    Update {
        id: String,
        #[arg(short, long)]
        label: Option<String>,
        #[arg(short, long)]
        kind: Option<String>,
    },
    /// Restart an existing task with the same command, kind, and label
    Restart { id: String },
    /// Wait (event-driven, zero-CPU) for a task to finish and show its tail output
    Wait {
        id: String,
        /// Max seconds to wait (default 30, max 120)
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Live-follow output from a running task (zero-polling, Ctrl+C detaches safely)
    Follow { id: String },
    /// Stop a running task and its process group
    Stop { id: String },
    /// Delete a task record and its categorized log file
    Delete { id: String },
    /// Prune finished tasks and delete their logs (optionally filtered by --kind)
    Prune {
        #[arg(short, long)]
        kind: Option<String>,
    },
}

#[derive(Subcommand)]
enum AgentsCmd {
    /// List subagents across sessions (optionally filtered by --parent)
    List {
        #[arg(short, long)]
        parent: Option<String>,
        #[arg(short = 'n', long, default_value_t = zenko_core::SUBAGENT_LIST_DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Show detailed subagent summary, structured JSON output, and mailbox messages
    Show { id: String },
    /// Spawn a scoped subagent (`explorer` | `architect` | `coder` | `verifier`)
    Spawn {
        /// Declarative role ID (`explorer` | `architect` | `coder` | `verifier`)
        role: String,
        /// Self-contained goal prompt
        goal: Vec<String>,
        /// Optional parent session ID
        #[arg(short, long)]
        parent: Option<String>,
        /// Workspace isolation mode (`inherit` | `worktree`)
        #[arg(short, long)]
        workspace: Option<String>,
        /// Optional model override
        #[arg(short, long)]
        model: Option<String>,
        /// Run in background and return agent ID immediately
        #[arg(long)]
        bg: bool,
        /// Fork a bounded digest of the parent session's active branch
        #[arg(long)]
        fork_context: bool,
        /// Snapshot uncommitted parent edits into the isolated git worktree
        #[arg(long)]
        include_uncommitted: bool,
        /// Optional wall-clock timeout ceiling in seconds
        #[arg(long)]
        max_duration: Option<u64>,
        /// Optional USD spend ceiling
        #[arg(long)]
        max_cost: Option<f64>,
    },
    /// Event-driven zero-CPU wait for a subagent to complete
    Wait {
        id: String,
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Send a targeted mailbox / steering / resume message to a subagent
    Message {
        id: String,
        message: Vec<String>,
        /// Do not auto-resume an idle/completed subagent (queue in mailbox only)
        #[arg(long)]
        no_resume: bool,
        /// Interrupt a currently running subagent and restart it with this guidance
        #[arg(long)]
        interrupt: bool,
    },
    /// Inspect git diff and changed files inside a worktree-isolated subagent
    Diff { id: String },
    /// Cleanly merge a completed worktree subagent's branch into the parent repo
    Merge {
        id: String,
        /// Keep the temporary worktree and branch after merging
        #[arg(long)]
        keep: bool,
    },
    /// Discard a completed/stopped subagent's git worktree and branch without merging
    Discard { id: String },
    /// Prune orphaned crash-leftover worktree directories under ~/.zenko/worktrees/
    Prune,
    /// Cascade-kill a subagent, its descendants, and child shell jobs
    Kill {
        id: String,
        /// Preserve the git worktree instead of removing it
        #[arg(long)]
        keep_worktree: bool,
    },
}

#[derive(Subcommand)]
enum HubCmd {
    /// List categorized files & projects inside ~/zenko_hub/
    List {
        #[arg(short, long)]
        category: Option<String>,
        #[arg(short = 'n', long, default_value_t = 100)]
        limit: usize,
    },
    /// Print resolved ~/zenko_hub/ root path (ensuring subfolders exist)
    Path,
}

#[derive(Subcommand)]
enum QueueCmd {
    /// List durable queued follow-up prompts for a session
    List { session_id: String },
    /// Cancel a queued follow-up prompt by ID
    Cancel { session_id: String, queue_id: String },
}

#[derive(Subcommand)]
enum ClarifyCmd {
    /// List pending interactive clarification cards
    List {
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Answer a pending clarification card by ID and option ID
    Answer {
        id: String,
        option_id: String,
        #[arg(long)]
        custom: Option<String>,
    },
}

#[derive(Subcommand)]
enum CheckpointsCmd {
    /// List saved workspace checkpoints (optionally filtered by session ID)
    List {
        #[arg(short, long)]
        session: Option<String>,
        #[arg(short = 'n', long, default_value_t = zenko_core::CHECKPOINT_LIST_DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Preview unified diff and manual hand-edit conflicts for a checkpoint
    Diff { checkpoint_id: String },
    /// Rewind code and/or conversation to a specific checkpoint ID
    Rewind {
        checkpoint_id: String,
        #[arg(long, default_value = "code_and_conversation")]
        mode: String,
        #[arg(long = "file")]
        files: Vec<String>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        no_summary: bool,
    },
    /// Prune expired workspace checkpoints and run shadow-git garbage collection
    Prune,
}

#[derive(Subcommand)]
enum DagCmd {
    /// List all durable SQLite Task DAG nodes for a session
    List { session_id: String },
    /// List topologically ready (unblocked pending) Task DAG nodes for a session
    Ready { session_id: String },
    /// Create a new Task DAG node in SQLite
    Create {
        session_id: String,
        title: String,
        #[arg(long)]
        id: Option<String>,
        #[arg(long = "dep")]
        depends_on: Vec<String>,
        #[arg(long)]
        verify: Option<String>,
        #[arg(long, default_value_t = 0)]
        priority: i32,
    },
    /// Update a Task DAG node's status, notes, or verification evidence
    Update {
        session_id: String,
        task_id: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        evidence: Option<String>,
        #[arg(long)]
        notes: Option<String>,
    },
    /// Atomically claim a ready Task DAG node for an owner/worker ID
    Claim {
        session_id: String,
        task_id: String,
        #[arg(long, default_value = "cli")]
        owner: String,
    },
    /// Delete a Task DAG node (and clean dangling downstream dependencies)
    Delete { session_id: String, task_id: String },
    /// Clear all Task DAG nodes for a session
    Clear { session_id: String },
}

#[derive(Subcommand)]
enum PlanCmd {
    /// Approve a session's read-only plan and promote its permission mode (`autonomous` | `balanced` | `strict`)
    Approve {
        session_id: String,
        #[arg(long, default_value = "autonomous")]
        mode: String,
    },
}

#[derive(Subcommand)]
enum GatewayCmd {
    /// Generate a 6-character one-time pairing code for Telegram
    Pair {
        #[arg(long, default_value = "owner")]
        role: String,
    },
    /// List paired Telegram peers
    Peers,
}

#[derive(Subcommand)]
enum DebugCmd {
    /// Invoke a Zenko tool directly without calling an LLM (pass `list` to see all tools)
    Tool {
        /// Tool name (or `list`)
        name: String,
        /// Optional JSON object of tool arguments (e.g. '{"path":"Cargo.toml"}')
        args: Option<String>,
        /// Optional workspace directory override
        #[arg(short, long)]
        workspace: Option<PathBuf>,
    },
    /// Test a shell command against the deterministic POSIX lexer & ExecPolicy evaluator
    Policy {
        /// Shell command string to inspect
        command: String,
        /// Override permission mode (strict | balanced | autonomous)
        #[arg(long)]
        mode: Option<String>,
    },
    /// Test a prompt against the deterministic Phase 2 Orchestrator Triage & Model Router
    Triage {
        /// Prompt text to classify
        prompt: String,
        /// Override permission mode (strict | balanced | autonomous)
        #[arg(long)]
        mode: Option<String>,
    },
    /// Inspect the frozen KV-cache system prompt snapshot, hash, and token estimate
    Prompt {
        /// Optional workspace directory override
        #[arg(short, long)]
        workspace: Option<PathBuf>,
        /// Print the full assembled system prompt text
        #[arg(long)]
        full: bool,
    },
    /// Inspect categorized JSONL log directory sizes, SQLite row counts, and wire-capture status
    LogsDir,
    /// Generate a secret-scrubbed diagnostic JSON bundle safe for sharing in bug reports
    Bundle {
        /// Output file path (defaults to ./zenko-debug-bundle-<timestamp>.json)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Max recent log entries per category to include
        #[arg(short = 'n', long, default_value_t = 100)]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum ServiceCmd {
    /// Print the generated systemd --user unit file (`zenko.service`)
    Unit,
    /// Install `~/.config/systemd/user/zenko.service` and reload systemd
    Install {
        /// Also run `systemctl --user enable --now zenko.service`
        #[arg(long)]
        enable: bool,
    },
    /// Show `systemctl --user status zenko.service`
    Status,
}

#[derive(Subcommand)]
enum McpClientCmd {
    /// List configured external MCP client servers and their discovered deferred tools
    List,
    /// Add or update an external MCP server (`stdio` or Streamable HTTP `2025-03-26`)
    Add {
        /// Unique server name (tools become `mcp__<name>__<tool>`)
        #[arg(long)]
        name: String,
        /// Transport (`stdio` | `http`)
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// Executable command (for `stdio` transport)
        #[arg(long, default_value = "")]
        command: String,
        /// Command argument (repeatable, for `stdio`)
        #[arg(long = "arg")]
        args: Vec<String>,
        /// Streamable HTTP endpoint URL (for `http` transport)
        #[arg(long, default_value = "")]
        url: String,
        /// SecretVault key name used as `Authorization: Bearer <secret>` (for `http`)
        #[arg(long)]
        auth_secret: Option<String>,
        /// Environment secret mapping `ENV_VAR=VAULT_KEY` (repeatable, for `stdio`)
        #[arg(long = "env-secret")]
        env_secrets: Vec<String>,
        /// Optional remote tool allowlist (repeatable)
        #[arg(long = "allow-tool")]
        enabled_tools: Vec<String>,
        /// Optional remote tool denylist overriding allowlist (repeatable)
        #[arg(long = "deny-tool")]
        disabled_tools: Vec<String>,
        /// Optional risk override (`safe` | `moderate` | `dangerous`)
        #[arg(long)]
        risk: Option<String>,
        /// Allow concurrent parallel calls to this MCP server
        #[arg(long)]
        parallel: bool,
        /// Permit private/loopback URLs for local HTTP MCP servers
        #[arg(long)]
        allow_private_url: bool,
        /// Immediately connect and sync `tools/list` after saving
        #[arg(long)]
        sync: bool,
    },
    /// Remove an external MCP server and unregister its `mcp__<name>__*` tools
    Remove { name: String },
    /// Enable or disable a configured external MCP server
    Toggle {
        name: String,
        /// Explicitly set enabled (`true` | `false`); omit to invert current state
        #[arg(long)]
        enabled: Option<bool>,
    },
    /// Connect to a configured external MCP server and refresh its cached deferred tool catalog
    Sync { name: String },
}

#[derive(Subcommand)]
enum LspCmd {
    /// Show configured LSP languages and currently pooled active language server processes
    Status,
    /// Cleanly stop all currently pooled LSP language server processes
    Stop,
}

#[derive(Subcommand)]
enum SkillsCmd {
    /// List installed global and workspace skills (including pending drafts)
    List,
    /// Show a skill's SKILL.md content (or a specific reference subfile)
    Show {
        slug: String,
        #[arg(long)]
        file: Option<String>,
    },
    /// Create or update a procedural skill under `~/.zenko/skills/`
    Create {
        slug: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        content: String,
    },
    /// Enable or disable a skill
    Toggle {
        slug: String,
        #[arg(long)]
        enabled: bool,
    },
    /// Approve a pending draft skill so it activates in `<available_skills>`
    Approve { slug: String },
    /// List `.history/` version backups for skills
    History {
        #[arg(long)]
        slug: Option<String>,
    },
    /// Roll back a skill to a previous `.history/` snapshot ID
    Rollback { snapshot_id: String },
    /// Delete a skill (automatically snapshots pre-delete backup into `.history/`)
    Delete { slug: String },
}

#[derive(Subcommand)]
enum GoalsCmd {
    /// List persistent 24/7 autonomous Goal Loops
    List {
        #[arg(short = 'n', long, default_value_t = zenko_core::GOAL_LOOP_LIST_DEFAULT_LIMIT)]
        limit: usize,
    },
    /// Show detailed status and recent verification runs for a Goal Loop
    Show { id: String },
    /// Create a new bounded autonomous verify-until-green Goal Loop
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        goal: String,
        /// Verification command argv (`--verify cargo --verify test --verify --workspace`)
        #[arg(long = "verify", required = true)]
        verify_cmd: Vec<String>,
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        interval: Option<u64>,
        #[arg(long)]
        max_iterations: Option<usize>,
        #[arg(long)]
        max_budget: Option<f64>,
        #[arg(long)]
        stalls: Option<usize>,
        #[arg(long)]
        auto_rewind: bool,
        #[arg(long)]
        model: Option<String>,
    },
    /// Pause an active Goal Loop
    Pause { id: String },
    /// Resume a paused/stalled Goal Loop
    Resume { id: String },
    /// Run a single Goal Loop iteration immediately
    Run { id: String },
    /// Delete a Goal Loop and its run ledger
    Delete { id: String },
}

#[derive(Subcommand)]
enum TriggersCmd {
    /// List configured webhook routing rules and workspace watchers with live rate-limit telemetry
    List,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli
        .config
        .unwrap_or_else(ZenkoConfig::default_config_path);
    let json_out = cli.json;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(DEFAULT_TOKIO_WORKERS)
        .max_blocking_threads(DEFAULT_TOKIO_BLOCKING_THREADS)
        .enable_all()
        .build()
        .context("failed to initialize tokio runtime")?;

    let exit_code: Result<i32> = rt.block_on(async move {
        match cli.command {
            Commands::Setup => {
                setup::run_interactive_setup(&config_path).await?;
                Ok(EXIT_OK)
            }
            Commands::Web { port, bind, info } => {
                match info {
                    true => commands::run_web_info(&config_path, json_out)?,
                    false => run_web_daemon(config_path, port, bind).await?,
                }
                Ok(EXIT_OK)
            }
            Commands::Status => commands::run_status(&config_path, json_out).await,
            Commands::Ask {
                prompt,
                session,
                workspace,
                model,
                mode,
                timeout,
                standalone,
                wait_approval,
                stream_json,
            } => {
                let output = match (stream_json, json_out) {
                    (true, _) => OutputMode::StreamJson,
                    (false, true) => OutputMode::Json,
                    (false, false) => OutputMode::Text,
                };
                commands::run_ask(
                    &config_path,
                    AskParams {
                        prompt,
                        session,
                        workspace,
                        model,
                        mode,
                        timeout_secs: timeout,
                        standalone,
                        wait_approval,
                        output,
                    },
                )
                .await?;
                Ok(EXIT_OK)
            }
            Commands::Chat {
                session,
                workspace,
                model,
                mode,
                standalone,
            } => {
                commands::run_chat(&config_path, session, workspace, model, mode, standalone)
                    .await?;
                Ok(EXIT_OK)
            }
            Commands::Attach { session_id, replay } => {
                commands::run_attach(&config_path, &session_id, replay, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Background { session_id } => {
                commands::run_session_background(&config_path, &session_id, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Sessions { action } => {
                match action {
                    SessionsCmd::List { limit } => {
                        commands::run_sessions_list(&config_path, limit, json_out).await?
                    }
                    SessionsCmd::Search {
                        query,
                        since,
                        channel,
                        workspace,
                        role,
                        limit,
                    } => {
                        commands::run_sessions_search(
                            &config_path,
                            &query,
                            since.as_deref(),
                            channel.as_deref(),
                            &workspace,
                            role.as_deref(),
                            limit,
                            json_out,
                        )
                        .await?
                    }
                    SessionsCmd::Show { id } => {
                        commands::run_session_show(&config_path, &id, json_out).await?
                    }
                    SessionsCmd::Clear { id } => {
                        commands::run_session_clear(&config_path, &id, json_out).await?
                    }
                    SessionsCmd::Delete { id } => {
                        commands::run_session_delete(&config_path, &id, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Approvals { action } => {
                match action {
                    ApprovalsCmd::List => commands::run_approvals_list(&config_path, json_out).await?,
                    ApprovalsCmd::Approve { id } => {
                        commands::run_approval_resolve(
                            &config_path,
                            &id,
                            ApprovalDecision::Approve,
                            json_out,
                        )
                        .await?
                    }
                    ApprovalsCmd::Always { id } => {
                        commands::run_approval_resolve(
                            &config_path,
                            &id,
                            ApprovalDecision::AlwaysAllow,
                            json_out,
                        )
                        .await?
                    }
                    ApprovalsCmd::Deny { id } => {
                        commands::run_approval_resolve(
                            &config_path,
                            &id,
                            ApprovalDecision::Deny,
                            json_out,
                        )
                        .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Memory { action } => {
                match action {
                    MemoryCmd::Search {
                        query,
                        scope,
                        limit,
                    } => {
                        commands::run_memory_search(&config_path, &query, &scope, limit, json_out)
                            .await?
                    }
                    MemoryCmd::Add {
                        title,
                        content,
                        kind,
                        scope,
                    } => {
                        commands::run_memory_add(
                            &config_path,
                            &title,
                            &content,
                            &kind,
                            &scope,
                            json_out,
                        )
                        .await?
                    }
                    MemoryCmd::Update {
                        id,
                        title,
                        content,
                        kind,
                        scope,
                    } => {
                        commands::run_memory_update(
                            &config_path,
                            &id,
                            title.as_deref(),
                            content.as_deref(),
                            kind.as_deref(),
                            scope.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                    MemoryCmd::Delete { id } => {
                        commands::run_memory_delete(&config_path, &id, json_out).await?
                    }
                    MemoryCmd::Dream { force } => {
                        commands::run_memory_dream(&config_path, force, json_out).await?
                    }
                    MemoryCmd::History { doc } => {
                        commands::run_memory_history(&config_path, doc.as_deref(), json_out).await?
                    }
                    MemoryCmd::Rollback { snapshot_id } => {
                        commands::run_memory_rollback(&config_path, &snapshot_id, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Cron { action } => {
                match action {
                    CronCmd::List => commands::run_cron_list(&config_path, json_out).await?,
                    CronCmd::Add {
                        name,
                        expr,
                        prompt,
                        script,
                        kind,
                    } => {
                        commands::run_cron_add(
                            &config_path,
                            &name,
                            &expr,
                            prompt.as_deref(),
                            script.as_deref(),
                            &kind,
                            json_out,
                        )
                        .await?
                    }
                    CronCmd::Edit {
                        id,
                        name,
                        expr,
                        prompt,
                        script,
                    } => {
                        commands::run_cron_update(
                            &config_path,
                            &id,
                            name.as_deref(),
                            expr.as_deref(),
                            prompt.as_deref(),
                            script.as_deref(),
                            None,
                            json_out,
                        )
                        .await?
                    }
                    CronCmd::Pause { id } => {
                        commands::run_cron_update(
                            &config_path,
                            &id,
                            None,
                            None,
                            None,
                            None,
                            Some(false),
                            json_out,
                        )
                        .await?
                    }
                    CronCmd::Resume { id } => {
                        commands::run_cron_update(
                            &config_path,
                            &id,
                            None,
                            None,
                            None,
                            None,
                            Some(true),
                            json_out,
                        )
                        .await?
                    }
                    CronCmd::Run { id } => {
                        commands::run_cron_run_now(&config_path, &id, json_out).await?
                    }
                    CronCmd::Delete { id } => {
                        commands::run_cron_delete(&config_path, &id, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Tasks { action } => {
                match action {
                    JobsCmd::List {
                        session,
                        kind,
                        limit,
                    } => {
                        commands::run_jobs_list(
                            &config_path,
                            session.as_deref(),
                            kind.as_deref(),
                            limit,
                            json_out,
                        )
                        .await?
                    }
                    JobsCmd::Start {
                        kind,
                        label,
                        workspace,
                        command,
                    } => {
                        commands::run_jobs_start(
                            &config_path,
                            &command,
                            kind.as_deref(),
                            label.as_deref(),
                            workspace,
                            json_out,
                        )
                        .await?
                    }
                    JobsCmd::Read {
                        id,
                        offset,
                        max_bytes,
                    } => {
                        commands::run_jobs_read(
                            &config_path,
                            &id,
                            offset,
                            max_bytes.unwrap_or_else(commands::default_job_read_bytes),
                            json_out,
                        )
                        .await?
                    }
                    JobsCmd::Input { id, text } => {
                        commands::run_jobs_input(&config_path, &id, &text, json_out).await?
                    }
                    JobsCmd::Update { id, label, kind } => {
                        commands::run_jobs_update(
                            &config_path,
                            &id,
                            label.as_deref(),
                            kind.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                    JobsCmd::Restart { id } => {
                        commands::run_jobs_restart(&config_path, &id, json_out).await?
                    }
                    JobsCmd::Wait { id, timeout } => {
                        commands::run_jobs_wait(&config_path, &id, timeout, json_out).await?
                    }
                    JobsCmd::Follow { id } => {
                        commands::run_jobs_follow(&config_path, &id, json_out).await?
                    }
                    JobsCmd::Stop { id } => {
                        commands::run_jobs_stop(&config_path, &id, json_out).await?
                    }
                    JobsCmd::Delete { id } => {
                        commands::run_jobs_delete(&config_path, &id, json_out).await?
                    }
                    JobsCmd::Prune { kind } => {
                        commands::run_jobs_prune(&config_path, kind.as_deref(), json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Agents { action } => {
                match action {
                    AgentsCmd::List { parent, limit } => {
                        commands::run_agents_list(&config_path, parent.as_deref(), limit, json_out)
                            .await?
                    }
                    AgentsCmd::Show { id } => {
                        commands::run_agents_show(&config_path, &id, json_out).await?
                    }
                    AgentsCmd::Spawn {
                        role,
                        goal,
                        parent,
                        workspace,
                        model,
                        bg,
                        fork_context,
                        include_uncommitted,
                        max_duration,
                        max_cost,
                    } => {
                        commands::run_agents_spawn(
                            &config_path,
                            &role,
                            &goal.join(" "),
                            parent.as_deref(),
                            workspace.as_deref(),
                            model.as_deref(),
                            bg,
                            fork_context,
                            include_uncommitted,
                            max_duration,
                            max_cost,
                            json_out,
                        )
                        .await?
                    }
                    AgentsCmd::Wait { id, timeout } => {
                        commands::run_agents_wait(&config_path, &id, timeout, json_out).await?
                    }
                    AgentsCmd::Message {
                        id,
                        message,
                        no_resume,
                        interrupt,
                    } => {
                        commands::run_agents_message(
                            &config_path,
                            &id,
                            &message.join(" "),
                            !no_resume,
                            interrupt,
                            json_out,
                        )
                        .await?
                    }
                    AgentsCmd::Diff { id } => {
                        commands::run_agents_diff(&config_path, &id, json_out).await?
                    }
                    AgentsCmd::Merge { id, keep } => {
                        commands::run_agents_merge(&config_path, &id, !keep, json_out).await?
                    }
                    AgentsCmd::Discard { id } => {
                        commands::run_agents_discard(&config_path, &id, json_out).await?
                    }
                    AgentsCmd::Prune => commands::run_agents_prune(&config_path, json_out).await?,
                    AgentsCmd::Kill { id, keep_worktree } => {
                        commands::run_agents_kill(&config_path, &id, !keep_worktree, json_out)
                            .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Hub { action } => {
                match action {
                    HubCmd::List { category, limit } => {
                        commands::run_hub_list(&config_path, category.as_deref(), limit, json_out)
                            .await?
                    }
                    HubCmd::Path => commands::run_hub_path(&config_path, json_out)?,
                }
                Ok(EXIT_OK)
            }
            Commands::Queue { action } => {
                match action {
                    QueueCmd::List { session_id } => {
                        commands::run_queue_list(&config_path, &session_id, json_out).await?
                    }
                    QueueCmd::Cancel {
                        session_id,
                        queue_id,
                    } => {
                        commands::run_queue_cancel(&config_path, &session_id, &queue_id, json_out)
                            .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Clarify { action } => {
                match action {
                    ClarifyCmd::List { session } => {
                        commands::run_clarifications_list(
                            &config_path,
                            session.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                    ClarifyCmd::Answer {
                        id,
                        option_id,
                        custom,
                    } => {
                        commands::run_clarification_answer(
                            &config_path,
                            &id,
                            &option_id,
                            custom.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Steer {
                session_id,
                message,
            } => {
                commands::run_session_steer(&config_path, &session_id, &message, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Logs {
                limit,
                category,
                channel,
                level,
                session,
                search,
                follow,
            } => {
                commands::run_logs(
                    &config_path,
                    LogsParams {
                        limit,
                        category,
                        channel,
                        level,
                        session,
                        search,
                        follow,
                        json_out,
                    },
                )
                .await?;
                Ok(EXIT_OK)
            }
            Commands::Trace { session_id } => {
                debug::run_trace(&config_path, &session_id, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Context { session_id } => {
                commands::run_session_context(&config_path, &session_id, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Undo {
                session_id,
                checkpoint,
                mode,
                files,
                force,
                no_summary,
            } => {
                commands::run_undo(
                    &config_path,
                    &session_id,
                    checkpoint.as_deref(),
                    &mode,
                    files,
                    force,
                    !no_summary,
                    json_out,
                )
                .await?;
                Ok(EXIT_OK)
            }
            Commands::Checkpoints { action } => {
                match action {
                    CheckpointsCmd::List { session, limit } => {
                        commands::run_checkpoints_list(
                            &config_path,
                            session.as_deref(),
                            limit,
                            json_out,
                        )
                        .await?
                    }
                    CheckpointsCmd::Diff { checkpoint_id } => {
                        commands::run_checkpoints_diff(&config_path, &checkpoint_id, json_out)
                            .await?
                    }
                    CheckpointsCmd::Rewind {
                        checkpoint_id,
                        mode,
                        files,
                        force,
                        no_summary,
                    } => {
                        commands::run_checkpoints_rewind(
                            &config_path,
                            &checkpoint_id,
                            &mode,
                            files,
                            force,
                            !no_summary,
                            json_out,
                        )
                        .await?
                    }
                    CheckpointsCmd::Prune => {
                        commands::run_checkpoints_prune(&config_path, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Dag { action } => {
                match action {
                    DagCmd::List { session_id } => {
                        commands::run_dag_list(&config_path, &session_id, false, json_out).await?
                    }
                    DagCmd::Ready { session_id } => {
                        commands::run_dag_list(&config_path, &session_id, true, json_out).await?
                    }
                    DagCmd::Create {
                        session_id,
                        title,
                        id,
                        depends_on,
                        verify,
                        priority,
                    } => {
                        commands::run_dag_create(
                            &config_path,
                            &session_id,
                            &title,
                            id.as_deref(),
                            depends_on,
                            verify.as_deref(),
                            priority,
                            json_out,
                        )
                        .await?
                    }
                    DagCmd::Update {
                        session_id,
                        task_id,
                        status,
                        owner,
                        evidence,
                        notes,
                    } => {
                        commands::run_dag_update(
                            &config_path,
                            &session_id,
                            &task_id,
                            status.as_deref(),
                            owner.as_deref(),
                            evidence.as_deref(),
                            notes.as_deref(),
                            json_out,
                        )
                        .await?
                    }
                    DagCmd::Claim {
                        session_id,
                        task_id,
                        owner,
                    } => {
                        commands::run_dag_claim(&config_path, &session_id, &task_id, &owner, json_out)
                            .await?
                    }
                    DagCmd::Delete {
                        session_id,
                        task_id,
                    } => {
                        commands::run_dag_delete(&config_path, &session_id, Some(&task_id), json_out)
                            .await?
                    }
                    DagCmd::Clear { session_id } => {
                        commands::run_dag_delete(&config_path, &session_id, None, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Plan { action } => {
                match action {
                    PlanCmd::Approve { session_id, mode } => {
                        commands::run_plan_approve(&config_path, &session_id, &mode, json_out)
                            .await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Debug { action } => {
                match action {
                    DebugCmd::Tool {
                        name,
                        args,
                        workspace,
                    } => {
                        debug::run_debug_tool(
                            &config_path,
                            &name,
                            args.as_deref(),
                            workspace,
                            json_out,
                        )
                        .await?
                    }
                    DebugCmd::Policy { command, mode } => {
                        debug::run_debug_policy(&config_path, &command, mode.as_deref(), json_out)?
                    }
                    DebugCmd::Triage { prompt, mode } => {
                        debug::run_debug_triage(&config_path, &prompt, mode.as_deref(), json_out)?
                    }
                    DebugCmd::Prompt { workspace, full } => {
                        debug::run_debug_prompt(&config_path, workspace, full, json_out)?
                    }
                    DebugCmd::LogsDir => debug::run_debug_logs_dir(&config_path, json_out)?,
                    DebugCmd::Bundle { output, limit } => {
                        debug::run_debug_bundle(&config_path, output, limit, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Service { action } => {
                match action {
                    ServiceCmd::Unit => service::run_service_unit(&config_path, json_out)?,
                    ServiceCmd::Install { enable } => {
                        service::run_service_install(&config_path, enable, json_out)?
                    }
                    ServiceCmd::Status => service::run_service_status(json_out)?,
                }
                Ok(EXIT_OK)
            }
            Commands::Gateway { action } => {
                match action {
                    GatewayCmd::Pair { role } => {
                        commands::run_gateway_pair(&config_path, &role, json_out).await?
                    }
                    GatewayCmd::Peers => commands::run_gateway_peers(&config_path, json_out).await?,
                }
                Ok(EXIT_OK)
            }
            Commands::Doctor => {
                commands::run_doctor(&config_path, json_out).await?;
                Ok(EXIT_OK)
            }
            Commands::Mcp => {
                mcp::run_mcp_stdio(config_path).await?;
                Ok(EXIT_OK)
            }
            Commands::McpClient { action } => {
                match action {
                    McpClientCmd::List => {
                        commands::run_mcp_client_list(&config_path, json_out).await?
                    }
                    McpClientCmd::Add {
                        name,
                        transport,
                        command,
                        args,
                        url,
                        auth_secret,
                        env_secrets,
                        enabled_tools,
                        disabled_tools,
                        risk,
                        parallel,
                        allow_private_url,
                        sync,
                    } => {
                        commands::run_mcp_client_add(
                            &config_path,
                            commands::McpClientAddParams {
                                name,
                                transport,
                                command,
                                args,
                                url,
                                auth_secret,
                                env_secrets,
                                enabled_tools,
                                disabled_tools,
                                risk,
                                parallel,
                                allow_private_url,
                                sync,
                            },
                            json_out,
                        )
                        .await?
                    }
                    McpClientCmd::Remove { name } => {
                        commands::run_mcp_client_remove(&config_path, &name, json_out).await?
                    }
                    McpClientCmd::Toggle { name, enabled } => {
                        commands::run_mcp_client_toggle(&config_path, &name, enabled, json_out)
                            .await?
                    }
                    McpClientCmd::Sync { name } => {
                        commands::run_mcp_client_sync(&config_path, &name, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Lsp { action } => {
                match action {
                    LspCmd::Status => commands::run_lsp_status(&config_path, json_out).await?,
                    LspCmd::Stop => commands::run_lsp_stop(&config_path, json_out).await?,
                }
                Ok(EXIT_OK)
            }
            Commands::Skills { action } => {
                match action {
                    SkillsCmd::List => commands::run_skills_list(&config_path, json_out).await?,
                    SkillsCmd::Show { slug, file } => {
                        commands::run_skill_show(&config_path, &slug, file.as_deref(), json_out)
                            .await?
                    }
                    SkillsCmd::Create {
                        slug,
                        name,
                        description,
                        content,
                    } => {
                        commands::run_skill_create(
                            &config_path,
                            &slug,
                            name.as_deref(),
                            &description,
                            &content,
                            json_out,
                        )
                        .await?
                    }
                    SkillsCmd::Toggle { slug, enabled } => {
                        commands::run_skill_toggle(&config_path, &slug, enabled, json_out).await?
                    }
                    SkillsCmd::Approve { slug } => {
                        commands::run_skill_approve(&config_path, &slug, json_out).await?
                    }
                    SkillsCmd::History { slug } => {
                        commands::run_skill_history(&config_path, slug.as_deref(), json_out).await?
                    }
                    SkillsCmd::Rollback { snapshot_id } => {
                        commands::run_skill_rollback(&config_path, &snapshot_id, json_out).await?
                    }
                    SkillsCmd::Delete { slug } => {
                        commands::run_skill_delete(&config_path, &slug, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Goals { action } => {
                match action {
                    GoalsCmd::List { limit } => {
                        commands::run_goals_list(&config_path, limit, json_out).await?
                    }
                    GoalsCmd::Show { id } => {
                        commands::run_goal_show(&config_path, &id, json_out).await?
                    }
                    GoalsCmd::Create {
                        name,
                        goal,
                        verify_cmd,
                        workspace,
                        interval,
                        max_iterations,
                        max_budget,
                        stalls,
                        auto_rewind,
                        model,
                    } => {
                        commands::run_goal_create(
                            &config_path,
                            zenko_core::GoalLoopCreateRequest {
                                name,
                                goal_text: goal,
                                verify_cmd,
                                workspace_dir: workspace.map(|p| p.display().to_string()),
                                session_id: None,
                                interval_secs: interval,
                                max_iterations,
                                max_budget_usd: max_budget,
                                max_consecutive_stalls: stalls,
                                checkpoint_each_iter: Some(true),
                                auto_rewind_on_stall: Some(auto_rewind),
                                model,
                                notify_channel: None,
                            },
                            json_out,
                        )
                        .await?
                    }
                    GoalsCmd::Pause { id } => {
                        commands::run_goal_set_status(
                            &config_path,
                            &id,
                            zenko_core::GoalLoopStatus::Paused,
                            json_out,
                        )
                        .await?
                    }
                    GoalsCmd::Resume { id } => {
                        commands::run_goal_set_status(
                            &config_path,
                            &id,
                            zenko_core::GoalLoopStatus::Active,
                            json_out,
                        )
                        .await?
                    }
                    GoalsCmd::Run { id } => {
                        commands::run_goal_run_now(&config_path, &id, json_out).await?
                    }
                    GoalsCmd::Delete { id } => {
                        commands::run_goal_delete(&config_path, &id, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Triggers { action } => {
                match action {
                    TriggersCmd::List => {
                        commands::run_triggers_list(&config_path, json_out).await?
                    }
                }
                Ok(EXIT_OK)
            }
            Commands::Export {
                session_id,
                all,
                format,
                since,
                channel,
                verified_only,
                min_turns,
                include_thinking,
                anonymize_paths,
                all_branches,
                output,
            } => {
                commands::run_export(
                    &config_path,
                    commands::ExportCliParams {
                        session_id,
                        all,
                        format,
                        since,
                        channel,
                        verified_only,
                        min_turns,
                        include_thinking,
                        anonymize_paths,
                        all_branches,
                        output,
                    },
                    json_out,
                )
                .await?;
                Ok(EXIT_OK)
            }
        }
    });

    rt.shutdown_timeout(Duration::from_secs(DEFAULT_SHUTDOWN_GRACE_SECS));
    match exit_code {
        Ok(EXIT_OK) => Ok(()),
        Ok(code) => std::process::exit(code),
        Err(e) => Err(e),
    }
}

async fn run_web_daemon(config_path: PathBuf, port: Option<u16>, bind: Option<String>) -> Result<()> {
    let mut cfg = ZenkoConfig::load_or_default(&config_path)?;
    if let Some(p) = port {
        cfg.webui.port = p;
    }
    if let Some(b) = bind {
        cfg.webui.bind = b;
    }

    let (runtime, log_buf) = AgentRuntime::build(config_path, cfg.clone())?;

    let tracing_level = match cfg.general.log_level.as_str() {
        "debug" | "trace" => cfg.general.log_level.as_str(),
        _ => "warn",
    };
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{tracing_level},hyper=warn,reqwest=warn")));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_target(true).compact())
        .with(LogBroadcastLayer::new(log_buf))
        .init();

    if cfg.webui.require_auth && !runtime.vault.has(VAULT_KEY_WEBUI_ADMIN)? {
        let tok = SecretVault::generate_token(16);
        runtime.vault.set(VAULT_KEY_WEBUI_ADMIN, &tok)?;
        // Print generated admin token ONLY to operator stderr on first boot; never write plaintext
        // credentials into SQLite logs, JSONL files, in-memory ring buffers, or SSE broadcasts.
        eprintln!("  [vault] Initial WebUI admin token generated (saved to vault.enc): {tok}");
        let entry = emit_hono_line(
            LogDirection::Warn,
            LogChannel::System,
            "initial WebUI admin token generated and saved to vault.enc (printed once to stderr)",
            Some(("vault", LogStatusTone::Warn)),
            Some("saved"),
        );
        runtime.logs.push(entry);
    }

    let cancel = CancellationToken::new();
    let tg_reload = Arc::new(Notify::new());

    let act_rt = runtime.clone();
    let act_cancel = cancel.clone();
    tokio::spawn(async move {
        run_activity_logger(act_rt, act_cancel).await;
    });

    let cron_rt = runtime.clone();
    let cron_cancel = cancel.clone();
    tokio::spawn(async move {
        cron_rt.run_cron_scheduler(cron_cancel).await;
    });

    let wake_rt = runtime.clone();
    let wake_cancel = cancel.clone();
    tokio::spawn(async move {
        wake_rt.run_job_wakeup_loop(wake_cancel).await;
    });

    let dream_rt = runtime.clone();
    let dream_cancel = cancel.clone();
    tokio::spawn(async move {
        run_dream_worker(dream_rt, dream_cancel).await;
    });

    let goal_rt = runtime.clone();
    let goal_cancel = cancel.clone();
    tokio::spawn(async move {
        run_goal_loop_scheduler(goal_rt, goal_cancel).await;
    });

    let watch_rt = runtime.clone();
    let watch_cancel = cancel.clone();
    tokio::spawn(async move {
        run_workspace_watchers(watch_rt, watch_cancel).await;
    });

    let reaper_rt = runtime.clone();
    let reaper_cancel = cancel.clone();
    tokio::spawn(async move {
        reaper_rt.run_external_tool_reaper(reaper_cancel).await;
    });

    // Non-blocking startup sweep removing crash-orphaned git worktrees under ~/.zenko/worktrees/
    let prune_rt = runtime.clone();
    tokio::spawn(async move {
        if let Ok(rep) = prune_rt.subagents.prune_orphan_worktrees(&prune_rt).await {
            if rep.removed_count > 0 {
                let entry = emit_hono_line(
                    LogDirection::Work,
                    LogChannel::System,
                    &format!("pruned {} orphan subagent worktree(s)", rep.removed_count),
                    Some(("worktree", LogStatusTone::Ok)),
                    Some("boot"),
                );
                prune_rt.logs.push(entry);
            }
        }
    });

    let ipc_rt = runtime.clone();
    let ipc_cancel = cancel.clone();
    tokio::spawn(async move {
        ipc_rt.run_ipc_server(ipc_cancel).await;
    });

    let tg_gw = Arc::new(TelegramGateway::new(runtime.clone(), tg_reload.clone()));
    let tg_cancel = cancel.clone();
    tokio::spawn(async move {
        tg_gw.run(tg_cancel).await;
    });

    let sig_cancel = cancel.clone();
    let sig_runtime = runtime.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigterm = signal(SignalKind::terminate()).ok();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = async {
                    if let Some(ref mut s) = sigterm {
                        s.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        let _ = emit_hono_line(
            LogDirection::Warn,
            LogChannel::System,
            "shutdown signal received",
            Some(("stopping", LogStatusTone::Warn)),
            None,
        );
        sig_runtime.cancel_all_sessions();
        sig_cancel.cancel();

        if tokio::signal::ctrl_c().await.is_ok() {
            std::process::exit(130);
        }
    });

    let provider_desc = find_provider(&cfg.provider.kind);
    let rss_mb = zenko_runtime::current_rss_bytes() as f64 / (1024.0 * 1024.0);
    let stream_meta = match cfg.provider.enable_streaming {
        true => "stream on",
        false => "stream off",
    };
    let (tg_status, tg_tone, tg_meta) = match cfg.telegram.enabled {
        true => ("enabled", LogStatusTone::Info, "polling"),
        false => ("idle", LogStatusTone::Neutral, "run `zenko setup`"),
    };

    for entry in [
        emit_hono_line(
            LogDirection::Out,
            LogChannel::System,
            &format!("webui    http://{}:{}", cfg.webui.bind, cfg.webui.port),
            Some(("ready", LogStatusTone::Ok)),
            Some(&format!("{rss_mb:.1} MB RSS")),
        ),
        emit_hono_line(
            LogDirection::Out,
            LogChannel::System,
            &format!(
                "provider {} · {}",
                provider_desc.label,
                match cfg.provider.lead_model.is_empty() {
                    true => "auto",
                    false => cfg.provider.lead_model.as_str(),
                }
            ),
            Some(("ready", LogStatusTone::Ok)),
            Some(stream_meta),
        ),
        emit_hono_line(
            LogDirection::Out,
            LogChannel::System,
            "telegram gateway",
            Some((tg_status, tg_tone)),
            Some(tg_meta),
        ),
    ] {
        runtime.logs.push(entry);
    }

    let flush_logs = runtime.logs.clone();
    let stop_jobs = runtime.jobs.clone();
    let stop_runtime = runtime.clone();
    let serve_res = match cfg.webui.enabled {
        true => zenko_webui::serve(runtime, tg_reload, cancel.clone()).await,
        false => {
            cancel.cancelled().await;
            Ok(())
        }
    };

    // Always cancel background tasks (cron/telegram/goal loops/IPC) and flush state even if webui bind failed
    cancel.cancel();
    stop_runtime.subagents.kill_all(&stop_runtime).await;
    stop_runtime.shutdown_external_tools().await;
    stop_jobs.stop_all().await;
    let stopped_entry = emit_hono_line(
        LogDirection::Out,
        LogChannel::System,
        "zenko web",
        Some(("stopped", LogStatusTone::Neutral)),
        None,
    );
    flush_logs.push(stopped_entry);
    flush_logs.flush();
    serve_res?;
    Ok(())
}
