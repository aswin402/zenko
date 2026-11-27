use crate::config::PermissionMode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub fn new_id() -> String {
    Ulid::new().to_string()
}

/// Returns `len` characters drawn exclusively from the 80-bit random tail of a ULID
/// (chars `10..26`), preventing timestamp-prefix collisions within the same second/millisecond.
pub fn new_random_suffix(len: usize) -> String {
    let id = new_id();
    let take = len.clamp(1, 16);
    id[id.len().saturating_sub(take)..].to_string()
}

/// UTF-8 safe byte-bounded prefix slice (never panics on multi-byte character boundaries).
pub fn truncate_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    &s[..s.floor_char_boundary(max_bytes.min(s.len()))]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Running,
    WaitingApproval,
    Paused,
    Error,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::Paused => "paused",
            Self::Error => "error",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "waiting_approval" => Self::WaitingApproval,
            "paused" => Self::Paused,
            "error" => Self::Error,
            _ => Self::Idle,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub external_chat_id: Option<String>,
    pub external_thread_id: Option<String>,
    pub model_override: Option<String>,
    pub permission_mode: PermissionMode,
    pub status: SessionStatus,
    pub head_event_id: Option<String>,
    pub frozen_system_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_role: Option<String>,
    #[serde(default)]
    pub agent_depth: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    /// Number of direct child subagents spawned by this session (computed via SQL aggregate).
    #[serde(default)]
    pub subagent_count: usize,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRole {
    System,
    User,
    Assistant,
    Tool,
}

impl EventRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "system" => Self::System,
            "user" => Self::User,
            "tool" => Self::Tool,
            _ => Self::Assistant,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Message,
    Thinking,
    ToolCall,
    ToolResult,
    CompactionCheckpoint,
    SystemNote,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Thinking => "thinking",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::CompactionCheckpoint => "compaction_checkpoint",
            Self::SystemNote => "system_note",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "thinking" => Self::Thinking,
            "tool_call" => Self::ToolCall,
            "tool_result" => Self::ToolResult,
            "compaction_checkpoint" => Self::CompactionCheckpoint,
            "system_note" => Self::SystemNote,
            _ => Self::Message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub branch_index: i64,
    pub role: EventRole,
    pub kind: EventKind,
    pub content_json: serde_json::Value,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: EventRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Safe,
    Moderate,
    Dangerous,
}

impl RiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Moderate => "moderate",
            Self::Dangerous => "dangerous",
        }
    }
}

/// Declarative tool schema loading strategy for prompt-cache efficiency.
/// `Eager` tools are always sent in `tools[]`; `Deferred` tools appear as a compact 1-line
/// catalog in Tier 2 and load full JSON schemas dynamically via `tool_search` (or direct invocation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposure {
    #[default]
    Eager,
    Deferred,
}

/// Declarative domain grouping for all Zenko tools (Hermes-inspired ToolSets, pure Rust zero hardcoding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolSet {
    #[default]
    CoreFs,
    CodeIntel,
    Execution,
    Git,
    Web,
    Memory,
    Orchestration,
    Subagents,
    Automation,
    Mcp,
}

impl ToolSet {
    pub const ALL: &'static [Self] = &[
        Self::CoreFs,
        Self::CodeIntel,
        Self::Execution,
        Self::Git,
        Self::Web,
        Self::Memory,
        Self::Orchestration,
        Self::Subagents,
        Self::Automation,
        Self::Mcp,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CoreFs => "core_fs",
            Self::CodeIntel => "code_intel",
            Self::Execution => "execution",
            Self::Git => "git",
            Self::Web => "web",
            Self::Memory => "memory",
            Self::Orchestration => "orchestration",
            Self::Subagents => "subagents",
            Self::Automation => "automation",
            Self::Mcp => "mcp",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "core_fs" | "fs" | "files" | "file" | "core" => Some(Self::CoreFs),
            "code_intel" | "code" | "lsp" | "search" | "intel" => Some(Self::CodeIntel),
            "execution" | "exec" | "shell" | "jobs" | "tasks" => Some(Self::Execution),
            "git" | "vcs" => Some(Self::Git),
            "web" | "http" | "browser" | "internet" => Some(Self::Web),
            "memory" | "mem" | "skills" | "context" | "sessions" => Some(Self::Memory),
            "orchestration" | "orch" | "plan" | "brainstorm" => Some(Self::Orchestration),
            "subagents" | "subagent" | "agents" | "agent" => Some(Self::Subagents),
            "automation" | "cron" | "goals" | "goal" | "schedule" => Some(Self::Automation),
            "mcp" | "external" => Some(Self::Mcp),
            _ => None,
        }
    }
}

/// Live environmental context evaluated by each tool's `is_available()` predicate
/// so unavailable tools never pollute prompts or catalogs.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolAvailabilityContext {
    pub has_mcp_servers: bool,
    pub has_skills: bool,
    pub is_subagent: bool,
    pub read_only: bool,
}

/// Compact summary of one permitted `ToolSet` returned by `tool_search(action="list_toolsets")`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSetSummary {
    pub id: String,
    pub label: String,
    pub description: String,
    pub eager_tools: Vec<String>,
    pub deferred_tools: Vec<String>,
}

/// Declarative single step inside a `batch_tools` composite/dataflow invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchToolCallItem {
    pub tool: String,
    #[serde(default)]
    pub args: serde_json::Value,
    /// Optional variable name capturing this step's (optionally extracted) output for downstream `${var}` templates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_as: Option<String>,
    /// When true, suppresses returning raw step output to LLM context (keeps intermediate data off-context like RLM/PTC).
    #[serde(default)]
    pub quiet: bool,
    /// Optional deterministic pure-Rust output projection (`raw`, `lines`, `first_line`, `paths`, `contains:<text>`, `head:<n>`, `tail:<n>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<String>,
}

/// Request payload forwarded from `batch_tools` into `ToolBatchBridge`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchToolRequest {
    pub calls: Vec<BatchToolCallItem>,
    #[serde(default)]
    pub stop_on_error: bool,
    pub max_chars_per_call: usize,
}

/// Per-step result emitted by `ToolBatchBridge::execute_batch`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchToolStepOutcome {
    pub index: usize,
    pub tool: String,
    pub is_error: bool,
    pub skipped: bool,
    #[serde(default)]
    pub quiet: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_var: Option<String>,
    pub duration_ms: u64,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output_preview: Option<String>,
    #[serde(default)]
    pub read_less_tokens_saved: u64,
    #[serde(default)]
    pub output_dedup_tokens_saved: u64,
    #[serde(default)]
    pub shunt_tokens_saved: u64,
    #[serde(default)]
    pub ccr_recovery: bool,
}

/// Declarative PTY terminal action for persistent `shell_exec(session=...)` sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PtyAction {
    #[default]
    Exec,
    Send,
    Close,
    List,
}

impl PtyAction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "exec" | "run" => Some(Self::Exec),
            "send" | "stdin" | "input" | "write" => Some(Self::Send),
            "close" | "stop" | "kill" => Some(Self::Close),
            "list" | "status" => Some(Self::List),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exec => "exec",
            Self::Send => "send",
            Self::Close => "close",
            Self::List => "list",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub risk: RiskLevel,
}

/// Metadata returned when an oversized tool output is spilled atomically to `~/.zenko/artifacts/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSpillRecord {
    pub handle: String,
    pub abs_path: String,
    pub total_bytes: usize,
    pub total_lines: usize,
    pub preview: String,
}

/// Deterministic cumulative file-operation & artifact ledger carried across Stage-B compactions
/// (Oh My Pi pattern: ensures the agent never forgets edited, created, deleted, or read files).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionFileLedger {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_read: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_modified: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_created: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_deleted: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
}

impl CompactionFileLedger {
    pub fn is_empty(&self) -> bool {
        self.files_read.is_empty()
            && self.files_modified.is_empty()
            && self.files_created.is_empty()
            && self.files_deleted.is_empty()
            && self.artifacts.is_empty()
    }
}

/// Durable session-scoped scratchpad variable surviving Stage-B compactions (Prime Agent pattern).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionVarEntry {
    pub session_id: String,
    pub key: String,
    pub summary: String,
    pub value: String,
    pub byte_len: usize,
    pub updated_at: DateTime<Utc>,
}

/// Persisted `EventKind::CompactionCheckpoint` payload stored in SQLite for zero-loss,
/// cache-friendly long-horizon session continuity (Oh My Pi + Codex + Hermes synthesis).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionCheckpointPayload {
    pub version: u32,
    pub summary: String,
    pub first_kept_event_id: String,
    pub compacted_events_count: usize,
    pub estimated_tokens_before: usize,
    pub estimated_tokens_after: usize,
    pub model: String,
    pub created_at: DateTime<Utc>,
    /// Hermes-style verbatim Turn-1 user goal anchor preserved across repeated compactions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_goal: Option<String>,
    /// Oh My Pi-style deterministic cumulative file-operation & artifact ledger.
    #[serde(default)]
    pub file_ledger: CompactionFileLedger,
}

/// Pure-English output conciseness tier (`off`, `lite`, `full`, `ultra`).
/// Inspired by ASD-STE100 Simplified Technical English with strict Verbatim Payload Protection
/// and Auto-Clarity safety hatch (zero non-English variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConciseMode {
    Off,
    #[default]
    Lite,
    Full,
    Ultra,
}

impl ConciseMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Lite => "lite",
            Self::Full => "full",
            Self::Ultra => "ultra",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "normal" | "standard" | "disabled" => Some(Self::Off),
            "lite" | "light" | "crisp" => Some(Self::Lite),
            "full" | "concise" | "terse" | "caveman" => Some(Self::Full),
            "ultra" | "max" | "extreme" | "ultracave" => Some(Self::Ultra),
            _ => None,
        }
    }

    pub fn from_str_loose(raw: &str) -> Self {
        Self::parse(raw).unwrap_or_default()
    }

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Declarative action enum for reusable Landlock-sandboxed `script_block` automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptBlockAction {
    List,
    Save,
    Read,
    Run,
    Delete,
}

impl ScriptBlockAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Save => "save",
            Self::Read => "read",
            Self::Run => "run",
            Self::Delete => "delete",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "list" | "ls" => Some(Self::List),
            "save" | "create" | "upsert" | "write" => Some(Self::Save),
            "read" | "get" | "show" | "view" => Some(Self::Read),
            "run" | "exec" | "execute" => Some(Self::Run),
            "delete" | "remove" | "rm" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn is_mutating(self) -> bool {
        matches!(self, Self::Save | Self::Run | Self::Delete)
    }
}

/// Deterministic summary returned by pure-Rust Markdown memory document compression.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocCompressReport {
    pub doc_name: String,
    pub mode: ConciseMode,
    pub original_bytes: usize,
    pub compressed_bytes: usize,
    pub bytes_saved: usize,
    pub tokens_saved: usize,
    pub savings_pct: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_snapshot_id: Option<String>,
    pub dry_run: bool,
    pub headings_preserved: usize,
    pub code_blocks_preserved: usize,
    pub preview: String,
}

