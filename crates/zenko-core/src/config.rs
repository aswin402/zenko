use crate::constants::*;
use crate::error::{Result, ZenkoError};
use crate::registry::find_provider;
use crate::types::{HUB_CATEGORIES, HubEntry, RiskLevel};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Strict read-only research & planning mode (blocks all file/shell/memory mutations; rank 0).
    Plan,
    Strict,
    Balanced,
    /// Default full-freedom mode: auto-approves normal coding/shell tools and asks permission
    /// only for dangerous/destructive commands (`rm -rf`, `sudo`, force-push, pipe-to-shell, etc.).
    #[default]
    Autonomous,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Strict => "strict",
            Self::Balanced => "balanced",
            Self::Autonomous => "autonomous",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "plan" | "readonly" | "read_only" => Self::Plan,
            "strict" => Self::Strict,
            "balanced" => Self::Balanced,
            "autonomous" | "auto" => Self::Autonomous,
            _ => Self::Autonomous,
        }
    }

    /// Monotonic privilege rank (`Plan < Strict < Balanced < Autonomous`) for deterministic permission ceilings.
    pub fn rank(self) -> u8 {
        match self {
            Self::Plan => 0,
            Self::Strict => 1,
            Self::Balanced => 2,
            Self::Autonomous => 3,
        }
    }

    /// Returns the stricter (lower-privilege) of two permission modes so child subagents
    /// can never escalate above their parent's permission ceiling (Hermes monotonic guarantee).
    pub fn strictest(a: Self, b: Self) -> Self {
        match a.rank() <= b.rank() {
            true => a,
            false => b,
        }
    }

    pub fn is_read_only_plan(self) -> bool {
        matches!(self, Self::Plan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PermissionRequest,
    SubagentStart,
    SubagentStop,
    Stop,
    SessionEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub event: HookEvent,
    #[serde(default)]
    pub matcher: String,
    pub command: String,
    #[serde(default = "default_hook_timeout")]
    pub timeout_secs: u64,
}

fn default_hook_timeout() -> u64 {
    crate::constants::DEFAULT_HOOK_TIMEOUT_SECS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_hub_dir")]
    pub hub_dir: String,
    #[serde(default = "default_workspace_dir")]
    pub workspace_dir: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_max_tool_iterations")]
    pub max_tool_iterations: usize,
    #[serde(default = "default_daily_spend_limit")]
    pub daily_spend_limit_usd: f64,
    #[serde(default)]
    pub setup_completed: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            hub_dir: default_hub_dir(),
            workspace_dir: default_workspace_dir(),
            log_level: default_log_level(),
            max_tool_iterations: default_max_tool_iterations(),
            daily_spend_limit_usd: default_daily_spend_limit(),
            setup_completed: false,
        }
    }
}

fn default_data_dir() -> String {
    DEFAULT_DATA_DIR.to_string()
}
fn default_hub_dir() -> String {
    DEFAULT_HUB_DIR.to_string()
}
fn default_workspace_dir() -> String {
    DEFAULT_WORKSPACE_DIR.to_string()
}
fn default_log_level() -> String {
    DEFAULT_LOG_LEVEL.to_string()
}
fn default_max_tool_iterations() -> usize {
    DEFAULT_MAX_TOOL_ITERATIONS
}
fn default_daily_spend_limit() -> f64 {
    DEFAULT_DAILY_SPEND_LIMIT_USD
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebUiConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_true")]
    pub require_auth: bool,
}

impl Default for WebUiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind: default_bind(),
            port: default_port(),
            require_auth: true,
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_bind() -> String {
    DEFAULT_WEBUI_BIND.to_string()
}
fn default_port() -> u16 {
    DEFAULT_WEBUI_PORT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default = "default_provider_kind")]
    pub kind: String,
    #[serde(default = "default_openrouter_url")]
    pub base_url: String,
    #[serde(default = "default_lead_model")]
    pub lead_model: String,
    #[serde(default = "default_fast_model")]
    pub fast_model: String,
    #[serde(default = "default_fast_model")]
    pub plan_model: String,
    #[serde(default = "default_fast_model")]
    pub advisor_model: String,
    #[serde(default = "default_fast_model")]
    pub compaction_model: String,
    #[serde(default = "default_fast_model")]
    pub dream_model: String,
    #[serde(default = "default_fallbacks")]
    pub fallbacks: Vec<String>,
    #[serde(default = "default_provider_sort")]
    pub provider_sort: String,
    #[serde(default = "default_true")]
    pub enable_streaming: bool,
    #[serde(default = "default_true")]
    pub enable_prompt_caching: bool,
    #[serde(default = "default_true")]
    pub sticky_session_routing: bool,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: default_provider_kind(),
            base_url: default_openrouter_url(),
            lead_model: default_lead_model(),
            fast_model: default_fast_model(),
            plan_model: default_fast_model(),
            advisor_model: default_fast_model(),
            compaction_model: default_fast_model(),
            dream_model: default_fast_model(),
            fallbacks: default_fallbacks(),
            provider_sort: default_provider_sort(),
            enable_streaming: true,
            enable_prompt_caching: true,
            sticky_session_routing: true,
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
        }
    }
}

