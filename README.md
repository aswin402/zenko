<div align="center">
  <a href="https://github.com/aswin402/zenko">
    <img src="assets/logo.svg" alt="Zenko Logo" width="76" height="76" />
  </a>

  <h1>Zenko</h1>

  <p>
    <strong>Autonomous local-first engineering — always on, always yours.</strong>
    <br />
    100% native Rust 24/7 AI agent &amp; multi-agent orchestrator with an embedded WebUI and zero Node.js at runtime.
  </p>

  <p>
    <a href="CHANGELOG.md"><img src="https://img.shields.io/badge/version-0.0.53-ea5829?style=flat-square" alt="Version 0.0.53" /></a>
    <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.88%2B-141417?style=flat-square&logo=rust&logoColor=f5f5f6" alt="Rust 1.88+" /></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-141417?style=flat-square" alt="MIT License" /></a>
    <img src="https://img.shields.io/badge/runtime-Zero%20Node%20%7C%20Zero%20Python%20%7C%20rustls-141417?style=flat-square" alt="Zero Runtime Dependencies" />
  </p>

  <p>
    <a href="#getting-started">Getting Started</a>
    &middot;
    <a href="#key-capabilities">Capabilities</a>
    &middot;
    <a href="#cli-reference">CLI</a>
    &middot;
    <a href="#design-system">Design System</a>
    &middot;
    <a href="CHANGELOG.md">Changelog</a>
  </p>
</div>

---

## About

**Zenko** compiles into a single stripped Rust binary (`~15 MB`) with its React/Tailwind WebUI embedded directly via `rust-embed`. It runs continuously at minimal RAM using `mimalloc`, `rustls`, bounded Tokio workers, and single-file SQLite WAL storage — serving both **humans** (WebUI, Telegram, Terminal TUI/REPL) and **agents** (`--json`/`--stream-json`, Unix IPC, OpenAI-compatible `/v1/*`, outbound MCP stdio).

### Built With

- **Core Runtime:** Rust (Edition 2024), Tokio, Axum, SQLite (WAL + FTS5), `rustls`, `ChaCha20-Poly1305`, `mimalloc`
- **Embedded WebUI:** React 19, TypeScript, Tailwind CSS v4, Motion, Apache ECharts, Assistant-UI Elements, `rust-embed`
- **TUI & Gateways:** Ratatui, Telegram Bot API, MCP (`stdio` + Streamable HTTP), LSP JSON-RPC 2.0

---

## Key Capabilities

| Area | Highlights |
| :--- | :--- |
| **Orchestration & Safety** | LLM-sovereign Orchestrator v2 (`orchestrate_control` live posture & cognitive gear, iterative Metis/Momus brainstorming, dynamic `critic`/`researcher`/`custom` specialists, zero keyword blocks, zero `/commands`), Read-Only `Plan` mode, SQLite Task DAG, Shadow-Git checkpoints & 3-mode Rewind |
| **Async Execution (`zenko_task`)** | 3-state parallel tool scheduler, live background build/dev runner with ANSI logs & log clearing, mid-flight steering, and `git worktree` subagents |
| **Memory & Skills** | Surgical `patch_doc` with 1-click `.history/` rollback, hybrid FTS5/RRF/SimHash recall, auto-compaction, Dream consolidation, and `SKILL.md` drafting |
| **Deep Engineering Tools** | Native `glob_find`, argv-only `git_tool`, pluggable `web_search` cascade, pooled LSP diagnostics-on-edit, and Universal MCP Client |
| **24/7 Autonomy & Gateways** | 0-token Script Cron, bounded Goal Loops with `verify_cmd`, HMAC-signed webhook routing, file watchers, and Telegram Forum Topic sync |
| **Visual Dashboard & Studio** | ECharts GitHub-style activity heatmap, storage & `zenko_vault` inspector, searchable Theme Studio, 9 card surfaces, 10 icon families, Fox & profile avatars |

---

## Getting Started

### Prerequisites

- **Rust** `1.88+` (`cargo`)
- **Bun** (or **Node.js 20+**) — build-time only for bundling `webui/dist` (zero JS runtime needed once compiled)

### Build from Source

```bash
# 1. Clone the repository
git clone https://github.com/aswin402/zenko.git
cd zenko

# 2. Install WebUI build dependencies
cd webui && bun install && cd ..

# 3. Build optimized stripped release binary (embeds WebUI automatically)
./scripts/build.sh --release
```

