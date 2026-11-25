//! Data-driven registries for LLM providers, messaging gateways, and streaming controls.
//! Zero hardcoded model lists, context windows, or pricing — models are discovered live
//! from each provider's `/models` API and hydrated via the live metadata registry.

use crate::{
    ConciseMode, SUBAGENT_ROLE_CUSTOM, TOOL_AGENT_KILL, TOOL_AGENT_LIST, TOOL_AGENT_MESSAGE,
    TOOL_AGENT_WAIT, TOOL_AGENT_WORKTREE, TOOL_APPLY_PATCH, TOOL_ASK_CLARIFICATION,
    TOOL_BATCH_TOOLS, TOOL_CODE_SEARCH, TOOL_CONTEXT_INSPECT, TOOL_CRON, TOOL_GIT, TOOL_GLOB_FIND,
    TOOL_GOAL_LOOP, TOOL_HASH_EDIT, TOOL_INVOKE_TOOL, TOOL_LIST_DIR, TOOL_LSP, TOOL_MEMORY,
    TOOL_ORCHESTRATE_CONTROL, TOOL_PLAN_UPDATE, TOOL_READ_FILE, TOOL_RECORD_BRAINSTORM,
    TOOL_SCRIPT_BLOCK, TOOL_SESSION_SEARCH, TOOL_SESSION_TASK, TOOL_SHELL_EXEC, TOOL_SKILL_MANAGE,
    TOOL_SKILL_VIEW, TOOL_SPAWN_AGENT, TOOL_TOKEN_ECONOMY, TOOL_TOOL_SEARCH, TOOL_WEB_FETCH,
    TOOL_WEB_SEARCH, TOOL_WRITE_FILE, ToolSet, VAULT_KEY_BRAVE_SEARCH, VAULT_KEY_CUSTOM_PROVIDER,
    VAULT_KEY_DEEPSEEK, VAULT_KEY_GEMINI, VAULT_KEY_MINIMAX, VAULT_KEY_OLLAMA, VAULT_KEY_OPENAI,
    VAULT_KEY_OPENROUTER, VAULT_KEY_TAVILY_SEARCH, VAULT_KEY_TELEGRAM, VAULT_KEY_WEBHOOK_SECRET,
};
use serde::Serialize;

/// Wire protocol dialect handled by `zenko-providers` via pure `match` dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDialect {
    OpenRouter,
    MiniMax,
    OpenAiCompat,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProviderDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default_base_url: &'static str,
    /// Provider key(s) inside the live `models.dev` metadata registry used to hydrate
    /// context limits, output limits, pricing, tool-calling, and reasoning flags.
    pub metadata_keys: &'static [&'static str],
    pub requires_api_key: bool,
    pub is_local_free: bool,
    pub vault_secret_key: &'static str,
    pub supports_prompt_caching: bool,
    pub supports_sticky_routing: bool,
    pub dialect: ProviderDialect,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct GatewayDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub vault_secret_key: &'static str,
    pub token_prompt: &'static str,
    pub supports_pairing: bool,
    pub supports_draft_stream: bool,
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingOptionId {
    ResponseStreaming,
    PromptCaching,
    StickySessionRouting,
    TelegramDraftStreaming,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct StreamingOptionDescriptor {
    pub id: StreamingOptionId,
    pub label: &'static str,
    pub description: &'static str,
}

/// Dynamic model-tier selector for specialized subagent roles (zero hardcoded model IDs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentModelTier {
    Fast,
    Plan,
    Lead,
    Advisor,
}

/// Declarative default workspace isolation mode per subagent role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentWorkspaceDefault {
    Inherit,
    Worktree,
}

/// Declarative metadata descriptor for each `ToolSet` domain (zero hardcoding across CLI/WebUI/Tools/Runtime).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ToolSetDescriptor {
    pub id: ToolSet,
    pub slug: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

pub const TOOLSET_REGISTRY: &[ToolSetDescriptor] = &[
    ToolSetDescriptor {
        id: ToolSet::CoreFs,
        slug: "core_fs",
        label: "Core Filesystem & Batch",
        description: "Hash-anchored reading, structural outlines, multi-hunk edits, patches, directory listing, and composable batches",
    },
    ToolSetDescriptor {
        id: ToolSet::CodeIntel,
        slug: "code_intel",
        label: "Code Search & Intelligence",
        description: "Fast `.gitignore`-aware code search, glob file discovery, and IDE-grade Language Server Protocol navigation/diagnostics",
    },
    ToolSetDescriptor {
        id: ToolSet::Execution,
        slug: "execution",
        label: "Shell & Background Jobs",
        description: "Policy-guarded shell execution, background process streaming, interactive stdin, wait/stop, and task supervision",
    },
    ToolSetDescriptor {
        id: ToolSet::Git,
        slug: "git",
        label: "Git & Version Control",
        description: "Structured status, diff, log, blame, branching, staging, and guarded commits",
    },
    ToolSetDescriptor {
        id: ToolSet::Web,
        slug: "web",
        label: "Web Search & Fetch",
        description: "Pluggable web search providers and SSRF-hardened web page fetching",
    },
    ToolSetDescriptor {
        id: ToolSet::Memory,
        slug: "memory",
        label: "Memory, Skills & Context",
        description: "Persistent hybrid memory, cross-session search, progressive skill loading/management, tool discovery, and context inspection",
    },
    ToolSetDescriptor {
        id: ToolSet::Orchestration,
        slug: "orchestration",
        label: "Orchestration & Planning",
        description: "Iterative brainstorming, execution plans, SQLite task DAGs, live posture/tier control, and structured user clarifications",
    },
    ToolSetDescriptor {
        id: ToolSet::Subagents,
        slug: "subagents",
        label: "Multi-Agent Delegation",
        description: "Async subagent spawning, worktree isolation, inter-agent mailbox messaging, event-driven waiting, and lifecycle management",
    },
    ToolSetDescriptor {
        id: ToolSet::Automation,
        slug: "automation",
        label: "Cron & Autonomous Goals",
        description: "Recurring scheduled tasks and persistent budget-bounded autonomous goal loops",
    },
    ToolSetDescriptor {
        id: ToolSet::Mcp,
        slug: "mcp",
        label: "External MCP Integrations",
        description: "Dynamic Model Context Protocol remote tools and resources",
    },
];