impl ProviderConfig {
    /// Unified single-source-of-truth provider switch logic shared by WebUI Composer,
    /// Setup API, and Ratatui TUI wizard. Resets cross-provider model slots whenever
    /// switching to a different provider kind so stale models never leak across providers.
    pub fn switch_provider(
        &mut self,
        kind: &str,
        base_url_override: Option<&str>,
        lead_model_override: Option<&str>,
        sync_fast_model: bool,
    ) -> bool {
        let desc = find_provider(kind);
        let switched = !self.kind.eq_ignore_ascii_case(desc.id);
        self.kind = desc.id.to_string();
        if let Some(url) = base_url_override.map(str::trim).filter(|s| !s.is_empty()) {
            self.base_url = url.to_string();
        } else if switched {
            self.base_url = desc.default_base_url.to_string();
        }
        if switched {
            self.plan_model.clear();
            self.advisor_model.clear();
            self.compaction_model.clear();
            self.dream_model.clear();
            self.fallbacks.clear();
            if lead_model_override.map(str::trim).unwrap_or("").is_empty() {
                self.lead_model.clear();
                self.fast_model.clear();
            }
        }
        if let Some(m) = lead_model_override.map(str::trim).filter(|s| !s.is_empty()) {
            self.lead_model = m.to_string();
            if sync_fast_model {
                self.fast_model = m.to_string();
            }
        }
        switched
    }
}

fn default_provider_kind() -> String {
    DEFAULT_PROVIDER_ID.to_string()
}
fn default_openrouter_url() -> String {
    find_provider(DEFAULT_PROVIDER_ID).default_base_url.to_string()
}
fn default_lead_model() -> String {
    String::new()
}
fn default_fast_model() -> String {
    String::new()
}
fn default_fallbacks() -> Vec<String> {
    Vec::new()
}
fn default_provider_sort() -> String {
    DEFAULT_PROVIDER_SORT.to_string()
}
fn default_temperature() -> f32 {
    DEFAULT_TEMPERATURE
}
fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_tg_mode")]
    pub mode: String,
    #[serde(default = "default_true")]
    pub stream_drafts: bool,
    #[serde(default = "default_poll_timeout")]
    pub poll_timeout_secs: u64,
    #[serde(default)]
    pub default_permission: PermissionMode,
    #[serde(default)]
    pub bot_username: Option<String>,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_tg_mode(),
            stream_drafts: true,
            poll_timeout_secs: default_poll_timeout(),
            default_permission: PermissionMode::Autonomous,
            bot_username: None,
        }
    }
}

fn default_tg_mode() -> String {
    DEFAULT_TELEGRAM_MODE.to_string()
}
fn default_poll_timeout() -> u64 {
    DEFAULT_TELEGRAM_POLL_TIMEOUT_SECS
}

/// Declarative webhook routing rule (`[[webhooks.rules]]` in `zenko.toml`) using RFC 6901
/// JSON pointer matching (zero regex) with built-in cooldown and hourly rate caps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRuleConfig {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Source route segment `/api/webhooks/{source}` (or `*` for any source).
    pub source: String,
    /// Optional RFC 6901 JSON pointer into payload (e.g. `"/action"`, `"/ref"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_pointer: Option<String>,
    /// Allowed scalar string values at `match_pointer` (empty matches any existing value).
    #[serde(default)]
    pub match_values: Vec<String>,
    /// Prompt template supporting `{source}`, `{rule_id}`, and `{payload}` placeholders.
    #[serde(default)]
    pub prompt_template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional Goal Loop ID to wake/resume immediately when this webhook matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_loop_id: Option<String>,
    #[serde(default = "default_webhook_cooldown")]
    pub cooldown_secs: u64,
    #[serde(default = "default_webhook_max_per_hour")]
    pub max_per_hour: usize,
}

fn default_webhook_cooldown() -> u64 {
    WEBHOOK_DEFAULT_COOLDOWN_SECS
}
fn default_webhook_max_per_hour() -> usize {
    WEBHOOK_DEFAULT_MAX_PER_HOUR
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebhooksConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rules: Vec<WebhookRuleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// Hermes-style safety gate: agent/Dream-authored skills save as `status = "draft"` (`enabled = false`)
    /// until explicitly approved by the operator via CLI/WebUI/HTTP.
    #[serde(default = "default_true")]
    pub require_approval: bool,
    /// Allow Dream consolidation to propose reusable procedural skills as drafts after complex runs.
    #[serde(default = "default_true")]
    pub dream_propose_skills: bool,
    #[serde(default = "default_skill_max_history")]
    pub max_history_snapshots: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            require_approval: true,
            dream_propose_skills: true,
            max_history_snapshots: default_skill_max_history(),
        }
    }
}

