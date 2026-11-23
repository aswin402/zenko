use clap::{Parser, Subcommand};
use std::path::PathBuf;
use zenko_core::ATTACH_DEFAULT_REPLAY_EVENTS;

#[derive(Parser)]
#[command(
    name = "zenko",
    version,
    about = "Zenko — 100% Native Rust 24/7 Autonomous AI Agent for Humans & AI Agents (Terminal CLI, MCP, WebUI, Telegram)"
)]
pub(crate) struct Cli {
    /// Path to zenko.toml config file
    #[arg(long, global = true)]
    pub(crate) config: Option<PathBuf>,

    /// Emit machine-readable JSON output (ideal for AI agents & scripts)
    #[arg(long, global = true)]
    pub(crate) json: bool,

    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
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
pub(crate) enum SessionsCmd {
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
pub(crate) enum ApprovalsCmd {
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
pub(crate) enum MemoryCmd {
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
pub(crate) enum CronCmd {
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
pub(crate) enum JobsCmd {
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
pub(crate) enum AgentsCmd {
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
pub(crate) enum HubCmd {
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
pub(crate) enum QueueCmd {
    /// List durable queued follow-up prompts for a session
    List { session_id: String },
    /// Cancel a queued follow-up prompt by ID
    Cancel { session_id: String, queue_id: String },
}

#[derive(Subcommand)]
pub(crate) enum ClarifyCmd {
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
pub(crate) enum CheckpointsCmd {
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
pub(crate) enum DagCmd {
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
pub(crate) enum PlanCmd {
    /// Approve a session's read-only plan and promote its permission mode (`autonomous` | `balanced` | `strict`)
    Approve {
        session_id: String,
        #[arg(long, default_value = "autonomous")]
        mode: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum GatewayCmd {
    /// Generate a 6-character one-time pairing code for Telegram
    Pair {
        #[arg(long, default_value = "owner")]
        role: String,
    },
    /// List paired Telegram peers
    Peers,
}

#[derive(Subcommand)]
pub(crate) enum DebugCmd {
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
pub(crate) enum ServiceCmd {
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
pub(crate) enum McpClientCmd {
    /// List configured external MCP client servers and their discovered deferred tools
    List,
    /// Add or update an external MCP server (`stdio` or Streamable HTTP `2025-03-26`)
    Add(Box<McpClientAddArgs>),
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
pub(crate) enum LspCmd {
    /// Show configured LSP languages and currently pooled active language server processes
    Status,
    /// Cleanly stop all currently pooled LSP language server processes
    Stop,
}

#[derive(Subcommand)]
pub(crate) enum SkillsCmd {
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
pub(crate) enum GoalsCmd {
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
pub(crate) enum TriggersCmd {
    /// List configured webhook routing rules and workspace watchers with live rate-limit telemetry
    List,
}


#[derive(clap::Args)]
pub(crate) struct McpClientAddArgs {
    /// Unique server name (tools become `mcp__<name>__<tool>`)
    #[arg(long)]
    pub(crate) name: String,
    /// Transport (`stdio` | `http`)
    #[arg(long, default_value = "stdio")]
    pub(crate) transport: String,
    /// Executable command (for `stdio` transport)
    #[arg(long, default_value = "")]
    pub(crate) command: String,
    /// Command argument (repeatable, for `stdio`)
    #[arg(long = "arg")]
    pub(crate) args: Vec<String>,
    /// Streamable HTTP endpoint URL (for `http` transport)
    #[arg(long, default_value = "")]
    pub(crate) url: String,
    /// SecretVault key name used as `Authorization: Bearer <secret>` (for `http`)
    #[arg(long)]
    pub(crate) auth_secret: Option<String>,
    /// Environment secret mapping `ENV_VAR=VAULT_KEY` (repeatable, for `stdio`)
    #[arg(long = "env-secret")]
    pub(crate) env_secrets: Vec<String>,
    /// Optional remote tool allowlist (repeatable)
    #[arg(long = "allow-tool")]
    pub(crate) enabled_tools: Vec<String>,
    /// Optional remote tool denylist overriding allowlist (repeatable)
    #[arg(long = "deny-tool")]
    pub(crate) disabled_tools: Vec<String>,
    /// Optional risk override (`safe` | `moderate` | `dangerous`)
    #[arg(long)]
    pub(crate) risk: Option<String>,
    /// Allow concurrent parallel calls to this MCP server
    #[arg(long)]
    pub(crate) parallel: bool,
    /// Permit private/loopback URLs for local HTTP MCP servers
    #[arg(long)]
    pub(crate) allow_private_url: bool,
    /// Immediately connect and sync `tools/list` after saving
    #[arg(long)]
    pub(crate) sync: bool,
}
