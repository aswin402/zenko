# Zenko — Technical Implementation Plan (`IMPLEMENTATION_PLAN.md`)

**Language & Runtime:** 100% Native Rust (Edition 2024 / MSRV 1.88+), single static binary  
**Build Priority Order:**
1. **Phase 1 (MVP Launch):** Core + Memory + OpenRouter Provider + Native Tools + 24/7 Daemon + CLI Setup (`zenko setup`) + Pure-Rust Customizable WebUI + Telegram Gateway
2. **Phase 2 (24/7 Compounding Autonomy):** Cron/Heartbeats, Sub-Agents & A2A, Context Compaction, Hybrid FTS5+`sqlite-vec` + Dream Consolidation, `SKILL.md` + `/refine`, `rmcp` MCP, Landlock/WASM Sandbox, LSP/DAP, `systemd` installer
3. **Phase 3 (Terminal Workbench):** Detachable Ratatui TUI (`zenko tui`) over existing Daemon IPC
4. **Phase 4 (Multi-Channel Expansion):** Discord, Slack Socket Mode, Matrix, Email

---

## 1. Cargo Workspace & Crate Architecture

```text
zenko/
├── Cargo.toml                     # Workspace root, shared dep versions, release LTO/size profile
├── .cargo/config.toml             # Optional musl static linking & fast mold/lld linker flags
├── docs/
│   ├── PRD.md
│   ├── DESIGN.md
│   ├── IMPLEMENTATION_PLAN.md
│   └── TODO.md
├── config/
│   ├── zenko.example.toml
│   └── execpolicy.example.toml
└── crates/
    ├── zenko-core/                # Shared traits, canonical events, config, UiCustomization schema, errors
    ├── zenko-memory/              # SQLite WAL, event tree, FTS5 + sqlite-vec RRF, Markdown snapshots, Dream
    ├── zenko-providers/           # Shared reqwest(rustls) SSE engine, OpenRouter (primary) + direct adapters
    ├── zenko-tools/               # Hash-anchored edit, apply_patch, ripgrep search, shell, web fetch, MCP, sandbox
    ├── zenko-runtime/             # 24/7 daemon kernel, message bus, ReAct loop, hooks, execpolicy, vault, cron, A2A
    ├── zenko-gateways/            # Feature-gated Telegram (Phase 1) + Webhooks + Discord/Slack/Matrix (Phase 4)
    ├── zenko-webui/               # Axum + Maud + Tailwind v4 Standalone CSS + embedded SVG icon packs + SSE/WS
    ├── zenko-tui/                 # (Phase 3) Detachable Ratatui + Crossterm client over Unix socket / WS
    └── zenko-cli/                 # Final binary `zenko`: setup wizard, daemon, web, ask, gateway, doctor, sandbox
```

### Feature Flag Strategy (Compile Only What You Run)
```toml
[features]
default = ["webui", "gateway-telegram", "gateway-webhooks", "sandbox-landlock"]
webui = ["dep:zenko-webui"]
tui = ["dep:zenko-tui"]
gateway-telegram = ["zenko-gateways/telegram"]
gateway-webhooks = ["zenko-gateways/webhooks"]
gateway-discord = ["zenko-gateways/discord"]
gateway-slack = ["zenko-gateways/slack"]
vector-search = ["zenko-memory/sqlite-vec"]
mcp = ["zenko-tools/rmcp"]
wasm-plugins = ["zenko-tools/wasmtime"]
rhai-scripts = ["zenko-tools/rhai"]
sandbox-landlock = ["zenko-tools/landlock"]
```

---

## 2. Core Crates & Exact Dependency Matrix

Every dependency is chosen for **zero OpenSSL (pure `rustls`)**, **low memory footprint**, and **static `musl` compatibility**:

