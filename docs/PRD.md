# Zenko — Product Requirements Document (PRD)

**Project Name:** Zenko (`zenko`)  
**Version:** 1.0 (Planning Baseline)  
**Target Stack:** 100% Native Rust Single-Binary Runtime (`x86_64` / `aarch64` Linux & macOS)  
**Primary Initial Scope:** Core Daemon + CLI Setup + Customizable Pure-Rust WebUI + OpenRouter Provider + Telegram Gateway *(Ratatui TUI in Phase 3)*

---

## 1. Executive Summary & Vision

**Zenko** is a 24/7, local-first, ultra-low-resource autonomous AI agent written natively in Rust. While popular agent frameworks (OpenClaw, nanobot, Claude Code, Hermes Agent, Prime Agent, Oh My Pi) proved what persistent AI agents can accomplish, most rely on heavy Node.js or Python runtimes consuming **500 MB–2 GB of RAM**, slow cold starts, and fragile runtime dependencies.

Zenko synthesizes the best architectural breakthroughs of the 2025–2026 agent ecosystem into a **single static Rust binary** targeting **~5–15 MB idle RSS**, **~0.0% idle CPU**, **<10 ms startup**, kernel-level sandboxing, a deeply customizable **shadcn/Tailwind-grade WebUI**, an omnichannel **Telegram-first gateway**, and an interactive **CLI setup wizard** — with a detachable **Ratatui TUI** added cleanly on top of the exact same daemon protocol.

---

## 2. Competitive Intelligence & Borrowed Specialties

Zenko intentionally distills the single strongest superpower from each leading autonomous agent:

| Reference Agent | Signature Specialties Analyzed | How Zenko Adopts & Improves It |
| :--- | :--- | :--- |
| **Claude Code** *(Anthropic)* | • Deterministic Lifecycle Hooks (`PreToolUse`, `PostToolUse`, `Stop`, `PermissionRequest`, etc.)<br>• Subagent Summary Contract (isolated child context returns concise summary)<br>• Multi-layer context compaction preserving invariants<br>• Per-action permission modes & path-scoped rules (`CLAUDE.md`) | Native Rust Hook Engine (`zenko-hooks`) executing deterministic shell/HTTP/policy gates that LLM prompts cannot override; project rules via `ZENKO.md` + `.zenko/rules/`; Tokio subagent summary contracts. |
| **Hermes Agent** *(Nous Research)* | • KV-cache-friendly frozen prompt/memory snapshots (`USER.md`, `MEMORY.md`)<br>• Self-authoring & self-refining `SKILL.md` (`agentskills.io`)<br>• Instant mid-flight cancellation<br>• Native scheduled cron delivering to any messaging channel | Frozen prefix assembly paired with OpenRouter sticky `session_id` caching (up to 80% input cost reduction); `CancellationToken` wired through HTTP streams & OS process groups; SQLite-backed `sleep_until` cron. |
| **Prime Agent** *(Prime Intellect)* | • Detached daemon-backed marathon execution (survives disconnect/reboot)<br>• Durable tree-structured trajectories for crash recovery & branching<br>• Recursive subagents + Agent-to-Agent (A2A) messaging<br>• Continual Harness (`/refine`) evidence-backed improvement<br>• Autonomous heartbeats | 100% detached `zenko daemon` with SQLite WAL event tree (`parent_id`), auto-resume after crash/reboot, recursive Tokio subagent A2A mailbox, `/refine` trajectory reviewer, and pure-Rust optional `rhai` script control layer (replacing heavy Python/IPython). |
| **Oh My Pi (`omp`)** | • **Hash-anchored line editing** (eliminates whitespace/line-drift edit failures)<br>• Strict `Plan -> Inspect -> Edit -> Run -> Debug -> Test -> Verify` loop<br>• Built-in LSP & DAP code intelligence/debugging<br>• Per-role model routing + multi-format doc parsing | Built-in **hash-anchored edit tool** + tolerant patcher in Rust; per-role OpenRouter model routing (`lead`, `fast`, `subagent`, `dream`, `compaction`); modular LSP/DAP client tools in Phase 2. |
| **OpenAI Codex CLI (`codex-rs`)** | • Pure-Rust rewrite for speed & safety<br>• Kernel sandboxing (`Landlock` + `Bubblewrap` + `Seccomp` / macOS `Seatbelt`) + standalone `sandbox` tester<br>• Structured tolerant `apply_patch` (`seek_sequence`)<br>• Declarative `execpolicy` & typed JSON-RPC 2.0 App Server | Native Linux Landlock/Bubblewrap sandbox + `zenko sandbox` CLI verifier; atomic `apply_patch` with tolerant seeking; declarative `execpolicy.toml`; typed JSON-RPC 2.0 + SSE/WS daemon control plane shared by WebUI, CLI, IDEs, and later Ratatui TUI. |
| **nanobot / ZeroClaw / IronClaw** | • Unified `InboundMessage` / `OutboundMessage` message bus<br>• Background **"Dream" memory consolidation**<br>• Single-file SQLite FTS5 + `sqlite-vec` hybrid search<br>• WASM (`wasmtime`) plugin sandbox + boundary-injected credential vault | Single-file SQLite WAL (2 MB cache cap) with FTS5 + `sqlite-vec` RRF; idle-window Dream consolidator; encrypted `chacha20poly1305` secret vault where secrets are injected strictly at outbound HTTP/gateway boundaries (never exposed to LLM or tool env). |