/// Live token-budget & KV-cache breakdown for `context_inspect`, CLI `zenko context`, and WebUI drawer.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContextBreakdown {
    pub session_id: String,
    pub model: String,
    pub context_window: u64,
    pub total_tokens: usize,
    pub utilization_pct: f64,
    pub tier1_tokens: usize,
    pub tier1_hash: String,
    pub tier2_tokens: usize,
    pub repo_rules_loaded: Vec<String>,
    pub tool_schema_tokens: usize,
    pub eager_tools_count: usize,
    pub deferred_active_count: usize,
    pub deferred_available_count: usize,
    pub history_tokens: usize,
    pub microcompacted_tools_count: usize,
    pub microcompacted_tokens_saved: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_checkpoint_id: Option<String>,
    pub checkpoints_count: usize,
    pub spilled_artifacts_count: usize,
    pub cache_hit_pct: f64,
    #[serde(default)]
    pub concise_mode: ConciseMode,
    #[serde(default)]
    pub auto_clarity: bool,
    #[serde(default)]
    pub read_less_enabled: bool,
    #[serde(default)]
    pub read_less_calls_compressed: usize,
    #[serde(default)]
    pub read_less_bytes_saved: u64,
    #[serde(default)]
    pub read_less_tokens_saved: usize,
    #[serde(default)]
    pub economy_net_tokens_saved: i64,
    #[serde(default)]
    pub economy_active_micro_skills: usize,
    #[serde(default)]
    pub economy_pruned_tools_count: usize,
    #[serde(default)]
    pub economy_p90_turn_calls: usize,
    #[serde(default)]
    pub minimalism_mode: CodeMinimalismMode,
    #[serde(default)]
    pub economy_lines_added: u64,
    #[serde(default)]
    pub economy_lines_deleted: u64,
    #[serde(default)]
    pub zmode: ZMode,
    #[serde(default)]
    pub zmode_customized: bool,
    #[serde(default)]
    pub cache_stability_warnings: Vec<String>,
    #[serde(default)]
    pub scoped_rules_loaded: Vec<String>,
    #[serde(default)]
    pub nested_repo_docs_count: usize,
    #[serde(default)]
    pub project_brain_pages_count: usize,
    #[serde(default)]
    pub okf_concepts_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_cache_key: Option<String>,
    #[serde(default)]
    pub prefix_drift_reason: PrefixDriftReason,
    #[serde(default)]
    pub prefix_hit_ratio_pct: u8,
    #[serde(default)]
    pub calibrated_bytes_per_token: f64,
    #[serde(default)]
    pub session_vars_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            diff: None,
            metadata: None,
        }
    }

    pub fn ok_with_diff(content: impl Into<String>, diff: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            diff: Some(diff.into()),
            metadata: None,
        }
    }

    pub fn err(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
            diff: None,
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageStats {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_price_per_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_price_per_m: Option<f64>,
    #[serde(default)]
    pub is_free: bool,
    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_reasoning: bool,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Low,
    #[default]
    Medium,
    High,
}

pub const REASONING_EFFORTS: &[&str] = &["low", "medium", "high"];

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" | "minimal" => Some(Self::Low),
            "medium" | "balanced" | "standard" => Some(Self::Medium),
            "high" | "deep" | "max" => Some(Self::High),
            _ => None,
        }
    }

    pub fn from_tier(tier: ExecutionTier) -> Self {
        match tier {
            ExecutionTier::Instant => Self::Low,
            ExecutionTier::Standard => Self::Medium,
            ExecutionTier::Orchestrated => Self::High,
        }
    }

    pub fn escalated(self) -> Self {
        match self {
            Self::Low => Self::Medium,
            Self::Medium | Self::High => Self::High,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub session_id: String,
    pub model: String,
    pub fallbacks: Vec<String>,
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub temperature: f32,
    pub max_tokens: u32,
    #[serde(default = "default_req_true")]
    pub enable_streaming: bool,
    pub enable_prompt_caching: bool,
    pub sticky_session_routing: bool,
    pub provider_sort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Deterministic LMCache-style prefix anchor key for sticky KV-cache routing on
    /// OpenAI / vLLM / SGLang + LMCache backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

fn default_req_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LlmDelta {
    Thinking(String),
    Text(String),
    ToolCallComplete(ToolCall),
    Usage(UsageStats),
    Done { finish_reason: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub id: String,
    pub channel: String,
    pub peer_id: String,
    pub username: Option<String>,
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub session_id: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub text: String,
    pub approval: Option<ApprovalRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    AlwaysAllowed,
    Expired,
}

impl ApprovalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::AlwaysAllowed => "always_allowed",
            Self::Expired => "expired",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            "always_allowed" => Self::AlwaysAllowed,
            "expired" => Self::Expired,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Deny,
    AlwaysAllow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub id: String,
    pub session_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub args_preview: String,
    pub risk: RiskLevel,
    pub status: ApprovalStatus,
    pub resolved_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerRole {
    Owner,
    Trusted,
    ReadOnly,
}

impl PeerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Trusted => "trusted",
            Self::ReadOnly => "readonly",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "owner" => Self::Owner,
            "trusted" => Self::Trusted,
            _ => Self::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayPeer {
    pub channel: String,
    pub peer_id: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub role: PeerRole,
    pub paired_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingCode {
    pub code: String,
    pub channel: String,
    pub role: PeerRole,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

/// Typed memory taxonomy (Claude Code + Hermes synthesis) with declarative temporal decay half-lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    User,
    Feedback,
    Project,
    Reference,
    #[default]
    Fact,
    Lesson,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
            Self::Fact => "fact",
            Self::Lesson => "lesson",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "user" | "preference" | "preferences" => Self::User,
            "feedback" | "rule" | "correction" => Self::Feedback,
            "project" | "architecture" | "decision" => Self::Project,
            "reference" | "pointer" | "link" => Self::Reference,
            "lesson" | "episode" | "bug" => Self::Lesson,
            _ => Self::Fact,
        }
    }

    /// Declarative Ebbinghaus decay half-life in days (user preferences & feedback decay slowest).
    pub fn half_life_days(self) -> f64 {
        match self {
            Self::User | Self::Feedback => 120.0,
            Self::Reference => 60.0,
            Self::Lesson => 45.0,
            Self::Project | Self::Fact => 30.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MemoryKindMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub half_life_days: u32,
}

/// Declarative single-source-of-truth memory taxonomy shared by Tools, Dream, CLI, API & WebUI.
pub const MEMORY_KINDS: &[MemoryKindMeta] = &[
    MemoryKindMeta {
        id: "project",
        label: "Project",
        description: "Architecture decisions, workspace invariants, and design constraints",
        half_life_days: 30,
    },
    MemoryKindMeta {
        id: "fact",
        label: "Fact",
        description: "Verified technical facts, environment quirks, and build/test notes",
        half_life_days: 30,
    },
    MemoryKindMeta {
        id: "user",
        label: "User",
        description: "Developer preferences, workflow habits, and communication style",
        half_life_days: 120,
    },
    MemoryKindMeta {
        id: "feedback",
        label: "Feedback",
        description: "Explicit corrections, banned patterns, and quality rules",
        half_life_days: 120,
    },
    MemoryKindMeta {
        id: "lesson",
        label: "Lesson",
        description: "Bug root causes, post-mortems, and debugging takeaways",
        half_life_days: 45,
    },
    MemoryKindMeta {
        id: "reference",
        label: "Reference",
        description: "External links, endpoints, dashboards, and service pointers",
        half_life_days: 60,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub scope: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub salience: f64,
    #[serde(default)]
    pub access_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_accessed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub content_simhash: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<String>,
    #[serde(default)]
    pub archived: bool,
    pub created_at: DateTime<Utc>,
}

/// Scored recall hit from Zenko's pure-Rust RRF hybrid memory search with live path staleness flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredMemoryHit {
    pub entry: MemoryEntry,
    pub score: f64,
    #[serde(default)]
    pub stale_paths: Vec<String>,
}

/// Outcome of `upsert_memory_dedup` indicating whether an entry was newly inserted or merged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUpsertResult {
    pub entry: MemoryEntry,
    pub merged: bool,
    pub similarity: f64,
}

/// Summary of a zero-LLM Stage-0 memory hygiene sweep.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryHygieneReport {
    pub duplicates_merged: usize,
    pub entries_archived: usize,
    pub stale_flagged: usize,
}

/// Versioned snapshot stored under `~/.zenko/memories/.history/` for 1-click rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDocSnapshot {
    pub id: String,
    pub doc_name: String,
    pub reason: String,
    pub bytes: u64,
    pub created_at: DateTime<Utc>,
}

/// Filter parameters for cross-session FTS5 `session_search`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionSearchFilter {
    pub query: String,
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub workspace_dir: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub exclude_session_id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Cross-session FTS5 search hit grouped with session context and matched BM25 snippet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSearchHit {
    pub session_id: String,
    pub session_title: String,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    pub event_id: String,
    pub role: String,
    pub snippet: String,
    pub rank: f64,
    pub created_at: DateTime<Utc>,
}

/// Surrounding turn returned by Hermes-style `session_search(around_event_id)` lineage expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContextTurn {
    pub event_id: String,
    pub role: String,
    pub kind: String,
    pub text: String,
    pub is_anchor: bool,
    pub created_at: DateTime<Utc>,
}

/// Window of ±N turns centered on a matched `session_search` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContextWindow {
    pub session_id: String,
    pub session_title: String,
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    pub anchor_event_id: String,
    pub events: Vec<SessionContextTurn>,
}

/// Cached Stage-1 rollout digest per session (Codex `codex-rs` pattern).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRolloutSummary {
    pub session_id: String,
    pub scope: String,
    pub digest: String,
    pub session_updated_at: String,
    pub created_at: DateTime<Utc>,
}

/// Persisted audit record for every autonomous or triggered Dream consolidation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamRunRecord {
    pub id: String,
    pub trigger: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub sessions_scanned: usize,
    pub memories_added: usize,
    pub memories_merged: usize,
    pub memories_archived: usize,
    pub docs_patched: usize,
    #[serde(default)]
    pub skills_proposed: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

/// Live Dream subsystem status for CLI, API, and WebUI Memory view.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DreamStatus {
    pub enabled: bool,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark_ts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_ts: Option<String>,
    pub pending_sessions: usize,
    pub recent_runs: Vec<DreamRunRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CronKind {
    Cron,
    Heartbeat,
    /// Hermes-inspired zero-LLM scheduled script running directly from `~/zenko_hub/scripts/`.
    Script,
}

impl CronKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cron => "cron",
            Self::Heartbeat => "heartbeat",
            Self::Script => "script",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "heartbeat" => Self::Heartbeat,
            "script" | "no_agent" => Self::Script,
            _ => Self::Cron,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub cron_expr: String,
    pub kind: CronKind,
    pub prompt: String,
    /// Relative script filename/subpath inside `~/zenko_hub/scripts/` when `kind == CronKind::Script`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_path: Option<String>,
    pub target_channel: String,
    pub target_chat_id: Option<String>,
    pub target_thread_id: Option<String>,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
}

/// Unified typed category for every asynchronous task managed by ZenkoTasks.
/// Each category stores its streaming logs cleanly under `~/.zenko/tasks/<kind>/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Shell,
    Build,
    Dev,
    Debug,
    Script,
    Cron,
    Browser,
    Subagent,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TaskKindMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub subdir: &'static str,
    pub description: &'static str,
}

pub const TASK_KINDS: &[TaskKindMeta] = &[
    TaskKindMeta {
        id: "shell",
        label: "Shell",
        subdir: "shell",
        description: "Ad-hoc shell commands and one-off CLI operations",
    },
    TaskKindMeta {
        id: "build",
        label: "Build",
        subdir: "build",
        description: "Compilations, bundlers, linters, and test suites",
    },
    TaskKindMeta {
        id: "dev",
        label: "Dev",
        subdir: "dev",
        description: "Long-running dev servers, hot reloaders, and file watchers",
    },
    TaskKindMeta {
        id: "debug",
        label: "Debug",
        subdir: "debug",
        description: "Debuggers, profilers, traces, and diagnostics",
    },
    TaskKindMeta {
        id: "script",
        label: "Script",
        subdir: "script",
        description: "Reusable scripts executed from ~/zenko_hub/scripts/",
    },
    TaskKindMeta {
        id: "cron",
        label: "Cron",
        subdir: "cron",
        description: "24/7 scheduled cron jobs and zero-token script runs",
    },
    TaskKindMeta {
        id: "browser",
        label: "Browser",
        subdir: "browser",
        description: "Headless & CDP browser automation sessions",
    },
    TaskKindMeta {
        id: "subagent",
        label: "Subagent",
        subdir: "subagent",
        description: "Asynchronous child agents and isolated worktree workers",
    },
];

#[derive(Debug, Clone, Copy, Serialize)]
pub struct HubCategoryMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub subdir: &'static str,
    pub description: &'static str,
}

pub const HUB_CATEGORIES: &[HubCategoryMeta] = &[
    HubCategoryMeta {
        id: "projects",
        label: "Projects",
        subdir: crate::constants::HUB_SUBDIR_PROJECTS,
        description: "Standalone apps & repositories scaffolded by Zenko",
    },
    HubCategoryMeta {
        id: "scripts",
        label: "Scripts",
        subdir: crate::constants::HUB_SUBDIR_SCRIPTS,
        description: "Reusable automation & zero-token No-Agent cron scripts",
    },
    HubCategoryMeta {
        id: "media",
        label: "Media",
        subdir: crate::constants::HUB_SUBDIR_MEDIA,
        description: "Generated images, videos, and audio assets",
    },
    HubCategoryMeta {
        id: "artifacts",
        label: "Artifacts",
        subdir: crate::constants::HUB_SUBDIR_ARTIFACTS,
        description: "Reports, exports, and spilled large command outputs",
    },
];