pub fn find_toolset_descriptor(set: ToolSet) -> &'static ToolSetDescriptor {
    TOOLSET_REGISTRY
        .iter()
        .find(|d| d.id == set)
        .unwrap_or(&TOOLSET_REGISTRY[0])
}

/// Single-source-of-truth declarative subagent role profile shared by Runtime, Tools, CLI, API & WebUI.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SubagentRoleDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub model_tier: SubagentModelTier,
    pub default_workspace: SubagentWorkspaceDefault,
    pub max_iterations: usize,
    pub read_only: bool,
    pub can_spawn_children: bool,
    /// When non-empty, ONLY tools listed here are visible/callable by this role.
    pub allowed_tools: &'static [&'static str],
    /// Explicit tools forbidden for this role (applied even when `allowed_tools` is empty).
    pub denied_tools: &'static [&'static str],
    pub system_addendum: &'static str,
}

impl SubagentRoleDescriptor {
    /// Deterministic allowlist/denylist check (zero regex, zero hardcoding).
    pub fn permits_tool(&self, tool_name: &str) -> bool {
        if self.denied_tools.contains(&tool_name) {
            return false;
        }
        if !self.can_spawn_children && tool_name == TOOL_SPAWN_AGENT {
            return false;
        }
        match self.allowed_tools.is_empty() {
            true => true,
            false => self.allowed_tools.contains(&tool_name),
        }
    }
}