---

## 3. Target Users & Core Use Cases

1. **Solo Developers & Indie Builders:** Run a 24/7 personal coding, research, and ops agent on a $4–$6/month VPS, home server, Raspberry Pi, or laptop without burning gigabytes of RAM.
2. **Power Users Managing Autonomous Workflows via Mobile:** Trigger tasks, approve sensitive shell/git actions via Telegram inline buttons, receive scheduled briefings/alerts, and inspect deep traces later in the WebUI.
3. **Self-Hosters & Privacy-Conscious Teams:** Single-binary zero-dependency deployment, strict kernel sandboxing, boundary-isolated credentials, full local SQLite audit logs, and OpenRouter BYOK or local Ollama compatibility.

---

## 4. Functional Requirements

### 4.1 Phase 1 — Launch MVP (Core + CLI Setup + WebUI + OpenRouter + Telegram)

#### FR-1: CLI & Guided First-Run Setup (`zenko-cli`)
- `zenko setup` (interactive terminal wizard) AND first-run WebUI Setup Wizard:
  - Create `~/.zenko/` config & SQLite database, initialize `SOUL.md`, `USER.md`, `MEMORY.md`, `ZENKO.md`.
  - Configure **OpenRouter API Key** (encrypted into vault), validate connectivity, fetch model catalog, and pick default + fallback + lightweight models.
  - Configure **WebUI** bind address, port, and admin access token/passcode.
  - Configure **Telegram Gateway** bot token, generate a one-time **6-character pairing code**, and configure initial allowed chat IDs.
  - Select default **Execution Policy** (`strict`, `balanced`, `autonomous`).
- Core CLI commands: `zenko daemon [start|stop|restart|status]`, `zenko setup`, `zenko ask "<prompt>"`, `zenko web`, `zenko doctor` (health/sandbox/connectivity diagnostics), `zenko gateway pair`, `zenko config`, `zenko sandbox test`.

#### FR-2: 24/7 Core Daemon & Agent Engine (`zenko-runtime` + `zenko-core`)
- Detached background daemon managing all active sessions, gateways, schedulers, and WebUI concurrently.
- Unified internal async message bus (`InboundMessage` / `OutboundMessage` / `AgentEvent`).
- Durable append-only SQLite WAL event tree supporting session creation, branching, message editing/resending, crash recovery, and resumption.
- KV-cache-friendly frozen prompt assembly (`SOUL.md` + `USER.md` + `MEMORY.md` + active `ZENKO.md` rules + skill index).
- Full mid-flight cancellation (`CancellationToken`) for LLM streams and running child tool processes.
- Deterministic Lifecycle Hook pipeline (`PreToolUse`, `PostToolUse`, `PermissionRequest`, `UserPromptSubmit`, `Stop`, `SessionStart`, `SessionEnd`).
- Declarative `execpolicy` permission engine with interactive Human-in-the-Loop (HITL) approvals routed simultaneously to WebUI and Telegram.