### Quick Start

```bash
# 1. Configure provider, model, streaming & optional Telegram via interactive TUI
./target/release/zenko setup

# 2. Launch the 24/7 daemon & embedded WebUI (http://127.0.0.1:7878)
./target/release/zenko web

# 3. Optional: health check, terminal chat, or headless JSON execution
./target/release/zenko doctor
./target/release/zenko chat
./target/release/zenko ask --json "Inspect workspace architecture"
```

---

## CLI Reference

```bash
# Sessions & live steering
zenko ask | chat | attach | steer | background | queue | context | sessions

# Planning, Task DAG & workspace rewind
zenko plan approve
zenko dag list | ready | create | update | claim
zenko checkpoints list | diff | rewind | prune
zenko undo

# Background tasks (zenko_task) & worktree subagents
zenko tasks list | start | read | wait | follow | stop | delete | prune
zenko agents list | spawn | wait | diff | merge | discard | kill

# Memory, skills, 24/7 goal loops & triggers
zenko memory search | add | dream | history | rollback
zenko skills list | show | create | approve | toggle | history | rollback
zenko goals list | show | create | pause | resume | run | delete
zenko cron | triggers | export | mcp-client | lsp | mcp | gateway pair | logs | service
```

---

## Architecture

```text
crates/
├── zenko-core       # Types, config, ChaCha20-Poly1305 vault, registries, Studio schema
├── zenko-memory     # SQLite WAL + FTS5/RRF/SimHash search, snapshots, Dream consolidation
├── zenko-providers  # Streaming OpenRouter, MiniMax, Anthropic, OpenAI, Gemini, Ollama
├── zenko-tools      # FS/hash-edit, shell, glob, git, web search, LSP pool, MCP client
├── zenko-runtime    # ReAct loop, scheduler, orchestrator, subagents, checkpoints, goals
├── zenko-gateways   # Telegram bot gateway & HMAC webhook router
├── zenko-webui      # Axum REST/SSE API, dashboard analytics & embedded SPA (rust-embed)
└── zenko-cli        # Single native binary, Ratatui setup TUI, REPL, IPC & outbound MCP
```

---

## Design System

Zenko's WebUI and TUI follow a strict **60 / 30 / 10** visual hierarchy (default **Obsidian Vermilion**), with every surface, chart, tooltip, icon family, font, and avatar customizable live in Theme Studio (`⌘.`).

| Role | Share | Token | Default Value | Usage |
| :--- | :---: | :--- | :--- | :--- |
| **Dominant Canvas** | 60% | `--bg` | `#0a0a0b` | Matte primary workspace backdrop |
| **Secondary Surface** | 30% | `--surface` / `--card` | `#0f0f11` / `#141417` | Sidebars, docks, top-hairline cards |
| **Primary / Muted Text** | — | `--fg` / `--muted` | `#f5f5f6` / `#8a8a93` | Editorial prose & tabular metadata |
| **Single Accent** | 10% | `--accent` | `#ea5829` | Active indicators, fox emblem, heatmap ramp |

- **Typography:** Inter / Obel Mono / JetBrains Mono / Fontzz typefaces + custom local font uploads.
- **Surfaces:** Minimal top-hairline cards by default (`minimal_top`) with 9 selectable global card surface styles and borderless data tables.
- **Privacy & Security:** All API keys and tokens are encrypted at rest in `~/.zenko/vault.enc` (`ChaCha20-Poly1305`, `0600`), scrubbed from all logs/exports, and never exposed in the Dashboard vault inspector.

---

## Documentation

- [CHANGELOG.md](CHANGELOG.md) — Release history (`v0.0.11`)
- [docs/DESIGN.md](docs/DESIGN.md) — WebUI Design System & Customization Spec
- [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md) — Rust Systems Architecture
- [docs/PRD.md](docs/PRD.md) — Product Requirements
- [config/zenko.example.toml](config/zenko.example.toml) — Example daemon configuration

---

## License

Distributed under the **MIT License**. See [LICENSE](LICENSE) for details.

## Acknowledgments

- [Best-README-Template](https://github.com/othneildrew/Best-README-Template) &amp; [awesome-design-md](https://github.com/voltagent/awesome-design-md)
- [Apache ECharts](https://echarts.apache.org/) · [Assistant-UI](https://www.assistant-ui.com/) · [Ratatui](https://ratatui.rs/) · [Axum](https://github.com/tokio-rs/axum)