pub const SUBAGENT_ROLE_REGISTRY: &[SubagentRoleDescriptor] = &[
    SubagentRoleDescriptor {
        id: "explorer",
        label: "Explorer",
        description: "Fast read-only codebase & memory reconnaissance worker (strictly zero mutations)",
        model_tier: SubagentModelTier::Fast,
        default_workspace: SubagentWorkspaceDefault::Inherit,
        max_iterations: 8,
        read_only: true,
        can_spawn_children: false,
        allowed_tools: &[
            TOOL_READ_FILE,
            TOOL_LIST_DIR,
            TOOL_CODE_SEARCH,
            TOOL_GLOB_FIND,
            TOOL_BATCH_TOOLS,
            TOOL_INVOKE_TOOL,
            TOOL_GIT,
            TOOL_LSP,
            TOOL_WEB_FETCH,
            TOOL_WEB_SEARCH,
            TOOL_MEMORY,
            TOOL_SESSION_SEARCH,
            TOOL_SKILL_VIEW,
            TOOL_TOOL_SEARCH,
            TOOL_CONTEXT_INSPECT,
            TOOL_SESSION_TASK,
            TOOL_AGENT_LIST,
            TOOL_AGENT_MESSAGE,
        ],
        denied_tools: &[
            TOOL_WRITE_FILE,
            TOOL_HASH_EDIT,
            TOOL_APPLY_PATCH,
            TOOL_SHELL_EXEC,
            TOOL_SCRIPT_BLOCK,
            TOOL_CRON,
            TOOL_ASK_CLARIFICATION,
            TOOL_SPAWN_AGENT,
            TOOL_AGENT_KILL,
            TOOL_AGENT_WORKTREE,
        ],
        system_addendum: "You are a Zenko Explorer subagent operating in strict read-only isolation. Investigate the assigned scope rapidly using read-only tools, never attempt file writes or shell commands, and return a concise, factual summary citing exact file paths and line numbers.",
    },
    SubagentRoleDescriptor {
        id: "architect",
        label: "Architect",
        description: "Deep design & decomposition strategist; may fan out parallel read-only explorers",
        model_tier: SubagentModelTier::Plan,
        default_workspace: SubagentWorkspaceDefault::Inherit,
        max_iterations: 10,
        read_only: true,
        can_spawn_children: true,
        allowed_tools: &[
            TOOL_READ_FILE,
            TOOL_LIST_DIR,
            TOOL_CODE_SEARCH,
            TOOL_GLOB_FIND,
            TOOL_BATCH_TOOLS,
            TOOL_INVOKE_TOOL,
            TOOL_GIT,
            TOOL_LSP,
            TOOL_WEB_FETCH,
            TOOL_WEB_SEARCH,
            TOOL_MEMORY,
            TOOL_SESSION_SEARCH,
            TOOL_SKILL_VIEW,
            TOOL_TOOL_SEARCH,
            TOOL_CONTEXT_INSPECT,
            TOOL_TOKEN_ECONOMY,
            TOOL_RECORD_BRAINSTORM,
            TOOL_PLAN_UPDATE,
            TOOL_SESSION_TASK,
            TOOL_SPAWN_AGENT,
            TOOL_AGENT_LIST,
            TOOL_AGENT_MESSAGE,
            TOOL_AGENT_WAIT,
            TOOL_AGENT_KILL,
        ],
        denied_tools: &[
            TOOL_WRITE_FILE,
            TOOL_HASH_EDIT,
            TOOL_APPLY_PATCH,
            TOOL_SHELL_EXEC,
            TOOL_SCRIPT_BLOCK,
            TOOL_CRON,
            TOOL_ASK_CLARIFICATION,
            TOOL_AGENT_WORKTREE,
        ],
        system_addendum: "You are a Zenko Architect subagent operating in read-only planning mode. Analyze trade-offs, invariants, blast radius, and step dependencies; optionally fan out parallel `explorer` subagents for wide codebase discovery; return a crisp, actionable implementation specification without modifying files.",
    },
    SubagentRoleDescriptor {
        id: "coder",
        label: "Coder",
        description: "Full-capability implementation worker defaulting to an isolated git worktree",
        model_tier: SubagentModelTier::Lead,
        default_workspace: SubagentWorkspaceDefault::Worktree,
        max_iterations: 18,
        read_only: false,
        can_spawn_children: false,
        allowed_tools: &[],
        denied_tools: &[
            TOOL_CRON,
            TOOL_GOAL_LOOP,
            TOOL_SKILL_MANAGE,
            TOOL_ORCHESTRATE_CONTROL,
            TOOL_ASK_CLARIFICATION,
            TOOL_SPAWN_AGENT,
            TOOL_AGENT_WORKTREE,
        ],
        system_addendum: "You are a Zenko Coder subagent working in an isolated workspace. Implement the assigned goal cleanly following repo rules, run targeted verification checks/tests, and return a concise report listing modified files, verification output, and any remaining caveats.",
    },
    SubagentRoleDescriptor {
        id: "verifier",
        label: "Verifier",
        description: "Independent test, lint, and audit verifier (may run tests; cannot edit source files)",
        model_tier: SubagentModelTier::Advisor,
        default_workspace: SubagentWorkspaceDefault::Inherit,
        max_iterations: 10,
        read_only: false,
        can_spawn_children: false,
        allowed_tools: &[],
        denied_tools: &[
            TOOL_WRITE_FILE,
            TOOL_HASH_EDIT,
            TOOL_APPLY_PATCH,
            TOOL_CRON,
            TOOL_GOAL_LOOP,
            TOOL_SKILL_MANAGE,
            TOOL_ORCHESTRATE_CONTROL,
            TOOL_ASK_CLARIFICATION,
            TOOL_SPAWN_AGENT,
            TOOL_AGENT_KILL,
            TOOL_AGENT_WORKTREE,
        ],
        system_addendum: "You are a Zenko Verifier subagent. Independently audit code changes, run build/test/lint commands to confirm correctness, hunt for regressions or edge cases, and never modify source files. Report exact pass/fail evidence.",
    },
    SubagentRoleDescriptor {
        id: "critic",
        label: "Critic",
        description: "Adversarial gap & blind-spot reviewer (Metis + Momus style; strictly read-only)",
        model_tier: SubagentModelTier::Advisor,
        default_workspace: SubagentWorkspaceDefault::Inherit,
        max_iterations: 10,
        read_only: true,
        can_spawn_children: false,
        allowed_tools: &[
            TOOL_READ_FILE,
            TOOL_LIST_DIR,
            TOOL_CODE_SEARCH,
            TOOL_GLOB_FIND,
            TOOL_BATCH_TOOLS,
            TOOL_INVOKE_TOOL,
            TOOL_GIT,
            TOOL_LSP,
            TOOL_MEMORY,
            TOOL_SESSION_SEARCH,
            TOOL_SKILL_VIEW,
            TOOL_TOOL_SEARCH,
            TOOL_CONTEXT_INSPECT,
            TOOL_TOKEN_ECONOMY,
            TOOL_RECORD_BRAINSTORM,
            TOOL_SESSION_TASK,
            TOOL_AGENT_LIST,
            TOOL_AGENT_MESSAGE,
        ],
        denied_tools: &[
            TOOL_WRITE_FILE,
            TOOL_HASH_EDIT,
            TOOL_APPLY_PATCH,
            TOOL_SHELL_EXEC,
            TOOL_SCRIPT_BLOCK,
            TOOL_CRON,
            TOOL_ASK_CLARIFICATION,
            TOOL_SPAWN_AGENT,
            TOOL_AGENT_KILL,
            TOOL_AGENT_WORKTREE,
        ],
        system_addendum: "You are an independent Zenko Critic subagent in strict read-only mode. Stress-test the proposed design, plan, or diff ruthlessly: uncover hidden assumptions, blind spots, edge cases, race conditions, security flaws, and simpler alternatives grounded in inspected files. Never edit files; return sharp, actionable critique.",
    },
    SubagentRoleDescriptor {
        id: "researcher",
        label: "Researcher",
        description: "Focused web, documentation, memory, and cross-session history researcher (strictly read-only)",
        model_tier: SubagentModelTier::Fast,
        default_workspace: SubagentWorkspaceDefault::Inherit,
        max_iterations: 10,
        read_only: true,
        can_spawn_children: false,
        allowed_tools: &[
            TOOL_WEB_SEARCH,
            TOOL_WEB_FETCH,
            TOOL_MEMORY,
            TOOL_SESSION_SEARCH,
            TOOL_SKILL_VIEW,
            TOOL_READ_FILE,
            TOOL_LIST_DIR,
            TOOL_CODE_SEARCH,
            TOOL_GLOB_FIND,
            TOOL_BATCH_TOOLS,
            TOOL_INVOKE_TOOL,
            TOOL_TOOL_SEARCH,
            TOOL_CONTEXT_INSPECT,
            TOOL_AGENT_LIST,
            TOOL_AGENT_MESSAGE,
        ],
        denied_tools: &[
            TOOL_WRITE_FILE,
            TOOL_HASH_EDIT,
            TOOL_APPLY_PATCH,
            TOOL_SHELL_EXEC,
            TOOL_SCRIPT_BLOCK,
            TOOL_CRON,
            TOOL_ASK_CLARIFICATION,
            TOOL_SPAWN_AGENT,
            TOOL_AGENT_KILL,
            TOOL_AGENT_WORKTREE,
        ],
        system_addendum: "You are a Zenko Researcher subagent in strict read-only mode. Gather authoritative external documentation, web findings, prior session context, and repository facts rapidly and concisely, citing exact sources, URLs, or file paths.",
    },
    SubagentRoleDescriptor {
        id: SUBAGENT_ROLE_CUSTOM,
        label: "Custom Specialist",
        description: "Dynamic LLM-defined specialist persona with custom directive, cognitive tier, and optional read-only/tool scope",
        model_tier: SubagentModelTier::Lead,
        default_workspace: SubagentWorkspaceDefault::Inherit,
        max_iterations: 14,
        read_only: false,
        can_spawn_children: false,
        allowed_tools: &[],
        denied_tools: &[
            TOOL_CRON,
            TOOL_GOAL_LOOP,
            TOOL_SKILL_MANAGE,
            TOOL_ASK_CLARIFICATION,
            TOOL_SPAWN_AGENT,
            TOOL_AGENT_WORKTREE,
            TOOL_ORCHESTRATE_CONTROL,
        ],
        system_addendum: "You are a dynamically scoped Zenko specialist subagent. Execute the parent's assigned directive cleanly within your permitted tool scope and workspace, and return a concise, evidence-backed result.",
    },
];