| Concern | Chosen Crate(s) | Rationale |
| :--- | :--- | :--- |
| **Async Runtime** | `tokio` (tuned features, no unused bloat), `tokio-util` (`CancellationToken`) | Event-driven epoll/kqueue, bounded worker threads (`worker_threads(2)`). |
| **Global Allocator** | `mimalloc` | Eliminates `glibc`/`musl` heap fragmentation across months of 24/7 uptime. |
| **HTTP Client & TLS** | `reqwest` (`default-features = false, features = ["rustls-tls", "json", "stream", "gzip"]`) | One shared `Client` instance across OpenRouter, Telegram, and Web Fetch (~1–2 MB RAM total). |
| **Web Server & WS/SSE** | `axum` 0.8 + `tower-http` (`compression-gzip`, `trace`, `limit`) | Low-RAM async HTTP/WS/SSE server hosting both WebUI and daemon control API. |
| **Pure-Rust HTML & CSS** | `maud` + **Tailwind CSS v4 Standalone CLI** + `include_bytes!` | Compile-time type-safe HTML macros; zero Node.js; zero runtime template parsing. |
| **Database & Search** | `rusqlite` (`bundled`, `fts5`) + optional `sqlite-vec` | Single-file ACID SQLite WAL with hard 2 MB cache cap; zero external DB server. |
| **CLI & Setup Wizard** | `clap` (derive) + `dialoguer` / `inquire` + `owo-colors` | Ergonomic interactive `zenko setup` wizard + clean subcommands. |
| **Code Search & Diffs** | `ignore` + `grep-searcher` + `grep-regex` + `similar` | Native `ripgrep`-grade search + unified diff generation inside Rust. |
| **HTML -> Markdown** | `htmd` or lightweight `scraper`-based cleaner | Fast, low-RAM web page extraction without headless Chrome. |
| **Crypto & Vault** | `chacha20poly1305` + `argon2` + `zeroize` + `rand` + `subtle` | Authenticated secret encryption at rest, constant-time token comparison, zeroized memory. |
| **Observability** | `tracing` + `tracing-subscriber` + custom `broadcast` ring layer | Structured logging streamed live to WebUI `/telemetry` without unbounded RAM buffers. |
| **Cron Parsing** | `croner` or `cron` | Zero-polling next-fire timestamp calculation driving `tokio::time::sleep_until`. |
| **MCP (Phase 2)** | `rmcp` (official Rust MCP SDK) | Stdio + Streamable HTTP MCP client and server. |
| **Sandboxing (Phase 2)** | `landlock` (Linux) + `wasmtime` (opt-in) + `rhai` (opt-in) | Kernel FS/process restrictions + capability-based WASM plugin isolation. |
| **TUI (Phase 3)** | `ratatui` + `crossterm` | Event-driven detachable terminal client. |

---

## 3. Core Abstractions & Traits (`zenko-core`)

### 3.1 Fundamental Traits
```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    async fn stream_chat(
        &self,
        req: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmDelta>> + Send>>>;
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;               // name, description, JSON schema, risk_level
    async fn execute(
        &self,
        ctx: &ToolContext,
        args: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<ToolOutput>;
}

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> ChannelCapabilities; // streaming_draft, inline_buttons, topics
    async fn run(&self, bus: BusHandle, cancel: CancellationToken) -> Result<()>;
    async fn send(&self, msg: OutboundMessage) -> Result<SentMessageRef>;
    async fn update(&self, msg_ref: &SentMessageRef, patch: MessagePatch) -> Result<()>;
}
```

### 3.2 Unified Message Bus & Daemon Protocol
All clients (WebUI, CLI, Telegram, future Ratatui TUI, future Discord/Slack) speak the exact same typed event envelope:
- **Inbound (`InboundMessage`):** `{ id, channel, peer_id, thread_id, session_id, user_text, attachments, metadata }`
- **Live Agent Stream (`AgentEvent`):**
  - `SessionUpdated { session_id, status }`
  - `ThinkingDelta { session_id, turn_id, text }`
  - `TextDelta { session_id, turn_id, text }`
  - `ToolCallStarted { session_id, call_id, tool_name, args_preview, risk }`
  - `ToolApprovalRequested { approval_id, session_id, call_id, summary, risk }`
  - `ToolApprovalResolved { approval_id, decision, resolved_by }`
  - `ToolCallFinished { session_id, call_id, status, duration_ms, diff_preview, output_preview }`
  - `TurnCompleted { session_id, turn_id, usage, cost_usd, cache_read_tokens }`
  - `DaemonTelemetry { rss_bytes, cpu_pct, active_sessions, gateways }`