/// Declarative token-prefix rules for deterministic, zero-regex `TaskKind` inference.
const TASK_KIND_PREFIX_RULES: &[(&[&str], TaskKind)] = &[
    (&["cargo", "build"], TaskKind::Build),
    (&["cargo", "check"], TaskKind::Build),
    (&["cargo", "test"], TaskKind::Build),
    (&["cargo", "clippy"], TaskKind::Build),
    (&["cargo", "watch"], TaskKind::Dev),
    (&["cargo", "run"], TaskKind::Dev),
    (&["npm", "run", "build"], TaskKind::Build),
    (&["npm", "run", "test"], TaskKind::Build),
    (&["npm", "test"], TaskKind::Build),
    (&["npm", "run", "dev"], TaskKind::Dev),
    (&["vite"], TaskKind::Dev),
    (&["next", "dev"], TaskKind::Dev),
    (&["next", "build"], TaskKind::Build),
    (&["pytest"], TaskKind::Build),
    (&["make"], TaskKind::Build),
    (&["cmake"], TaskKind::Build),
    (&["go", "build"], TaskKind::Build),
    (&["go", "test"], TaskKind::Build),
    (&["uvicorn"], TaskKind::Dev),
    (&["gdb"], TaskKind::Debug),
    (&["lldb"], TaskKind::Debug),
    (&["strace"], TaskKind::Debug),
    (&["perf"], TaskKind::Debug),
    (&["playwright"], TaskKind::Browser),
    (&["chromium"], TaskKind::Browser),
];

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Build => "build",
            Self::Dev => "dev",
            Self::Debug => "debug",
            Self::Script => "script",
            Self::Cron => "cron",
            Self::Browser => "browser",
            Self::Subagent => "subagent",
        }
    }

    pub fn subdir(self) -> &'static str {
        self.as_str()
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "build" | "compile" | "test" => Self::Build,
            "dev" | "server" | "watch" => Self::Dev,
            "debug" | "trace" | "profile" => Self::Debug,
            "script" | "scripts" => Self::Script,
            "cron" | "schedule" | "heartbeat" => Self::Cron,
            "browser" | "web" | "cdp" => Self::Browser,
            "subagent" | "agent" | "worker" => Self::Subagent,
            _ => Self::Shell,
        }
    }

    pub fn parse_filter(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        match trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
            true => None,
            false => Some(Self::from_str_loose(trimmed)),
        }
    }

    /// Deterministically infers `TaskKind` from leading command tokens without regex.
    pub fn infer_from_command(cmd: &str) -> Self {
        let lower = cmd.to_ascii_lowercase();
        let tokens: Vec<&str> = lower.split_whitespace().take(4).collect();
        if tokens.is_empty() {
            return Self::Shell;
        }
        let hub_script_prefix = format!("{}{}/", crate::constants::HUB_PATH_PREFIX, crate::constants::HUB_SUBDIR_SCRIPTS);
        let hub_script_segment = format!("/{}/", crate::constants::HUB_SUBDIR_SCRIPTS);
        if tokens
            .iter()
            .any(|t| t.contains(&hub_script_prefix) || (t.contains("zenko_hub") && t.contains(&hub_script_segment)))
        {
            return Self::Script;
        }
        for (prefix, kind) in TASK_KIND_PREFIX_RULES {
            if tokens.len() >= prefix.len()
                && tokens[..prefix.len()]
                    .iter()
                    .zip(*prefix)
                    .all(|(a, b)| *a == *b)
            {
                return *kind;
            }
        }
        Self::Shell
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubEntry {
    pub category: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
}

/// Typed resource lock identity used by the 3-state parallel tool wave scheduler.
/// Prevents string-prefix drift across tools while guaranteeing collision-free keying.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ResourceLockKey {
    File(String),
    Job(String),
    Agent(String),
    Task(String),
    Git,
    Mcp(String),
    Skill(String),
    Goal(String),
}

/// Three-state declarative resource-lock concurrency algebra:
/// - `Shared`: read-only operations; run concurrently with adjacent `Shared` calls.
/// - `Keyed(key)`: scoped mutation (e.g. canonical file path or job ID); calls with distinct keys
///   run concurrently while calls sharing the same key serialize deterministically.
/// - `Global`: cross-cutting mutation / shell / HITL approval barrier; runs strictly sequentially.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolConcurrency {
    Shared,
    Keyed(ResourceLockKey),
    #[default]
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QueuedPromptStatus {
    #[default]
    Queued,
    Dispatched,
    Cancelled,
}

impl QueuedPromptStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Dispatched => "dispatched",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dispatched" => Self::Dispatched,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedPrompt {
    pub id: String,
    pub session_id: String,
    pub text: String,
    pub status: QueuedPromptStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SendMode {
    Auto,
    Steer,
    #[default]
    Queue,
    Interrupt,
}

impl SendMode {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "steer" => Self::Steer,
            "interrupt" | "cancel" | "replace" => Self::Interrupt,
            _ => Self::Queue,
        }
    }
}

/// Pure deterministic runtime state snapshot driving `SendMode::Auto` routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionDispatchState {
    pub is_running: bool,
    pub active_jobs: usize,
    pub active_subagents: usize,
    pub is_monitoring_tool_active: bool,
}

/// Deterministic outcome of resolving how an incoming user message should be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoDispatchDecision {
    StartTurn,
    Steer,
    Queue,
}