fn default_skill_max_history() -> usize {
    SKILL_MAX_HISTORY_SNAPSHOTS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WatcherActionKind {
    #[default]
    Prompt,
    Script,
    GoalLoop,
}

impl WatcherActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Script => "script",
            Self::GoalLoop => "goal_loop",
        }
    }
}

/// Declarative low-CPU workspace file-change watcher (`[[watchers]]` in `zenko.toml`)
/// reusing Zenko's zero-regex `CompiledGlob`, debounce, rate caps, and self-edit loop prevention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceWatcherConfig {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_workspace_dir")]
    pub path: String,
    #[serde(default = "default_watcher_glob")]
    pub glob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_glob: Option<String>,
    #[serde(default = "default_watcher_poll")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_watcher_debounce")]
    pub debounce_secs: u64,
    #[serde(default = "default_watcher_cooldown")]
    pub cooldown_secs: u64,
    #[serde(default = "default_watcher_max_per_hour")]
    pub max_per_hour: usize,
    #[serde(default)]
    pub action: WatcherActionKind,
    #[serde(default)]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_loop_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

fn default_watcher_glob() -> String {
    "**/*".to_string()
}
fn default_watcher_poll() -> u64 {
    WATCHER_DEFAULT_POLL_INTERVAL_SECS
}
fn default_watcher_debounce() -> u64 {
    WATCHER_DEFAULT_DEBOUNCE_SECS
}
fn default_watcher_cooldown() -> u64 {
    WATCHER_DEFAULT_COOLDOWN_SECS
}
fn default_watcher_max_per_hour() -> usize {
    WATCHER_DEFAULT_MAX_PER_HOUR
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyConfig {
    #[serde(default)]
    pub mode: PermissionMode,
    #[serde(default = "default_shell_timeout")]
    pub shell_timeout_secs: u64,
    #[serde(default = "default_auto_background_secs")]
    pub auto_background_after_secs: u64,
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
    /// Enforce OS-native Linux Landlock filesystem write sandboxing on spawned shell commands
    /// (confines writes strictly to workspace, hub, `/tmp`, and `$CARGO_HOME`; gracefully degrades on non-Landlock hosts).
    #[serde(default = "default_true")]
    pub os_sandbox: bool,
    #[serde(default = "default_allowed_shell_prefixes")]
    pub allowed_shell_prefixes: Vec<String>,
    #[serde(default = "default_denied_shell_patterns")]
    pub denied_shell_patterns: Vec<String>,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            mode: PermissionMode::Autonomous,
            shell_timeout_secs: default_shell_timeout(),
            auto_background_after_secs: default_auto_background_secs(),
            max_output_bytes: default_max_output_bytes(),
            os_sandbox: true,
            allowed_shell_prefixes: default_allowed_shell_prefixes(),
            denied_shell_patterns: default_denied_shell_patterns(),
        }
    }
}