- **IPC Transports:**
  - Local Unix Domain Socket (`~/.zenko/zenko.sock`) with JSON-RPC 2.0 line framing (used by `zenko ask`, `zenko status`, and later `zenko tui`).
  - Authenticated HTTP REST + SSE (`/api/events`) + WebSocket (`/api/ws`) served by `axum` (used by `zenko-webui` and remote clients).

---

## 4. SQLite WAL Database Schema (`zenko-memory`)

Single file `~/.zenko/zenko.db` initialized with strict low-RAM pragmas:
```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA cache_size = -2000;   -- ~2 MB hard page-cache cap
PRAGMA mmap_size = 0;        -- predictable RSS
PRAGMA temp_store = MEMORY;
```

### Core Tables
1. **`sessions`**: `id (ULID)`, `title`, `channel`, `external_chat_id`, `external_thread_id`, `model_override`, `permission_mode`, `status (idle|running|waiting_approval|paused|error)`, `head_event_id`, `frozen_system_hash`, `total_tokens`, `total_cost_usd`, `created_at`, `updated_at`.
2. **`session_events`** *(Tree-structured durable trajectory — enables crash recovery + assistant-ui branching/edit-resend!)*:
   - `id (ULID)`, `session_id`, `parent_id (nullable ULID)`, `branch_index`, `role (system|user|assistant|tool)`, `kind (message|thinking|tool_call|tool_result|compaction_checkpoint|system_note)`, `content_json`, `tokens_in`, `tokens_out`, `cache_read_tokens`, `cost_usd`, `created_at`.
3. **`approvals`**: `id`, `session_id`, `call_id`, `tool_name`, `args_json`, `risk_level`, `status (pending|approved|denied|always_allowed|expired)`, `resolved_by`, `created_at`, `resolved_at`.
4. **`gateway_peers`**: `channel`, `peer_id`, `username`, `display_name`, `role (owner|trusted|readonly)`, `allowed_tools_json`, `paired_at`.
5. **`pairing_codes`**: `code (6-char)`, `channel`, `role`, `expires_at`, `used_at`.
6. **`memory_entries`** + **`memory_fts (FTS5)`** + optional **`memory_vec (vec0)`**:
   - `id`, `scope (global|project|session)`, `kind (fact|preference|lesson|episode)`, `title`, `content`, `salience`, `last_accessed_at`, `created_at`.
7. **`cron_jobs`**: `id`, `name`, `cron_expr`, `kind (cron|heartbeat)`, `prompt`, `target_channel`, `target_chat_id`, `target_thread_id`, `enabled`, `next_run_at`, `last_run_at`, `last_status`.
8. **`ui_profiles`** *(First-class Component Customization Engine!)*:
   - `id`, `name`, `is_active`, `config_json` (stores full `UiCustomization` struct: mode, color tokens, button style/shape/size/motion, icon pack/stroke/style/size/overrides, surface, bg_pattern, radius, fonts, density, sidebar mode, chat layout, animation preset, orb style, widget toggles), `updated_at`.
9. **`usage_ledger`**: `id`, `session_id`, `provider`, `model`, `prompt_tokens`, `completion_tokens`, `cache_read_tokens`, `cost_usd`, `created_at` (powers daily budget cap & Telemetry counters).

---

## 5. OpenRouter Provider Deep Design (`zenko-providers`)

1. **Endpoint & Headers:**
   - Base URL: `https://openrouter.ai/api/v1` (overridable for proxies/Ollama).
   - Headers: `Authorization: Bearer <injected-from-vault>`, `HTTP-Referer: https://github.com/zenko-agent/zenko`, `X-Title: Zenko Agent`, **`x-session-id: <session_id>`** (activates OpenRouter Sticky Provider Routing for KV-cache hits!).