/// Pure algebraic resolver (zero `if/else` chains):
/// - Idle session -> `StartTurn`
/// - Running + managing/monitoring background jobs or subagents -> `Steer`
/// - Running focused foreground work -> `Queue`
pub fn resolve_auto_send_mode(state: SessionDispatchState) -> AutoDispatchDecision {
    match (
        state.is_running,
        state.active_jobs > 0 || state.active_subagents > 0 || state.is_monitoring_tool_active,
    ) {
        (false, _) => AutoDispatchDecision::StartTurn,
        (true, true) => AutoDispatchDecision::Steer,
        (true, false) => AutoDispatchDecision::Queue,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundJobStatus {
    Running,
    Completed,
    Failed,
    Stopped,
    Interrupted,
}

impl BackgroundJobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "stopped" => Self::Stopped,
            _ => Self::Interrupted,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundJobRecord {
    pub id: String,
    pub session_id: String,
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub command: String,
    pub workspace_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub status: BackgroundJobStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub log_path: String,
    pub bytes_written: u64,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobReadChunk {
    pub job: BackgroundJobRecord,
    pub offset: u64,
    pub next_offset: u64,
    pub total_bytes: u64,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonTelemetry {
    pub pid: u32,
    pub uptime_secs: u64,
    pub rss_bytes: u64,
    pub db_size_bytes: u64,
    pub active_sessions: usize,
    #[serde(default)]
    pub active_jobs: usize,
    pub pending_approvals: usize,
    pub daily_cost_usd: f64,
    pub total_cost_usd: f64,
    pub cache_hit_pct: f64,
    pub telegram_connected: bool,
    pub telegram_bot_username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DailyActivityPoint {
    pub date: String,
    pub events: u64,
    pub turns: u64,
    pub tool_calls: u64,
    pub tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolUsageStat {
    pub name: String,
    pub calls: u64,
    pub errors: u64,
    pub avg_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelUsageStat {
    pub provider: String,
    pub model: String,
    pub calls: u64,
    pub tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageSlice {
    pub id: String,
    pub label: String,
    pub path: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardCounts {
    pub sessions: u64,
    pub messages: u64,
    pub tool_calls: u64,
    pub memories: u64,
    pub skills_enabled: u64,
    pub skills_total: u64,
    pub cron_jobs: u64,
    pub goal_loops: u64,
    pub background_jobs: u64,
    pub subagents: u64,
    pub checkpoints: u64,
    pub mcp_servers: u64,
    pub paired_peers: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardAnalytics {
    pub telemetry: DaemonTelemetry,
    pub vault: crate::vault::VaultStats,
    pub counts: DashboardCounts,
    pub total_storage_bytes: u64,
    pub storage: Vec<StorageSlice>,
    pub daily_activity: Vec<DailyActivityPoint>,
    pub top_tools: Vec<ToolUsageStat>,
    pub top_models: Vec<ModelUsageStat>,
    pub channels: std::collections::BTreeMap<String, u64>,
    pub streak_days: u32,
    pub active_days: u32,
    pub total_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    #[serde(default = "default_log_category")]
    pub category: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

fn default_log_category() -> String {
    "system".to_string()
}

impl LogEntry {
    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_turn(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub fn with_call(mut self, call_id: impl Into<String>) -> Self {
        self.call_id = Some(call_id.into());
        self
    }

    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogFilter {
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl LogFilter {
    /// Deterministic predicate matching a live `LogEntry` against this filter.
    pub fn matches(&self, entry: &LogEntry) -> bool {
        let cat_ok = self
            .category
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
            .is_none_or(|c| entry.category.eq_ignore_ascii_case(c));

        let chan_ok = self
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
            .is_none_or(|c| entry.target.eq_ignore_ascii_case(c));

        let level_ok = self
            .level
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"))
            .is_none_or(|lvl| entry.level.eq_ignore_ascii_case(lvl));

        let sess_ok = self
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none_or(|sid| entry.session_id.as_deref() == Some(sid));

        let search_ok = self
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none_or(|needle| {
                let n = needle.to_ascii_lowercase();
                entry.message.to_ascii_lowercase().contains(&n)
                    || entry.target.to_ascii_lowercase().contains(&n)
                    || entry
                        .payload
                        .as_ref()
                        .is_some_and(|p| p.to_string().to_ascii_lowercase().contains(&n))
            });

        cat_ok && chan_ok && level_ok && sess_ok && search_ok
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTier {
    Instant,
    #[default]
    Standard,
    Orchestrated,
}

impl ExecutionTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Instant => "instant",
            Self::Standard => "standard",
            Self::Orchestrated => "orchestrated",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "instant" | "fast" => Self::Instant,
            "orchestrated" | "deep" | "plan" => Self::Orchestrated,
            _ => Self::Standard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelRoutingRole {
    Fast,
    #[default]
    Lead,
    Plan,
    Advisor,
}

pub const MODEL_ROUTING_ROLES: &[&str] = &["fast", "lead", "plan", "advisor"];

impl ModelRoutingRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Lead => "lead",
            Self::Plan => "plan",
            Self::Advisor => "advisor",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "fast" | "quick" => Some(Self::Fast),
            "lead" | "default" | "standard" => Some(Self::Lead),
            "plan" | "architect" => Some(Self::Plan),
            "advisor" | "deep" | "critic" => Some(Self::Advisor),
            _ => None,
        }
    }
}

/// LLM-sovereign execution posture declared dynamically via `orchestrate_control` (zero `/commands`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TurnPosture {
    Direct,
    Research,
    Brainstorm,
    Plan,
    #[default]
    Implement,
    Verify,
    Review,
}

pub const TURN_POSTURES: &[&str] = &[
    "direct",
    "research",
    "brainstorm",
    "plan",
    "implement",
    "verify",
    "review",
];

impl TurnPosture {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Research => "research",
            Self::Brainstorm => "brainstorm",
            Self::Plan => "plan",
            Self::Implement => "implement",
            Self::Verify => "verify",
            Self::Review => "review",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "direct" => Some(Self::Direct),
            "research" | "explore" | "investigate" => Some(Self::Research),
            "brainstorm" | "ideate" | "design" => Some(Self::Brainstorm),
            "plan" => Some(Self::Plan),
            "implement" | "execute" | "build" => Some(Self::Implement),
            "verify" | "test" => Some(Self::Verify),
            "review" | "critique" | "audit" => Some(Self::Review),
            _ => None,
        }
    }
}

/// Live orchestration directive emitted by the LLM via `orchestrate_control`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrchestrateControlRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<TurnPosture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cognitive_tier: Option<ModelRoutingRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concise_mode: Option<ConciseMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimalism_mode: Option<CodeMinimalismMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zmode: Option<ZMode>,
    #[serde(default)]
    pub refresh_recon: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recon_query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// Structured acknowledgement returned by `orchestrate_control` to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrchestrateControlReceipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<TurnPosture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cognitive_tier: Option<ModelRoutingRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concise_mode: Option<ConciseMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimalism_mode: Option<CodeMinimalismMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zmode: Option<ZMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recon_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default)]
    pub model_locked_by_override: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OrchestratorPhase {
    #[default]
    Recon,
    Plan,
    Execute,
    Verify,
    Done,
}

impl OrchestratorPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recon => "recon",
            Self::Plan => "plan",
            Self::Execute => "execute",
            Self::Verify => "verify",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    Direct,
    Research,
    Advisory,
    #[default]
    Implement,
}

impl IntentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Research => "research",
            Self::Advisory => "advisory",
            Self::Implement => "implement",
        }
    }

    /// Returns true when the user explicitly asked for read-only analysis, investigation,
    /// or architectural comparison rather than workspace mutations.
    pub fn is_read_only(self) -> bool {
        matches!(self, Self::Research | Self::Advisory)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeliberationDepth {
    #[default]
    Standard,
    Deliberate,
}

impl DeliberationDepth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Deliberate => "deliberate",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriageReport {
    pub tier: ExecutionTier,
    #[serde(default)]
    pub intent: IntentKind,
    #[serde(default)]
    pub deliberation: DeliberationDepth,
    pub complexity_score: i32,
    pub ambiguity_score: i32,
    #[serde(default)]
    pub risk_score: i32,
    pub matched_signals: Vec<String>,
    pub recommended_model_role: ModelRoutingRole,
    #[serde(default)]
    pub recommended_effort: ReasoningEffort,
    pub needs_recon: bool,
    pub needs_brainstorm: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReconSummary {
    pub workspace_entries: Vec<String>,
    pub repo_markers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_brief: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub okf_summary: Option<String>,
    #[serde(default)]
    pub okf_hits: Vec<String>,
    pub memory_hits: Vec<String>,
    #[serde(default)]
    pub session_hits: Vec<String>,
    #[serde(default)]
    pub brain_hits: Vec<String>,
    pub active_tasks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub git_dirty_files: Vec<String>,
    #[serde(default)]
    pub mentioned_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_plan_summary: Option<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainstormOption {
    pub id: String,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub pros: Vec<String>,
    #[serde(default)]
    pub cons: Vec<String>,
    #[serde(default = "default_medium_tag")]
    pub blast_radius: String,
    #[serde(default = "default_high_tag")]
    pub reversibility: String,
    #[serde(default = "default_option_score")]
    pub safety_score: u8,
    #[serde(default = "default_option_score")]
    pub testability_score: u8,
    #[serde(default)]
    pub touched_files: Vec<String>,
    #[serde(default)]
    pub rollback_steps: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub recommended: bool,
}

fn default_medium_tag() -> String {
    "medium".to_string()
}

fn default_high_tag() -> String {
    "high".to_string()
}

fn default_option_score() -> u8 {
    crate::constants::ORCH_DEFAULT_OPTION_SCORE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Blocked,
}

impl PlanStepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "in_progress" | "active" | "running" => Self::InProgress,
            "completed" | "done" | "finished" => Self::Completed,
            "blocked" | "stuck" => Self::Blocked,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub status: PlanStepStatus,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub id: String,
    pub goal: String,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub rubric_warnings: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

/// Lifecycle stage for iterative, multi-round LLM-sovereign brainstorming (`exploring` -> `refined` -> `committed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrainstormStage {
    Exploring,
    Refined,
    #[default]
    Committed,
}

pub const BRAINSTORM_STAGES: &[&str] = &["exploring", "refined", "committed"];

impl BrainstormStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exploring => "exploring",
            Self::Refined => "refined",
            Self::Committed => "committed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "exploring" | "draft" | "open" => Some(Self::Exploring),
            "refined" | "converging" | "revised" => Some(Self::Refined),
            "committed" | "final" | "decided" => Some(Self::Committed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainstormSpec {
    pub id: String,
    #[serde(default)]
    pub stage: BrainstormStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_spec_id: Option<String>,
    pub problem: String,
    #[serde(default)]
    pub invariants: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub blind_spots: Vec<String>,
    #[serde(default)]
    pub decision_criteria: Vec<String>,
    #[serde(default)]
    pub options: Vec<BrainstormOption>,
    #[serde(default)]
    pub selected_option_id: String,
    #[serde(default)]
    pub selection_rationale: String,
    #[serde(default)]
    pub critique: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub pre_mortem: Vec<String>,
    #[serde(default)]
    pub verification_plan: Vec<String>,
    #[serde(default)]
    pub rubric_warnings: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClarificationStatus {
    #[default]
    Pending,
    Answered,
    AutoResolved,
    Cancelled,
}

impl ClarificationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Answered => "answered",
            Self::AutoResolved => "auto_resolved",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationRecord {
    pub id: String,
    pub session_id: String,
    pub question: String,
    #[serde(default)]
    pub context: String,
    pub options: Vec<ClarificationOption>,
    pub recommended_option_id: String,
    pub status: ClarificationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationAnswer {
    pub selected_option_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_answer: Option<String>,
    pub resolved_by: String,
    pub auto_resolved: bool,
}

// ─── Phase 5: Async Subagents, Git Worktree Isolation & Inter-Agent Mailbox ────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentWorkspaceMode {
    #[default]
    Inherit,
    Worktree,
}

impl SubagentWorkspaceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Worktree => "worktree",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "worktree" | "isolated" | "git_worktree" => Self::Worktree,
            _ => Self::Inherit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    #[default]
    Running,
    Completed,
    Failed,
    Killed,
    /// Marked on daemon startup when a prior process exited ungracefully mid-subagent.
    Interrupted,
}

impl SubagentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Killed => "killed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "completed" | "done" => Self::Completed,
            "failed" | "error" => Self::Failed,
            "killed" | "cancelled" | "stopped" => Self::Killed,
            "interrupted" => Self::Interrupted,
            _ => Self::Running,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// Typed live delivery disposition returned by `send_message` (zero raw magic strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDeliveryAction {
    Steered,
    Interrupted,
    Resumed,
    Queued,
}

impl AgentDeliveryAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steered => "steered",
            Self::Interrupted => "interrupted",
            Self::Resumed => "resumed",
            Self::Queued => "queued",
        }
    }
}

/// Typed action enum for the `agent_worktree` tool (single declarative source of truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeAction {
    Diff,
    Merge,
    Discard,
    Prune,
}

pub const WORKTREE_ACTIONS: &[&str] = &["diff", "merge", "discard", "prune"];

impl WorktreeAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "diff" => Some(Self::Diff),
            "merge" => Some(Self::Merge),
            "discard" => Some(Self::Discard),
            "prune" => Some(Self::Prune),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnRequest {
    pub parent_session_id: String,
    pub role: String,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default)]
    pub run_in_background: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_mode: Option<SubagentWorkspaceMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Optional bounded fork of the parent session's active branch (gated by `subagents.allow_fork_context`).
    #[serde(default)]
    pub fork_context: bool,
    /// When using `worktree` isolation, capture uncommitted parent edits via non-invasive `git stash create`.
    #[serde(default)]
    pub include_uncommitted: bool,
    /// Optional per-subagent wall-clock timeout ceiling in seconds (clamped to `[subagents].max_duration_secs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_secs: Option<u64>,
    /// Optional per-subagent USD spend ceiling (clamped to `[subagents].max_cost_usd`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    /// Optional LLM-authored specialist instructions/persona (used with `role: "custom"` or to specialize any role).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_directive: Option<String>,
    /// Optional declarative cognitive tier (`fast` | `lead` | `plan` | `advisor`) routed dynamically to configured slots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cognitive_tier: Option<ModelRoutingRole>,
    /// Optional explicit read-only clamp (strictly enforced alongside parent's monotonic permission ceiling).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// Optional narrowed allowlist of tool names for surgical subagent focus (intersected with role policy).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_subset: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentRecord {
    pub id: String,
    pub parent_session_id: String,
    pub role: String,
    pub depth: u8,
    pub goal: String,
    pub status: SubagentStatus,
    pub workspace_mode: SubagentWorkspaceMode,
    pub workspace_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_base_commit: Option<String>,
    #[serde(default)]
    pub worktree_dirty_baseline: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_stat: Option<String>,
    #[serde(default)]
    pub changed_files: Vec<String>,
    pub tokens_used: u64,
    pub cost_usd: f64,
    /// Hierarchical subtree totals (self + all recursive descendants).
    #[serde(default)]
    pub tree_tokens: u64,
    #[serde(default)]
    pub tree_cost_usd: f64,
    #[serde(default)]
    pub descendant_count: usize,
    #[serde(default)]
    pub resume_count: u32,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

fn default_agent_resume_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AgentMessageOptions {
    /// When true (default), messaging a completed/failed/killed child resumes its exact session & worktree.
    #[serde(default = "default_agent_resume_true")]
    pub resume: bool,
    /// When true and the target subagent is actively running, interrupts its current step and restarts with the new guidance.
    #[serde(default)]
    pub interrupt: bool,
}

impl Default for AgentMessageOptions {
    fn default() -> Self {
        Self {
            resume: true,
            interrupt: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMailMessage {
    pub id: String,
    pub from_session_id: String,
    pub to_session_id: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Live runtime delivery disposition (`steered` | `interrupted` | `resumed` | `queued`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_action: Option<AgentDeliveryAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreeDiffReport {
    pub agent_id: String,
    pub branch: String,
    pub worktree_path: String,
    pub repo_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default)]
    pub dirty_baseline: bool,
    pub has_changes: bool,
    pub changed_files: Vec<String>,
    pub diff_stat: String,
    pub patch_preview: String,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub cleaned_up: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreePruneReport {
    pub removed_dirs: Vec<String>,
    pub removed_count: usize,
}

// ─── Phase 6: Durable SQLite Task DAG, Read-Only Plan Mode & Workspace Checkpoints ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionTaskStatus {
    #[default]
    Pending,
    InProgress,
    Blocked,
    Done,
    Failed,
}

impl SessionTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "in_progress" | "active" | "running" | "started" => Self::InProgress,
            "blocked" => Self::Blocked,
            "done" | "completed" | "complete" => Self::Done,
            "failed" | "error" => Self::Failed,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }

    pub fn to_plan_status(self) -> PlanStepStatus {
        match self {
            Self::Pending => PlanStepStatus::Pending,
            Self::InProgress => PlanStepStatus::InProgress,
            Self::Done => PlanStepStatus::Completed,
            Self::Blocked | Self::Failed => PlanStepStatus::Blocked,
        }
    }

    pub fn from_plan_status(s: PlanStepStatus) -> Self {
        match s {
            PlanStepStatus::Pending => Self::Pending,
            PlanStepStatus::InProgress => Self::InProgress,
            PlanStepStatus::Completed => Self::Done,
            PlanStepStatus::Blocked => Self::Blocked,
        }
    }
}

pub const SESSION_TASK_STATUSES: &[&str] = &[
    "pending",
    "in_progress",
    "blocked",
    "done",
    "failed",
];

pub fn new_session_task_id() -> String {
    format!("{}{}", crate::constants::SESSION_TASK_ID_PREFIX, new_random_suffix(8))
}

/// Declarative action set for the `session_task` tool (zero magic strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionTaskAction {
    Create,
    Update,
    Claim,
    List,
    Ready,
    Delete,
    Clear,
}

pub const SESSION_TASK_ACTIONS: &[&str] = &[
    "create",
    "update",
    "claim",
    "list",
    "ready",
    "delete",
    "clear",
];

impl SessionTaskAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "create" | "add" => Some(Self::Create),
            "update" | "edit" | "set" => Some(Self::Update),
            "claim" | "take" => Some(Self::Claim),
            "list" | "all" => Some(Self::List),
            "ready" | "next" => Some(Self::Ready),
            "delete" | "remove" => Some(Self::Delete),
            "clear" | "reset" => Some(Self::Clear),
            _ => None,
        }
    }
}

/// Durable SQLite-backed Task DAG node surviving compaction, restarts, and parallel subagent claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTask {
    pub id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_event_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_label: Option<String>,
    #[serde(default)]
    pub status: SessionTaskStatus,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub priority: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Partial update patch for `session_task(action="update")`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionTaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub active_label: Option<Option<String>>,
    pub status: Option<SessionTaskStatus>,
    pub depends_on: Option<Vec<String>>,
    pub add_depends_on: Vec<String>,
    pub owner_agent: Option<Option<String>>,
    pub verification_cmd: Option<Option<String>>,
    pub verification_evidence: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointBackend {
    #[default]
    ShadowGit,
    ContentBlobs,
}

impl CheckpointBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShadowGit => "shadow_git",
            Self::ContentBlobs => "content_blobs",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "content_blobs" | "blobs" => Self::ContentBlobs,
            _ => Self::ShadowGit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RewindMode {
    #[default]
    CodeAndConversation,
    CodeOnly,
    ConversationOnly,
}

pub const REWIND_MODES: &[&str] = &["code_and_conversation", "code_only", "conversation_only"];

impl RewindMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CodeAndConversation => "code_and_conversation",
            Self::CodeOnly => "code_only",
            Self::ConversationOnly => "conversation_only",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "code" | "code_only" | "files" => Self::CodeOnly,
            "conversation" | "conversation_only" | "chat" => Self::ConversationOnly,
            _ => Self::CodeAndConversation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCheckpoint {
    pub id: String,
    pub session_id: String,
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub workspace_dir: String,
    pub backend: CheckpointBackend,
    pub pre_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_ref: Option<String>,
    pub trigger_tool: String,
    pub label: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub is_safety_backup: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckpointDiffReport {
    pub checkpoint_id: String,
    pub session_id: String,
    pub workspace_dir: String,
    pub changed_files: Vec<String>,
    pub manual_edit_conflicts: Vec<String>,
    pub diff_stat: String,
    pub patch_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindRequest {
    pub checkpoint_id: String,
    #[serde(default)]
    pub mode: RewindMode,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub overwrite_manual_edits: bool,
    #[serde(default)]
    pub summarize_abandoned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindReport {
    pub checkpoint_id: String,
    pub session_id: String,
    pub mode: RewindMode,
    #[serde(default)]
    pub restored_files: Vec<String>,
    #[serde(default)]
    pub deleted_files: Vec<String>,
    #[serde(default)]
    pub preserved_manual_edits: Vec<String>,
    #[serde(default)]
    pub skipped_symlinks: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_checkpoint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_head_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewind_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckpointPruneReport {
    pub removed_checkpoints: usize,
    pub reclaimed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    SessionUpdated {
        session_id: String,
        status: SessionStatus,
        title: Option<String>,
    },
    UserMessageAccepted {
        session_id: String,
        event_id: String,
        text: String,
    },
    ThinkingDelta {
        session_id: String,
        turn_id: String,
        text: String,
    },
    TextDelta {
        session_id: String,
        turn_id: String,
        text: String,
    },
    ToolCallStarted {
        session_id: String,
        call_id: String,
        tool_name: String,
        args_preview: String,
        risk: RiskLevel,
    },
    ToolApprovalRequested {
        approval: ApprovalRecord,
    },
    ToolApprovalResolved {
        approval_id: String,
        session_id: String,
        decision: ApprovalDecision,
        resolved_by: String,
    },
    ToolCallFinished {
        session_id: String,
        call_id: String,
        tool_name: String,
        is_error: bool,
        duration_ms: u64,
        diff: Option<String>,
        output_preview: String,
    },
    TurnCompleted {
        session_id: String,
        turn_id: String,
        event_id: String,
        usage: UsageStats,
    },
    TurnError {
        session_id: String,
        message: String,
    },
    GatewayPaired {
        peer: GatewayPeer,
    },
    JobUpdated {
        job: BackgroundJobRecord,
    },
    JobOutputDelta {
        job_id: String,
        session_id: String,
        stream: TerminalStreamKind,
        chunk: String,
        bytes_written: u64,
    },
    SteeringAccepted {
        session_id: String,
        preview: String,
    },
    QueueUpdated {
        session_id: String,
        queued_count: usize,
    },
    OrchestratorUpdated {
        session_id: String,
        stage: String,
        tier: ExecutionTier,
        detail: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    BrainstormRecorded {
        session_id: String,
        spec: BrainstormSpec,
    },
    PlanUpdated {
        session_id: String,
        plan: ExecutionPlan,
    },
    ClarificationRequested {
        clarification: ClarificationRecord,
    },
    ClarificationResolved {
        clarification_id: String,
        session_id: String,
        selected_option_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_answer: Option<String>,
        resolved_by: String,
    },
    SubagentSpawned {
        agent: SubagentRecord,
    },
    SubagentCompleted {
        agent: SubagentRecord,
    },
    SubagentMessage {
        message: AgentMailMessage,
    },
    SessionTasksUpdated {
        session_id: String,
        tasks: Vec<SessionTask>,
    },
    CheckpointCreated {
        checkpoint: WorkspaceCheckpoint,
    },
    SessionRewound {
        report: RewindReport,
    },
    PlanApproved {
        session_id: String,
        new_mode: PermissionMode,
        task_count: usize,
    },
    McpServersUpdated {
        servers: Vec<McpServerStatus>,
    },
    SkillUpdated {
        slug: String,
        action: String,
        status: String,
    },
    GoalLoopUpdated {
        goal: GoalLoopRecord,
    },
    Telemetry(DaemonTelemetry),
    Log(LogEntry),
}

// ─── Phase 7: Deep Engineering Tools (`glob_find`, `git_tool`, `lsp_tool`), `web_search` & MCP Client ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GlobSort {
    #[default]
    Mtime,
    Path,
}

impl GlobSort {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "path" | "name" | "alpha" => Self::Path,
            _ => Self::Mtime,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GlobFileType {
    #[default]
    File,
    Dir,
    Any,
}

impl GlobFileType {
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dir" | "directory" | "dirs" => Self::Dir,
            "any" | "all" => Self::Any,
            _ => Self::File,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitAction {
    Status,
    Diff,
    Log,
    Blame,
    Show,
    Branch,
    Stage,
    Unstage,
    Commit,
}

pub const GIT_ACTIONS: &[&str] = &[
    "status", "diff", "log", "blame", "show", "branch", "stage", "unstage", "commit",
];

impl GitAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "status" => Some(Self::Status),
            "diff" => Some(Self::Diff),
            "log" => Some(Self::Log),
            "blame" => Some(Self::Blame),
            "show" => Some(Self::Show),
            "branch" | "branches" => Some(Self::Branch),
            "stage" | "add" => Some(Self::Stage),
            "unstage" | "reset_staged" => Some(Self::Unstage),
            "commit" => Some(Self::Commit),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Diff => "diff",
            Self::Log => "log",
            Self::Blame => "blame",
            Self::Show => "show",
            Self::Branch => "branch",
            Self::Stage => "stage",
            Self::Unstage => "unstage",
            Self::Commit => "commit",
        }
    }
}

/// Evaluates whether a `git_tool` invocation mutates git state (shared across Policy, Plan Mode,
/// Checkpoint capture, and ToolConcurrency — zero duplicated `if/else` checks).
pub fn is_git_tool_args_mutating(args: &serde_json::Value) -> bool {
    let action = args
        .get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(GitAction::parse);
    match action {
        Some(GitAction::Stage | GitAction::Unstage | GitAction::Commit) => true,
        Some(GitAction::Branch) => {
            let sub = args
                .get("branch_action")
                .or_else(|| args.get("mode"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("list")
                .trim()
                .to_ascii_lowercase();
            let has_name = args
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            matches!(sub.as_str(), "create" | "checkout" | "switch" | "delete") || has_name && sub != "list"
        }
        Some(
            GitAction::Status
            | GitAction::Diff
            | GitAction::Log
            | GitAction::Blame
            | GitAction::Show,
        ) => false,
        None => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspAction {
    Status,
    Diagnostics,
    Definition,
    TypeDefinition,
    Implementation,
    References,
    Hover,
    DocumentSymbols,
    WorkspaceSymbols,
    IncomingCalls,
    OutgoingCalls,
    Rename,
}

pub const LSP_ACTIONS: &[&str] = &[
    "status",
    "diagnostics",
    "definition",
    "type_definition",
    "implementation",
    "references",
    "hover",
    "document_symbols",
    "workspace_symbols",
    "incoming_calls",
    "outgoing_calls",
    "rename",
];

impl LspAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "status" => Some(Self::Status),
            "diagnostics" | "diag" | "errors" => Some(Self::Diagnostics),
            "definition" | "goto_definition" | "gotodefinition" => Some(Self::Definition),
            "type_definition" | "typedefinition" | "goto_type_definition" => {
                Some(Self::TypeDefinition)
            }
            "implementation" | "implementations" | "goto_implementation" => {
                Some(Self::Implementation)
            }
            "references" | "find_references" | "findreferences" => Some(Self::References),
            "hover" | "type" => Some(Self::Hover),
            "document_symbols" | "documentsymbol" | "symbols" | "outline" => {
                Some(Self::DocumentSymbols)
            }
            "workspace_symbols" | "workspacesymbol" => Some(Self::WorkspaceSymbols),
            "incoming_calls" | "incomingcalls" => Some(Self::IncomingCalls),
            "outgoing_calls" | "outgoingcalls" => Some(Self::OutgoingCalls),
            "rename" | "rename_symbol" | "renamesymbol" => Some(Self::Rename),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Diagnostics => "diagnostics",
            Self::Definition => "definition",
            Self::TypeDefinition => "type_definition",
            Self::Implementation => "implementation",
            Self::References => "references",
            Self::Hover => "hover",
            Self::DocumentSymbols => "document_symbols",
            Self::WorkspaceSymbols => "workspace_symbols",
            Self::IncomingCalls => "incoming_calls",
            Self::OutgoingCalls => "outgoing_calls",
            Self::Rename => "rename",
        }
    }
}

/// Evaluates whether an `lsp_tool` invocation mutates workspace files (shared across ExecPolicy,
/// Plan Mode, Checkpoint capture, ExecutionGuard, and ToolConcurrency — zero duplicated checks).
pub fn is_lsp_tool_args_mutating(args: &serde_json::Value) -> bool {
    let action = args
        .get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(LspAction::parse);
    match action {
        Some(LspAction::Rename) => !args
            .get("dry_run")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        _ => false,
    }
}

/// Declarative per-hunk edit operation for `hash_edit` (Oh My Pi Hashline pattern).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HashEditOp {
    #[default]
    Replace,
    InsertAfter,
    InsertBefore,
    Delete,
}

impl HashEditOp {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "replace" => Some(Self::Replace),
            "insert_after" | "insertafter" | "after" => Some(Self::InsertAfter),
            "insert_before" | "insertbefore" | "before" => Some(Self::InsertBefore),
            "delete" | "remove" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::InsertAfter => "insert_after",
            Self::InsertBefore => "insert_before",
            Self::Delete => "delete",
        }
    }
}

/// Declarative file-level action for atomic multi-file `apply_patch` transactions (Codex pattern).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PatchFileAction {
    #[default]
    Update,
    Create,
    Delete,
    Move,
}

impl PatchFileAction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "update" | "edit" | "modify" => Some(Self::Update),
            "create" | "add" => Some(Self::Create),
            "delete" | "remove" => Some(Self::Delete),
            "move" | "rename" => Some(Self::Move),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Create => "create",
            Self::Delete => "delete",
            Self::Move => "move",
        }
    }
}