fn default_shell_timeout() -> u64 {
    DEFAULT_SHELL_TIMEOUT_SECS
}
fn default_auto_background_secs() -> u64 {
    DEFAULT_AUTO_BACKGROUND_SECS
}
fn default_max_output_bytes() -> usize {
    DEFAULT_MAX_OUTPUT_BYTES
}
fn default_allowed_shell_prefixes() -> Vec<String> {
    DEFAULT_ALLOWED_SHELL_PREFIXES
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}
fn default_denied_shell_patterns() -> Vec<String> {
    DEFAULT_DENIED_SHELL_PATTERNS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_recon: bool,
    #[serde(default = "default_true")]
    pub auto_brainstorm: bool,
    #[serde(default = "default_true")]
    pub phase_gating: bool,
    #[serde(default = "default_true")]
    pub adaptive_model_routing: bool,
    #[serde(default = "default_true")]
    pub adaptive_effort: bool,
    #[serde(default = "default_true")]
    pub loop_circuit_breaker: bool,
    #[serde(default = "default_true")]
    pub verification_gate: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_recon: true,
            auto_brainstorm: true,
            phase_gating: true,
            adaptive_model_routing: true,
            adaptive_effort: true,
            loop_circuit_breaker: true,
            verification_gate: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_microcompact_ratio")]
    pub microcompact_trigger_ratio: f64,
    #[serde(default = "default_compaction_ratio")]
    pub compaction_trigger_ratio: f64,
    #[serde(default = "default_true")]
    pub artifact_spilling: bool,
    #[serde(default = "default_true")]
    pub deferred_tool_loading: bool,
    /// Freeze the top-level `tools[]` JSON array across turns so activating deferred/MCP tools
    /// via `tool_search` + `invoke_tool` never invalidates provider KV prompt caches.
    #[serde(default = "default_true")]
    pub freeze_tool_schemas: bool,
    #[serde(default = "default_true")]
    pub auto_repo_rules: bool,
    /// Unified Zenko context & efficiency gear (`lite`, `balanced` [default], `high`, `ultra`).
    #[serde(default)]
    pub zmode: crate::types::ZMode,
    /// Native English output compression intensity (`off`, `lite`, `full`, `ultra`).
    #[serde(default)]
    pub concise_mode: crate::types::ConciseMode,
    /// Automatically relax terse phrasing for security warnings, irreversible confirmations, and Plan Mode trade-offs.
    #[serde(default = "default_true")]
    pub auto_clarity: bool,
    /// Enable Read-Less structural compression on bulky tool outputs with 100% verbatim `artifact:` recovery.
    #[serde(default = "default_true")]
    pub read_less: bool,
    /// Minimum byte threshold before Read-Less structural compression evaluates a tool output.
    #[serde(default = "default_read_less_min_bytes")]
    pub read_less_min_bytes: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            microcompact_trigger_ratio: default_microcompact_ratio(),
            compaction_trigger_ratio: default_compaction_ratio(),
            artifact_spilling: true,
            deferred_tool_loading: true,
            freeze_tool_schemas: true,
            auto_repo_rules: true,
            zmode: crate::types::ZMode::Balanced,
            concise_mode: crate::types::ConciseMode::Lite,
            auto_clarity: true,
            read_less: true,
            read_less_min_bytes: default_read_less_min_bytes(),
        }
    }
}

fn default_microcompact_ratio() -> f64 {
    DEFAULT_MICROCOMPACT_TRIGGER_RATIO
}

fn default_compaction_ratio() -> f64 {
    DEFAULT_COMPACTION_TRIGGER_RATIO
}

fn default_read_less_min_bytes() -> usize {
    READ_LESS_DEFAULT_MIN_BYTES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_true")]
    pub workspace_scoping: bool,
    #[serde(default = "default_true")]
    pub dedup_enabled: bool,
    #[serde(default = "default_true")]
    pub verify_file_staleness: bool,
    #[serde(default = "default_true")]
    pub dream_enabled: bool,
    #[serde(default = "default_dream_min_sessions")]
    pub dream_min_sessions: usize,
    #[serde(default = "default_dream_cooldown_hours")]
    pub dream_cooldown_hours: u64,
    #[serde(default = "default_true")]
    pub dream_auto_patch_docs: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            workspace_scoping: true,
            dedup_enabled: true,
            verify_file_staleness: true,
            dream_enabled: true,
            dream_min_sessions: default_dream_min_sessions(),
            dream_cooldown_hours: default_dream_cooldown_hours(),
            dream_auto_patch_docs: true,
        }
    }
}

fn default_dream_min_sessions() -> usize {
    DREAM_DEFAULT_MIN_SESSIONS
}

fn default_dream_cooldown_hours() -> u64 {
    DREAM_DEFAULT_COOLDOWN_HOURS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_subagent_max_depth")]
    pub max_depth: u8,
    #[serde(default = "default_subagent_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default = "default_true")]
    pub worktrees_enabled: bool,
    #[serde(default = "default_true")]
    pub auto_cleanup_worktrees: bool,
    #[serde(default = "default_true")]
    pub allow_fork_context: bool,
    #[serde(default = "default_subagent_wait_secs")]
    pub default_wait_secs: u64,
    #[serde(default = "default_subagent_max_duration_secs")]
    pub max_duration_secs: u64,
    #[serde(default = "default_subagent_max_cost_usd")]
    pub max_cost_usd: f64,
}

impl Default for SubagentsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_depth: default_subagent_max_depth(),
            max_concurrent: default_subagent_max_concurrent(),
            worktrees_enabled: true,
            auto_cleanup_worktrees: true,
            allow_fork_context: true,
            default_wait_secs: default_subagent_wait_secs(),
            max_duration_secs: default_subagent_max_duration_secs(),
            max_cost_usd: default_subagent_max_cost_usd(),
        }
    }
}

fn default_subagent_max_depth() -> u8 {
    MAX_SUBAGENT_DEPTH
}

fn default_subagent_max_concurrent() -> usize {
    MAX_CONCURRENT_SUBAGENTS
}

fn default_subagent_wait_secs() -> u64 {
    SUBAGENT_DEFAULT_WAIT_SECS
}

fn default_subagent_max_duration_secs() -> u64 {
    SUBAGENT_DEFAULT_MAX_DURATION_SECS
}