#### FR-3: OpenRouter First-Class Provider (`zenko-providers`)
- Native streaming SSE client built on a single shared `reqwest` (`rustls-tls`) connection pool.
- Full tool/function calling with streaming delta assembly and strict JSON schema validation.
- Sticky prompt caching via `x-session-id` / `session_id` + explicit `cache_control` breakpoints for Anthropic/Claude models via OpenRouter.
- Automatic model fallback chains (`models: [...]`), provider routing preferences (`price` / `throughput` / `latency`), and per-role model assignment (`lead`, `subagent`, `compaction`, `dream`).
- Real-time model catalog sync (`GET /api/v1/models`) + exact token usage, cache hit ratio, and USD cost tracking per turn, session, and day (with configurable daily spend cap).
- Trait-based abstraction so direct Anthropic, OpenAI, Gemini, and local Ollama adapters plug in cleanly alongside OpenRouter.

#### FR-4: Core Native Toolset (`zenko-tools`)
- **File Read / Write / List** (path-jailed to configured workspace roots).
- **Hash-Anchored Line Edit** (inspired by Oh My Pi) + **Tolerant `apply_patch`** (`seek_sequence`, inspired by Codex) so edits succeed reliably on first try with unified diff preview.
- **Ultra-Fast Code Search** (`ignore` + `grep-searcher`, native `ripgrep` speed without spawning external binaries).
- **Sandboxed Shell Execution** (`tokio::process` with process-group kill, timeout, output truncation/disk-spill above 16 KB, and approval gating via `execpolicy`).
- **Web Fetch** (native `reqwest` + lightweight Rust HTML-to-Markdown extractor).
- **Memory & Snapshot Tool** (structured patch/append/prune on `USER.md`, `MEMORY.md`, and SQLite episodic notes).
- **Cron/Schedule Tool** (agent can self-schedule reminders and recurring jobs).

#### FR-5: Pure-Rust Customizable WebUI (`zenko-webui`)
- Embedded directly inside the `zenko` binary (served by `axum`, zero Node.js at runtime).
- Inspired visually by **BeUI** (agent primitives), **BeautifulUI** (modern SaaS motion blocks), and **RareUI** (interactive status orbs, dynamic micro-interactions), with secondary patterns from **assistant-ui** (thread branching/editing), **shadcn/ui** (primitives), **BoardUI** (dashboards), **UIArc**, and **Transitions.dev**.
- **Core Pages:**
  1. **Onboarding / Setup Studio** (if unconfigured or re-runnable anytime).
  2. **Agent Workbench (Chat & Execution):** Multi-session sidebar, live AI status orb, streaming Markdown + code blocks, collapsible Thinking/Reasoning traces, live Tool Execution cards with syntax-highlighted diffs, HITL Approve/Deny cards, message branching/edit/resend, prompt composer with model/mode/skill switcher and stop button.
  3. **Gateways Console:** Telegram bot connection status, QR/one-time pairing codes, allowlist manager, per-chat permission tier, live message traffic inspector.
  4. **Memory & Skills Studio:** Split editor for `SOUL.md`, `USER.md`, `MEMORY.md`, `ZENKO.md`, `skills/*.md`, plus search inspector.
  5. **Automation & Logs:** Cron/Heartbeat builder, Hooks & `execpolicy` rules viewer, live `tracing` log stream, real-time RSS RAM / CPU / Token / USD cost telemetry.
  6. **Component Customization Studio (Mandatory Core Feature):** Live customization of **all UI components** — themes & custom color palettes, **button styles/variants/shapes/effects**, **icon packs/stroke weights/sizes**, surface styles (flat / bordered / elevated / glass), corner radius scale, spacing density, typography (UI + mono fonts), motion/animation intensity, AI status orb style, chat bubble/layout style, sidebar style — with real-time live preview, instant global application, and save/export/import named presets!

#### FR-6: Production Telegram Gateway (`zenko-gateways`)
- Direct Bot API over shared `reqwest` (`rustls`) client (long-polling default for zero-port-forwarding setup; optional webhook mode).
- **Live Streaming Responses:** Uses Telegram's native `sendMessageDraft` streaming (with automatic fallback to rate-limited `editMessageText`).
- **Forum Topic Isolation:** Maps `(chat_id, message_thread_id)` to independent Zenko sessions so one Telegram group can host separate projects/topics cleanly.
- **Interactive Inline Approvals:** Renders `[✓ Approve] [✗ Deny] [✓ Always Allow]` inline keyboards for guarded tools, calls `answerCallbackQuery` immediately, and updates message state cleanly.
- **Security:** One-time pairing codes, explicit user/chat allowlists, read-only vs. full-tool per-chat policies, and `/stop`, `/new`, `/model`, `/status`, `/skills`, `/compact` slash commands.
- **Rate-Limit Resilience:** Built-in token-bucket outbound queue (~1 msg/s per chat, 20 msg/min per group, 30 msg/s global) with exact `429 retry_after` handling.