pub fn find_subagent_role(id: &str) -> Option<&'static SubagentRoleDescriptor> {
    let trimmed = id.trim();
    SUBAGENT_ROLE_REGISTRY
        .iter()
        .find(|r| r.id.eq_ignore_ascii_case(trimmed))
}

pub const PROVIDER_REGISTRY: &[ProviderDescriptor] = &[
    ProviderDescriptor {
        id: "openrouter",
        label: "OpenRouter",
        description: "Unified router (live free & frontier models, fallbacks, sticky KV caching)",
        default_base_url: "https://openrouter.ai/api/v1",
        metadata_keys: &["openrouter"],
        requires_api_key: true,
        is_local_free: false,
        vault_secret_key: VAULT_KEY_OPENROUTER,
        supports_prompt_caching: true,
        supports_sticky_routing: true,
        dialect: ProviderDialect::OpenRouter,
    },
    ProviderDescriptor {
        id: "minimax",
        label: "MiniMax",
        description: "MiniMax OpenAI-compatible API with interleaved thinking & tool use",
        default_base_url: "https://api.minimax.io/v1",
        metadata_keys: &["minimax", "minimax-cn"],
        requires_api_key: true,
        is_local_free: false,
        vault_secret_key: VAULT_KEY_MINIMAX,
        supports_prompt_caching: true,
        supports_sticky_routing: false,
        dialect: ProviderDialect::MiniMax,
    },
    ProviderDescriptor {
        id: "openai",
        label: "OpenAI",
        description: "Direct OpenAI Chat Completions API",
        default_base_url: "https://api.openai.com/v1",
        metadata_keys: &["openai"],
        requires_api_key: true,
        is_local_free: false,
        vault_secret_key: VAULT_KEY_OPENAI,
        supports_prompt_caching: true,
        supports_sticky_routing: false,
        dialect: ProviderDialect::OpenAiCompat,
    },
    ProviderDescriptor {
        id: "deepseek",
        label: "DeepSeek",
        description: "DeepSeek OpenAI-compatible reasoning & chat API",
        default_base_url: "https://api.deepseek.com/v1",
        metadata_keys: &["deepseek"],
        requires_api_key: true,
        is_local_free: false,
        vault_secret_key: VAULT_KEY_DEEPSEEK,
        supports_prompt_caching: true,
        supports_sticky_routing: false,
        dialect: ProviderDialect::OpenAiCompat,
    },
    ProviderDescriptor {
        id: "gemini",
        label: "Google Gemini",
        description: "Google AI Studio OpenAI-compatible endpoint",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        metadata_keys: &["google"],
        requires_api_key: true,
        is_local_free: false,
        vault_secret_key: VAULT_KEY_GEMINI,
        supports_prompt_caching: true,
        supports_sticky_routing: false,
        dialect: ProviderDialect::OpenAiCompat,
    },
    ProviderDescriptor {
        id: "ollama",
        label: "Ollama (Local)",
        description: "Local offline inference — 100% free, zero API key required",
        default_base_url: "http://127.0.0.1:11434/v1",
        metadata_keys: &[],
        requires_api_key: false,
        is_local_free: true,
        vault_secret_key: VAULT_KEY_OLLAMA,
        supports_prompt_caching: false,
        supports_sticky_routing: false,
        dialect: ProviderDialect::OpenAiCompat,
    },
    ProviderDescriptor {
        id: "custom",
        label: "Custom OpenAI-Compatible",
        description: "vLLM, llama.cpp, LiteLLM, or private proxy endpoint",
        default_base_url: "http://127.0.0.1:8000/v1",
        metadata_keys: &[],
        requires_api_key: false,
        is_local_free: false,
        vault_secret_key: VAULT_KEY_CUSTOM_PROVIDER,
        supports_prompt_caching: false,
        supports_sticky_routing: false,
        dialect: ProviderDialect::OpenAiCompat,
    },
];

pub const GATEWAY_REGISTRY: &[GatewayDescriptor] = &[
    GatewayDescriptor {
        id: "telegram",
        label: "Telegram",
        description: "Long-polling bot, live draft stream, topics & inline approvals",
        vault_secret_key: VAULT_KEY_TELEGRAM,
        token_prompt: "Paste @BotFather Bot Token",
        supports_pairing: true,
        supports_draft_stream: true,
        available: true,
    },
    GatewayDescriptor {
        id: "webhooks",
        label: "HMAC Webhooks",
        description: "Signed HTTP webhook triggers via X-Zenko-Signature",
        vault_secret_key: VAULT_KEY_WEBHOOK_SECRET,
        token_prompt: "Set HMAC-SHA256 Signing Secret (leave blank to auto-generate)",
        supports_pairing: false,
        supports_draft_stream: false,
        available: true,
    },
];