fn default_subagent_max_cost_usd() -> f64 {
    SUBAGENT_DEFAULT_MAX_COST_USD
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_checkpoint_max_per_session")]
    pub max_per_session: usize,
    #[serde(default = "default_checkpoint_retention_days")]
    pub retention_days: u64,
    #[serde(default = "default_checkpoint_max_file_bytes")]
    pub max_file_bytes: u64,
    #[serde(default = "default_true")]
    pub preserve_manual_edits: bool,
}

impl Default for CheckpointsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_per_session: default_checkpoint_max_per_session(),
            retention_days: default_checkpoint_retention_days(),
            max_file_bytes: default_checkpoint_max_file_bytes(),
            preserve_manual_edits: true,
        }
    }
}

fn default_checkpoint_max_per_session() -> usize {
    CHECKPOINT_MAX_PER_SESSION
}
fn default_checkpoint_retention_days() -> u64 {
    CHECKPOINT_RETENTION_DAYS
}
fn default_checkpoint_max_file_bytes() -> u64 {
    CHECKPOINT_MAX_FILE_BYTES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `auto` | `searxng` | `tavily` | `brave` | `duckduckgo`
    #[serde(default = "default_web_search_provider")]
    pub provider: String,
    #[serde(default)]
    pub searxng_url: String,
    /// Explicit opt-in allowing self-hosted localhost/LAN SearXNG instances (keeps public SSRF guards intact).
    #[serde(default)]
    pub allow_private_searxng: bool,
    #[serde(default = "default_web_search_limit")]
    pub default_limit: usize,
    #[serde(default = "default_web_search_timeout")]
    pub timeout_secs: u64,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: default_web_search_provider(),
            searxng_url: String::new(),
            allow_private_searxng: false,
            default_limit: default_web_search_limit(),
            timeout_secs: default_web_search_timeout(),
        }
    }
}

fn default_web_search_provider() -> String {
    WEB_SEARCH_DEFAULT_PROVIDER.to_string()
}
fn default_web_search_limit() -> usize {
    WEB_SEARCH_DEFAULT_LIMIT
}
fn default_web_search_timeout() -> u64 {
    WEB_SEARCH_TIMEOUT_SECS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerOverrideConfig {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Automatically sync and attach fresh LSP errors to `hash_edit` / `write_file` / `apply_patch`
    /// results when a warm language server is already running for that workspace root (Oh My Pi pattern).
    #[serde(default = "default_true")]
    pub auto_diagnostics_on_edit: bool,
    #[serde(default = "default_lsp_idle_timeout")]
    pub idle_timeout_secs: u64,
    #[serde(default = "default_lsp_diag_wait_ms")]
    pub diagnostics_wait_ms: u64,
    #[serde(default = "default_lsp_req_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default)]
    pub servers: Vec<LspServerOverrideConfig>,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_diagnostics_on_edit: true,
            idle_timeout_secs: default_lsp_idle_timeout(),
            diagnostics_wait_ms: default_lsp_diag_wait_ms(),
            request_timeout_secs: default_lsp_req_timeout(),
            servers: Vec::new(),
        }
    }
}

fn default_lsp_idle_timeout() -> u64 {
    LSP_IDLE_TIMEOUT_SECS
}
fn default_lsp_diag_wait_ms() -> u64 {
    LSP_DIAGNOSTICS_WAIT_MS
}
fn default_lsp_req_timeout() -> u64 {
    LSP_REQUEST_TIMEOUT_SECS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    #[default]
    Stdio,
    Http,
}

impl McpTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "http" | "https" | "sse" | "streamable_http" => Self::Http,
            _ => Self::Stdio,
        }
    }
}