---

### 4.2 Phase 2 — 24/7 Autonomy, Compounding Intelligence & Hardening
- **Native Cron & Heartbeats:** Zero-CPU `sleep_until` scheduler persisted in SQLite; autonomous periodic check-ins that stay silent unless action/alert is required.
- **Recursive Sub-Agents & A2A:** Spawn parallel Tokio subagents (`spawn_agent`, `message_agent`, `wait_agent`) with narrowed tool scopes, cheaper OpenRouter model routing, and summary contracts.
- **Multi-Layer Context Compaction:** Auto-compact at ~75% context window while preserving active goals, modified-file invariants, and pinned memory.
- **Hybrid Memory + "Dream" Consolidation:** FTS5 (BM25) + `sqlite-vec` (int8/binary quantized embeddings) merged via SQL Reciprocal Rank Fusion (RRF), plus idle-time Dream pass consolidating episodic logs into `MEMORY.md` and candidate `SKILL.md` drafts.
- **Continual Harness (`/refine`):** Inspects completed/failed trajectories and proposes evidence-backed patches to skills, rules, or memory.
- **MCP (`rmcp`) + Sandboxing:** Full MCP client/server support, Linux `Landlock` + `Bubblewrap` + `Seccomp` tool sandboxing, optional `wasmtime` WASM plugin runner, optional `rhai` pure-Rust script tool, optional LSP/DAP code tools, and `zenko daemon install` (`systemd --user` service with `MemoryMax` & `WatchdogSec`).

### 4.3 Phase 3 — Detachable Ratatui TUI (`zenko-tui`)
- Thin `ratatui` + `crossterm` client attaching to the running daemon over Unix Domain Socket (`~/.zenko/zenko.sock`) or authenticated WebSocket.
- Event-driven rendering (0% idle CPU), multi-pane sessions/subagents tree, Markdown & tool diff cards, one-keystroke approvals, and live resource telemetry.

### 4.4 Phase 4 — Additional Gateways
- Feature-gated Discord (`twilight-gateway`), Slack (Socket Mode WS), HMAC Webhooks, Matrix, and Email (IMAP/SMTP) adapters.

---

## 5. Non-Functional Requirements (SLAs & Budgets)

| Metric | Target | Hard Ceiling |
| :--- | :--- | :--- |
| **Idle Daemon RAM (RSS)** *(Core + WebUI + Telegram)* | **5–15 MB** | < 30 MB |
| **Active Streaming RAM (RSS)** *(1–3 concurrent sessions)* | **15–35 MB** | < 64 MB |
| **Idle CPU Utilization** | **0.0% – 0.1%** | < 0.5% |
| **Cold Boot Time** | **< 10 ms** | < 50 ms |
| **Release Binary Size** *(stripped, LTO, bundled SQLite + embedded WebUI)* | **8–18 MB** | < 25 MB |
| **WebUI First Paint (Localhost)** | **< 30 ms** | < 100 ms |
| **Crash Recovery Time** | **< 2 s** | < 5 s |
| **External Runtime Dependencies** | **0** *(No Node.js, Python, Docker, or external DB required)* | 0 |

---

## 6. Security & Safety Requirements

1. **Zero Secret Exposure:** All API keys and bot tokens encrypted at rest (`ChaCha20-Poly1305` + Argon2id/OS keyring, memory wiped via `zeroize`), injected exclusively by trusted Rust transport layers — never placed into LLM prompts or tool subprocess environment variables.
2. **Default-Deny Gateways:** Unpaired Telegram users are rejected silently or given a pairing prompt; no commands execute without allowlist membership.
3. **Authenticated Control Plane:** WebUI and HTTP/WS API bind to `127.0.0.1` by default, require bearer/session token auth (constant-time verification), CSRF protection, and strict CORS.
4. **Defense-in-Depth Execution:** Path canonicalization jailing for file tools, `execpolicy` rules + deterministic `PreToolUse` hooks + HITL approvals, escalating to Linux `Landlock`/Bubblewrap and `wasmtime` capability sandboxes.
5. **Spend & Loop Guards:** Hard configurable max tool iterations per turn, max subagent depth, max daily USD spend on OpenRouter, and runaway-loop detection.