pub const STREAMING_REGISTRY: &[StreamingOptionDescriptor] = &[
    StreamingOptionDescriptor {
        id: StreamingOptionId::ResponseStreaming,
        label: "Response Streaming (SSE)",
        description: "Stream assistant tokens live to WebUI & CLI as generated",
    },
    StreamingOptionDescriptor {
        id: StreamingOptionId::PromptCaching,
        label: "KV Prompt Caching",
        description: "Pin frozen system & memory prefix for up to 80% lower input cost",
    },
    StreamingOptionDescriptor {
        id: StreamingOptionId::StickySessionRouting,
        label: "Sticky Session Routing",
        description: "Pin session requests to the same upstream provider for cache hits",
    },
    StreamingOptionDescriptor {
        id: StreamingOptionId::TelegramDraftStreaming,
        label: "Telegram Live Drafts",
        description: "Stream live response drafts via sendMessageDraft in Telegram",
    },
];

/// Declarative Web Search provider descriptor (zero hardcoded provider tables in tools or UI).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct WebSearchProviderDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub requires_api_key: bool,
    pub vault_secret_key: Option<&'static str>,
}

pub const WEB_SEARCH_PROVIDER_REGISTRY: &[WebSearchProviderDescriptor] = &[
    WebSearchProviderDescriptor {
        id: "auto",
        label: "Auto Cascade",
        description: "SearXNG (if URL set) -> Tavily -> Brave -> DuckDuckGo zero-key fallback",
        requires_api_key: false,
        vault_secret_key: None,
    },
    WebSearchProviderDescriptor {
        id: "searxng",
        label: "SearXNG (Self-Hosted)",
        description: "Private JSON metasearch via your own SearXNG instance",
        requires_api_key: false,
        vault_secret_key: None,
    },
    WebSearchProviderDescriptor {
        id: "tavily",
        label: "Tavily Search",
        description: "AI-optimized search snippets & answers (Vault: TAVILY_API_KEY)",
        requires_api_key: true,
        vault_secret_key: Some(VAULT_KEY_TAVILY_SEARCH),
    },
    WebSearchProviderDescriptor {
        id: "brave",
        label: "Brave Search",
        description: "Independent web index REST API (Vault: BRAVE_API_KEY)",
        requires_api_key: true,
        vault_secret_key: Some(VAULT_KEY_BRAVE_SEARCH),
    },
    WebSearchProviderDescriptor {
        id: "duckduckgo",
        label: "DuckDuckGo (Zero-Key)",
        description: "Built-in HTML state-machine fallback requiring zero API keys",
        requires_api_key: false,
        vault_secret_key: None,
    },
];

/// Declarative Language Server Protocol (LSP) registry for `lsp_tool` (zero hardcoded binary tables).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LspServerDescriptor {
    pub id: &'static str,
    pub language_id: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub root_markers: &'static [&'static str],
}

pub const LSP_SERVER_REGISTRY: &[LspServerDescriptor] = &[
    LspServerDescriptor {
        id: "rust-analyzer",
        language_id: "rust",
        command: "rust-analyzer",
        args: &[],
        extensions: &["rs"],
        root_markers: &["Cargo.toml"],
    },
    LspServerDescriptor {
        id: "typescript-language-server",
        language_id: "typescript",
        command: "typescript-language-server",
        args: &["--stdio"],
        extensions: &["ts", "tsx", "js", "jsx", "mjs", "cjs"],
        root_markers: &["tsconfig.json", "jsconfig.json", "package.json"],
    },
    LspServerDescriptor {
        id: "pyright-langserver",
        language_id: "python",
        command: "pyright-langserver",
        args: &["--stdio"],
        extensions: &["py", "pyi"],
        root_markers: &["pyproject.toml", "pyrightconfig.json", "setup.py"],
    },
    LspServerDescriptor {
        id: "gopls",
        language_id: "go",
        command: "gopls",
        args: &["serve"],
        extensions: &["go"],
        root_markers: &["go.work", "go.mod"],
    },
    LspServerDescriptor {
        id: "clangd",
        language_id: "cpp",
        command: "clangd",
        args: &["--background-index"],
        extensions: &["c", "h", "cc", "cpp", "cxx", "hpp", "hh"],
        root_markers: &["compile_commands.json", "CMakeLists.txt", ".clangd"],
    },
];

pub fn find_lsp_server_by_extension(ext: &str) -> Option<&'static LspServerDescriptor> {
    let clean = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    LSP_SERVER_REGISTRY
        .iter()
        .find(|s| s.extensions.iter().any(|e| *e == clean))
}

pub fn find_lsp_server_by_id(id: &str) -> Option<&'static LspServerDescriptor> {
    LSP_SERVER_REGISTRY
        .iter()
        .find(|s| s.id.eq_ignore_ascii_case(id.trim()))
}

pub fn find_provider(id: &str) -> &'static ProviderDescriptor {
    PROVIDER_REGISTRY
        .iter()
        .find(|p| p.id.eq_ignore_ascii_case(id))
        .unwrap_or(&PROVIDER_REGISTRY[0])
}

pub fn find_gateway(id: &str) -> Option<&'static GatewayDescriptor> {
    GATEWAY_REGISTRY
        .iter()
        .find(|g| g.id.eq_ignore_ascii_case(id))
}

/// Declarative English conciseness profile descriptor (shared by prompt builder, runtime, CLI, and WebUI).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ConciseModeDescriptor {
    pub mode: ConciseMode,
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub target_reduction_pct: u8,
    pub directive: &'static str,
}