/// Declarative external MCP server specification (`[[mcp_servers]]` in `zenko.toml`).
/// Synthesizes Codex CLI's allow/deny/timeout model + Hermes's parallel/HTTP transport + Zenko Vault secret indirection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default)]
    pub transport: McpTransport,
    /// Executable path or command name (for `transport = "stdio"`).
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Streamable HTTP endpoint URL (for `transport = "http"`).
    #[serde(default)]
    pub url: String,
    /// Static non-secret env vars passed to stdio child processes.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Map of `ENV_VAR_NAME -> VAULT_SECRET_KEY` resolved securely from `SecretVault` at spawn time (zero plaintext tokens in TOML!).
    #[serde(default)]
    pub env_secrets: BTreeMap<String, String>,
    /// Optional Vault secret key injected as `Authorization: Bearer <secret>` for Streamable HTTP servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header_secret: Option<String>,
    /// Optional extra non-secret HTTP headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub required: bool,
    /// Optional allowlist of remote tool names; if non-empty only listed tools are exposed.
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    /// Explicit denylist of remote tool names (always overrides `enabled_tools`).
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_override: Option<RiskLevel>,
    /// Explicit opt-in trusting this server's MCP `annotations.readOnlyHint == true` to downgrade
    /// read-only tools to `RiskLevel::Safe` (allowing them in Read-Only Plan Mode). Default `false`.
    #[serde(default)]
    pub trust_read_only_hints: bool,
    /// When true, tools from this MCP server may execute concurrently (`ToolConcurrency::Shared`).
    #[serde(default)]
    pub supports_parallel: bool,
    /// Explicit opt-in allowing localhost/LAN HTTP MCP servers.
    #[serde(default)]
    pub allow_private_url: bool,
    #[serde(default = "default_mcp_startup_timeout")]
    pub startup_timeout_secs: u64,
    #[serde(default = "default_mcp_tool_timeout")]
    pub tool_timeout_secs: u64,
    #[serde(default = "default_mcp_idle_timeout")]
    pub idle_timeout_secs: u64,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: McpTransport::Stdio,
            command: String::new(),
            args: Vec::new(),
            url: String::new(),
            env: BTreeMap::new(),
            env_secrets: BTreeMap::new(),
            auth_header_secret: None,
            headers: BTreeMap::new(),
            enabled: true,
            required: false,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
            risk_override: None,
            trust_read_only_hints: false,
            supports_parallel: false,
            allow_private_url: false,
            startup_timeout_secs: default_mcp_startup_timeout(),
            tool_timeout_secs: default_mcp_tool_timeout(),
            idle_timeout_secs: default_mcp_idle_timeout(),
        }
    }
}

impl McpServerConfig {
    /// Deterministic allow/deny filter matching Codex CLI semantics (`disabled_tools` overrides `enabled_tools`).
    pub fn permits_remote_tool(&self, remote_name: &str) -> bool {
        if self.disabled_tools.iter().any(|d| d == remote_name) {
            return false;
        }
        match self.enabled_tools.is_empty() {
            true => true,
            false => self.enabled_tools.iter().any(|e| e == remote_name),
        }
    }
}

fn default_mcp_startup_timeout() -> u64 {
    MCP_STARTUP_TIMEOUT_SECS
}
fn default_mcp_tool_timeout() -> u64 {
    MCP_TOOL_TIMEOUT_SECS
}
fn default_mcp_idle_timeout() -> u64 {
    MCP_IDLE_TIMEOUT_SECS
}

/// Tier-aware Shunt context-protection settings (inspired by Spotify `portal-ai-plugins/shunt`).
/// Intercepts unconstrained full-file reads and unpiped bulk shell dumps (`cat`/`less`/`nl`...)
/// on expensive model tiers while automatically yielding on `fast_model` / Fast-tier subagents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShuntConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: crate::types::ShuntMode,
    #[serde(default = "default_shunt_min_lines")]
    pub min_lines: usize,
}

impl Default for ShuntConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: crate::types::ShuntMode::default(),
            min_lines: default_shunt_min_lines(),
        }
    }
}

fn default_shunt_min_lines() -> usize {
    SHUNT_DEFAULT_MIN_LINES
}

/// Experience-driven cross-session Token Economy settings (inspired by Microsoft TokenEconomizer).
/// All core mechanisms run deterministically with zero extra LLM calls by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomyConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_micro_skills: bool,
    #[serde(default = "default_true")]
    pub auto_tool_prune: bool,
    #[serde(default = "default_true")]
    pub p90_loop_guard: bool,
    #[serde(default)]
    pub optional_fast_judge: bool,
    #[serde(default = "default_economy_micro_skill_min_sessions")]
    pub micro_skill_min_sessions: u32,
    #[serde(default = "default_economy_tool_prune_min_prompts")]
    pub tool_prune_min_prompts: u32,
    #[serde(default = "default_economy_tool_prune_min_sessions")]
    pub tool_prune_min_sessions: u32,
    #[serde(default = "default_economy_tool_prune_reaudit_tasks")]
    pub tool_prune_reaudit_tasks: u32,
    #[serde(default = "default_economy_p90_multiplier")]
    pub p90_multiplier: f64,
    #[serde(default)]
    pub minimalism_mode: crate::types::CodeMinimalismMode,
    #[serde(default = "default_true")]
    pub auto_bloat_hint: bool,
    #[serde(default)]
    pub shunt: ShuntConfig,
}

impl Default for EconomyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_micro_skills: true,
            auto_tool_prune: true,
            p90_loop_guard: true,
            optional_fast_judge: false,
            micro_skill_min_sessions: default_economy_micro_skill_min_sessions(),
            tool_prune_min_prompts: default_economy_tool_prune_min_prompts(),
            tool_prune_min_sessions: default_economy_tool_prune_min_sessions(),
            tool_prune_reaudit_tasks: default_economy_tool_prune_reaudit_tasks(),
            p90_multiplier: default_economy_p90_multiplier(),
            minimalism_mode: crate::types::CodeMinimalismMode::default(),
            auto_bloat_hint: true,
            shunt: ShuntConfig::default(),
        }
    }
}