/// Declarative output format for `web_fetch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebFetchFormat {
    #[default]
    Markdown,
    Text,
    Raw,
}

impl WebFetchFormat {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" => Some(Self::Markdown),
            "text" | "plain" => Some(Self::Text),
            "raw" => Some(Self::Raw),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Raw => "raw",
        }
    }
}

/// Session-scoped file-freshness check outcome (Claude Code stale-read guard pattern).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileFreshnessCheck {
    /// On-disk SHA-256 matches the last recorded session read/write snapshot.
    Fresh,
    /// Target file exists on disk and has not been read or written in this session.
    UnreadExisting,
    /// Target file was previously read/written in this session, but changed externally on disk.
    ExternallyModified {
        recorded_sha256: String,
        current_sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerRuntimeStatus {
    pub server_id: String,
    pub language_id: String,
    pub workspace_root: String,
    pub running: bool,
    pub open_documents: usize,
    pub cached_diagnostics_files: usize,
    pub last_used_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResultItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResponse {
    pub provider: String,
    pub query: String,
    pub results: Vec<WebSearchResultItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempted_providers: Vec<String>,
}

/// MCP ToolAnnotations parsed from `tools/list` (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolAnnotations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDiscoveredTool {
    pub server_name: String,
    pub remote_name: String,
    pub full_name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub annotations: McpToolAnnotations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerStatus {
    pub name: String,
    pub transport: String,
    pub enabled: bool,
    pub connected: bool,
    pub tool_count: usize,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub catalog_dirty: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<DateTime<Utc>>,
}

// ─── Phase 8: Procedural Skills, 24/7 Bounded Goal Loops, Event Triggers & Trajectory Export ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    #[default]
    Active,
    Draft,
}

impl SkillStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Draft => "draft",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "draft" | "pending" => Self::Draft,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillAuthor {
    #[default]
    User,
    Agent,
    Dream,
}

impl SkillAuthor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Dream => "dream",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "agent" => Self::Agent,
            "dream" => Self::Dream,
            _ => Self::User,
        }
    }
}

/// Versioned backup stored under `~/.zenko/skills/.history/` for instant rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillVersionSnapshot {
    pub id: String,
    pub slug: String,
    pub reason: String,
    pub bytes: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillManageAction {
    List,
    Create,
    Patch,
    WriteSubfile,
    RemoveSubfile,
    Toggle,
    Approve,
    History,
    Rollback,
    Delete,
}