2. **KV-Cache Optimization (Hermes Pattern + OpenRouter):**
   - Assemble frozen system prefix (`SOUL.md` + `USER.md` + `MEMORY.md` + `ZENKO.md` + Skill summaries) once per session.
   - Attach explicit `cache_control: { "type": "ephemeral" }` markers on the system prompt and last stable history checkpoint when routing to Anthropic/Gemini models via OpenRouter.
3. **Resilience & Per-Role Routing:**
   - Config defines role profiles:
     - `lead` (e.g., `anthropic/claude-sonnet-4.6` or `openai/gpt-5.4` or `deepseek/deepseek-v3.2`)
     - `fast` / `subagent` / `compaction` / `dream` (e.g., `google/gemini-2.5-flash` or `anthropic/claude-haiku-4.5`)
     - `fallbacks`: ordered `models: [...]` array passed natively to OpenRouter so outages transparently fail over.
   - Provider preferences (`provider: { sort: "price" | "throughput", allow_fallbacks: true }`).
4. **Zero-Allocation Streaming SSE Parser:**
   - Reads raw `reqwest::Response::bytes_stream()` line-by-line, parses `data: {...}` chunks without buffering entire responses, emits `ThinkingDelta` (`reasoning` / `reasoning_content` field), `TextDelta`, incremental `ToolCall` argument chunks, and final `usage` (`include: true`) including real USD cost (`cost` field from OpenRouter).
5. **Model Discovery & Spend Guard:**
   - `GET /api/v1/models` cached locally (24h TTL) to populate WebUI & CLI setup model pickers with context lengths, tool-calling capability badges, and $/1M token pricing.
   - Pre-flight spend check against `usage_ledger` daily sum before every LLM call.

---

## 6. Native Tools, Editing & Security (`zenko-tools` + `zenko-runtime`)

### 6.1 High-Reliability File Editing (Oh My Pi + Codex Specialties)
- **`read_file` with Hash-Anchored Lines:** Optionally returns lines formatted as `42:a7f| fn main() {` where `a7f` is a 3-hex-char content hash of line 42.
- **`hash_edit` (Oh My Pi specialty):** Agent targets edits by `(line_number, hash)` anchors — eliminates indentation/whitespace mismatch failures and prevents silent stale edits if the file changed!
- **`apply_patch` (Codex specialty):** Supports structured multi-file diffs with tolerant `seek_sequence` matching, workspace boundary validation, atomic temp-file-and-rename writes, and unified diff output rendered live in WebUI.

### 6.2 Deterministic Lifecycle Hooks & `execpolicy` (Claude Code + Codex Specialties)
Every tool call passes through a deterministic 4-stage pipeline before execution:
1. **Path & Workspace Guard:** Canonicalizes paths (`dunce`/`fs::canonicalize`) and rejects symlink escapes outside allowed roots.
2. **Lifecycle Hook (`PreToolUse`):** Runs configured local command/policy hooks (JSON on `stdin`, exit code `0` allow / `2` deny with feedback message). Prompts CANNOT bypass hooks.
3. **`execpolicy` Evaluator:** Matches `(tool_name, command_ast / args)` against declarative TOML rules -> returns `Allow`, `Deny`, or `AskUser`.
4. **Synchronized HITL Approval:** When `AskUser` triggers, Daemon inserts an `approvals` row and broadcasts `ToolApprovalRequested` to **both WebUI and Telegram simultaneously**. Whichever channel approves/denies first resolves the `oneshot::Sender` and immediately updates the other UI!

### 6.3 Encrypted Credential Vault (IronClaw Specialty)
- Secrets (`OPENROUTER_API_KEY`, `TELEGRAM_BOT_TOKEN`, `WEBUI_ADMIN_HASH`) are encrypted at rest in `~/.zenko/vault.enc` using `ChaCha20-Poly1305` keyed via OS Keyring (or Argon2id-derived master key / machine keyfile with `0600` permissions).
- Decrypted secrets live inside `Zeroizing<String>` wrappers strictly inside `zenko-providers` and `zenko-gateways`. Child shell processes run with a sanitized `env_clear()` allowlist (`PATH`, `HOME`, `TERM`, `LANG`) so an injected prompt executing `env` NEVER leaks API keys!