fn default_economy_micro_skill_min_sessions() -> u32 {
    ECONOMY_MICRO_SKILL_MIN_SESSIONS
}
fn default_economy_tool_prune_min_prompts() -> u32 {
    ECONOMY_TOOL_PRUNE_MIN_PROMPTS
}
fn default_economy_tool_prune_min_sessions() -> u32 {
    ECONOMY_TOOL_PRUNE_MIN_SESSIONS
}
fn default_economy_tool_prune_reaudit_tasks() -> u32 {
    ECONOMY_TOOL_PRUNE_REAUDIT_TASKS
}
fn default_economy_p90_multiplier() -> f64 {
    ECONOMY_LOOP_P90_MULTIPLIER
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ZenkoConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub webui: WebUiConfig,
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub webhooks: WebhooksConfig,
    #[serde(default)]
    pub policy: PolicyConfig,
    #[serde(default)]
    pub orchestrator: OrchestratorConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub economy: EconomyConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub subagents: SubagentsConfig,
    #[serde(default)]
    pub checkpoints: CheckpointsConfig,
    #[serde(default)]
    pub web_search: WebSearchConfig,
    #[serde(default)]
    pub lsp: LspConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub watchers: Vec<WorkspaceWatcherConfig>,
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
}

impl ZenkoConfig {
    pub fn default_config_path() -> PathBuf {
        expand_tilde("~/.zenko/zenko.toml")
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)
            .map_err(|e| ZenkoError::Config(format!("failed to parse {}: {e}", path.display())))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)
            .map_err(|e| ZenkoError::Config(format!("failed to serialize config: {e}")))?;
        std::fs::write(path, raw)?;
        Ok(())
    }

    pub fn resolved_data_dir(&self) -> PathBuf {
        expand_tilde(&self.general.data_dir)
    }

    pub fn resolved_hub_dir(&self) -> PathBuf {
        expand_tilde(&self.general.hub_dir)
    }

    /// Creates the default `~/zenko_hub/` categorized directory structure (`projects/`,
    /// `scripts/`, `media/{images,videos,audio}/`, `artifacts/`) if not yet present.
    pub fn ensure_hub_layout(&self) -> Result<PathBuf> {
        let root = self.resolved_hub_dir();
        std::fs::create_dir_all(&root)?;
        for sub in HUB_REQUIRED_SUBDIRS {
            std::fs::create_dir_all(root.join(sub))?;
        }
        Ok(root)
    }

    /// Lists categorized creations inside `~/zenko_hub/` (`projects/`, `scripts/`, `media/`, `artifacts/`).
    pub fn list_hub_entries(
        &self,
        category_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HubEntry>> {
        let hub_root = self.ensure_hub_layout()?;
        Ok(list_hub_entries_at(&hub_root, category_filter, limit))
    }

    pub fn resolved_workspace_dir(&self) -> PathBuf {
        let p = expand_tilde(&self.general.workspace_dir);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&p))
                .unwrap_or(p)
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.resolved_data_dir().join("zenko.db")
    }

    pub fn vault_path(&self) -> PathBuf {
        self.resolved_data_dir().join("vault.enc")
    }

    pub fn vault_key_path(&self) -> PathBuf {
        self.resolved_data_dir().join("vault.key")
    }

    pub fn socket_path(&self) -> PathBuf {
        self.resolved_data_dir().join("zenko.sock")
    }
}

pub fn list_hub_entries_at(
    hub_root: &Path,
    category_filter: Option<&str>,
    limit: usize,
) -> Vec<HubEntry> {
    let cap = limit.clamp(1, 200);
    let filter = category_filter
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
    let mut out: Vec<HubEntry> = Vec::new();

    for cat in HUB_CATEGORIES {
        if let Some(want) = filter
            && !cat.id.eq_ignore_ascii_case(want) {
                continue;
            }
        let base = hub_root.join(cat.subdir);
        collect_hub_dir(&base, &base, cat.id, 0, &mut out);
    }

    out.sort_by_key(|a| std::cmp::Reverse(a.modified_at));
    out.truncate(cap);
    out
}

fn collect_hub_dir(
    root: &Path,
    dir: &Path,
    category: &str,
    depth: usize,
    out: &mut Vec<HubEntry>,
) {
    if depth > 3 || out.len() >= 256 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for dent in rd.flatten() {
        let path = dent.path();
        let Ok(meta) = dent.metadata() else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let modified_at = meta
            .modified()
            .map(chrono::DateTime::<Utc>::from)
            .unwrap_or_else(|_| Utc::now());
        if meta.is_dir() {
            if depth == 0 && category == HUB_SUBDIR_PROJECTS {
                out.push(HubEntry {
                    category: category.to_string(),
                    rel_path: rel,
                    is_dir: true,
                    size_bytes: 0,
                    modified_at,
                });
            } else {
                collect_hub_dir(root, &path, category, depth + 1, out);
            }
        } else {
            out.push(HubEntry {
                category: category.to_string(),
                rel_path: rel,
                is_dir: false,
                size_bytes: meta.len(),
                modified_at,
            });
        }
    }
}