impl SkillManageAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "list" | "ls" => Some(Self::List),
            "create" | "save" | "upsert" => Some(Self::Create),
            "patch" | "edit" => Some(Self::Patch),
            "write_subfile" | "write_file" => Some(Self::WriteSubfile),
            "remove_subfile" | "remove_file" => Some(Self::RemoveSubfile),
            "toggle" | "enable" | "disable" => Some(Self::Toggle),
            "approve" => Some(Self::Approve),
            "history" | "versions" => Some(Self::History),
            "rollback" | "restore" => Some(Self::Rollback),
            "delete" | "remove" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Create => "create",
            Self::Patch => "patch",
            Self::WriteSubfile => "write_subfile",
            Self::RemoveSubfile => "remove_subfile",
            Self::Toggle => "toggle",
            Self::Approve => "approve",
            Self::History => "history",
            Self::Rollback => "rollback",
            Self::Delete => "delete",
        }
    }

    pub fn is_mutating(self) -> bool {
        !matches!(self, Self::List | Self::History)
    }
}

pub fn is_skill_manage_args_mutating(args: &serde_json::Value) -> bool {
    args.get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(SkillManageAction::parse)
        .map(SkillManageAction::is_mutating)
        .unwrap_or(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalLoopStatus {
    #[default]
    Active,
    Paused,
    Succeeded,
    Failed,
    BudgetExhausted,
    Stalled,
}

impl GoalLoopStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::BudgetExhausted => "budget_exhausted",
            Self::Stalled => "stalled",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "paused" => Self::Paused,
            "succeeded" | "done" | "completed" => Self::Succeeded,
            "failed" | "error" => Self::Failed,
            "budget_exhausted" | "budget" => Self::BudgetExhausted,
            "stalled" => Self::Stalled,
            _ => Self::Active,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::BudgetExhausted | Self::Stalled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalLoopAction {
    List,
    Create,
    Status,
    Pause,
    Resume,
    RunNow,
    Delete,
}

impl GoalLoopAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "list" | "ls" => Some(Self::List),
            "create" | "add" | "start" => Some(Self::Create),
            "status" | "get" | "show" => Some(Self::Status),
            "pause" | "stop" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "run_now" | "run" | "trigger" => Some(Self::RunNow),
            "delete" | "remove" => Some(Self::Delete),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Create => "create",
            Self::Status => "status",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::RunNow => "run_now",
            Self::Delete => "delete",
        }
    }

    pub fn is_mutating(self) -> bool {
        !matches!(self, Self::List | Self::Status)
    }
}

pub fn is_goal_loop_args_mutating(args: &serde_json::Value) -> bool {
    args.get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(GoalLoopAction::parse)
        .map(GoalLoopAction::is_mutating)
        .unwrap_or(true)
}

pub fn new_goal_loop_id() -> String {
    format!("{}{}", crate::constants::GOAL_LOOP_ID_PREFIX, new_random_suffix(10))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalLoopRecord {
    pub id: String,
    pub name: String,
    pub session_id: String,
    pub workspace_dir: String,
    pub goal_text: String,
    /// Deterministic argv verification command executed without `/bin/sh` interpolation.
    pub verify_cmd: Vec<String>,
    pub interval_secs: u64,
    pub max_iterations: usize,
    pub max_budget_usd: f64,
    pub max_consecutive_stalls: usize,
    pub checkpoint_each_iter: bool,
    pub auto_rewind_on_stall: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub notify_channel: String,
    pub status: GoalLoopStatus,
    pub iterations_done: usize,
    pub consecutive_stalls: usize,
    pub cost_spent_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verify_exit: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verify_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verify_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalLoopRun {
    pub id: String,
    pub goal_id: String,
    pub iteration: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    pub verify_exit_code: i32,
    pub verify_output_excerpt: String,
    pub verify_hash: String,
    pub tokens_used: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub outcome: String,
    pub reward: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalLoopCreateRequest {
    pub name: String,
    pub goal_text: String,
    pub verify_cmd: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub interval_secs: Option<u64>,
    #[serde(default)]
    pub max_iterations: Option<usize>,
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    #[serde(default)]
    pub max_consecutive_stalls: Option<usize>,
    #[serde(default)]
    pub checkpoint_each_iter: Option<bool>,
    #[serde(default)]
    pub auto_rewind_on_stall: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_channel: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GoalLoopPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_cmd: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<GoalLoopStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_stalls: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerStatusEntry {
    pub id: String,
    pub kind: String,
    pub enabled: bool,
    pub target: String,
    pub fires_last_hour: usize,
    pub max_per_hour: usize,
    pub cooldown_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<DateTime<Utc>>,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct TriggerRateSnapshot {
    pub fires_last_hour: usize,
    pub last_fired_at: Option<DateTime<Utc>>,
}

/// Bounded sliding-window rate limiter & per-trigger cooldown tracker shared across
/// declarative webhook routing rules and workspace file-change watchers.
#[derive(Default)]
pub struct TriggerRateLimiter {
    buckets: std::sync::Mutex<
        std::collections::HashMap<String, std::collections::VecDeque<DateTime<Utc>>>,
    >,
}

impl TriggerRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check_and_record(
        &self,
        key: &str,
        cooldown_secs: u64,
        max_per_hour: usize,
    ) -> std::result::Result<(), String> {
        let now = Utc::now();
        let hour_cutoff = now - chrono::Duration::hours(1);
        let Ok(mut map) = self.buckets.lock() else {
            return Err("trigger rate limiter lock poisoned".into());
        };
        // Bound bucket table memory: evict expired entries and enforce max bucket cap.
        if map.len() >= crate::constants::DEFAULT_RATE_LIMITER_MAX_ENTRIES {
            map.retain(|_, q| {
                while q.front().is_some_and(|t| *t < hour_cutoff) {
                    q.pop_front();
                }
                !q.is_empty()
            });
            if map.len() >= crate::constants::DEFAULT_RATE_LIMITER_MAX_ENTRIES
                && !map.contains_key(key)
                && let Some(oldest_key) = map
                    .iter()
                    .min_by_key(|(_, q)| q.back().copied())
                    .map(|(k, _)| k.clone())
                {
                    map.remove(&oldest_key);
                }
        }
        let queue = map.entry(key.to_string()).or_default();
        while queue.front().is_some_and(|t| *t < hour_cutoff) {
            queue.pop_front();
        }
        if cooldown_secs > 0
            && let Some(last) = queue.back() {
                let elapsed = (now - *last).num_seconds().max(0) as u64;
                if elapsed < cooldown_secs {
                    return Err(format!(
                        "cooldown active ({}s remaining)",
                        cooldown_secs - elapsed
                    ));
                }
            }
        let cap = max_per_hour.max(1);
        if queue.len() >= cap {
            return Err(format!("hourly rate limit reached ({cap}/hr)"));
        }
        queue.push_back(now);
        Ok(())
    }

    pub fn snapshot(&self, key: &str) -> TriggerRateSnapshot {
        let now = Utc::now();
        let hour_cutoff = now - chrono::Duration::hours(1);
        let Ok(mut map) = self.buckets.lock() else {
            return TriggerRateSnapshot {
                fires_last_hour: 0,
                last_fired_at: None,
            };
        };
        let Some(queue) = map.get_mut(key) else {
            return TriggerRateSnapshot {
                fires_last_hour: 0,
                last_fired_at: None,
            };
        };
        while queue.front().is_some_and(|t| *t < hour_cutoff) {
            queue.pop_front();
        }
        TriggerRateSnapshot {
            fires_last_hour: queue.len(),
            last_fired_at: queue.back().copied(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[default]
    Jsonl,
    ShareGpt,
    Markdown,
}

impl ExportFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jsonl => "jsonl",
            Self::ShareGpt => "sharegpt",
            Self::Markdown => "markdown",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Jsonl | Self::ShareGpt => "application/x-ndjson; charset=utf-8",
            Self::Markdown => "text/markdown; charset=utf-8",
        }
    }

    pub fn file_ext(self) -> &'static str {
        match self {
            Self::Jsonl | Self::ShareGpt => "jsonl",
            Self::Markdown => "md",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "sharegpt" | "share_gpt" | "sft" => Self::ShareGpt,
            "md" | "markdown" => Self::Markdown,
            _ => Self::Jsonl,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportOptions {
    #[serde(default)]
    pub format: ExportFormat,
    #[serde(default)]
    pub include_thinking: bool,
    #[serde(default)]
    pub anonymize_paths: bool,
    #[serde(default)]
    pub all_branches: bool,
    #[serde(default)]
    pub verified_only: bool,
    #[serde(default)]
    pub min_turns: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportReport {
    pub format: String,
    pub sessions_exported: usize,
    pub events_exported: usize,
    pub bytes: usize,
    pub content: String,
}

/// Lifecycle state of an experience-learned repository micro-skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MicroSkillStatus {
    #[default]
    Candidate,
    Active,
    Retired,
}

impl MicroSkillStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }

    pub fn from_str_loose(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "active" => Self::Active,
            "retired" => Self::Retired,
            _ => Self::Candidate,
        }
    }
}

/// Corroborated 1-line repository/environment fact learned across sessions (zero-LLM default).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoMicroSkill {
    pub id: String,
    pub repo_scope: String,
    pub signature: String,
    pub fact: String,
    pub failed_cmd: String,
    pub fixed_cmd: String,
    pub corroborations: u32,
    pub session_ids: Vec<String>,
    pub need_hits: u32,
    pub exposures: u32,
    pub contradictions: u32,
    pub status: MicroSkillStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Per-repository tool utilization & soft-pruning state (demotes unused non-core tools to deferred).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoToolUsage {
    pub repo_scope: String,
    pub tool_name: String,
    pub call_count: u64,
    pub prompts_since_used: u32,
    pub sessions_observed: u32,
    pub estimated_schema_tokens: u32,
    pub pruned_to_deferred: bool,
    pub manually_pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// Categorized token savings vs. optimizer overhead event recorded in `economy_ledger`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomyLedgerKind {
    ReadLess,
    OutputDedup,
    PrefixChunkRef,
    Shunt,
    ToolPrune,
    MicroSkill,
    LoopGuard,
    CcrRecovery,
    RawBypass,
    OverheadDream,
    OverheadJudge,
}

impl EconomyLedgerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadLess => "read_less",
            Self::OutputDedup => "output_dedup",
            Self::PrefixChunkRef => "prefix_chunk_ref",
            Self::Shunt => "shunt",
            Self::ToolPrune => "tool_prune",
            Self::MicroSkill => "micro_skill",
            Self::LoopGuard => "loop_guard",
            Self::CcrRecovery => "ccr_recovery",
            Self::RawBypass => "raw_bypass",
            Self::OverheadDream => "overhead_dream",
            Self::OverheadJudge => "overhead_judge",
        }
    }

    pub fn from_str_loose(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "read_less" => Some(Self::ReadLess),
            "output_dedup" => Some(Self::OutputDedup),
            "prefix_chunk_ref" => Some(Self::PrefixChunkRef),
            "shunt" => Some(Self::Shunt),
            "tool_prune" => Some(Self::ToolPrune),
            "micro_skill" => Some(Self::MicroSkill),
            "loop_guard" => Some(Self::LoopGuard),
            "ccr_recovery" => Some(Self::CcrRecovery),
            "raw_bypass" => Some(Self::RawBypass),
            "overhead_dream" => Some(Self::OverheadDream),
            "overhead_judge" => Some(Self::OverheadJudge),
            _ => None,
        }
    }
}

/// Declarative enforcement posture for Zenko's tier-aware Shunt large-read guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShuntMode {
    Off,
    Advise,
    #[default]
    Redirect,
}

impl ShuntMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Advise => "advise",
            Self::Redirect => "redirect",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "disabled" | "none" => Some(Self::Off),
            "advise" | "soft" | "warn" => Some(Self::Advise),
            "redirect" | "hard" | "gate" => Some(Self::Redirect),
            _ => None,
        }
    }

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Typed action recorded in Shunt tool metadata (`redirect` vs `advise`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShuntActionKind {
    Redirect,
    Advise,
}

impl ShuntActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Redirect => "redirect",
            Self::Advise => "advise",
        }
    }
}

/// Typed source category for Shunt interceptions (`read_file` vs `shell_read`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShuntSourceKind {
    ReadFile,
    ShellRead,
}

impl ShuntSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadFile => "read_file",
            Self::ShellRead => "shell_read",
        }
    }
}

/// Declarative file-dump behavior class for shell file-reading binaries in `SHUNT_SHELL_BIN_SPECS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShuntShellBinBehavior {
    FullDump,
    HeadSlice,
    TailSlice,
}

/// Per-command-family token savings breakdown (`cargo`, `git`, `tsc`, `lint`, `test`, `pkg`, `docker`, `custom`, `mcp`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandSavingsStat {
    pub command_family: String,
    pub calls: u64,
    pub tokens_saved: u64,
    #[serde(default)]
    pub raw_bypasses: u64,
}

/// Uncompressed high-volume command family discovered across recent SQLite session history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissedSavingsOpportunity {
    pub command_prefix: String,
    pub calls: u64,
    pub total_bytes: u64,
    pub estimated_missed_tokens: u64,
    pub suggested_filter_toml: String,
}