---

## 7. Pure-Rust WebUI & Customization Engine (`zenko-webui`)

### 7.1 Zero-Node Build Pipeline
1. **CSS Generation:** Uses the official **Tailwind CSS v4 Standalone CLI** (`tailwindcss` single binary) scanning `crates/zenko-webui/src/**/*.rs` against `crates/zenko-webui/assets/input.css` (which defines shadcn/BeUI OKLCH tokens + RareUI/Transitions keyframes + component variant rules).
2. **Embedded Assets:** Compiled minified `app.css`, embedded SVG icon pack sprites (`lucide.svg`, `phosphor.svg`, `tabler.svg`, `heroicons.svg`, `pixel.svg`), and tiny (<15 KB) vanilla `zenko-ui.js` (SSE event stream binder, live theme preview CSS-var updater, `Cmd+K` palette keyboard nav, auto-scroll stick-to-bottom, syntax diff toggles) are embedded at compile time via `include_bytes!` with pre-compressed gzip support and `ETag` caching.
3. **Component Library (`crates/zenko-webui/src/components/`):**
   - `button.rs` — Unified `Button` respecting global/override `ButtonStyle` (`solid`, `soft`, `outline`, `ghost`, `glass`, `glow`, `tactile3d`, `shimmer`), `ButtonShape`, `ButtonSize`, `ButtonMotion`.
   - `icon.rs` — Renders `<svg><use href="/assets/icons/{pack}.svg#{name}"/></svg>` respecting stroke width, style (`outline|duotone|solid`), size, and user icon overrides.
   - `orb.rs` — RareUI-inspired **Zenko Status Orb** (pure CSS/SVG animated variants: `liquid`, `pulse`, `matrix`, `minimal`, `hidden`).
   - `agent/` — BeUI/assistant-ui inspired: `thinking_trace.rs`, `tool_card.rs`, `diff_view.rs`, `approval_banner.rs`, `message_branch.rs`, `composer.rs`.
   - `primitives/` — shadcn/BeautifulUI inspired: `card.rs`, `dialog.rs`, `sheet.rs`, `tabs.rs`, `badge.rs`, `select.rs`, `switch.rs`, `table.rs`, `toast.rs`, `command_palette.rs`, `stat_counter.rs`.
   - `customizer.rs` — Full live **Component Customization Studio** + slide-over Quick Appearance Drawer synced via `PATCH /api/ui/profile` and instant DOM `:root` token updates!