pub const CONCISE_MODE_REGISTRY: &[ConciseModeDescriptor] = &[
    ConciseModeDescriptor {
        mode: ConciseMode::Off,
        id: "off",
        label: "Off",
        description: "Standard full English prose with no output compression",
        target_reduction_pct: 0,
        directive: "",
    },
    ConciseModeDescriptor {
        mode: ConciseMode::Lite,
        id: "lite",
        label: "Lite",
        description: "Filler-free professional English: drop pleasantries and hedging, keep clear sentences",
        target_reduction_pct: 30,
        directive: "Write crisp, direct English. Drop pleasantries, throat-clearing, hedging, and repetition while keeping clear complete sentences.",
    },
    ConciseModeDescriptor {
        mode: ConciseMode::Full,
        id: "full",
        label: "Full",
        description: "Telegraphic English (ASD-STE100 style): short active-voice fragments, zero fluff",
        target_reduction_pct: 60,
        directive: "Write ultra-terse telegraphic English: drop filler, articles, and padding; use short active-voice fragments and bullets. State each fact once.",
    },
    ConciseModeDescriptor {
        mode: ConciseMode::Ultra,
        id: "ultra",
        label: "Ultra",
        description: "Maximum English token density: minimal fragments, arrows (A -> B), zero narrative padding",
        target_reduction_pct: 75,
        directive: "Write maximum-density telegraphic English: minimal fragments, causal arrows (`cause -> effect -> fix`), compact bullet lines, zero narrative padding. State each fact once.",
    },
];

pub fn find_concise_mode(mode: ConciseMode) -> &'static ConciseModeDescriptor {
    CONCISE_MODE_REGISTRY
        .iter()
        .find(|d| d.mode == mode)
        .unwrap_or(&CONCISE_MODE_REGISTRY[0])
}

/// Declarative built-in English workflow skill seeded via `SnapshotManager::ensure_defaults()`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct DefaultSkillTemplate {
    pub filename: &'static str,
    pub content: &'static str,
}

pub const DEFAULT_SKILL_TEMPLATES: &[DefaultSkillTemplate] = &[
    DefaultSkillTemplate {
        filename: "systematic-debug.md",
        content: "---\n\
name: Systematic Debugging\n\
description: Reproduce, isolate root cause, apply minimal fix, and verify with tests.\n\
enabled: true\n\
---\n\n\
1. Read the full error message and stack trace before editing.\n\
2. Search relevant symbols using `code_search` and inspect lines with `read_file`.\n\
3. Form a single falsifiable root-cause hypothesis and verify it.\n\
4. Apply a minimal fix via `hash_edit` or `apply_patch`, then run tests via `shell_exec`.\n",
    },
    DefaultSkillTemplate {
        filename: "concise-review.md",
        content: "---\n\
name: Concise Code Review\n\
description: Ultra-terse English code review returning one-line findings (`L<line>: bug|risk|nit|q: <problem>. <fix>.`) with zero filler.\n\
enabled: true\n\
---\n\n\
- Format every finding strictly on one line: `path:L<line> [bug|risk|nit|q]: <problem>. <fix>.`\n\
- Zero throat-clearing, pleasantries, or restating unchanged code.\n\
- Preserve exact symbol names, line numbers, error codes, and file paths verbatim.\n\
- Auto-Clarity: expand to clear full English sentences for security flaws, data-loss risks, or architectural regressions.\n\
- End with a single verdict line (`approve`, `changes_requested`, or `blocked`) plus finding counts.\n",
    },
    DefaultSkillTemplate {
        filename: "concise-commit.md",
        content: "---\n\
name: Concise Commit\n\
description: Generate crisp English Conventional Commit messages from staged diffs without blindly staging or pushing.\n\
enabled: true\n\
---\n\n\
- Inspect staged changes via `git_tool(action=\"diff\", staged=true)` and `git_tool(action=\"status\")`.\n\
- Format subject as `<type>(<scope>): <imperative lowercase summary>` (<= 72 chars, no trailing period).\n\
- Add terse body bullets ONLY for non-obvious rationale or breaking changes (`BREAKING CHANGE:`).\n\
- Never auto-stage, commit, or push unless explicitly requested by the user.\n",
    },
    DefaultSkillTemplate {
        filename: "concise-explore.md",
        content: "---\n\
name: Concise Explore\n\
description: Fast read-only codebase reconnaissance returning exact path:line citations and dense technical facts.\n\
enabled: true\n\
---\n\n\
- Use `glob_find`, `code_search`, and `read_file(mode=\"outline\")` first; read targeted line ranges only.\n\
- Report findings strictly as `path:Lstart-Lend -- <symbol/fact>` bullets.\n\
- Zero speculative filler; note unverified gaps explicitly in one line.\n",
    },
    DefaultSkillTemplate {
        filename: "token-economy-audit.md",
        content: "---\n\
name: Token Economy Audit\n\
description: Audit per-repository token savings, corroborated micro-skills, soft-pruned tools, and P90 loop guard baselines.\n\
enabled: true\n\
---\n\n\
- Call `invoke_tool(tool=\"token_economy\", args={\"action\":\"status\"})` to inspect net tokens saved, active micro-skills, pruned tools, and P90 turn envelope.\n\
- Retire any stale or contradicted repository micro-skill via `invoke_tool(tool=\"token_economy\", args={\"action\":\"retire_micro_skill\",\"id\":\"...\"})`.\n\
- Soft-demote persistently unused non-core tools via `apply_prune` (still reachable via `invoke_tool`) or pin/restore required tools via `restore_tool`.\n\
- Report net ROI honestly: gross savings (Read-Less + Tool Pruning + Micro-Skills + Loop Guard) minus background Dream/Judge overhead.\n",
    },
];