impl ZenkoConfig {
    /// Atomically applies a declarative `ZMode` preset (`lite`, `balanced`, `high`, `ultra`)
    /// across `ContextConfig` and `EconomyConfig` while preserving all safety & KV-cache invariants.
    pub fn apply_zmode(&mut self, mode: crate::types::ZMode) {
        let d = crate::registry::find_zmode(mode);
        self.context.zmode = d.mode;
        self.context.concise_mode = d.concise_mode;
        self.context.auto_clarity = true;
        self.context.read_less = d.read_less;
        self.context.read_less_min_bytes = d.read_less_min_bytes;
        self.context.microcompact_trigger_ratio = d.microcompact_trigger_ratio;
        self.context.compaction_trigger_ratio = d.compaction_trigger_ratio;
        self.context.artifact_spilling = true;
        self.context.deferred_tool_loading = true;
        self.context.freeze_tool_schemas = true;

        self.economy.enabled = true;
        self.economy.minimalism_mode = d.minimalism_mode;
        self.economy.auto_bloat_hint = d.auto_bloat_hint;
        self.economy.auto_micro_skills = d.auto_micro_skills;
        self.economy.auto_tool_prune = d.auto_tool_prune;
        self.economy.tool_prune_min_prompts = d.tool_prune_min_prompts;
        self.economy.tool_prune_min_sessions = d.tool_prune_min_sessions;
        self.economy.p90_loop_guard = d.p90_loop_guard;
        self.economy.p90_multiplier = d.p90_multiplier;
    }

    /// Returns `true` when individual context or economy settings diverge from the active `ZMode` preset.
    pub fn is_zmode_customized(&self) -> bool {
        let d = crate::registry::find_zmode(self.context.zmode);
        self.context.concise_mode != d.concise_mode
            || self.context.read_less != d.read_less
            || self.context.read_less_min_bytes != d.read_less_min_bytes
            || (self.context.microcompact_trigger_ratio - d.microcompact_trigger_ratio).abs() > 1e-6
            || (self.context.compaction_trigger_ratio - d.compaction_trigger_ratio).abs() > 1e-6
            || self.economy.minimalism_mode != d.minimalism_mode
            || self.economy.auto_bloat_hint != d.auto_bloat_hint
            || self.economy.auto_micro_skills != d.auto_micro_skills
            || self.economy.auto_tool_prune != d.auto_tool_prune
            || self.economy.p90_loop_guard != d.p90_loop_guard
            || !self.economy.enabled
    }
}

pub fn expand_tilde(input: &str) -> PathBuf {
    if let Some(stripped) = input.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if input == "~"
        && let Some(home) = dirs::home_dir() {
            return home;
        }
    PathBuf::from(input)
}

#[cfg(test)]
mod tests {
    use super::ProviderConfig;

    #[test]
    fn switch_provider_resets_cross_provider_model_slots() {
        let mut p = ProviderConfig {
            kind: "openrouter".into(),
            lead_model: "anthropic/claude-sonnet-4".into(),
            fast_model: "openai/gpt-4o-mini".into(),
            ..Default::default()
        };

        p.switch_provider("minimax", None, Some("MiniMax-M3.1-Flash"), true);
        assert_eq!(p.kind, "minimax");
        assert_eq!(p.lead_model, "MiniMax-M3.1-Flash");
        assert_eq!(p.fast_model, "MiniMax-M3.1-Flash");
        assert!(p.base_url.contains("minimax"));
    }

    #[test]
    fn zmode_default_is_balanced_and_apply_roundtrips_cleanly() {
        let mut cfg = super::ZenkoConfig::default();
        assert_eq!(cfg.context.zmode, crate::types::ZMode::Balanced);
        assert!(
            !cfg.is_zmode_customized(),
            "ZenkoConfig::default() must match ZMode::Balanced exactly"
        );

        for desc in crate::registry::ZMODE_REGISTRY {
            cfg.apply_zmode(desc.mode);
            assert_eq!(cfg.context.zmode, desc.mode);
            assert!(!cfg.is_zmode_customized());
            assert!(cfg.context.freeze_tool_schemas);
            assert!(cfg.context.artifact_spilling);
            assert!(cfg.context.auto_clarity);
            assert!(cfg.economy.enabled);
        }

        cfg.apply_zmode(crate::types::ZMode::Balanced);
        cfg.context.concise_mode = crate::types::ConciseMode::Ultra;
        assert!(cfg.is_zmode_customized());
    }
}
