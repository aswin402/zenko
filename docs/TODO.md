# Zenko — Master Implementation TODO & Roadmap (`TODO.md`)

Priority order: **Core + CLI Setup + Customizable Pure-Rust WebUI + OpenRouter + Telegram Gateway FIRST** -> **24/7 Autonomy & Hardening SECOND** -> **Ratatui TUI THIRD** -> **Additional Gateways FOURTH**.

---

## Milestone 0: Project Scaffolding & Build Infrastructure
- [x] Initialize Cargo workspace root (`Cargo.toml`) with all Phase 1 crates (`zenko-core`, `zenko-memory`, `zenko-providers`, `zenko-tools`, `zenko-runtime`, `zenko-gateways`, `zenko-webui`, `zenko-cli`) and `onpkg.json` manifest
- [x] Configure `[profile.release]` (`opt-level = "z"`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`) and `mimalloc` global allocator
- [x] Configure strict `rustls` TLS policy (zero OpenSSL) across workspace dependencies
- [x] Set up Tailwind CSS v4 Standalone CLI build helper script (`scripts/build-css.sh`) + embedded zero-Node design system stylesheet (`crates/zenko-webui/assets/app.css`)
- [x] Bundle 5 embedded SVG icon packs (`lucide`, `phosphor`, `tabler`, `heroicons`, `pixel`) in `crates/zenko-webui/src/icons.rs`
- [x] Create `config/zenko.example.toml` and `config/execpolicy.example.toml`

---

## Milestone 1: Foundation (`zenko-core`, `zenko-memory`, `zenko-providers`, `zenko-tools`)

### `zenko-core`
- [x] Define core traits: `LlmProvider`, `Tool`, `ToolContext`, `ChannelCapabilities`
- [x] Define canonical domain types: `Session`, `SessionEvent`, `InboundMessage`, `OutboundMessage`, `AgentEvent`, `ApprovalRecord`, `UsageStats`
- [x] Define `ZenkoConfig` (TOML loader/saver + tilde expansion + defaults)
- [x] Define `UiCustomization` full schema (themes, 12 color tokens, 8 button styles/4 shapes/3 sizes/4 motions, 5 icon packs/stroke/style/size/overrides, surfaces, radius, typography, density, sidebar/chat layout, animation presets, orb styles, widget toggles) + 8 built-in presets

### `zenko-memory`
- [x] Implement `SqliteStore` with `rusqlite` (`bundled`), WAL mode, and hard low-RAM pragmas (`cache_size = -2000`, `mmap_size = 0`)
- [x] Write SQL schema for `sessions`, `session_events` (tree with `parent_id`), `approvals`, `gateway_peers`, `pairing_codes`, `memory_entries` + `memory_fts`, `cron_jobs`, `ui_profiles`, `usage_ledger`, `kv_state`
- [x] Implement workspace snapshot manager (`SOUL.md`, `USER.md`, `MEMORY.md`, `ZENKO.md`, `skills/*.md`) with token estimator and frozen prompt hash generator
- [x] Implement session tree operations: append event, reconstruct active branch, fork/branch from past message, crash-recovery scan for interrupted `running` sessions
- [x] Write unit tests for WAL branching and FTS5 search

### `zenko-providers` (OpenRouter First-Class)
- [x] Build shared `reqwest::Client` (`rustls-tls`) + low-allocation line-by-line SSE stream parser
- [x] Implement `OpenRouterProvider`:
  - [x] Streaming chat completions (`TextDelta`, `ThinkingDelta`, incremental `ToolCall` assembly)
  - [x] Sticky provider routing (`x-session-id` + `session_id`) + explicit `cache_control` breakpoints
  - [x] Model fallback array (`models: [...]`) + provider sort (`price` / `throughput`)
  - [x] Per-role model config (`lead`, `fast`, `compaction`, `dream`)
  - [x] Real token/cache/USD cost extraction (`usage: { include: true }`)
  - [x] Live model catalog discovery (`GET /api/v1/models`) & key verification
- [x] Support OpenAI-compatible custom `base_url` (Ollama / vLLM)
- [x] Write SSE streaming & tool-call delta assembly unit tests

### `zenko-tools` (Phase 1 Core Tools)
- [x] Implement canonical workspace path jail helper (prevents `..` and symlink escape)
- [x] Implement `read_file` with **Oh My Pi hash-anchored line format** (`42:a7f| ...`)
- [x] Implement **`hash_edit`** (Oh My Pi specialty) with hash mismatch detection and unified diff output
- [x] Implement **`apply_patch`** (Codex specialty) with tolerant `seek_sequence` matching and atomic file writes
- [x] Implement `write_file` and `list_dir`
- [x] Implement native `code_search` using `ignore` + `grep-searcher` (`ripgrep` engine)
- [x] Implement `shell_exec` using `tokio::process` with sanitized env (`env_clear()`), timeout/cancellation kill, and output truncation
- [x] Implement `web_fetch` using shared `reqwest` + lightweight HTML-to-text cleaner
- [x] Implement `memory_tool` (patch/append/search `USER.md`, `MEMORY.md`, and SQLite FTS5) & `cron_tool`
- [x] Write unit tests for hash-anchored edits, `apply_patch` drift tolerance, and path-jail escape rejection

---

## Milestone 2: 24/7 Core Daemon & CLI Setup (`zenko-runtime`, `zenko-cli`)

### `zenko-runtime`
- [x] Implement encrypted Credential Vault (`ChaCha20-Poly1305` + `0600` keyfile + `zeroize`) with boundary-only secret injection
- [x] Implement unified async broadcast event bus (`broadcast::Sender<AgentEvent>`)
- [x] Implement Core ReAct Agent Loop with frozen KV-cache prompt prefix, streaming LLM -> Tool dispatch -> SQLite event tree, instant `CancellationToken` cancellation, max-iteration guard, and daily OpenRouter spend cap
- [x] Implement Deterministic Lifecycle Hook runner (`UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`)
- [x] Implement declarative `execpolicy` engine (`strict`, `balanced`, `autonomous` + allow/deny rules) and synchronized HITL Approval broker (WebUI <-> Telegram)
- [x] Implement Daemon Control Plane: Unix Domain Socket JSON-RPC server + Linux `/proc/self/status` `VmRSS` telemetry sampler
- [x] Implement `tracing` ring-buffer broadcaster for live WebUI log streaming
- [x] Implement zero-CPU `sleep_until` Cron & Heartbeat scheduler

### `zenko-cli`
- [x] Build CLI subcommands (`clap`): `setup`, `daemon`, `status`, `ask`, `gateway pair|peers`, `web`, `doctor`
- [x] Build interactive **`zenko setup` wizard** (workspace, spend cap, OpenRouter key test, WebUI auth token, optional Telegram verify + pairing code, policy & theme preset)
- [x] Build `zenko doctor` diagnostics

---

## Milestone 3: Customizable Pure-Rust WebUI (`zenko-webui`)

### Server, Auth & Real-Time Transport
- [x] Set up `axum` router embedded in `zenko daemon` (bound to `127.0.0.1` by default, gzip compression, body limits)
- [x] Implement constant-time admin token auth (cookie session + bearer API auth + `/login`)
- [x] Implement full REST endpoints + SSE live event stream (`/api/events`)
- [x] Embed compiled CSS (`app.css`), 5 SVG icon packs, and tiny vanilla client engine (`zenko-ui.js`) directly inside the single binary

### Design System, Components & Full Customization Studio
- [x] Build BeUI / BeautifulUI / RareUI / assistant-ui / shadcn pure-Rust component suite (Thinking Trace, Tool Call Cards with colorized unified diffs, HITL Approval Banners, Message Branching + Edit & Resend, RareUI Living Status Orb, Command Palette `⌘K`, Quick Customizer Sheet `⌘.`)
- [x] Build **First-Run Web Setup Wizard (`/setup`)**
- [x] Build **Agent Workbench (`/` & `/sessions/:id`)**
- [x] Build **Component Customization Studio (`/settings/appearance`)** with live preview stage, 8 presets, 12 color pickers, 8 button styles × 4 shapes × 3 sizes × 4 motions, 5 icon packs + stroke/style/size/per-slot overrides, surfaces, textures, radius, fonts, density, sidebar/chat layouts, orb styles, widget toggles, named profiles, and JSON export/import
- [x] Build **Gateways Console (`/gateways`)**, **Memory & Skills Studio (`/memory`)**, **Cron & Policy (`/automation`)**, and **Telemetry & Live Logs (`/telemetry`)**
- [x] Add end-to-end Axum router & UI profile persistence integration test

---

## Milestone 4: Production Telegram Gateway (`zenko-gateways`)
- [x] Implement direct Telegram Bot API client over shared `reqwest::Client` (`rustls`)
- [x] Implement long-polling runner (`getUpdates`, persisted `offset` in SQLite) with hot-reload `Notify`
- [x] Implement global & per-chat rate limiter honoring `429 retry_after`
- [x] Implement 6-char `/pair <CODE>` flow + strict allowlist & role enforcement
- [x] Implement Forum Topics session isolation (`(chat_id, message_thread_id)`)
- [x] Implement native **`sendMessageDraft`** live token streaming with automatic throttled `editMessageText` fallback & safe 4096-char chunking
- [x] Implement interactive **Inline Keyboard Tool Approvals** synchronized live with WebUI
- [x] Implement Telegram slash commands (`/start`, `/pair`, `/new`, `/stop`, `/status`, `/model`, `/mode`, `/help`)
- [x] Implement HMAC-SHA256 webhook signature verifier (`zenko-gateways::verify_hmac_sha256`)

---

## Milestone 5: Phase 2 — Advanced Autonomy, Memory & Hardening
- [ ] Recursive Sub-Agents & A2A Messaging (`spawn_agent`, `message_agent`, `wait_agent`) with Claude-Code summary contract
- [ ] Multi-Layer Context Auto-Compaction (auto-triggers at ~75% context window, preserving active goals & modified files)
- [ ] Upgrade `zenko-memory` with **`sqlite-vec`** + SQL **Reciprocal Rank Fusion (RRF)** hybrid search
- [ ] Background **"Dream" Memory Consolidator** (idle-window episodic -> `MEMORY.md` + candidate `SKILL.md`)
- [ ] Prime-Agent-style **`/refine`** trajectory evaluator & skill auto-refinement
- [ ] Official **`rmcp` MCP client & server** (stdio + Streamable HTTP)
- [ ] **Linux `Landlock` + `Bubblewrap` + `Seccomp` tool sandbox** + `zenko sandbox test` CLI verifier
- [ ] Optional feature-gated **`wasmtime` WASM plugin sandbox**, **`rhai` script control tool**, and **LSP/DAP bridge**
- [ ] `zenko daemon install` hardened `systemd --user` service generator (`MemoryMax=128M`, `WatchdogSec=60`)

---

## Milestone 6: Phase 3 — Detachable Ratatui TUI (`zenko-tui`)
- [ ] Implement thin IPC client connecting `zenko tui` to running daemon Unix socket (`~/.zenko/zenko.sock`)
- [ ] Build event-driven `ratatui` + `crossterm` dashboard (sessions tree, streaming Markdown & diffs, `[A]/[D]` approvals, live RSS/cost bar, theme synced with active WebUI profile)

---

## Milestone 7: Phase 4 — Additional Gateways & Release Certification
- [ ] Feature-gated **Discord** (`twilight-gateway`), **Slack** (Socket Mode WS), **Matrix**, and **Email** gateways
- [ ] 24-hour soak test & RSS/CPU benchmark certification + static `musl` (`x86_64` & `aarch64`) release builds