/// Renders corroborated active repository micro-skills into a compact Tier-2 XML block.
/// Returns `None` when no active corroborated skills exist (zero prompt bloat).
pub fn render_repo_micro_skills_block(skills: &[crate::types::RepoMicroSkill]) -> Option<String> {
    let active: Vec<&crate::types::RepoMicroSkill> = skills
        .iter()
        .filter(|s| s.status == crate::types::MicroSkillStatus::Active && !s.fact.trim().is_empty())
        .take(crate::constants::ECONOMY_MICRO_SKILL_MAX_INJECT)
        .collect();
    if active.is_empty() {
        return None;
    }
    let mut lines = Vec::with_capacity(active.len() + 3);
    lines.push("<repo_micro_skills>".to_string());
    lines.push("Verified workspace shortcuts learned from prior sessions in this repository (auto-retired if contradicted):".to_string());
    for s in active {
        lines.push(format!("- [{}x] {}", s.corroborations.max(1), s.fact.trim()));
    }
    lines.push("</repo_micro_skills>".to_string());
    Some(lines.join("\n"))
}

/// Renders a compact, KV-cache-safe Tier-2 directive informing the orchestrator of Zenko's active
/// Token Economy engine and deferred `token_economy` tool (even when Caveman Mode is `Off`).
/// Returns `None` when the economy engine is disabled or the active role cannot access `token_economy`.
pub fn render_economy_prompt_block(
    enabled: bool,
    can_use_tool: bool,
    read_only_plan: bool,
) -> Option<String> {
    if !enabled || !can_use_tool {
        return None;
    }
    let body = match read_only_plan {
        true => format!(
            "- Background micro-skills, soft tool pruning, Read-Less compression, and 1.25x P90 loop guard are active for this repository.\n\
             - Read-only inspection: use `{TOOL_INVOKE_TOOL}(tool=\"{TOOL_TOKEN_ECONOMY}\", args={{\"action\":\"status\"|\"micro_skills\"|\"prune_report\"|\"ledger\"}})` or load `token-economy-audit` via `{TOOL_SKILL_VIEW}` when auditing token efficiency."
        ),
        false => format!(
            "- Background micro-skill mining, soft tool pruning, Read-Less compression, and 1.25x P90 loop guard run automatically for this repository.\n\
             - Proactive hygiene: persist verified non-shell repo workarounds via `{TOOL_INVOKE_TOOL}(tool=\"{TOOL_TOKEN_ECONOMY}\", args={{\"action\":\"add_micro_skill\",\"fact\":\"...\"}})`, retire stale `<repo_micro_skills>` entries via `retire_micro_skill`, or load `token-economy-audit` via `{TOOL_SKILL_VIEW}` when auditing token waste or restoring/pruning tools."
        ),
    };
    Some(format!("<token_economy>\n{body}\n</token_economy>"))
}

/// Renders the Tier-2 English `<concise_mode>` directive block (returns `None` when `Off`).
/// Enforces Verbatim Payload Protection and optional Auto-Clarity safety hatch with zero Chinese terms.
pub fn render_concise_prompt_block(
    mode: ConciseMode,
    auto_clarity: bool,
    read_only_plan: bool,
) -> Option<String> {
    if !mode.is_active() {
        return None;
    }
    let desc = find_concise_mode(mode);
    let clarity_clause = match (auto_clarity, read_only_plan) {
        (true, true) => "\n- Auto-Clarity (Plan Mode active): use full, unmistakable English sentences for security warnings, irreversible trade-offs, and architectural proposals.",
        (true, false) => "\n- Auto-Clarity: automatically relax compression into clear full English sentences for security warnings, destructive/irreversible confirmations, architectural disagreements, or user confusion.",
        (false, _) => "",
    };
    let efficiency_clause = match read_only_plan {
        true => "\n- Autonomous efficiency: load matching concise skills (`concise-review`, `concise-explore`) via `skill_view` when reviewing diffs or exploring codebases.",
        false => "\n- Autonomous efficiency: load matching concise skills (`concise-review`, `concise-commit`, `concise-explore`) via `skill_view` when reviewing, committing, or exploring; save recurring multi-step shell routines via `script_block`; compact bloated memory docs via `memory_tool(action=\"compress_doc\")`.",
    };
    Some(format!(
        "<concise_mode level=\"{}\">\n\
         - {}\n\
         - Verbatim invariant: NEVER alter or abbreviate code blocks, symbol names, file paths, CLI commands, stack traces, line numbers, or exact error messages.{clarity_clause}{efficiency_clause}\n\
         - Mode control: switch live via `orchestrate_control(concise_mode=\"off|lite|full|ultra\")` if the user asks to change verbosity.\n\
         </concise_mode>",
        desc.id, desc.directive
    ))
}