/// Single failed inline `[[tests.<name>]]` case from a declarative `filters.toml` verification run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomFilterTestFailure {
    pub filter_name: String,
    pub test_name: String,
    pub expected: String,
    pub actual: String,
}

/// Verification summary for `~/.zenko/filters.toml` or `<workspace>/.zenko/filters.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomFilterVerifyReport {
    pub file_path: String,
    pub exists: bool,
    pub valid: bool,
    pub rule_count: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub test_failures: Vec<CustomFilterTestFailure>,
}

/// SHA-256 trust status for project-local `<workspace>/.zenko/filters.toml` + user-global rule count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomFilterTrustStatus {
    pub project_file_path: String,
    pub project_file_exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_sha256: Option<String>,
    pub trusted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_sha256: Option<String>,
    pub rule_count: usize,
    pub user_rule_count: usize,
}

/// Aggregated per-repository net Token Economy ROI summary (gross savings minus overhead).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EconomyLedgerSummary {
    pub repo_scope: String,
    pub read_less_tokens_saved: u64,
    #[serde(default)]
    pub output_dedup_tokens_saved: u64,
    #[serde(default)]
    pub output_dedup_hits: u64,
    #[serde(default)]
    pub prefix_chunk_ref_hits: u64,
    #[serde(default)]
    pub prefix_chunk_ref_tokens_saved: u64,
    #[serde(default)]
    pub shunt_hits: u64,
    #[serde(default)]
    pub shunt_tokens_saved: u64,
    #[serde(default)]
    pub ccr_recoveries: u64,
    #[serde(default)]
    pub raw_bypasses: u64,
    pub tool_prune_tokens_saved: u64,
    pub micro_skill_injections: u64,
    pub loop_guard_interventions: u64,
    pub overhead_tokens: u64,
    pub net_tokens_saved: i64,
    pub estimated_usd_saved: f64,
    pub p90_turn_calls: usize,
    pub envelope_calls: usize,
    pub baseline_samples: usize,
    pub active_micro_skills: usize,
    pub candidate_micro_skills: usize,
    pub retired_micro_skills: usize,
    pub pruned_tools_count: usize,
    pub completed_tasks: u64,
}

/// Full Token Economy report returned by `GET /api/economy` and the deferred `token_economy` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenEconomyReport {
    pub repo_scope: String,
    pub config: crate::config::EconomyConfig,
    pub summary: EconomyLedgerSummary,
    #[serde(default)]
    pub code_metrics: RepoCodeMetrics,
    #[serde(default)]
    pub by_command: Vec<CommandSavingsStat>,
    #[serde(default)]
    pub filter_status: CustomFilterTrustStatus,
    pub micro_skills: Vec<RepoMicroSkill>,
    pub tool_usage: Vec<RepoToolUsage>,
    pub pruned_tools: Vec<String>,
    pub prune_candidates: Vec<String>,
}

/// Declarative Code Minimalism intensity profile (inspired by DietrichGebert/ponytail; zero /commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CodeMinimalismMode {
    Off,
    #[default]
    Lite,
    Full,
    Ultra,
}

impl CodeMinimalismMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Lite => "lite",
            Self::Full => "full",
            Self::Ultra => "ultra",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disabled" => Some(Self::Off),
            "lite" | "light" => Some(Self::Lite),
            "full" | "default" | "standard" => Some(Self::Full),
            "ultra" | "strict" | "max" => Some(Self::Ultra),
            _ => None,
        }
    }

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Unified declarative Zenko context & efficiency gear (`lite`, `balanced` [default], `high`, `ultra`).
/// Coordinates Concise Prose, Code Minimalism, 3-tier Read-Less/Dedup/Scrub thresholds,
/// Microcompaction/Compaction budgets, and Token Economy guardrails from one single registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ZMode {
    Lite,
    #[default]
    Balanced,
    High,
    Ultra,
}

impl ZMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lite => "lite",
            Self::Balanced => "balanced",
            Self::High => "high",
            Self::Ultra => "ultra",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "lite" | "light" => Some(Self::Lite),
            "balanced" | "default" | "standard" => Some(Self::Balanced),
            "high" | "full" | "aggressive" => Some(Self::High),
            "ultra" | "max" | "strict" => Some(Self::Ultra),
            _ => None,
        }
    }
}

/// Declarative action enum for the deferred read-only `code_minimalism` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeMinimalismAction {
    AuditDiff,
    AuditRepo,
    DebtScan,
    Gain,
}

impl CodeMinimalismAction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "audit_diff" | "diff" | "review" => Some(Self::AuditDiff),
            "audit_repo" | "audit" | "repo" => Some(Self::AuditRepo),
            "debt_scan" | "debt" | "shortcuts" => Some(Self::DebtScan),
            "gain" | "metrics" | "scoreboard" => Some(Self::Gain),
            _ => None,
        }
    }
}

/// Real per-repository code churn, bloat-hint, and shortcut-debt metrics persisted in SQLite.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoCodeMetrics {
    pub repo_scope: String,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub net_loc_delta: i64,
    pub turns_with_edits: u64,
    pub new_files_created: u64,
    pub bloat_hints_issued: u64,
    pub debt_markers_last_seen: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

/// Ranked large-file entry returned by `code_minimalism(action="audit_repo")`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LargeFileFinding {
    pub path: String,
    pub lines: usize,
}

/// Ranked long-function entry returned by `code_minimalism(action="audit_repo")`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongFunctionFinding {
    pub path: String,
    pub line: usize,
    pub span_lines: usize,
    pub signature: String,
}

/// Cross-file duplicate helper symbol candidate returned by `audit_repo` and `audit_diff`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateSymbolFinding {
    pub symbol: String,
    pub locations: Vec<String>,
}

/// Thin wrapper type candidate (possible speculative abstraction) returned by `audit_repo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinWrapperFinding {
    pub path: String,
    pub line: usize,
    pub span_lines: usize,
    pub kind: String,
    pub name: String,
}

/// Oversized single-hunk addition detected inside a git diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OversizedHunkFinding {
    pub path: String,
    pub start_line: usize,
    pub added_lines: usize,
}

/// Single comment-scoped shortcut/debt marker found by `debt_scan`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebtItem {
    pub path: String,
    pub line: usize,
    pub marker: String,
    pub text: String,
}

/// Structured output of `code_minimalism(action="debt_scan")`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DebtScanReport {
    pub files_scanned: usize,
    pub total_found: usize,
    pub truncated: bool,
    pub items: Vec<DebtItem>,
}

/// Nearby nested repo-rule / DOX file whose governed subtree had structural edits without doc updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoxDriftFinding {
    pub doc_path: String,
    pub changed_files: Vec<String>,
    pub new_files_count: usize,
}

/// Repo-native `project_brain` page category (`mindmuxai/brain.md` open standard).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainPageCategory {
    #[default]
    Decision,
    Project,
    Concept,
    Person,
    Organization,
    Reference,
}

impl BrainPageCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Concept => "concept",
            Self::Decision => "decision",
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Reference => "reference",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "project" => Some(Self::Project),
            "concept" => Some(Self::Concept),
            "decision" => Some(Self::Decision),
            "person" => Some(Self::Person),
            "organization" | "org" => Some(Self::Organization),
            "reference" | "ref" => Some(Self::Reference),
            _ => None,
        }
    }
}

/// Lifecycle state of a repo-native `project_brain` page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainPageStatus {
    #[default]
    Active,
    Draft,
    Archived,
}

impl BrainPageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Draft => "draft",
            Self::Archived => "archived",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "active" => Some(Self::Active),
            "draft" => Some(Self::Draft),
            "archived" | "archive" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// Append-only timeline event kind recording how project truth evolved or was reversed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainTimelineKind {
    #[default]
    Decision,
    Evidence,
    Update,
    Reversal,
    Archived,
}

impl BrainTimelineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Evidence => "evidence",
            Self::Update => "update",
            Self::Reversal => "reversal",
            Self::Archived => "archived",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "decision" => Some(Self::Decision),
            "evidence" => Some(Self::Evidence),
            "update" => Some(Self::Update),
            "reversal" | "reversed" => Some(Self::Reversal),
            "archived" | "archive" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// Fixed blueprint root slots inside `brain/` (`mindmuxai/brain.md` open standard).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainRootSlot {
    Background,
    Architecture,
    Flow,
    Mindmap,
    Stack,
    Roadmap,
}

impl BrainRootSlot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Architecture => "architecture",
            Self::Flow => "flow",
            Self::Mindmap => "mindmap",
            Self::Stack => "stack",
            Self::Roadmap => "roadmap",
        }
    }

    pub fn filename(self) -> &'static str {
        match self {
            Self::Background => "background.md",
            Self::Architecture => "architecture.md",
            Self::Flow => "flow.md",
            Self::Mindmap => "mindmap.md",
            Self::Stack => "stack.md",
            Self::Roadmap => "roadmap.md",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let norm = raw.trim().to_ascii_lowercase();
        let trimmed = norm.strip_suffix(".md").unwrap_or(&norm);
        match trimmed {
            "background" => Some(Self::Background),
            "architecture" | "arch" => Some(Self::Architecture),
            "flow" | "flows" => Some(Self::Flow),
            "mindmap" => Some(Self::Mindmap),
            "stack" => Some(Self::Stack),
            "roadmap" => Some(Self::Roadmap),
            _ => None,
        }
    }
}

/// Declarative action enum for the deferred `project_brain` tool (pure `match`, zero string ladders).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainAction {
    Init,
    ListPages,
    ReadPage,
    CreatePage,
    UpdateTruth,
    AppendTimeline,
    ArchivePage,
    ReadRoot,
    UpdateRoot,
    Search,
    Impact,
}

impl BrainAction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "init" | "setup" => Some(Self::Init),
            "list_pages" | "list" => Some(Self::ListPages),
            "read_page" | "read" | "get" => Some(Self::ReadPage),
            "create_page" | "create" => Some(Self::CreatePage),
            "update_truth" | "update" => Some(Self::UpdateTruth),
            "append_timeline" | "timeline" => Some(Self::AppendTimeline),
            "archive_page" | "archive" => Some(Self::ArchivePage),
            "read_root" => Some(Self::ReadRoot),
            "update_root" => Some(Self::UpdateRoot),
            "search" | "query" => Some(Self::Search),
            "impact" | "graph" | "links" => Some(Self::Impact),
            _ => None,
        }
    }

    pub fn is_mutating(self) -> bool {
        matches!(
            self,
            Self::Init
                | Self::CreatePage
                | Self::UpdateTruth
                | Self::AppendTimeline
                | Self::ArchivePage
                | Self::UpdateRoot
        )
    }
}

/// Returns `true` when `project_brain` arguments request a mutating repository write
/// (used by `PermissionMode::Plan` policy and workspace mutation watermark).
pub fn is_brain_tool_args_mutating(args: &serde_json::Value) -> bool {
    args.get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(BrainAction::parse)
        .is_none_or(BrainAction::is_mutating)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrainTimelineEntry {
    pub time: String,
    pub kind: BrainTimelineKind,
    pub summary: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub affects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrainPage {
    pub id: String,
    pub title: String,
    pub category: BrainPageCategory,
    pub status: BrainPageStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created: String,
    pub updated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_after: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    pub compiled_truth: String,
    #[serde(default)]
    pub timeline: Vec<BrainTimelineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainPageSummary {
    pub id: String,
    pub title: String,
    pub category: BrainPageCategory,
    pub status: BrainPageStatus,
    #[serde(default)]
    pub tags: Vec<String>,
    pub updated: String,
    pub timeline_count: usize,
    pub has_reversals: bool,
    pub truth_preview: String,
    #[serde(default)]
    pub freshness: OkfFreshnessState,
    #[serde(default)]
    pub trust_tier: OkfTrustTier,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainSearchHit {
    pub summary: BrainPageSummary,
    pub score: f64,
    pub affects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainImpactGraph {
    pub page_id: String,
    pub direct_affects: Vec<String>,
    pub transitive_affects: Vec<String>,
    pub referenced_by: Vec<String>,
}

/// Structured output of `code_minimalism(action="audit_diff")`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffAuditReport {
    pub files_changed: Vec<String>,
    pub new_files: Vec<String>,
    pub lines_added: u64,
    pub lines_deleted: u64,
    pub net_loc_delta: i64,
    #[serde(default)]
    pub added_comment_lines: u64,
    #[serde(default)]
    pub added_code_lines: u64,
    pub manifest_changes: Vec<String>,
    pub oversized_hunks: Vec<OversizedHunkFinding>,
    pub duplicate_symbols: Vec<DuplicateSymbolFinding>,
    #[serde(default)]
    pub dox_drift: Vec<DoxDriftFinding>,
    pub findings: Vec<String>,
}

/// Potentially unused runtime dependency declared in a project manifest (`Cargo.toml` / `package.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DormantDependencyFinding {
    pub manifest: String,
    pub package: String,
}

/// Behavioral trait / interface / abstract class with <= 1 concrete implementation in the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleImplAbstractionFinding {
    pub path: String,
    pub line: usize,
    pub kind: String,
    pub name: String,
    pub impl_count: usize,
}

/// Source file exhibiting heavy comment/docstring prose bloat or long narrative comment blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentBloatFinding {
    pub path: String,
    pub comment_lines: usize,
    pub code_lines: usize,
    pub comment_ratio_pct: usize,
    pub longest_block_lines: usize,
    pub longest_block_line: usize,
}

/// Structured output of `code_minimalism(action="audit_repo")`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoAuditReport {
    pub files_scanned: usize,
    pub total_lines: usize,
    pub large_files: Vec<LargeFileFinding>,
    pub long_functions: Vec<LongFunctionFinding>,
    pub duplicate_symbols: Vec<DuplicateSymbolFinding>,
    pub thin_wrappers: Vec<ThinWrapperFinding>,
    #[serde(default)]
    pub single_impl_abstractions: Vec<SingleImplAbstractionFinding>,
    #[serde(default)]
    pub comment_bloat: Vec<CommentBloatFinding>,
    #[serde(default)]
    pub dormant_dependencies: Vec<DormantDependencyFinding>,
    pub debt_count: usize,
}