### 7.2 Live Customization Data Contract (`UiCustomization`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiCustomization {
    pub mode: ThemeMode,                    // Dark | Light | System
    pub preset_id: String,                  // obsidian_zinc, midnight_indigo, ...
    pub colors: ColorTokens,                // bg, surface, card, border, fg, muted, primary, accent, ...
    pub buttons: ButtonCustomization,       // style, shape, size, motion
    pub icons: IconCustomization,           // pack, stroke_width, style, size, overrides: HashMap<String, String>
    pub surface: SurfaceCustomization,      // card_style, bg_pattern, border_contrast, radius_px
    pub typography: TypographyConfig,       // ui_font, mono_font, base_size_px
    pub layout: LayoutCustomization,        // density, sidebar_mode, chat_layout, inspector_mode
    pub motion: MotionCustomization,        // preset (snappy|spring|reduced|instant), orb_style
    pub widgets: WidgetToggles,             // show_thinking, show_cost, show_line_numbers, ...
}
```

---

## 8. Telegram Gateway Architecture (`zenko-gateways::telegram`)

Built directly on Zenko's shared `reqwest::Client` (no bloated middleware crates):
1. **Long-Polling Loop (`getUpdates` with `timeout=30`):** Zero open ports or TLS reverse proxies required; resumes cleanly from persisted `last_update_id` in SQLite. Optional webhook endpoint `/webhooks/telegram` also supported.
2. **Pairing & Authorization:**
   - Unpaired users sending `/pair <CODE>` validate against `pairing_codes` -> recorded in `gateway_peers` -> WebUI Setup/Gateways page immediately displays live paired confirmation!
   - All other unauthorized messages are ignored (logged at `debug` level).
3. **Topic-Scoped Sessions:** Maps `(chat_id, message_thread_id.unwrap_or(0))` -> deterministic `session_id` so Forum Topics stay cleanly isolated.
4. **Live Streaming (`sendMessageDraft` + fallback):**
   - Buffers incoming `TextDelta` tokens and flushes via Telegram `sendMessageDraft` every ~350–500 ms (or falls back to throttled `editMessageText` every ~1.2 s for older clients/groups), finishing with a clean final message chunked at Telegram's 4096-char limit with safe HTML/Markdown formatting.
5. **Interactive Tool Approvals:**
   - Subscribes to `ToolApprovalRequested`; sends a concise message with `inline_keyboard`: `[✓ Approve] [✗ Deny] [✓ Always Allow]` (`callback_data = "apr:<id>:allow|deny|always"`).
   - On `callback_query`, immediately calls `answerCallbackQuery`, resolves daemon approval, and edits the button markup to show `✓ Approved by @user` (and clears WebUI banner simultaneously).
6. **Token-Bucket Outbound Queue:**
   - Enforces per-chat (~1 req/s), per-group (20 req/min), and global (30 req/s) limits with automatic `429 retry_after` sleep handling.
7. **Built-in Telegram Slash Commands:** `/new`, `/stop`, `/status`, `/model`, `/mode`, `/skills`, `/compact`, `/help`.

---

## 9. CLI Setup & Operations (`zenko-cli`)

```bash
zenko setup                 # Interactive first-run wizard (OpenRouter, Telegram, WebUI, Policy, Theme)
zenko daemon start          # Starts 24/7 background daemon (WebUI + Gateways + Scheduler + IPC)
zenko daemon status         # Prints PID, uptime, RSS MB, CPU %, active sessions, gateways, spend
zenko daemon stop           # Graceful shutdown (flushes SQLite WAL, cancels streams cleanly)
zenko daemon install        # (Phase 2) Installs hardened systemd --user service unit
zenko web                   # Opens WebUI URL in default browser (or prints LAN/Tailscale URL + token)
zenko ask "..."             # One-shot or interactive CLI prompt over daemon Unix socket
zenko gateway pair telegram # Generates a fresh 6-char pairing code (10 min TTL)
zenko doctor                # Checks SQLite health, OpenRouter key, Telegram bot, Landlock/bwrap support
zenko sandbox test -- <cmd> # (Phase 2) Dry-runs a command under active execpolicy + Landlock profile
zenko tui                   # (Phase 3) Attaches Ratatui TUI thin client to running daemon
```

---

## 10. Low-RAM, Speed & Verification Plan

### Release Build Profile
```toml
[profile.release]
opt-level = "z"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

### Automated Verification & Benchmarks
1. **Unit & Integration Tests (`cargo test --workspace`):**
   - SQLite WAL event tree branching, crash recovery, and FTS5/RRF retrieval tests.
   - OpenRouter SSE parser tests (mocked HTTP server streaming reasoning, tool call deltas, fallback errors, usage/cost parsing).
   - Hash-anchored edit & tolerant `apply_patch` edge-case fuzz/property tests.
   - `execpolicy` & `PreToolUse` hook enforcement + path-jail traversal attack tests.
   - Telegram rate-limiter, 4096-char chunking, topic isolation, pairing, and callback approval tests.
   - WebUI SSR route tests + `UiCustomization` CSS variable generation + XSS HTML-escaping tests (`maud` auto-escapes by default!).
2. **Resource Budget Benchmark (`zenko-bench`):**
   - Automated RSS check via `/proc/<pid>/status` (`VmRSS`) after boot, after 100 WebUI renders, and after 24h simulated idle heartbeat — asserts **idle RSS < 20 MB** and **idle CPU < 0.2%**.