/// Pure arithmetic hex `#RRGGBB` parser (zero regex, zero heap allocation).
pub fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let bytes = hex.trim().strip_prefix('#').unwrap_or(hex.trim()).as_bytes();
    match bytes {
        [r1, r2, g1, g2, b1, b2] => Some((
            hex_pair(*r1, *r2)?,
            hex_pair(*g1, *g2)?,
            hex_pair(*b1, *b2)?,
        )),
        _ => None,
    }
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_pair(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_nibble(hi)? << 4) | hex_nibble(lo)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_rgb_without_regex() {
        assert_eq!(parse_hex_rgb("#ea5829"), Some((0xea, 0x58, 0x29)));
        assert_eq!(parse_hex_rgb("#0a0a0b"), Some((10, 10, 11)));
        assert_eq!(parse_hex_rgb("invalid"), None);
    }

    #[test]
    fn provider_registry_lookup_works() {
        assert_eq!(find_provider("openrouter").id, "openrouter");
        let mm = find_provider("minimax");
        assert_eq!(mm.id, "minimax");
        assert_eq!(mm.dialect, ProviderDialect::MiniMax);
        assert_eq!(mm.default_base_url, "https://api.minimax.io/v1");
        assert_eq!(mm.vault_secret_key, VAULT_KEY_MINIMAX);
        assert!(mm.metadata_keys.contains(&"minimax"));
        assert!(!find_provider("ollama").requires_api_key);
        assert_eq!(find_provider("unknown").id, "openrouter");
    }

    #[test]
    fn subagent_role_registry_enforces_tool_permissions() {
        let explorer = find_subagent_role("explorer").expect("explorer");
        assert!(explorer.read_only);
        assert!(!explorer.can_spawn_children);
        assert!(explorer.permits_tool("read_file"));
        assert!(explorer.permits_tool("code_search"));
        assert!(!explorer.permits_tool("write_file"));
        assert!(!explorer.permits_tool("shell_exec"));
        assert!(!explorer.permits_tool(TOOL_SPAWN_AGENT));

        let coder = find_subagent_role("coder").expect("coder");
        assert_eq!(coder.default_workspace, SubagentWorkspaceDefault::Worktree);
        assert!(coder.permits_tool("write_file"));
        assert!(coder.permits_tool("shell_exec"));
        assert!(!coder.permits_tool("cron_tool"));
        assert!(!coder.permits_tool(TOOL_SPAWN_AGENT));

        let architect = find_subagent_role("architect").expect("architect");
        assert!(architect.can_spawn_children);
        assert!(architect.permits_tool(TOOL_SPAWN_AGENT));
        assert!(!architect.permits_tool("write_file"));

        let verifier = find_subagent_role("verifier").expect("verifier");
        assert!(verifier.permits_tool("shell_exec"));
        assert!(!verifier.permits_tool("write_file"));
        assert!(!verifier.permits_tool("apply_patch"));

        for role in SUBAGENT_ROLE_REGISTRY {
            assert!(
                !role.permits_tool(TOOL_ASK_CLARIFICATION),
                "subagent role `{}` must never spawn user clarification cards",
                role.id
            );
            assert!(
                !role.permits_tool(TOOL_AGENT_WORKTREE),
                "subagent role `{}` must never mutate sibling worktrees",
                role.id
            );
        }
    }

    #[test]
    fn permission_mode_strictest_is_monotonic() {
        use crate::PermissionMode;
        assert_eq!(
            PermissionMode::strictest(PermissionMode::Plan, PermissionMode::Strict),
            PermissionMode::Plan
        );
        assert_eq!(
            PermissionMode::strictest(PermissionMode::Autonomous, PermissionMode::Plan),
            PermissionMode::Plan
        );
        assert_eq!(
            PermissionMode::strictest(PermissionMode::Strict, PermissionMode::Balanced),
            PermissionMode::Strict
        );
        assert_eq!(
            PermissionMode::strictest(PermissionMode::Balanced, PermissionMode::Strict),
            PermissionMode::Strict
        );
        assert_eq!(
            PermissionMode::strictest(PermissionMode::Autonomous, PermissionMode::Balanced),
            PermissionMode::Balanced
        );
        assert_eq!(
            PermissionMode::strictest(PermissionMode::Autonomous, PermissionMode::Autonomous),
            PermissionMode::Autonomous
        );
    }

    #[test]
    fn concise_mode_registry_is_pure_english_and_roundtrips() {
        for desc in CONCISE_MODE_REGISTRY {
            assert_eq!(ConciseMode::parse(desc.id), Some(desc.mode));
            assert_eq!(desc.mode.as_str(), desc.id);
            assert!(
                desc.directive.is_ascii() && desc.description.is_ascii() && desc.label.is_ascii(),
                "concise mode `{}` must stay strictly ASCII English",
                desc.id
            );
        }
        assert!(render_concise_prompt_block(ConciseMode::Off, true, false).is_none());
        let ultra = render_concise_prompt_block(ConciseMode::Ultra, true, false).expect("ultra");
        assert!(ultra.contains("level=\"ultra\""));
        assert!(ultra.contains("Verbatim invariant"));
        assert!(ultra.contains("Auto-Clarity"));
        assert!(ultra.contains("script_block"));
        assert!(ultra.contains("compress_doc"));
        assert!(ultra.is_ascii());

        let plan_ro = render_concise_prompt_block(ConciseMode::Full, true, true).expect("plan_ro");
        assert!(plan_ro.contains("concise-review"));
        assert!(!plan_ro.contains("script_block"));
        assert!(!plan_ro.contains("compress_doc"));
        assert!(plan_ro.is_ascii());

        for tmpl in DEFAULT_SKILL_TEMPLATES {
            assert!(tmpl.filename.ends_with(".md") && tmpl.content.is_ascii());
        }
    }

    #[test]
    fn economy_prompt_block_respects_flags_and_read_only_mode() {
        assert!(render_economy_prompt_block(false, true, false).is_none());
        assert!(render_economy_prompt_block(true, false, false).is_none());

        let full = render_economy_prompt_block(true, true, false).expect("full");
        assert!(full.contains("<token_economy>"));
        assert!(full.contains(TOOL_TOKEN_ECONOMY));
        assert!(full.contains("add_micro_skill"));
        assert!(full.contains("token-economy-audit"));

        let ro = render_economy_prompt_block(true, true, true).expect("ro");
        assert!(ro.contains("Read-only inspection"));
        assert!(!ro.contains("add_micro_skill"));
        assert!(ro.contains("token-economy-audit"));

        let architect = find_subagent_role("architect").expect("architect");
        let critic = find_subagent_role("critic").expect("critic");
        assert!(architect.permits_tool(TOOL_TOKEN_ECONOMY));
        assert!(critic.permits_tool(TOOL_TOKEN_ECONOMY));
    }
}