/// Structured output of `code_minimalism(action="gain")`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimalismGainReport {
    pub repo_scope: String,
    pub mode: CodeMinimalismMode,
    pub metrics: RepoCodeMetrics,
    pub economy_summary: EconomyLedgerSummary,
    pub deletion_ratio_pct: f64,
}

/// Declarative action enum for the deferred `token_economy` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomyAction {
    Status,
    MicroSkills,
    AddMicroSkill,
    RetireMicroSkill,
    PruneReport,
    ApplyPrune,
    RestoreTool,
    Reaudit,
    Ledger,
    Discover,
    VerifyFilters,
}

impl EconomyAction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "status" | "overview" => Some(Self::Status),
            "micro_skills" | "microskills" | "list_skills" => Some(Self::MicroSkills),
            "add_micro_skill" | "add_skill" => Some(Self::AddMicroSkill),
            "retire_micro_skill" | "retire_skill" | "retire" => Some(Self::RetireMicroSkill),
            "prune_report" | "toolprune" | "tools" => Some(Self::PruneReport),
            "apply_prune" | "prune" => Some(Self::ApplyPrune),
            "restore_tool" | "restore" => Some(Self::RestoreTool),
            "reaudit" | "reset_prune" => Some(Self::Reaudit),
            "ledger" | "roi" | "savings" | "gain" => Some(Self::Ledger),
            "discover" | "missed" | "opportunities" => Some(Self::Discover),
            "verify_filters" | "filters" => Some(Self::VerifyFilters),
            _ => None,
        }
    }

    pub fn is_mutating(self) -> bool {
        !matches!(
            self,
            Self::Status
                | Self::MicroSkills
                | Self::PruneReport
                | Self::Ledger
                | Self::Discover
                | Self::VerifyFilters
        )
    }
}

pub fn is_economy_args_mutating(args: &serde_json::Value) -> bool {
    args.get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(EconomyAction::parse)
        .map(EconomyAction::is_mutating)
        .unwrap_or(true)
}

/// Declarative action enum for the unified deferred `code_refactor` tool
/// (combining pure-Rust `ast-grep` structural search/rewrite, `sd`/`ripide` multi-file
/// code-only replace & import-aware file move, `tokei` FSM stats, and `difftastic` structural diff).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeRefactorAction {
    StructSearch,
    StructRewrite,
    Replace,
    MoveImports,
    Stats,
    StructDiff,
}

impl CodeRefactorAction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "struct_search" | "ast_search" | "pattern_search" => Some(Self::StructSearch),
            "struct_rewrite" | "ast_rewrite" | "rewrite" => Some(Self::StructRewrite),
            "replace" | "find_replace" | "rename" => Some(Self::Replace),
            "move_imports" | "move_file" | "move" => Some(Self::MoveImports),
            "stats" | "loc" | "languages" => Some(Self::Stats),
            "struct_diff" | "semantic_diff" | "diff" => Some(Self::StructDiff),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StructSearch => "struct_search",
            Self::StructRewrite => "struct_rewrite",
            Self::Replace => "replace",
            Self::MoveImports => "move_imports",
            Self::Stats => "stats",
            Self::StructDiff => "struct_diff",
        }
    }

    pub fn can_mutate(self) -> bool {
        matches!(self, Self::StructRewrite | Self::Replace | Self::MoveImports)
    }
}

/// Returns `true` iff a `code_refactor` invocation mutates workspace files on disk.
/// Any read-only action (`struct_search`, `stats`, `struct_diff`) OR any mutating action
/// invoked with `dry_run: true` returns `false` (permitted in Plan Mode and read-only subagents).
pub fn is_code_refactor_args_mutating(args: &serde_json::Value) -> bool {
    let Some(action) = args
        .get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(CodeRefactorAction::parse)
    else {
        return true;
    };
    let dry_run = args
        .get("dry_run")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    action.can_mutate() && !dry_run
}

// ── TurboQuant, OKF (Open Knowledge Format) & Chained Prefix-Cache Types ────

/// Bit-width mode for Zenko's data-oblivious TurboQuant scalar quantizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TurboQuantBits {
    Bits2,
    #[default]
    Bits4,
}

impl TurboQuantBits {
    pub fn bits_per_coord(self) -> u8 {
        match self {
            Self::Bits2 => 2,
            Self::Bits4 => 4,
        }
    }

    pub fn from_u8(bits: u8) -> Option<Self> {
        match bits {
            2 => Some(Self::Bits2),
            4 => Some(Self::Bits4),
            _ => None,
        }
    }

    pub fn coords_per_byte(self) -> usize {
        match self {
            Self::Bits2 => 4,
            Self::Bits4 => 2,
        }
    }
}

/// Lifecycle status of an Open Knowledge Format (OKF) concept document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OkfDocStatus {
    #[default]
    Active,
    Draft,
    Deprecated,
    Superseded,
}

impl OkfDocStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Draft => "draft",
            Self::Deprecated => "deprecated",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "active" | "published" | "verified" => Some(Self::Active),
            "draft" | "wip" => Some(Self::Draft),
            "deprecated" | "retired" | "archived" => Some(Self::Deprecated),
            "superseded" | "replaced" => Some(Self::Superseded),
            _ => None,
        }
    }
}

/// Calendar freshness evaluated from OKF `stale_after` vs current time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OkfFreshnessState {
    Fresh,
    ExpiringSoon,
    Stale,
    #[default]
    Unspecified,
}

impl OkfFreshnessState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::ExpiringSoon => "expiring_soon",
            Self::Stale => "stale",
            Self::Unspecified => "unspecified",
        }
    }

    pub fn rank_weight(self) -> f64 {
        match self {
            Self::Fresh => crate::OKF_FRESHNESS_WEIGHT_FRESH,
            Self::ExpiringSoon => crate::OKF_FRESHNESS_WEIGHT_EXPIRING,
            Self::Stale => crate::OKF_FRESHNESS_WEIGHT_STALE,
            Self::Unspecified => crate::OKF_FRESHNESS_WEIGHT_UNSPECIFIED,
        }
    }
}

/// Provenance & verification trust tier for an OKF concept or Project Brain page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OkfTrustTier {
    Verified,
    #[default]
    Unverified,
    SourceDrifted,
    Deprecated,
}

impl OkfTrustTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unverified => "unverified",
            Self::SourceDrifted => "source_drifted",
            Self::Deprecated => "deprecated",
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::Verified => "VERIFIED",
            Self::Unverified => "UNVERIFIED",
            Self::SourceDrifted => "DRIFTED",
            Self::Deprecated => "DEPRECATED",
        }
    }

    pub fn rank_weight(self) -> f64 {
        match self {
            Self::Verified => crate::OKF_TRUST_WEIGHT_VERIFIED,
            Self::Unverified => crate::OKF_TRUST_WEIGHT_UNVERIFIED,
            Self::SourceDrifted => crate::OKF_TRUST_WEIGHT_DRIFTED,
            Self::Deprecated => crate::OKF_TRUST_WEIGHT_DEPRECATED,
        }
    }
}

/// Declarative action enum for the deferred `knowledge_catalog` OKF tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkfAction {
    List,
    Search,
    Read,
    Graph,
    AuditFreshness,
    Upsert,
}

impl OkfAction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "list" | "ls" => Some(Self::List),
            "search" | "query" => Some(Self::Search),
            "read" | "get" | "show" => Some(Self::Read),
            "graph" | "links" | "neighbors" => Some(Self::Graph),
            "audit_freshness" | "audit" | "freshness" => Some(Self::AuditFreshness),
            "upsert" | "write" | "create" | "update" => Some(Self::Upsert),
            _ => None,
        }
    }

    pub fn is_mutating(self) -> bool {
        matches!(self, Self::Upsert)
    }
}

/// Returns `true` when `knowledge_catalog` arguments request a mutating repository write.
pub fn is_okf_tool_args_mutating(args: &serde_json::Value) -> bool {
    args.get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(OkfAction::parse)
        .is_none_or(OkfAction::is_mutating)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkfConcept {
    pub id: String,
    pub rel_path: String,
    pub doc_type: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub status: OkfDocStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_after: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub outbound_links: Vec<String>,
    #[serde(default)]
    pub missing_sources: Vec<String>,
    #[serde(default)]
    pub drifted_sources: Vec<String>,
    pub freshness: OkfFreshnessState,
    pub trust_tier: OkfTrustTier,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkfConceptSummary {
    pub id: String,
    pub rel_path: String,
    pub doc_type: String,
    pub title: String,
    pub status: OkfDocStatus,
    pub freshness: OkfFreshnessState,
    pub trust_tier: OkfTrustTier,
    pub link_count: usize,
    pub source_count: usize,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkfSearchHit {
    pub summary: OkfConceptSummary,
    pub score: f64,
    pub vector_cosine: f32,
    pub outbound_links: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkfGraphNeighborhood {
    pub concept_id: String,
    pub direct_outbound: Vec<String>,
    pub transitive_outbound: Vec<String>,
    pub backlinks: Vec<String>,
    pub broken_links: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OkfAuditIssueKind {
    Stale,
    ExpiringSoon,
    MissingSource,
    SourceDrifted,
    BrokenLink,
    Unverified,
}

impl OkfAuditIssueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::ExpiringSoon => "expiring_soon",
            Self::MissingSource => "missing_source",
            Self::SourceDrifted => "source_drifted",
            Self::BrokenLink => "broken_link",
            Self::Unverified => "unverified",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkfAuditIssue {
    pub concept_id: String,
    pub rel_path: String,
    pub kind: OkfAuditIssueKind,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OkfFreshnessAuditReport {
    pub total_concepts: usize,
    pub verified_count: usize,
    pub stale_count: usize,
    pub drifted_count: usize,
    pub broken_links_count: usize,
    pub issues: Vec<OkfAuditIssue>,
}

/// Semantic chunk category in Zenko's LMCache-inspired parent-chained prompt hash tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptChunkKind {
    SystemBase,
    ToolCatalog,
    HistoryMessage,
}

impl PromptChunkKind {
    pub fn tag_byte(self) -> u8 {
        match self {
            Self::SystemBase => 0x01,
            Self::ToolCatalog => 0x02,
            Self::HistoryMessage => 0x03,
        }
    }
}

/// Root cause of any prefix-cache divergence between consecutive turns in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrefixDriftReason {
    #[default]
    FirstTurn,
    FullHit,
    SystemMutated,
    ToolCatalogChanged,
    HistoryCompacted,
    HistoryDiverged,
}

impl PrefixDriftReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FirstTurn => "first_turn",
            Self::FullHit => "full_hit",
            Self::SystemMutated => "system_mutated",
            Self::ToolCatalogChanged => "tool_catalog_changed",
            Self::HistoryCompacted => "history_compacted",
            Self::HistoryDiverged => "history_diverged",
        }
    }
}

/// Deterministic prefix-cache reuse & drift evaluation for an outbound `ChatRequest`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrefixCacheEvaluation {
    pub prompt_cache_key: String,
    pub total_chunks: usize,
    pub matched_chunks: usize,
    pub reusable_tokens: usize,
    pub busted_tokens: usize,
    pub hit_ratio_pct: u8,
    pub drift_reason: PrefixDriftReason,
}
