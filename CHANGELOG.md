# Changelog

All notable changes to **Zenko** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.2] - 2026-11-23

### Fixed
- **Composer Model Selection Sync:** Synchronized both `provider.lead_model` and `provider.fast_model` on `PATCH /api/sessions/{id}`, cleared stale cross-provider model slots on provider switches, persisted `model_override` on `POST /api/sessions`, and forwarded the Composer's active model on `POST /api/sessions/{id}/messages` so agent replies always execute and display the exact model chosen in the Composer (e.g., `MiniMax-M3.1-Flash` instead of a stale `fast_model`).

### Changed
- **Accent-Colored Animated Markdown Links:** Styled links inside agent output prose (`.zenko-prose-link`) using the active global `--accent` token with a left-to-right animated underline sweep, subtle 12% accent hover highlight, animated external `↗` icon glide, safe `noopener noreferrer` external tab opening, and `prefers-reduced-motion` support.

## [0.0.1] - 2026-11-23

Initial public release of **Zenko** — a 100% native Rust 24/7 autonomous AI agent and multi-agent orchestrator with an embedded React/Tailwind WebUI (`rust-embed`, zero Node.js at runtime).

### Added

#### Native Rust Core & Low-RAM Runtime
- Modular 8-crate Rust workspace (`zenko-core`, `zenko-memory`, `zenko-providers`, `zenko-tools`, `zenko-runtime`, `zenko-gateways`, `zenko-webui`, `zenko-cli`) compiled with `mimalloc`, `rustls` (zero OpenSSL), bounded Tokio workers, and single-file SQLite WAL storage.
- Multi-provider LLM streaming gateway (`OpenRouter`, `MiniMax`, `Anthropic`, `OpenAI`, `Gemini`, `Ollama`, `Custom`) with chunk-safe UTF-8 SSE decoding, prompt caching, sticky routing, fallback chains, and live cost/token accounting.
- Interactive Ratatui TUI onboarding wizard (`zenko setup`) and single-command 24/7 daemon (`zenko web`) with Hono-style HTTP/runtime structured logging.

#### Autonomous Orchestration, Planning & Checkpoints
- Deterministic complexity triage (`Instant` / `Standard` / `Orchestrated`), parallel recon probes, read/plan-before-write phase gating, evidence-weighted brainstorming, interactive & headless clarifications, adaptive fast/lead routing, and doom-loop circuit breaking.
- Durable SQLite `session_tasks` DAG with Kahn's cycle guard and atomic claiming, Read-Only `Plan` permission mode with approval gate, non-invasive Shadow-Git & SHA-256 `ContentBlobs` workspace checkpoints, manual hand-edit preservation, and 3-mode Rewind (`code_and_conversation` | `code_only` | `conversation_only`) with undo-of-undo.

#### Async Execution, Subagents & 24/7 Automation
- 3-state parallel tool scheduler (`Shared` / `Keyed` / `Global`), mid-flight steering (`Steer` / `Queue` / `Interrupt`), durable SQLite follow-up queue, and multi-client `zenko attach`.
- **`zenko_task`** background process supervisor with live ANSI log streaming, stdin forwarding, foreground-to-background demotion, reactive idle-session wakeup, and persistent log clearing (`POST /api/jobs/{id}/clear` and `POST /api/jobs/prune`).
- Async subagents (`explorer`, `architect`, `coder`, `verifier`) with monotonic permission ceilings, pure-async `git worktree` isolation (`diff` / `merge` / `discard` / `prune`), fork context, budgets, and mailbox messaging.
- 0-token Script Cron (`CronKind::Script`), bounded 24/7 autonomous Goal Loops with `verify_cmd` and stall auto-rewind, RFC 6901 HMAC-verified webhook routing rules, filesystem watchers, and scrubbed trajectory exports (`jsonl` / `sharegpt` / `markdown`).

#### Memory, Context Engineering & Deep Engineering Tools
- 3-tier KV-cache-optimal prompt builder, lossless microcompaction, persisted iterative-merge compaction checkpoints, 35/65 head+tail artifact spilling, BM25 deferred `tool_search`, and `context_inspect`.
- Surgical `patch_doc` memory with `.history/` 1-click rollback, project-scoped `PROJECT_MEMORY.md`, hybrid FTS5 + RRF + SimHash/Jaccard recall, stale-path verification, `session_search` context windows, and autonomous Dream consolidation.
- Procedural `SKILL.md` authoring (`skill_manage`) with atomic version history and draft approval gate.
- Native `glob_find`, argv-only `git_tool`, pluggable `web_search` cascade (`searxng` → `tavily` → `brave` → `duckduckgo`), pooled JSON-RPC 2.0 `lsp_tool` with diagnostics-on-edit, Universal External MCP Client (`stdio` + Streamable HTTP), outbound MCP stdio server (`zenko mcp`), and OpenAI-compatible `/v1/*` API.

#### Embedded WebUI, Visual Dashboard & Theme Studio
- Embedded SPA (`rust-embed`, zero Node.js at runtime) featuring strict **60/30/10** Obsidian Vermilion visual discipline across dark and light modes.
- **Visual Analytics Dashboard** (`GET /api/dashboard`) powered by tree-shaken Apache ECharts: GitHub-style Zenko Activity Contribution Heatmap, Token & KV-Cache Savings trends, Storage Breakdown Donut & tables, `zenko_vault` ChaCha20-Poly1305 security & key inventory inspector, Top Tool Executions & Model Spend tables, and embedded Subagents, ZenkoTasks & Categorized Flight Recorder logs.
- **Hierarchical Theme & Component Studio** with live sidebar search (`/` shortcut + auto-scroll highlight) and animated live previews:
  - 9 global Card Surface modes (`Top Hairline` default, `Bottom Hairline`, `Borderless Fill`, `Ghost Outline`, `Flat Matte`, `Framed Card`, `Elevated Soft`, `Frosted Glass`, `Accent Edge`).
  - Unified **`Agent Activity`** display (`Collapsible+` default + 12 additional styles), Assistant-UI elements (`LoadingState`, `ThinkingIndicator`, `StreamingText`, `MessageActions`, `ErrorState`, `StoppedRun`, `DaySeparator`, `TerminalBlock`, `CodeDiff`, `ReviewableDiff`, `FileTree`, `PermissionGrant`, `CodeRunner`, `WebSearch`, `MemoryChips`, borderless `DataTable`).
  - 10 open-source/free Icon Families (including animated hover packs), custom font studio with **Obel Mono** & **Fontzz** typefaces, customizable **Hover Hints (Tooltips)**, and full **Dashboard** layout & module customization.
  - **Avatars & Profiles**: 10 handcrafted Fox emblems for Zenko, 9 human/user/profile avatars + Monogram for users, custom color pickers with theme-default reset, and hideable avatar frames (`Hidden (No Frame)` + 5 badge frames).

#### Telegram Gateway
- Long-polling / webhook Telegram bot with Forum Topic session isolation, zero-trust `/pair` onboarding, draft streaming, and 1-tap inline approval buttons for tools, clarifications, and draft skills.

### Security
- Encrypted `ChaCha20-Poly1305` Secret Vault (`~/.zenko/vault.enc`, `0600` permissions) with constant-time token verification, zero secret exposure in Dashboard/logs/exports, workspace jail & symlink escape guards, SSRF blocking, and deterministic ExecPolicy enforcement.
