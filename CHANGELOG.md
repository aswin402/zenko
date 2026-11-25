# Changelog

All notable changes to **Zenko** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.31] - 2026-11-25

Closed all four remaining end-to-end wiring seams across `batch_tools`, checkpoint rewinds, background subagent collection (`agent_wait`), and `code_minimalism` text summaries.

### Fixed
- **`batch_tools` Output-Dedup & CCR Recovery Unwrapping (`types.rs`, `batch.rs`, `runtime/bridges.rs`, `economy/mod.rs`):**
  - Added `output_dedup_tokens_saved` and `ccr_recovery` fields to `BatchToolStepOutcome`, extracted them cleanly across both sequential and wave-parallel `batch_tools` paths (without double-counting into `read_less_tokens_saved`), and unwrapped them in `record_tool_outcome_in_economy_trace`.
- **Unified Rewind Cache Invalidation (`runtime/mod.rs`, `tasks_checkpoints.rs`, `ipc.rs`, `goals.rs`, `mcp.rs`, `dag_checkpoints.rs`):**
  - Added `AgentRuntime::rewind_checkpoint` and routed all 5 rewind callers (WebUI, IPC, Goal Loops, MCP, CLI) through it so every rewind atomically clears `SessionOutputDedup` (and `file_state` on code restores), preventing stale `[dedup:]` references to abandoned branches.
- **Background Subagent Output Eligibility (`crates/zenko-core/src/constants.rs`):**
  - Added `TOOL_AGENT_WAIT` to `READ_LESS_ELIGIBLE_TOOLS` so bulky background subagent result summaries receive automatic 3-tier `dedup`/`scrub`/`read-less` compression.
- **`code_minimalism` Text Summary Parity (`crates/zenko-tools/src/minimalism.rs`):**
  - Surfaced added source code-vs-comment line ratios in `audit_diff` text output and `Output-Dedup` hits/savings + `CCR Recoveries` in `gain` text output.

## [0.0.30] - 2026-11-25

Implemented five native Rust Chisle-inspired minicode and context-efficiency upgrades (zero `/commands`, zero Node hooks, zero regex): single-implementation abstraction & comment-bloat auditing, session-level exact tool-output deduplication, lossless `scrub` & `glob_find` directory grouping, error-salvaging middle elision, and CCR backfire telemetry.

### Added
- **Single-Implementation Abstraction & Comment Bloat Analyzers (`crates/zenko-tools/src/minimalism.rs`, `zenko-core/src/{constants,types,registry}.rs`):**
  - Extended `code_minimalism(action="audit_repo")` with a zero-regex behavioral abstraction checker (`single_impl_abstractions`) flagging Rust `trait`s, `abstract class`es, and method-bearing TS/JS/Java/Kotlin/C# `interface`s with `<= 1` concrete implementation (excluding pure DTO/data interfaces).
  - Extended our single-pass 5-state comment byte FSM to compute file comment ratios and longest narrative comment blocks (`comment_bloat`) plus diff-level added comment-vs-code ratios in `audit_diff`.
  - Updated built-in English skills (`minimal-review.md`, `minimal-audit.md`) and Tier-2 `<code_minimalism>` directives with Context Diet rules and strict one-finding-per-line zero-praise reviews.
- **3-Tier Deterministic Tool-Output Compression (`crates/zenko-runtime/src/{read_less,artifacts,tool_router}.rs`):**
  - **Tier 1 (`SessionOutputDedup`):** Bounded per-session LRU deduplicator (`READ_LESS_DEDUP_MAX_ENTRIES_PER_SESSION`, auto-cleared on Stage-B compaction and session reset) replacing exact repeated outputs from allowlisted ephemeral tools with a 1-line marker citing the raw `artifact:` handle.
  - **Tier 2 (`ReadLessShape::Scrub` & `ReadLessShape::GlobList`):** Expanded `READ_LESS_ELIGIBLE_TOOLS` to `web_search`, `glob_find`, and `spawn_agent` (strictly keeping `read_file`, `code_search`, `lsp_tool`, and editors exempt), added always-on lossless `scrub` for medium outputs, and added directory-grouped compression for verbose `glob_find` listings.
  - **Tier 3 (Error-Salvaging Middle Elision):** Upgraded `format_head_tail_preview` in `artifacts.rs` to salvage up to 12 unique compiler/panic/failure lines with line numbers from the omitted middle slice before dropping middle noise.
- **Honest Backfire Telemetry & WebUI Visibility (`sqlite/economy.rs`, `economy/mod.rs`, `economy_tool.rs`, `TokenEconomyPanel.tsx`):**
  - Added `EconomyLedgerKind::OutputDedup` and `EconomyLedgerKind::CcrRecovery` tracking exact dedup hits/savings and how often `read_file` / `code_search` explicitly recovers a compressed `artifact:` handle, surfaced in `token_economy(action="status")`, Copy Summary Markdown, and the 60/30/10 WebUI Token Economy & Bloat Audit panel.

## [0.0.29] - 2026-11-25

Fixed stuck/glitchy Cargo build progress animations during `just dev` (and `just build` / `just check` / `just test`) by replacing the buggy external `cargo-pretty` (`nobubbles-0.1.1`) default with a native, tear-free Obsidian Vermilion Cargo progress renderer.

### Fixed
- **Native Tear-Free Cargo Progress Renderer (`scripts/cargo_pretty.py`, `scripts/build.sh`):**
  - Replaced external `cargo-pretty` (`nobubbles-0.1.1`) as the default TTY wrapper — fixing the false `100%` progress stall (`342/343` cached crates locking the bar at `100%` during `zenko-cli` compile/link), hardcoded 20-row absolute cursor addressing (`CSI row;colH`) overflowing short terminals, terminal DSR (`ESC[6n`) hangs, and double-invocation `--unit-graph` startup overhead.
  - Added a zero-dependency, zero-regex native progress renderer (`scripts/cargo_pretty.py`) consuming `cargo --message-format=json-render-diagnostics` on an independent 12.5 FPS (80ms) tick loop with DEC Synchronized Output (`ESC[?2026h`/`ESC[?2026l`), strictly relative 2–5 row inline cursor positioning, run-length batched 60/30/10 Obsidian Vermilion (`#ea5829`) comet animation, clean `SIGINT`/`SIGTERM` cursor restoration, `NO_COLOR=1` and non-TTY fallback, and live test-stream passthrough for `just test`.
  - Retained `ZENKO_PRETTY=external` (external `cargo-pretty`) and `ZENKO_PRETTY=0` (raw Cargo) overrides in `scripts/build.sh`.

## [0.0.28] - 2026-11-25

Completed full Ponytail-inspired Code Minimalism parity with zero-regex dormant dependency detection, accurate new/untracked file diff tracking, WebUI Bloat Audit & Copyable Summary controls, and a live Chat Context Popover minimalism switcher.

### Added
- **Zero-Regex Dormant Dependency Scanner (`crates/zenko-tools/src/minimalism.rs`, `constants.rs`, `types.rs`):**
  - Added `scan_dormant_dependencies` and `DormantDependencyFinding` to `RepoAuditReport`, cross-checking `[dependencies]` in `Cargo.toml` and runtime `"dependencies"` in `package.json` against actual workspace `.rs` / `.ts` / `.tsx` / `.js` / `.jsx` / `.css` references (skipping `-sys` crates, `@types/*`, and sorted `MINIMALISM_IGNORED_DORMANT_DEPS` via `binary_search`).
- **WebUI Bloat Audit Viewer & 1-Click Copy Markdown Summary (`webui/src/components/views/TokenEconomyPanel.tsx`):**
  - Added **Run Bloat Audit** (`GET /api/economy/minimalism/audit`) rendering diff findings, dormant dependencies, duplicate helpers, and thin wrappers strictly in global 60/30/10 theme tokens.
  - Added **Copy Summary** copying a concise Markdown Token Economy & Code Minimalism scoreboard directly to the clipboard.
- **Live Chat Context Popover Minimalism Switcher (`webui/src/components/chat/PromptComposer.tsx`):**
  - Added a compact 4-pill `Code Minimalism` (`Off | Lite | Full | Ultra`) live switcher right below `Concise English` inside the existing Context Breakdown popover (persisting globally and updating the active session immediately).

### Fixed
- **New-File & Untracked-File Diff Tracking (`crates/zenko-tools/src/minimalism.rs`):**
  - Updated `parse_unified_diff_counts` and `analyze_unified_diff` to detect `@@ -0,0 +` hunks emitted by `fs_tools::unified_diff` (`write_file` / `apply_patch`) without double-counting `--- /dev/null`, fixing `new_files_created` counts and new-file bloat guard triggers.
  - Updated `audit_git_diff_minimalism` (unstaged mode) to include brand-new untracked files from `git ls-files --others --exclude-standard` for LOC, oversized-hunk, manifest, and duplicate-symbol analysis.

## [0.0.27] - 2026-11-25

Implemented Zenko's **Native Rust Code Minimalism Engine** (inspired by `DietrichGebert/ponytail`; 100% pure Rust, zero `/commands`, zero regex, zero hardcoding) — adding the 7-rung Simplicity Ladder, deterministic diff/repo bloat & duplicate-helper analysis, comment-scoped shortcut debt scanning, real SQLite net-LOC telemetry, 1-shot orchestrator diff-bloat guards, deferred `code_minimalism` tool, 3 native English skills, and 60/30/10 WebUI controls.

### Added
- **Declarative Code Minimalism Profiles & 7-Rung Simplicity Ladder (`crates/zenko-core/src/{constants,types,config,registry}.rs`):**
  - Added `CodeMinimalismMode` (`off`, `lite`, `full`, `ultra`), `CODE_MINIMALISM_REGISTRY`, `SIMPLICITY_LADDER_RUNGS`, and `render_minimalism_prompt_block` injected into Tier-2 (KV-cache-safe, zero `/commands`, hard safety boundary invariant).
  - Wired live LLM-sovereign switching into `orchestrate_control(minimalism_mode="off|lite|full|ultra")` and persisted session resolution across turns.
  - Seeded 3 built-in English workflow skills in `DEFAULT_SKILL_TEMPLATES`: `minimal-review.md`, `minimal-audit.md`, and `debt-harvest.md`.
- **Zero-Regex Pure-Rust Diff, Bloat & Comment-Scoped Debt Engine (`crates/zenko-tools/src/minimalism.rs`):**
  - Implemented single-pass unified-diff LOC/new-file/manifest/oversized-hunk parser (`parse_unified_diff_counts` & `analyze_unified_diff`) cross-checked against workspace symbols via `outline::extract_outline` and O(log K) binary-searched `MINIMALISM_IGNORED_DUP_SYMBOLS`.
  - Implemented a 5-state comment-scoped byte FSM (`scan_file_debt_markers` / `scan_workspace_debt`) that scans `DEBT:`, `SHORTCUT:`, `HACK:`, `FIXME(debt):`, `TODO(debt):`, and `PONYTAIL:` markers strictly inside comments (zero false positives in string literals or constant tables).
  - Registered the deferred read-only `code_minimalism` tool (`ToolSet::CodeIntel`, actions `audit_diff`, `audit_repo`, `debt_scan`, `gain`) permitted in Read-Only Plan Mode and `explorer`/`architect`/`critic`/`researcher` subagents.
- **Automatic Orchestrator Diff-Bloat Guard & Real SQLite Net-LOC Ledger (`sqlite/economy.rs`, `economy/mod.rs`, `runtime/turn.rs`):**
  - Added SQLite `repo_code_metrics` table recording exact per-repo `+lines_added`, `-lines_deleted`, `new_files_created`, `bloat_hints_issued`, and `debt_markers_last_seen` from every mutating edit/patch/batch diff.
  - Added deterministic 1-shot `maybe_emit_bloat_hint` driven purely by `CODE_MINIMALISM_REGISTRY` thresholds, nudging the orchestrator to run `code_minimalism(action="audit_diff")` when additive churn or new files spike.
- **API & 60/30/10 WebUI Controls (`api/economy.rs`, `api/auth.rs`, `TokenEconomyPanel.tsx`, `PromptComposer.tsx`):**
  - Added `GET /api/economy/minimalism/debt`, `GET /api/economy/minimalism/audit`, and `minimalism_mode` / `auto_bloat_hint` support on `PATCH /api/economy/config`.
  - Integrated the 4-level Code Minimalism selector, Auto Diff Bloat Guard toggle, Net LOC telemetry card, and on-demand Shortcut Debt scanner cleanly into the existing Token Economy panel and Context Breakdown popover.

## [0.0.26] - 2026-11-25

Completed end-to-end Token Economy wiring across `tool_search`, Read-Only Plan Mode & subagents, live Context Breakdown, proactive Tier-2 prompt guidance, and `batch_tools` step telemetry.

### Fixed
- **Demotion-Aware `tool_search` & ToolSet Discovery (`crates/zenko-tools/src/lib.rs`, `runtime/bridges.rs`, `runtime/mod.rs`):**
  - Added `effective_exposure`, `is_deferred_or_demoted`, `exposure_counts_with_demotions`, `list_toolsets_with_demotions`, `search_deferred_bm25_with_demotions`, and `select_deferred_by_scope_with_demotions` so Token-Economy soft-pruned tools stay 100% discoverable via `tool_search` (by `query`, `names`, or `toolset`) and accurately listed as deferred in `list_toolsets`.
- **Read-Only Plan Mode & Subagent Gating for `token_economy` (`constants.rs`, `types.rs`, `policy.rs`, `guard.rs`, `registry.rs`):**
  - Added `EconomyAction::is_mutating`, `is_economy_args_mutating`, and `TOOL_TOKEN_ECONOMY` in `PLAN_MODE_SCOPED_TOOLS`, `evaluate_plan_mode_policy`, `classify_tool_category`, and read-only `architect`/`critic` subagent roles — permitting read-only inspection (`status`, `micro_skills`, `prune_report`, `ledger`) in Plan Mode while blocking mutating actions (`add_micro_skill`, `retire_micro_skill`, `apply_prune`, `restore_tool`, `reaudit`).
- **Live Per-Session Context Breakdown Accuracy (`crates/zenko-runtime/src/runtime/mod.rs`):**
  - Updated `inspect_session_context` to resolve each session's actual `workspace_dir` and `repo_scope`, include soft-demoted tools in schema/exposure counts, include `<token_economy>` and `<repo_micro_skills>` blocks in `tier2_tokens`, and report accurate per-workspace ledger metrics.
- **Proactive KV-Cache-Safe Orchestrator Guidance (`crates/zenko-core/src/registry.rs`, `runtime/turn.rs`):**
  - Added `render_economy_prompt_block` in Tier-2 (active whenever `economy.enabled` is true and the role permits `token_economy`, even when Caveman Mode is `Off`) so the orchestrator proactively uses `token_economy` and `token-economy-audit` without ever mutating Tier-1 KV prefixes.
- **`batch_tools` & `invoke_tool` Economy Trace Unwrapping (`types.rs`, `batch.rs`, `bridges.rs`, `economy/mod.rs`, `turn.rs`):**
  - Propagated `shell_command`, bounded `raw_output_preview` (`ECONOMY_SHELL_OUTPUT_PREVIEW_CHARS`), and `read_less_tokens_saved` through `BatchToolStepOutcome` metadata into `record_tool_outcome_in_economy_trace`, ensuring soft-demoted tools called inside `batch_tools` self-heal, batched shell fail→fix pairs mine micro-skills, and `invoke_tool` `"arguments"` aliases are traced cleanly.

## [0.0.25] - 2026-11-25

Implemented Zenko's **Native Rust Token Economy Engine** (inspired by `microsoft/TokenEconomizer`, 100% native Rust, zero Python, zero regex, zero hardcoding) — adding cross-session repository Micro-Skills, experience-driven soft Tool Pruning, empirical `1.25× P90` Turn-Envelope Loop Guard, and an honest Net ROI Economy Ledger across Core, SQLite, Runtime, Tools, and WebUI.

### Added
- **Zero-LLM Repository Micro-Skills (`crates/zenko-runtime/src/economy/micro_skills.rs`, `sqlite/economy.rs`, `registry.rs`):**
  - Deterministically extracts recurring fail→fix shell command patterns across turns using a pure byte/token command-head parser and 64-bit FNV-1a signatures (zero regex, zero extra LLM calls).
  - Enforces multi-session corroboration (`>= 2` distinct sessions) before promoting `candidate -> active`, injects verified facts into Tier-2 `<repo_micro_skills>` (keeping Tier-1 KV-cache prefix frozen), tracks exposure need-rate (`>= 0.40`), and immediately retires contradicted facts on 1 strike.
- **Experience-Driven Soft Tool Pruning (`crates/zenko-tools/src/lib.rs`, `sqlite/economy.rs`, `runtime/turn.rs`):**
  - Tracks per-repository tool utilization across prompts and sessions while permanently protecting core tools (`ECONOMY_PROTECTED_CORE_TOOLS`).
  - Automatically soft-demotes persistently unused non-core eager tools (`>= 10` prompts across `>= 2` sessions) to `Deferred` (saving full per-turn schema tokens while preserving 100% reachability via `invoke_tool`), self-heals immediately if invoked, and periodically re-audits every 5 completed tasks.
- **Empirical `1.25× P90` Turn-Envelope Loop Guard + Optional Fast Judge (`crates/zenko-runtime/src/orchestrator/guard.rs`, `runtime/mod.rs`):**
  - Records successful per-repo turn lengths in SQLite (`repo_turn_baselines`) and computes a deterministic nearest-rank `1.25× P90` call envelope.
  - Detects high error density and arg-varying repeated tool oscillations when breaching the repo envelope, issuing a bounded 1-shot `<orchestrator_hint type="p90_loop_guard">` redirect (zero LLM default, optional opt-in `fast_model` semantic judge).
- **Honest Net Economy Ledger (`economy_ledger`, `dream.rs`, `PromptComposer.tsx`):**
  - Accounts for Read-Less + Tool-Prune token savings, Micro-Skill injections, and Loop-Guard interventions **minus** background Dream & Fast-Judge overhead tokens/USD.
- **Deferred `token_economy` Tool, Built-in English Skill & WebUI Control Plane (`economy_tool.rs`, `api/economy.rs`, `TokenEconomyPanel.tsx`):**
  - Added deferred `token_economy` tool (`status`, `micro_skills`, `add_micro_skill`, `retire_micro_skill`, `prune_report`, `apply_prune`, `restore_tool`, `reaudit`, `ledger`) with zero eager KV-cache overhead.
  - Seeded built-in `token-economy-audit.md` skill in `DEFAULT_SKILL_TEMPLATES`.
  - Added `/api/economy*` endpoints, the Settings **Token Economy Engine** panel (live switches, Net ROI telemetry, micro-skill add/retire, tool defer/restore/re-audit), and Context Breakdown net economy metrics.

## [0.0.24] - 2026-11-25

Added 1-click Caveman Mode ON/OFF & compression level controls directly in the Chat Input bar and Settings view, plus full Theme Studio button style customization.

### Added
- **Chat Input Caveman Mode ON/OFF & Mode Button (`webui/src/components/chat/CavemanModeControl.tsx`, `PromptComposer.tsx`):**
  - Added a reusable dual-action Caveman Mode control inside the Chat Input toolbar featuring a 1-click ON/OFF toggle (remembering the last active level) and a Mode selector (`Off`, `Lite`, `Full`, `Ultra` from `CONCISE_MODE_REGISTRY`) with live Read-Less CCR & Auto-Clarity toggles.
- **Settings View Caveman Mode & Output Efficiency Card (`webui/src/components/views/SetupView.tsx`, `crates/zenko-webui/src/api/setup_gateways.rs`):**
  - Added a live-synced Caveman Mode ON/OFF switch, compression level cards showing target token savings, Auto-Clarity and Read-Less toggles, and a direct "Customize Chat Button" shortcut into Appearance Studio.
  - Extended `SetupSaveReq` (`POST /api/setup`) to accept optional `concise_mode`, `auto_clarity`, and `read_less` fields alongside immediate `PATCH /api/context/concise` syncing.
- **Appearance Studio Theme Customization (`crates/zenko-core/src/ui.rs`, `studio_schema.rs`, `StudioPreviews.tsx`, `AppearanceStudio.tsx`):**
  - Extended `ComposerSlotConfig` with `show_concise_toggle`, `concise_button_style` (`split_pill`, `accent_solid`, `soft_tint`, `outline_mono`, `minimal_ghost`, `segmented_bar`), `concise_button_shape` (`pill`, `rounded`, `square`), `concise_button_chrome` (`bordered_filled`, `border_only`, `background_only`, `minimal_bare`), and optional `concise_button_accent` override (defaulting strictly to the active 60/30/10 theme accent).
  - Added a dedicated **Chat Input Box -> Caveman Mode Button** studio page with an interactive live preview stage.

## [0.0.23] - 2026-11-25

Fixed `just dev` / `just setup` workspace invocation under `cargo-pretty` and eliminated cold dependency rebuilds on Linux `relatime` filesystems.

### Fixed
- **Virtual Workspace Launch (`scripts/build.sh`):**
  - Scoped `cargo-pretty` strictly to `build` and `test` subcommands (`cargo pretty run` calls `Metadata::root_package()`, which is `None` in virtual Cargo workspaces and fails with `error: a bin target must be available for cargo run`).
  - Updated `just dev` and `just setup` to build via `cargo_run build -p zenko-cli --bin zenko` (auto-building `webui/dist` if missing for `dev`) and then `exec target/debug/zenko web|setup "$@"` directly so the daemon and setup TUI own the TTY cleanly.
- **`relatime`-Safe `cargo-sweep` Retention (`scripts/build.sh`):**
  - Removed `cargo sweep --stamp/--file` (which evicted warm cached `.rlib`s on Linux `relatime` mounts where reads within 24h do not bump `atime`), retaining time-based (`--time`) + hard size-cap (`--maxsize`) sweeping and post-test binary pruning.

## [0.0.22] - 2026-11-25

Hardened Zenko's native English Concise, Read-Less, Markdown Compression, and Script Block subsystems after a full line-by-line audit — eliminating every scattered magic number and inline table, upgrading heuristic loops to deterministic algorithms, and fixing custom shebang execution in `script_block`.

### Changed
- **Zero-Hardcoding Centralization (`crates/zenko-core/src/constants.rs`, `registry.rs`, `snapshot.rs`):**
  - Centralized all Read-Less thresholds & matcher tables (`READ_LESS_*`), Markdown compression tables & protected prefixes (`DOC_COMPRESS_*`), Script Block limits (`SCRIPT_BLOCK_*`, `FILE_MODE_OWNER_EXEC`), and `DEFAULT_SKILL_TEMPLATES` (`systematic-debug`, `concise-review`, `concise-commit`, `concise-explore`) into `zenko-core`.
- **Drain-Style FNV-1a Log Template Clustering & RFC-4180 Quote-Aware CSV FSM (`crates/zenko-runtime/src/read_less.rs`):**
  - Replaced exact-line log deduplication with deterministic 64-bit FNV-1a digit/hex-normalized template clustering (`structural_log_template_hash`), collapsing repeating timestamped/numbered/hex-tagged log families (`[~xN similar lines]`) while preserving diagnostic errors and bottom tails verbatim.
  - Replaced naive `split(delim)` CSV detection with an RFC-4180 quote-aware finite-state column counter (`count_delimited_columns`) handling embedded commas and escaped `""` quotes cleanly without regex.
- **Single-Pass `O(N)` Longest-Match Prose Engine + Quoted-Literal Verbatim Locking (`crates/zenko-memory/src/compress.rs`):**
  - Replaced `O(P × N)` multi-pass string rescanning with a single-pass `O(N)` longest-match word-boundary scanner (`apply_phrase_tables_single_pass`).
  - Extended `segment_inline_line` and `StructuralDigest` validation to lock double-quoted `"literals"` (honoring `\"` escapes) verbatim alongside inline code spans, Markdown links, headings, tables, frontmatter, and fenced code blocks.

### Fixed
- **Custom `#!` Shebang Execution in `script_block` (`crates/zenko-tools/src/blocks.rs`):**
  - Added deterministic `#!` shebang parsing (`parse_shebang_command`) so saved script blocks with custom interpreters (e.g., `#!/usr/bin/env python3`, `#!/bin/bash -e`) execute under their declared interpreter inside the jailed workspace and Linux Landlock write sandbox instead of forcing `/bin/sh`.

## [0.0.21] - 2026-11-25

Closed the final three autonomous automation gaps in Zenko's native English Concise & Read-Less Engine so background memory compression, durable savings telemetry, and concise tool/skill adoption operate automatically without user prompting.

### Added
- **Dream Stage-0 & Post-Patch Auto-Compression (`crates/zenko-runtime/src/dream.rs`, `constants.rs`):**
  - Added deterministic, zero-token `auto_compress_bloated_memory_docs` inside Dream Stage-0 hygiene (runs even offline) and right after Stage-1 doc patches: automatically compacts `USER.md`, `MEMORY.md`, and `PROJECT_MEMORY.md` once they reach 65% of their token cap, saving a `.history/` rollback snapshot before writing and validating headings/code/paths verbatim.
- **Durable SQLite Read-Less Savings Across Restarts (`crates/zenko-runtime/src/runtime/turn.rs`, `mod.rs`):**
  - Persisted `read_less` compression metadata onto SQLite `EventKind::ToolResult` events and hydrated per-branch totals (`sum_persisted_read_less_stats`) inside `inspect_session_context`, so `context_inspect` and the WebUI Context Breakdown retain exact Read-Less savings across daemon restarts and branch switches without double-counting.
- **Autonomous Efficiency Prompt Guidance (`crates/zenko-core/src/registry.rs`):**
  - Added scope-aware autonomous efficiency guidance inside `<concise_mode>` prompting the orchestrator to proactively load `concise-review` / `concise-commit` / `concise-explore` via `skill_view`, save recurring shell workflows as Landlock-sandboxed `script_block`s, and compact bloated memory docs via `memory_tool(action="compress_doc")` (strictly omitting mutation hints in read-only Plan Mode and read-only subagents).
- **Low-Disk Cargo Profiles, Disk-Guarded Build Script & Root `justfile` (`Cargo.toml`, `.cargo/config.toml`, `scripts/build.sh`, `justfile`, `AGENTS.md`):**
  - Added lean `[profile.dev]` / `[profile.test]` settings (`debug = 0` for all 200+ third-party crates, `debug = "line-tables-only"` for workspace crates, `incremental = false`) slashing `target/` footprint by ~75–85% and eliminating multi-GB incremental cache accumulation across version bumps.
  - Upgraded `scripts/build.sh` with a pre-flight disk-space guard, `cargo-sweep` `--stamp`/`--file`/`--maxsize` automatic old-version eviction, post-test executable pruning, `/tmp/zenko_*` cleanup, and storage breakdown reporting (`disk`), exposed cleanly via root `justfile` (`just check`, `just test`, `just build`, `just release`, `just verify`, `just sweep`, `just clean`, `just disk`).

## [0.0.20] - 2026-11-25

Integrated a **100% pure-Rust, 100% English Native Concise & Read-Less Engine** across Zenko (`zenko-core`, `zenko-memory`, `zenko-tools`, `zenko-runtime`, `zenko-webui`, and the embedded WebUI) — bringing output compression modes, Read-Less + CCR input recovery, pure-Rust Markdown memory compression, English concise skills, reusable sandboxed script blocks, and live savings telemetry with zero Go proxy, zero Python, zero Node/JS hooks, zero `/commands`, zero non-English/Chinese modes, zero regex, and zero `.unwrap()`/`.expect()` in production paths.

### Added
- **Declarative English Concise Modes & Auto-Clarity (`zenko-core`, `zenko-runtime`):**
  - Added `ConciseMode` (`off`, `lite` default, `full`, `ultra`), `CONCISE_MODE_REGISTRY`, and `render_concise_prompt_block` in `crates/zenko-core/src/registry.rs` with strict Verbatim Payload Protection (code, paths, commands, stack traces, errors untouched) and Auto-Clarity safety hatch (auto-relaxes to full sentences on security warnings, irreversible confirmations, and Plan Mode trade-offs).
  - Propagated active `<concise_mode>` cleanly into both the main orchestrator and all spawned subagents while keeping the Tier-1 KV-cache prefix invariant; extended `orchestrate_control` with live `concise_mode` switching persisted on the session branch.
- **Read-Less Structural Tool-Output Compressor + Native CCR Recovery (`crates/zenko-runtime/src/read_less.rs`, `artifacts.rs`, `tool_router.rs`):**
  - Added zero-regex deterministic structural compression for bulky `shell_exec`, `job_read`, `git_tool`, `web_fetch`, `script_block`, and `mcp__*` outputs (JSON array/schema sampling, Cargo/Pytest/Go/JS test pass collapsing while preserving failures verbatim, CSV header+sample+tail, and build/log progress deduplication), strictly exempting `read_file` and code editors so line numbers and hash anchors stay intact.
  - Persisted 100% verbatim raw bytes atomically (`0600`) via `artifacts::persist_raw_ccr_artifact` (`artifact:<session>/<call>.txt`) for exact byte recovery via `read_file` or `code_search`, and tracked live per-session Read-Less savings in `ContextBreakdown` and `context_inspect`.
- **Pure-Rust Markdown Document Compressor (`crates/zenko-memory/src/compress.rs`, `snapshot.rs`, `memory_cron.rs`):**
  - Built a zero-regex finite-state English Markdown compressor replacing Python `compress.py`: preserves YAML frontmatter, headings, fenced code blocks, Markdown tables, inline code spans, URLs, and file paths verbatim with a strict structural integrity validator and automatic `.history/` rollback snapshot.
  - Exposed via `memory_tool(action="compress_doc", doc=..., mode=..., dry_run=...)` with Plan Mode safety gating (`dry_run: true` allowed read-only).
- **Built-In English Concise Workflow Skills (`crates/zenko-memory/src/snapshot.rs`):**
  - Seeded `concise-review.md` (`path:L<line> [bug|risk|nit|q]: <problem>. <fix>.`), `concise-commit.md` (terse Conventional Commits without auto-staging), and `concise-explore.md` (exact `path:Lstart-Lend` reconnaissance) via `ensure_defaults()`.
- **Reusable Landlock-Sandboxed Script Blocks (`crates/zenko-tools/src/blocks.rs`, `lib.rs`, `policy.rs`, `guard.rs`):**
  - Added deferred `script_block` tool (`list`, `save`, `read`, `run`, `delete` under `hub:blocks/<slug>.sh`) executing inside jailed workspace `cwd` + Linux Landlock write sandbox with zero JS/QuickJS engine bloat and Plan Mode read-only enforcement.
- **WebUI Concise Controls & Read-Less Telemetry (`crates/zenko-webui`, `webui/src/components/chat/PromptComposer.tsx`):**
  - Added `PATCH /api/context/concise`, exposed `CONCISE_MODE_REGISTRY` in `/api/bootstrap`, and added theme-compliant (60/30/10) English Concise Mode selector, Read-Less toggle, and live Read-Less CCR token savings inside the Context Breakdown popover.

## [0.0.19] - 2026-11-25

Upgraded Zenko's tool system and execution runtime into **Minicode Tool Architecture v4**, closing the four highest-impact gaps identified against Claude Code, OpenAI Codex CLI, Oh My Pi, and Prime Agent — in 100% pure Rust with zero hardcoding, zero `if/else` ladders, zero ad-hoc regex, zero `.unwrap()`/`.expect()` in production paths, and zero `/commands`.

### Added
- **KV-Cache-Safe Frozen Tool Surface & `invoke_tool` Proxy (`zenko-core`, `zenko-tools`, `zenko-runtime`):**
  - Added `ContextConfig.freeze_tool_schemas` (default `true`) so top-level `body["tools"]` stays byte-identical across turns and activating deferred or MCP tools never invalidates provider KV prompt caches.
  - Added ultra-lean eager `invoke_tool` (`InvokeToolProxy` in `crates/zenko-tools/src/batch.rs`) for direct policy-checked invocation of any discovered deferred or MCP tool while preserving full Plan Mode, read-only subagent, `PreToolUse`/`PostToolUse` hook, parallel `Shared` wave routing, and `OrchestratorGuard` accounting.
  - Upgraded `tool_search` (`crates/zenko-tools/src/context_tools.rs`) to return compact inline parameter schemas alongside `invoke_tool` usage hints so the model can invoke deferred tools immediately without schema-array mutation.
- **OS-Native Linux Landlock Kernel Write Sandbox (`crates/zenko-tools/src/sandbox.rs`, `shell.rs`, `jobs.rs`):**
  - Added a zero-dependency, pure-Rust + `libc` Linux Landlock ABI v1–v3 write sandbox (`ShellSandboxPolicy`, configurable via `PolicyConfig.os_sandbox = true`) using async-signal-safe pre-allocated `CString` paths in `pre_exec`.
  - Confines writes of all `shell_exec`, persistent PTY, and `LocalJobSupervisor` child processes strictly to `workspace_dir`, `hub_dir`, `/tmp`/`$TMPDIR`, `$CARGO_HOME`, and standard device sinks (`/dev/null`, `/dev/tty`, slave PTY), with graceful degradation on kernels without Landlock.
- **Pure-Rust Persistent Unix PTY Sessions (`crates/zenko-tools/src/pty.rs`, `shell.rs`, `runtime/mod.rs`):**
  - Added `PtySessionManager` using POSIX `libc` PTY primitives (`posix_openpt`, `grantpt`, `unlockpt`, `ptsname_r`) and Tokio `AsyncFd::register` epoll-backed I/O — zero extra crates, zero busy-polling, non-echo sentinel framing, zero-regex ANSI escape stripping (`strip_ansi_escapes`), bounded session count, idle reaping, and clean shutdown cleanup.
  - Extended `shell_exec` with `session`, `pty_action` (`exec` | `send` | `close` | `list`), `input`, and `wait_ms` so `cd`, `export`, virtualenvs, and `isatty()` state persist seamlessly across turns.
- **Zero-Python Dataflow Variable-Piped `batch_tools` (`crates/zenko-tools/src/batch.rs`, `runtime/bridges.rs`):**
  - Added per-step `as` variable binding, `quiet: true` output suppression (keeping noisy intermediate results off the LLM context window), zero-regex `extract` projections (`raw`, `lines`, `first_line`, `paths`, `contains:<text>`, `head:<n>`, `tail:<n>`), and `${var}` / `${var.first_line}` / `${var.last_line}` / `${var.line[i]}` / `${var.count}` template interpolation in downstream step arguments.

### Changed
- **Trimmed Orthogonal Eager Tool Set (`fs_tools.rs`, `orchestrator_tools.rs`, `context_tools.rs`, `runtime/mod.rs`):**
  - Moved `list_dir`, `record_brainstorm`, `plan_update`, and `orchestrate_control` to `ToolExposure::Deferred` and gated `skill_view` dynamically on `ToolAvailabilityContext.has_skills`, shrinking the default eager tool surface to `<= 13` orthogonal tools.
- **Tightened Scoped Policy Guard on Direct & Nested Dispatch (`tool_router.rs`):**
  - Hardened `tool_router::dispatch_tool_call` to enter a tightened `ActiveSessionScopeGuard` whenever `effective_permission` is stricter than the active scope so `invoke_tool` and `batch_tools` can never bypass Plan Mode or stricter caller overrides.

## [0.0.18] - 2026-11-24

Upgraded Zenko's minicode tool system and runtime dispatch pipeline into **Minicode Tool Architecture v3**, synthesizing patterns from Claude Code, OpenAI Codex CLI, and Oh My Pi in 100% pure Rust with zero hardcoding, zero `if/else` ladders, zero ad-hoc regex, and zero `/commands`.

### Added
- **Session File-Freshness & Stale-Read Guard (`SessionFileTracker`, `FileStateBridge` across `zenko-core`, `zenko-tools`, `zenko-runtime`):**
  - Added bounded LRU `SessionFileTracker` (`crates/zenko-runtime/src/file_state.rs`) wired into `ToolContext`, recording single-pass streaming SHA-256 content fingerprints on `read_file` and synchronizing post-mutation fingerprints on `write_file`, `hash_edit`, `apply_patch`, and `lsp_tool(action="rename")`.
  - Enforced **Guarded Freedom** on `write_file` (blocks blind overwrites of existing unread files with an inline hash-anchored preview unless `overwrite: true` is passed) and strict external-drift detection across `write_file`, `apply_patch`, and `lsp_tool(action="rename")` so out-of-band disk edits are never silently clobbered.
- **Two-Phase Atomic Multi-File `apply_patch` & Codex Patch Grammar (`crates/zenko-tools/src/codex_patch.rs`, `fs_tools.rs`):**
  - Added a pure-Rust, zero-regex finite-state parser for OpenAI Codex's native `*** Begin Patch` / `*** Add File:` / `*** Update File:` / `*** Move to:` / `*** Delete File:` / `*** End Patch` multi-file patch grammar.
  - Extended `apply_patch` with structured multi-file `files[]` transactions (`create`, `update`, `move`, `delete`) alongside single-file mode, executing two-phase validation completely in memory before touching disk (zero partial writes on failure) and returning a combined multi-file unified diff.
- **Fine-Grained Hashline Operations in `hash_edit` (`HashEditOp` in `zenko-core`, `fs_tools.rs`):**
  - Added Oh My Pi–style declarative `op` support (`replace`, `insert_after`, `insert_before`, `delete`) per hunk with deterministic conflict checking and bottom-up atomic application, allowing hash-anchored insertions without duplicating existing anchor lines.
- **IDE-Grade LSP Semantic Workspace Rename (`LspAction::Rename` in `zenko-core`, `lsp.rs`, `policy.rs`, `guard.rs`):**
  - Added `lsp_tool(action="rename")` with `textDocument/prepareRename` + `textDocument/rename`, workspace-jailed `WorkspaceEdit` (`changes` + `documentChanges` `TextDocumentEdit`/`CreateFile`/`RenameFile`/`DeleteFile`), deterministic UTF-16 → UTF-8 byte-offset mapping, `dry_run: true` unified diff previews, Plan Mode / read-only subagent / Strict risk gating (`is_lsp_tool_args_mutating`), and pre-mutation Shadow-Git checkpoint support.
- **Modular `ToolRouter` Dispatch Pipeline (`crates/zenko-runtime/src/tool_router.rs`):**
  - Extracted Zenko's tool dispatch loop out of `turn.rs` into a dedicated, stage-based `ToolRouter` pipeline (`Resolve & Availability -> Coerce -> Hooks & Policy/HITL -> Execute -> Post-Execution Enrichers`) shared across top-level turns and `batch_tools`, with multi-file edited-path extraction for warm LSP Diagnostics-on-Edit.
- **Structured HTML-to-Markdown `web_fetch` + Bounded 15-Min Cache & Jailed `shell_exec` `cwd` (`crates/zenko-tools/src/html_md.rs`, `web.rs`, `shell.rs`):**
  - Added a pure-Rust zero-regex finite-state HTML-to-Markdown converter (preserving headings, fenced/inline code, resolved links, lists, blockquotes, tables, and decoded named/numeric HTML entities while never mangling plain text/JSON/source code), a bounded 64-entry / 15-minute LRU+TTL cache (`format: "markdown" | "text" | "raw"`, `bypass_cache`), and workspace/hub-jailed `cwd` support on `shell_exec`.

### Changed
- **Centralized Tool v3 Constants & Plan Mode Scoping (`constants.rs`, `types.rs`, `policy.rs`, `guard.rs`, `runtime/mod.rs`):**
  - Centralized all v3 limits, caches, and enum registries in `crates/zenko-core/src/constants.rs`, moved `TOOL_LSP` into `PLAN_MODE_SCOPED_TOOLS` with per-argument mutation gating, and strengthened `SessionToolScope` so read-only subagent roles block mutating scoped tools automatically.

## [0.0.17] - 2026-11-24

Hardened Zenko's Minicode Tool System & Runtime Dispatch after a full forensic audit: fixed 8 subtle tool/runtime bugs, centralized every remaining tool literal and magic number into `zenko-core/src/constants.rs`, and refactored all remaining `if/else if` and disguised `match ()` ladders into declarative tables and pure `match` dispatch.

### Fixed
- **UTF-8 & Raw Byte String Fidelity in Structural Outline (`outline.rs`):** Fixed UTF-8 mojibake in `scan_brace_line` by buffering raw UTF-8 bytes cleanly instead of casting bytes to Latin-1 chars, and added full `r#*"` / `br#*"` / `cr#*"` Rust raw-string detection so quotes/braces inside raw byte strings never desync brace depth.
- **Byte-Budget vs. Past-EOF Attribution in Multi-Range `read_file` (`fs_tools.rs`):** Tracked `hit_byte_budget` separately so omitted slices after hitting the read byte cap report budget exhaustion instead of falsely claiming `past EOF`.
- **Safe Integer Coercion Bounds (`coerce.rs`):** Guarded float-to-`i64` coercion to the exact lossless IEEE-754 integer range (`±COERCE_SAFE_INT_MAX_F64`) so oversized numeric strings like `"1e30"` never silently saturate to `i64::MAX`.
- **Policy-First Deferred Tool Activation (`turn.rs`):** Moved auto-activation of deferred tools behind `PreToolUse` hooks and `evaluate_policy` so denied calls in Plan Mode never leak into `activated_deferred`.
- **Mixed Edit + Verify Tracking & Inner Batch Coercion (`guard.rs`, `runtime/mod.rs`):** Added per-step mutation/verification accounting for `batch_tools` in `OrchestratorGuard::record_tool_outcome`, and recursively coerced inner `batch_tools` arguments upfront so lazy pre-mutation Shadow-Git checkpoints inspect normalized inner payloads.
- **Scoped `context_inspect` Counts & MCP Shadow Protection (`lib.rs`, `mcp_client.rs`, `runtime/mod.rs`):** Scoped `inspect_session_context` tool counts and deferred catalogs via `effective_session_scope`, and added Hermes-style `ToolRegistry::register_if_absent` shadow protection for external MCP proxy tools.

### Changed
- **Zero Hardcoding & Declarative Tables (`constants.rs`, `outline.rs`, `policy.rs`, `jail.rs`, `glob.rs`, `search.rs`, `lib.rs`, `coerce.rs`, `shell.rs`, `web.rs`, `jobs.rs`, `memory_cron.rs`, `lsp.rs`, `web_search.rs`, `agents.rs`):**
  - Centralized all remaining tool name literals (`TOOL_*`, `SUBAGENT_ROLE_CUSTOM`) and magic constants (`FS_MAX_*`, `JAIL_MAX_SYMLINK_HOPS`, `LSP_LOCATION_SNIPPET_MAX_BYTES`, `TOOL_BM25_*`, `TOOL_FUZZY_*`, `OUTLINE_MAX_*`, `CODE_SEARCH_*`, `BATCH_TOOLS_MIN_STEP_CHARS`) in `zenko-core/src/constants.rs`.
  - Replaced all `match () { _ if ... }` prefix ladders in `outline.rs`, all 4 repetitive shell wrapper loops in `policy.rs` (`SHELL_WRAPPER_SPECS`), duplicated path display logic (`jail::format_display_rel_path`), and all remaining `if/else if` chains across `zenko-tools` and `zenko-runtime/src/policy.rs` with declarative tables and `match` dispatch.

## [0.0.16] - 2026-11-24

Upgraded Zenko's minicode tool system and runtime execution architecture into **Minicode Tool Architecture v2**, synthesizing patterns from Hermes Agent, Oh My OpenCode, and Prime Agent in 100% pure Rust with zero hardcoding, zero `if/else` ladders, zero ad-hoc regex, and zero `/commands`.

### Added
- **Declarative ToolSets & Dynamic Availability (`ToolSet`, `TOOLSET_REGISTRY`, `ToolAvailabilityContext` across `zenko-core`, `zenko-tools`, `zenko-runtime`):**
  - Added 10 declarative `ToolSet` domains (`core_fs`, `code_intel`, `execution`, `git`, `web`, `memory`, `orchestration`, `subagents`, `automation`, `mcp`) and per-tool `toolset()` / `is_available(&ToolAvailabilityContext)` trait predicates across all built-in and MCP tools.
  - Upgraded `tool_search` (`context_tools.rs`, `ContextBridge`) to support BM25 `query`, whole-domain `toolset` activation (including `all`), exact `names` batch activation, and `action="list_toolsets"`, strictly filtered by active role, Plan Mode, `tool_subset`, and environment availability.
- **Pure-Rust Self-Healing Dispatch Membrane (`crates/zenko-tools/src/coerce.rs`, `turn.rs`):**
  - Added deterministic finite-state JSON repair for malformed/fenced/truncated `{"_raw": ...}` SSE payloads, recursive schema-guided argument coercion (stringified ints/floats/booleans/JSON arrays, `camelCase` → schema `snake_case` key normalization, case-insensitive `enum` snapping), Damerau-Levenshtein + token fuzzy suggestions on unknown tools, and compact schema-hint error enrichment on parameter validation failures.
- **Zero-Dependency Structural Code Outline Engine (`crates/zenko-tools/src/outline.rs`, `read_file`):**
  - Added deterministic byte/token/brace/indentation structural outline extraction across Rust, TypeScript/JavaScript, Python, Go, C/C++/Java/C#/Kotlin/Swift, Markdown, and TOML — ignoring comments, strings, raw strings, and code fences, returning hash-anchored symbol spans via `read_file(mode="outline")`.
- **Wave-Aware Composable Batch Tool (`crates/zenko-tools/src/batch.rs`, `ToolBatchBridge`, `RuntimeBatchBridge`):**
  - Added eager `batch_tools` allowing up to 12 heterogeneous tool invocations per turn in wave-parallel mode (`stop_on_error: false`) or sequential short-circuit pipeline mode (`stop_on_error: true`), routing every inner call through full role scope, Plan Mode, `ExecPolicy`, argument coercion, hooks, LSP diagnostics-on-edit, and Zenko's 3-state parallel wave scheduler (with strict anti-recursion protection).

### Changed
- **Next-Gen Minicode File & Search Tools (`fs_tools.rs`, `search.rs`):**
  - Upgraded `hash_edit` to support atomic multi-hunk bottom-up `edits[]` with overlap rejection and **inline fresh hash-anchored recovery windows** on hash mismatch (eliminating wasted `read_file` retry turns).
  - Upgraded `apply_patch` with strict multi-occurrence ambiguity protection (reporting exact line numbers + hash previews instead of silently editing match #1), `replace_all`, `expected_occurrences`, and transactional multi-edit `edits[]`.
  - Upgraded `read_file` with multi-slice `ranges[]` streaming and `mode="outline"`.
  - Upgraded `code_search` with zero-regex `file_glob` (powered by `CompiledGlob`), `fixed_strings`, `output_mode` (`matches` / `files_only` / `count`), `context_lines`, optional `with_hashes`, and `hub:` path display parity.
- **Role-Scoped `<deferred_tools>` Catalog, Worker Hygiene & LSP Diagnostics Fix (`lib.rs`, `registry.rs`, `runtime/mod.rs`, `turn.rs`, `policy.rs`, `snapshot.rs`):**
  - Filtered Tier-2 `<deferred_tools>` catalogs strictly by active `SessionToolScope` (grouping by `ToolSet` and hiding denied or unavailable tools such as unconfigured MCP servers or root-only automation tools from subagents and Plan Mode).
  - Tightened `coder`, `verifier`, and `custom` worker `denied_tools` to exclude root-only `orchestrate_control`, `goal_loop`, and `skill_manage`, and enabled `batch_tools` across read-only specialist roles (`explorer`, `architect`, `critic`, `researcher`).
  - Fixed `extract_edited_rel_paths` for `apply_patch` so warm LSP Diagnostics-on-Edit fires on `apply_patch` edits as well as `hash_edit` and `write_file`.

## [0.0.15] - 2026-11-24

Rebuilt Zenko's core Orchestrator & Brainstorming engine into **Orchestrator v2 ("LLM-Sovereign Membrane")**, synthesizing ideas from Hermes Agent, Oh My OpenCode, and Prime Agent to hand full control and freedom to the LLM with zero hardcoded keyword dictionaries, zero `if/else` ladders, zero regex, and zero `/commands`.

### Added
- **Sovereign Live Control Tool (`orchestrate_control` across `zenko-core`, `zenko-tools`, `zenko-runtime`):**
  - Added eager tool `orchestrate_control` allowing Zenko to self-declare/transition turn posture (`direct`, `research`, `brainstorm`, `plan`, `implement`, `verify`, `review`), dynamically shift its own cognitive gear (`fast`, `lead`, `plan`, `advisor` configured model slots + `low`/`medium`/`high` reasoning effort) mid-turn while respecting explicit user model overrides, and trigger fresh 5-way parallel workspace/git/memory/DAG recon on demand.
- **Iterative Metis/Momus Brainstorming (`record_brainstorm` v2, `BrainstormSpec`, `ChatStream.tsx`):**
  - Upgraded `BrainstormSpec` with multi-round lifecycle `stage` (`exploring` → `refined` → `committed`), `parent_spec_id`, `assumptions`, `blind_spots`, `decision_criteria`, and adversarial `critique`, rendered cleanly inside the WebUI `BrainstormCard` under the active 60/30/10 theme.
  - Preserved 100% LLM selection authority (never overrides `selected_option_id` or forces choices on `exploring` drafts; returns only non-blocking structural reflections).
- **Category-Aware & Dynamic Specialist Subagents (`SUBAGENT_ROLE_REGISTRY`, `spawn_agent` v2, `agents.rs`):**
  - Added declarative read-only `critic` (adversarial Metis + Momus plan/diff reviewer on `Advisor` tier) and `researcher` (web/docs/memory/session investigator on `Fast` tier) specialist roles.
  - Added dynamic `custom` specialist composition (`custom_directive`, `cognitive_tier`, `read_only`, `tool_subset`) strictly bounded by the parent's monotonic `PermissionMode` ceiling and resource/cost limits.

### Changed
- **Language-Agnostic Structural Profiler & Neutral Situational Membrane (`triage.rs`, `orchestrator/mod.rs`, `snapshot.rs`):**
  - Replaced ~180 hardcoded English keyword/phrase rules in `triage.rs` with objective, language-neutral structural profiling (`PermissionMode::Plan`, `is_first_turn`, mentioned paths, prior errors) so prompts in any phrasing or language are never misclassified or locked out.
  - Replaced 7 hardcoded step-by-step protocol templates in `orchestrator/mod.rs` with a neutral Tier-3 `<situational_awareness>` snapshot and concise Tier-1 capability posture in `snapshot.rs`.
- **Zero Keyword-Guessed Handcuffs (`guard.rs`):**
  - Removed keyword-guessed pre-tool `IntentGate`/`PhaseGate` mutation blocks and hardcoded exploration-streak soft-limit nags; kept explicit `PermissionMode::Plan`, `ExecPolicy`, spend/iteration caps, and objective infinite-loop / doom-loop circuit breaking intact.
  - Broadened post-edit verification evidence to recognize any passing shell check, `lsp_tool` diagnostics, or `verifier`/`critic` subagent completion, and softened completion checks into a single non-blocking advisory note.

## [0.0.14] - 2026-11-24

Aligned chat message edges and the Chat Input Box to the exact same column width by hanging the Agent avatar outside the left edge and the User avatar outside the right edge on desktop viewports.

### Changed
- **Hanging Avatars & Unified Chat/Composer Column Width (`ChatStream.tsx`, `PromptComposer.tsx`, `App.tsx`, `StudioPreviews.tsx`):**
  - Positioned the Zenko Agent avatar cleanly outside the left edge (`sm:right-full sm:mr-3`) and the User avatar cleanly outside the right edge (`sm:left-full sm:ml-3`) on desktop screens while preserving inline layout on narrow mobile screens and expanding `.zenko-turn-item` horizontal paint gutters (`sm:-mx-12 sm:px-12`) so `content-visibility: auto` never clips hanging avatars.
  - Shared `CHAT_WIDTH_CLASS` and matching horizontal gutters (`px-5 sm:px-14 md:px-16`) between `ChatStream` and `PromptComposer` so assistant prose, tables, user bubbles, and the Chat Input Box stay pixel-aligned across every `content_width` preset (`narrow`, `balanced`, `wide`, `full`).

## [0.0.13] - 2026-11-24

Added colored markdown headings in chat output, made Skiper106 Smooth Caret the default across chat & search inputs, integrated Skiper UI (`skiper106`, `skiper101`, `skiper98`, `skiper96`, `skiper89`, `skiper94`, `skiper87`, `skiper43`, `skiper40`, `skiper26`, `skiper4`) and VengeanceUI (`line-hover-link`, `liquid-button`, `animated-button`) theme options, and installed Libraries.dev `border-beam` with active agent-task animations for the Chat Input Box.

### Added
- **Colored Markdown Headings in Chat Output (`components.chat_page.heading_style`, `index.css`, `StudioPreviews.tsx`):**
  - Colored all assistant markdown headings (`H1–H5`) with your active global theme accent by default, with 5 customizable styles under **Chat Page → Responses & Elements → Streaming Response**: `accent` (Default), `accent_bar`, `accent_underline`, `soft_tint`, and `neutral`.
- **Skiper106 Smooth Caret Default (`skiperui/smooth-caret.tsx`, `surface.input_caret_style`):**
  - Added `SmoothCaretInput` (Canvas 2D spring caret) and `SmoothCaretTextarea` (multiline wrapped mirror spring caret) as the **default** (`skiper106_smooth`) across the Chat Input Box and all Search inputs (Appearance Studio, Model Picker, Memory Search, Flight Recorder Logs, and Command Palette), with theme toggle options (`skiper106_smooth`, `native_accent`, `native_default`).
- **Libraries.dev `border-beam` & Active Agent Task Animations (`border-beam@1.4.1`, `ui/ComposerBeam.tsx`, `PromptComposer.tsx`):**
  - Added dedicated **Chat Input Box → Active Task Animation** page in Appearance Studio featuring `BorderBeam` (`md`, `sm`, `line`, `pulse-outside`, `pulse-inner`) plus 60/30/10 Accent Task Animations (`accent_orbit`, `accent_shimmer_edge`, `accent_breathe_halo`), trigger modes (`while_working` default, `always`, `on_focus`, `off`), color palettes (`theme_accent`, `colorful`, `mono`, `ocean`, `sunset`, `forest`, `candy`, `ice`, `gold`), and glow intensity slider (`0.2–1.0`).
- **Skiper UI Theme Options (`skiper101`, `skiper98`, `skiper96`, `skiper89`, `skiper94`, `skiper87`, `skiper43`, `skiper40`, `skiper26`, `skiper4`):**
  - **Tooltips (`Tooltip.tsx`):** Added `skiper101_arrow` (outlined directional arrow tooltip) and `skiper43_vercel` (precision clip-path inverted Vercel tooltip).
  - **Right Dock (`skiperui/vertical-tooltip-menu.tsx`, `FloatingDock.tsx`):** Added `skiper98_vtooltip` vertical tooltip menu with clip-path sliding labels.
  - **Scroll Fade & Progress (`skiperui/scroll-effects.tsx`, `CompactScrollArea.tsx`, `message-scroller.tsx`):** Added `surface.scroll_indicator` supporting `skiper87_fade` (boundary edge fades), `skiper89_bar` (top progress bar 001 + percentage pill), `skiper94_ring` (floating circular SVG ring 002), and `skiper87_and_89`.
  - **Animated Links (`index.css`, `ChatStream.tsx`, `rareui/streaming-response.tsx`):** Added pure-CSS Skiper40 links (`skiper40_bracket`, `skiper40_arrow_shift`, `skiper40_highlight_fill`) and exposed all 11 VengeanceUI `LineHoverLink` variants (including `strike`, `pulse`, `bounce`).
  - **Theme Toggle Transitions & Icons (`motion/theme-toggle.tsx`, `skiperui/theme-toggle-icons.tsx`):** Added Skiper26 View Transition variants (`skiper26_bottom_up`, `skiper26_top_down`, `skiper26_diagonal`) and 5 Skiper4 animated SVG toggle icons (`skiper4_eclipse`, `skiper4_rays`, `skiper4_horizon`, `skiper4_classic_fill`, `skiper4_minimal_dot`).
- **VengeanceUI Liquid & Animated Buttons (`vengeanceui/liquid-button.tsx`, `vengeanceui/animated-button.tsx`, `index.css`):**
  - Added `vengeance_liquid` and `vengeance_animated` to global `buttons.style` so primary buttons across the entire WebUI inherit glossy liquid wave or moving diagonal shine physics.

## [0.0.12] - 2026-11-24

Split Theme Studio's bundled `Borders, Errors & States` section into 5 separate independently customizable components and restored active theme accent-color highlighting on important bold words across all chat output renderers.

### Added
- **Separate Theme Studio Components (`studio_schema.rs`, `ui.rs`, `ChatElements.tsx`, `ChatStream.tsx`, `StudioPreviews.tsx`):**
  - Split `Chat Page → Responses & Elements → Borders, Errors & States` into 5 dedicated sub-component pages, each with its own focused live preview and independent controls:
    1. **Element Borders** (`surface.element_chrome`: `borderless`, `minimal`, `bordered`)
    2. **Error State** (`components.chat_page.error_state_style`: `aui_error_card`, `easeui_alert`, `beui_alert`, `compact_bar`, `minimal_text`)
    3. **Stopped Run** (`components.chat_page.stopped_run_style`: `aui_stopped_run`, `compact_banner`, `accent_rail`, `minimal_notice`)
    4. **File Tree** (`components.chat_page.file_tree_style`: `aui_file_tree`, `compact_list`, `minimal_flat`, `bordered_panel`)
    5. **Web Search** (`components.chat_page.web_search_style`: `aui_web_search`, `rareui_web_search`, `compact_cards`, `minimal_links`)
- **Customizable Important-Word Highlight (`components.chat_page.emphasis_style`, `index.css`, `StudioPreviews.tsx`):**
  - Added `Important Words Highlight` control under `Chat Page → Responses & Elements → Streaming Response` with 4 theme-aware styles: **Theme Accent** (`accent`, default), **Soft Accent Pill** (`accent_glow`), **Marker Pen Stroke** (`marker_stroke`), and **Neutral Bold** (`neutral_bold`).

### Fixed
- **Missing Accent Color on Important Words in Chat Output (`index.css`, `agents/streaming-response.tsx`, `rareui/web-search.tsx`, `assistant-ui/elements/streaming-text.tsx`, `lib/elements.ts`):**
  - Restored active theme `var(--accent)` coloring on `<strong>` / `<b>` markdown emphasis across `.zenko-prose`, `BeUI StreamingResponse`, `EaseUI StreamingResponse`, `RareUI StreamingResponse`, `RareUI WebSearch`, and `Word-Stream Accent` (`StreamingText` now deterministically parses `**bold**` segments alongside inline `` `code` ``).
  - Removed conflicting `[&_code]:bg-muted` and `[&_a]:underline` overrides from `BeUI StreamingResponse` so `.zenko-prose` accent inline code pills and animated underline links render cleanly without override clashes.

## [0.0.11] - 2026-11-24

Cleaned up the Theme page cards by removing redundant per-card dark/light switches and fixed end-to-end theme & live WebUI integration gaps across SkecherUI, KokonutUI, Skiper96, AIMRAN, EaseUI, RareUI, Motion-Brew, and Assistant-UI Elements.

### Fixed
- **Color Themes Page (`ui/ThemeSelector.tsx`):**
  - Removed the redundant per-card Dark/Light moon/sun toggle pills under every theme preset card so only the single global mode toggle governs Dark/Light mode.
- **SkecherUI Gooey Tooltip Portal Rendering (`skecherui/gooey-tooltip.tsx`, `ui/Tooltip.tsx`):**
  - Portal-rendered `SkecherGooeyTooltip` onto `document.body` at fixed viewport coordinates (`z-[9999]`) so liquid metaball tooltips never clip inside `overflow-hidden` sidebars, floating docks, scroll containers, or Studio preview panels, and added a distinct droplet stem preview in `ZenkoTooltipBadge`.
- **Live Chat Tool Calls, Citations & AIMRAN Copy Command (`chat/ChatElements.tsx`, `chat/ChatStream.tsx`, `assistant-ui/elements/tool-call.tsx`, `aimran/copy-command.tsx`, `StudioPreviews.tsx`):**
  - Wired `diff_style` (`kokonut_diff`, `aui_code_diff`, `aui_reviewable_diff`), `shell_style` (`aimran_copy_cmd`, `aui_terminal_block`, `aui_code_runner`), `web_search_style` (`rareui_web_search`, `aui_web_search`), `FileTree`, and `ToolError` directly into expanded `Collapsible+` / `Collapsible` tool rows so user selections apply live in default chat.
  - Added full multi-line `copyValue` support to `CopyCommand` and a clean CLI command preview in Studio `code_blocks`, and respected `citations_style` (`kokonut_citations`, `minimal_pills`) under `beui_inline` message actions.
- **Authentic Skiper96 Expandable Tabs Everywhere (`motion/tabs.tsx`, `SetupView.tsx`, `GatewaysView.tsx`, `FloatingActivityMonitor.tsx`, `MemoryView.tsx`):**
  - Upgraded `<Tabs variant="skiper96">` with spring-expanding icon + label transitions (`AnimatePresence` collapse/expand) and added semantic icons across Setup, Gateways, Memory, and Floating Activity Monitor tabs.
- **KokonutUI Context Meter & Dashboard Live Integration (`kokonutui/ai-context-meter.tsx`, `PromptComposer.tsx`, `DashboardView.tsx`, `AppearanceStudio.tsx`):**
  - Wired live `/api/sessions/:id/context` breakdown fetching on `KokonutContextMeter` open and ensured all context meters remain visible across compact viewports.
  - Wired `tableStyle={ui.surface.table_style}` (including `easeui_table`) into all 3 Dashboard tables, rendered `RecapCard`, `UsageRing`, `MixChart`, and `RareFaq` live inside `DashboardMiniPreview`, and auto-enabled Recap/Quota visibility when picking Motion-Brew or AIMRAN styles in Appearance Studio.
- **Strict 60/30/10 Theme Parity (`motion-brew/{prompt-input,sidebar-nav}.tsx`, `rareui/web-search.tsx`, `assistant-ui/elements/{file-tree,reviewable-diff}.tsx`, `App.tsx`):**
  - Replaced all remaining hardcoded `zinc-*`, `#242423`, `bg-white`, `text-black`, and `emerald`/`red` classes with Zenko CSS variables and aligned top/bottom-centered toast stacks cleanly.

## [0.0.10] - 2026-11-24

Expanded Theme Studio customization with configurable Toast styles & 6-position placement (defaulting to Top Center), assistant model badge, composer model trigger pill, and user message bubble chrome controls, plus AIMRAN Library microinteractions, KokonutUI AI components, SkecherUI Gooey Tooltips, VengeanceUI Line Hover Links, EaseUI Message Bubbles, Responsive Glass Sidebar, and Skiper96 tabs across every tabbed panel.

### Added
- **Toast Customization & Placement (`surface.toast_style`, `surface.toast_position`, `App.tsx`, `easeui/toast.tsx`, `StudioPreviews.tsx`):**
  - Added dedicated **Toasts & Notifications** page in Appearance Studio with 5 toast styles (`default`, `easeui_toast`, `minimal_pill`, `accent_bar`, `glass_toast`), 6 viewport placements (`top_center` default, `top_right`, `top_left`, `bottom_center`, `bottom_right`, `bottom_left`), visual anchor preview, and interactive Test Info / Success / Error triggers.
- **Chat Header Model Badge, Composer Model Trigger & User Bubble Customization (`ChatStream.tsx`, `ModelPicker.tsx`, `easeui/message-bubble.tsx`):**
  - Added `components.chat_page.model_badge_style` (`bordered_chip`, `border_only`, `soft_fill`, `accent_tint`, `minimal_text`, `hidden`) for customizing the model pill beside the Zenko assistant header.
  - Added `components.composer.model_trigger_chrome` (`bordered_filled`, `border_only`, `background_only`, `accent_tint`, `minimal_bare`) for customizing the composer provider/model trigger pill.
  - Expanded `components.chat_page.user_bubble` with **EaseUI Message Bubble** (`easeui_bubble`), **Solid Accent**, **Background Only**, and **Minimal Borderless** options.
- **AIMRAN Library Microinteractions (`webui/src/components/aimran/{copy-command,stretchtext,marker-highlight}.tsx`):**
  - Added `CopyCommand` (selection highlight sweep + self-drawing check icon, wired into `shell_style` and `code_block_style` as `aimran_copy_cmd`), `Stretchtext` (inline expanding text with no popups, wired into `thinking_style` and `dashboard.recap_style`), and `MarkerHighlight` (organic hand-drawn highlighter strokes, wired into `dashboard.recap_style`).
- **KokonutUI AI Component Suite (`webui/src/components/kokonutui/ai-{diff-review,citations,context-meter,tool-calls,tool-approval,loading}.tsx`):**
  - Added `kokonut_diff` (split/unified diff review with line numbers), `kokonut_citations` (`components.chat_page.citations_style`), `kokonut_meter` (live token breakdown context meter), `kokonut_tool_calls` (animated collapsible tool steps), `kokonut_approval` (permission card with Hold-to-Allow safeguard for dangerous tools), and `kokonut_loading` (wave spinner status indicator).
- **SkecherUI Gooey Tooltip, VengeanceUI Line Hover Link & Responsive Glass Sidebar (`skecherui/gooey-tooltip.tsx`, `vengeanceui/line-hover-link.tsx`, `ui/responsive-sidebar.tsx`):**
  - Added `surface.tooltip_style = "skecher_gooey"` spring-animated tooltip with subtle trailing arrow.
  - Added `surface.link_hover_style` across 9 animated link hover variants (`zenko_underline`, `slide`, `double`, `grow`, `fade`, `swap`, `sweep`, `arc`, `scribble`) in markdown prose and RareUI streaming responses.
  - Added `left_menubar.frame_style = "responsive_sidebar"` frosted glass collapsible sidebar with full keyboard navigation (`Enter`, `Space`, `Tab`, `Escape`) and slim theme scrollbar.
- **Skiper96 Tabs Everywhere (`SetupView.tsx`, `GatewaysView.tsx`, `FloatingActivityMonitor.tsx`):**
  - Extended `<Tabs variant={tabsVariant}>` (including `skiper96`) to Setup, Gateways, and the Floating Activity Monitor so tabbed views inherit the active global tab style everywhere.

## [0.0.9] - 2026-11-23

Fixed Motion-Brew, VengeanceUI, SmoothUI, RareUI, EaseUI, and Skiper96 Theme Studio customization gaps so every component obeys Zenko's global 60/30/10 theme, slim theme scrollbar, corner radius, surface styles, and slot controls.

### Fixed
- **Motion-Brew Left Menubar (`motion-brew/sidebar-nav.tsx`, `chat/Sidebar.tsx`):**
  - Removed the `ZA / Zenko Agent / Local Autonomous` account profile block from the sidebar footer.
  - Replaced the raw browser vertical scrollbar with Zenko's slim theme-colored `CompactScrollArea`.
  - Wired `left_menubar.new_session_style`, `left_menubar.session_item_style` (`soft_fill`, `left_accent_bar`, `bordered_card`, `minimal_text`), hover session deletion, and clean collapsible Settings / Clear Session actions.
- **Motion-Brew Prompt Input & SmoothUI Context Meter (`motion-brew/prompt-input.tsx`, `chat/PromptComposer.tsx`):**
  - Wired `composer.send_button_shape` and `composer.send_button_icon` directly into `motion_brew_prompt`, replaced dark-invisible border glows with theme-accent color-mix shimmer, and enforced `var(--radius-lg)`.
  - Fixed SmoothUI `AIContextMeter` hover breakdown so it works immediately with fallback breakdown stats, and applied `context_meter_chrome` uniformly across all context meter styles including `compact_text`.
- **VengeanceUI Glass Dock (`vengeanceui/glass-dock.tsx`, `beui/FloatingDock.tsx`):**
  - Wired all 4 `right_menubar.active_indicator` modes (`vermilion_pill`, `solid_accent`, `side_bar`, `dot_indicator`) and Zenko `Tooltip` customization (`tooltip_style`, `tooltip_shape`, `tooltip_motion`, `show_hover_hints`) across dock items and trailing controls.
- **RareUI Model Picker & Theme-Adapted RareUI / EaseUI / Skiper96 Components (`rareui/`, `easeui/`, `skiperui/`, `chat/ModelPicker.tsx`, `StudioPreviews.tsx`):**
  - Wired `composer.action_pills` (`pillClass`) and multi-provider switching into `rareui_picker`, replaced hardcoded light/dark zinc/black/white classes and raw scrollbars across RareUI (`model-picker`, `faq`, `streaming-response`, `web-search`, `code-block`, `ai-status`), EaseUI (`command-palette`, `table`, `alert`, `toast`, `code-block`, `tool-approval`, `tool-chip`, `pixel-loader`), and Skiper96 tabs with Zenko CSS variables, `.zenko-thin-scroll`, `.zenko-prose-link`, and `var(--radius)`.
  - Fixed Appearance Studio live previews (`OverlaysPreview`, `tabs_accordions`, `agent_activity`, `model_picker`) so every option visually updates its preview.
- **Motion-Brew Dashboard Widgets (`index.css`, `motion-brew/{activity-grid,mix-chart,quota-gauge,usage-ring}.tsx`, `DashboardView.tsx`):**
  - Wired `.zenko-mb-card` into all 9 `[data-surface="..."]` card styles and global `var(--radius)`.
  - Wired `dashboard.heatmap_shape` and full `heatmap_range` (`year`/`half_year`/`quarter`) into `ActivityGrid`, replaced invisible dark-mode strokes and rainbow hues in `QuotaGauge` and `UsageRing` with theme-aware `var(--border)` / `var(--accent)`, and replaced SaaS quota labels with Zenko runtime telemetry copy.

## [0.0.8] - 2026-11-23

Fixed stale embedded WebUI builds by tracking `webui/dist` in Cargo build scripts and rebuilding fresh dev & release binaries.

### Fixed
- **Automatic `rust-embed` WebUI Rebuild Tracking (`crates/zenko-webui/build.rs`):** Added a stack-based Cargo `build.rs` emitting `cargo:rerun-if-changed` across `webui/dist` so `cargo build` and `cargo build --release` always re-embed updated frontend assets even when no Rust files change.

## [0.0.7] - 2026-11-23

Expanded WebUI Theme Studio component library across SmoothUI, Skiper96, VengeanceUI, EaseUI, Motion-Brew, and RareUI, plus Left Menubar and Context Meter refinements.

### Added
- **SmoothUI Context Meter & Context Meter Chrome Customization (`webui/src/components/smoothui/ai-context-meter.tsx`, `PromptComposer.tsx`):**
  - Added SmoothUI `AIContextMeter` (`smoothui_meter`) with hover/click token breakdown popover and Motion-Brew compact `UsageRing` (`motion_brew_ring`) as selectable Context Meter styles.
  - Added `components.composer.context_meter_chrome` customization supporting Border + Fill (`bordered_filled`), Border Only (`border_only`), Background Only (`background_only`), and Both Hidden (`minimal_bare`) across all context meter styles.
- **Skiper96 Expandable Tabs & VengeanceUI Glass Dock (`skiperui/expandable-tabs.tsx`, `vengeanceui/glass-dock.tsx`, `tabs.tsx`, `FloatingDock.tsx`):**
  - Added `surface.tabs_variant = "skiper96"` expandable spring-pill tabs navigation across tabbed views and Appearance Studio.
  - Added `components.right_menubar.dock_style = "vengeance_glass_dock"` frosted glass dock with directional spring-gliding tooltips inheriting Zenko's 60/30/10 theme tokens.
- **9 EaseUI Theme Studio Options (`webui/src/components/easeui/`):**
  - Integrated selectable EaseUI `Alert` (`easeui_alert`), `CommandPalette` (`easeui_palette`), sortable & drag-resizable `Table` (`easeui_table`), swipe-to-dismiss `Toast` (`easeui_toast`), `ToolApproval` (`easeui_approval`), `ToolChip` (`easeui_tool_chip`), `StreamingResponse` (`easeui_stream`), `PixelLoader` (`easeui_pixel_loader`), and `CodeBlock` (`easeui_code`) across Chat, Dashboard, App overlays, and Studio live previews.
- **Motion-Brew Dashboard, Sidebar & Chat Input Kit (`motion-brew/recap-card`, `webui/src/components/motion-brew/`):**
  - Integrated `RecapCard` (`@source "../node_modules/motion-brew/dist"`), `ActivityGrid`, `MixChart`, `QuotaGauge`, and `UsageRing` into `DashboardView.tsx` via `dashboard.widget_kit` (`echarts` | `motion_brew` | `hybrid`) and module toggles (`show_recap_card`, `show_quota_gauges`).
  - Integrated Motion-Brew `SidebarNav` (`left_menubar.frame_style = "motion_brew_nav"`) into `Sidebar.tsx` and GSAP-animated `PromptInput` (`composer.shell_variant = "motion_brew_prompt"`) into `PromptComposer.tsx`.
- **6 RareUI Component Options (`webui/src/components/rareui/`):**
  - Integrated RareUI `Faq` (`surface.accordion_style = "rareui_faq"` + `dashboard.show_faq`), Prism-highlighted `CodeBlock` (`rareui_code`), `WebSearch` (`rareui_web_search`), `AiStatus` (`rareui_ai_status`), `StreamingResponse` (`rareui_stream`), and `ModelPicker` (`rareui_picker`).
- **Declarative Schema & Bundling (`crates/zenko-core/src/{ui.rs, studio_schema.rs}`, `webui/vite.config.ts`):**
  - Extended Rust `UiCustomization` persistence defaults and Appearance Studio control descriptors for all new options, added Vite `manualChunks` (`vendor-gsap`, `vendor-prism`, `vendor-motion-brew`), and extended global icon family shims in `themed-lucide.tsx`.

### Changed & Fixed
- **Left Menubar Icon Cleanup (`Sidebar.tsx`, `icon-packs.ts`):**
  - Replaced the New Session CTA icon with a clean compose (`edit`) icon and fixed the broken `itshover.plus` SVG path (previously rendering an alarm-clock icon) to a crisp `+` glyph.
  - Removed speech-bubble message icons from session rows in the left sidebar for a cleaner session list.

## [0.0.6] - 2026-11-23

Low-RAM daemon optimization, SQLite WAL compaction, zero-copy static WebUI serving, and 60/120fps streaming UI smoothness improvements.

### Changed
- **Streaming Compact `models.dev` Metadata Index (~46× Smaller on Disk, ~200× Less RAM):**
  - Replaced raw 6.78 MB `serde_json::Value` caching and deep `.clone()` calls in `crates/zenko-providers/src/{discovery,client}.rs` with `Arc<CompactModelRegistry>`, streamed via a custom `serde::de::Visitor` (`IgnoredAny`) that skips ~316 non-Zenko provider blocks without materializing the full JSON tree.
  - Stopped eagerly deserializing `models.dev` at daemon boot (`crates/zenko-runtime/src/runtime/mod.rs`); lazily hydrates compact `models_dev_compact_v2` (~145 KB) only on demand, automatically migrates & deletes legacy 6.78 MB `models_dev_registry_v1` rows in the background, and evicts expired model caches during idle reaper ticks.
- **SQLite WAL Size Cap & Automatic Idle Compaction (`crates/zenko-memory/src/sqlite/`):**
  - Enforced `PRAGMA journal_size_limit = 4 MiB` (`SQLITE_JOURNAL_SIZE_LIMIT_BYTES`), added `compact_wal()` (`PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize; PRAGMA shrink_memory;`) and `vacuum_if_bloated()`, and wired periodic idle WAL compaction into `run_external_tool_reaper`.
- **Zero-Copy Static WebUI Assets & ETag `304 Not Modified` (`crates/zenko-webui/src/lib.rs`):**
  - Served embedded release assets zero-copy directly from binary `.rodata` (`Cow::Borrowed`) with SHA-256 `ETag` / `If-None-Match` (`304 Not Modified`) handling and switched `CompressionLayer` to `CompressionLevel::Fastest`.
- **Bounded Event Ring Buffers & Low-RAM `mimalloc` (`no_thp`) Tuning:**
  - Enabled `mimalloc`'s `no_thp` compile-time feature in `Cargo.toml` (disabling 2 MiB Transparent Huge Page pinning on Linux with zero `unsafe` code), centralized `DEFAULT_EVENT_BROADCAST_CAPACITY` (`128`) and `DEFAULT_LOG_BUFFER_CAPACITY` (`200`) in `crates/zenko-core/src/constants.rs`, and added `MIMALLOC_PURGE_DELAY=10` / `MIMALLOC_ALLOW_LARGE_OS_PAGES=0` / `MIMALLOC_ARENA_EAGER_COMMIT=0` defaults to generated systemd units (`crates/zenko-cli/src/service.rs`).
- **60/120fps WebUI Streaming & Scrolling Smoothness (`webui/src/`):**
  - Coalesced high-frequency SSE `text_delta`, `thinking_delta`, `job_output_delta`, and `log` streams into `requestAnimationFrame` batches in `App.tsx` (max 1 React commit per display frame).
  - Wrapped `MarkdownProse`, `AssistantTurnView`, `CompactionCheckpointPill`, `Sidebar`, `FloatingDock`, and `PromptComposer` in `React.memo` with stable callbacks so streaming tokens never re-render historical turns or surrounding chrome.
  - Added CSS `content-visibility: auto` (`.zenko-turn-item`) on historical chat turns and toned down heavy `backdrop-blur-xl`/`backdrop-blur-2xl` GPU compositing layers on matte overlays (`FloatingDock`, `FloatingActivityMonitor`, `PromptComposer`, `index.css`).

## [0.0.5] - 2026-11-23

Phase 3 (P2) modular architecture refactoring, centralized Axum auth middleware, DRY & zero-hardcoding cleanup, frontend code splitting, dead code & dependency pruning, and strict zero-warning Clippy sweep across the entire workspace.

### Changed
- **Modularized 6 Large Monolithic Files into Cohesive Sub-Modules:**
  - `crates/zenko-webui/src/api/` (`mod`, `auth`, `sessions`, `agents`, `tasks_checkpoints`, `ui_models`, `setup_gateways`, `memory_skills_automation`, `openai_webhooks`, `mcp_lsp`): split 3,774-line `api.rs` and centralized admin auth into an Axum `require_api_auth_middleware` layer + `ApiError` wrapper in `lib.rs`, removing 105+ repetitive per-handler auth checks.
  - `crates/zenko-memory/src/sqlite/` (`mod`, `sessions`, `approvals_gateways`, `memories`, `cron_ui_logs`, `jobs_agents`, `tasks_checkpoints_goals`, `tests`): split 3,423-line `sqlite.rs` and removed the duplicate `get_all_session_events` helper (unified on `list_all_session_events`).
  - `crates/zenko-runtime/src/runtime/` (`mod`, `bridges`, `turn`, `loops`, `ipc`, `tests`): split 4,798-line `runtime.rs`.
  - `crates/zenko-cli/src/commands/` (`mod`, `ask_chat`, `sessions_memory`, `jobs_hub_queue`, `agents`, `dag_checkpoints`, `mcp_lsp`, `skills_goals_export`) + `crates/zenko-cli/src/{cli.rs, daemon.rs, main.rs}`: split 3,663-line `commands.rs` and 1,105-line `main.rs`, boxing `McpClientCmd::Add` (`McpClientAddArgs`) and extracting `get_or_build_runtime` in `mcp.rs`.
  - `webui/src/components/views/` (`shared`, `SetupView`, `GatewaysView`, `MemoryView`, `AutomationView`, `TelemetryView`, `LoginGate`): split 3,262-line `SecondaryViews.tsx` with clean minimal imports and a barrel re-export.
- **Frontend Code Splitting, Lazy Views & Zero-Regex Declarative Dispatch:**
  - Lazy-loaded `AppearanceStudio`, `DashboardView`, `SetupView`, `GatewaysView`, `MemoryView`, and `AutomationView` via `React.lazy` + `<Suspense>` (`webui/src/App.tsx`) and added Vite `manualChunks` (`vendor-echarts`, `vendor-motion`, `vendor-markdown`, `zenko-icon-packs`), shrinking the main entry chunk to `651 kB` (`186 kB` gzip) with ECharts (`613 kB`) loaded on demand.
  - Replaced remaining frontend regexes and imperative `if/else` ladders with deterministic parsers and declarative dispatch tables across `citations.ts`, `terminal-block.tsx`, `data-table.tsx`, `loader.tsx`, `ZenkoSelect.tsx`, `ChatElements.tsx`, and `AppearanceStudio.tsx`; aligned `data-table.tsx` badge tones and `ZenkoChart.tsx` default palette to strict 60/30/10 theme tokens.
- **DRY & Zero-Hardcoding Cleanup:**
  - Centralized `ProviderConfig::switch_provider(...)` in `crates/zenko-core/src/config.rs` (shared by `api/sessions.rs`, `api/setup_gateways.rs`, and `cli/setup/state.rs`).
  - Replaced boolean chains and magic literals with declarative tables/constants (`SECRET_JSON_KEY_SUBSTRINGS` / `NON_SECRET_TOKEN_FIELDS` in `redact.rs`, `DEFAULT_HOOK_TIMEOUT_SECS`, `HUB_PATH_PREFIX`, `ARTIFACT_PATH_PREFIX`, `LOG_SESSION_INDEX_FILE`, `WEB_SEARCH_MAX_LIMIT`, `CHECKPOINT_LIST_DEFAULT_LIMIT`, `LSP_IDLE_TIMEOUT_SECS`, and `enum DraftMode` in `telegram.rs`).
- **Strict Zero-Warning Clippy Sweep:** Resolved all Rust Clippy lints across all 8 crates and all test targets (`cargo clippy --workspace --all-targets -- -D warnings` passes with 0 warnings).

### Fixed
- **Runtime, Scheduler & IPC Edge Cases:** Re-enqueue popped prompt in `promote_queued_to_steer` if a session goes idle concurrently; check cron job existence before upserting next run so deleted cron jobs are never resurrected; bound incoming Unix IPC lines at `1 MiB` (`MAX_IPC_LINE_BYTES`); trim whitespace on task `id`/`depends_on` in `kahn_topological_order`; only advance `DREAM_KV_LAST_RUN_KEY` cooldown when `status == "completed"`; load up to 200 goal loops so active goals aren't starved behind finished ones.
- **API, CLI & Systemd Edge Cases:** Verified session existence (`404`) before mutating global config in `PATCH /api/sessions/{id}`; propagated config/storage errors in `api_update_session`, `api_switch_branch`, and `api_update_telegram`; sanitized session IDs in `Content-Disposition` headers (`api_export_session`); escaped `%`, `\`, `"`, and newlines in generated systemd `ExecStart` lines (`service.rs`); ignored `Ctrl`/`Alt` modifiers on printable characters in `zenko setup`; updated stale Telegram `/model` help text.

### Removed
- **Dead Legacy Files & 15 Unused Dependencies:** Removed obsolete `crates/zenko-webui/assets/*`, `scripts/build-css.sh`, duplicate `logo.svg` copies, unused `TextShimmer.tsx` and `smooth-scroll.tsx`; pruned unused Rust crate dependencies (`maud`, `tokio-stream`, `anyhow`, `futures-core`, `serde`, `tracing`, `zeroize`, moved `tower` to `dev-dependencies`) and 9 unused npm packages (`@radix-ui/react-*`, `class-variance-authority`, `next-themes`, `shiki`).

## [0.0.4] - 2026-11-23

Phase 2 (P1) concurrency, security, performance, provider/SSE, runtime reliability, and WebUI CSS/render polish across all 8 crates and the embedded React frontend.

### Security
- **SSRF DNS Rebinding & Redirect Hardening:** Added a custom `SsrfSafeResolver` (`reqwest::dns::Resolve`) in `crates/zenko-tools/src/web.rs` validating resolved IPs immediately prior to connect, removed the redirect-enabled fallback client in `WebFetchTool`, disabled redirects in `WebSearchTool`, validated IPv6 host authority parsing, and preserved RFC 3986 relative directory resolution and scheme validation on redirects.
- **MCP Vault Key Binding & SSRF Protection:** Bound `POST /api/mcp/servers` (`crates/zenko-webui/src/api.rs`) so `secret_values` can only write keys explicitly referenced by the server's `env_secrets` or `auth_header_secret` and strictly forbids overwriting reserved system/provider keys (`WEBUI_ADMIN_TOKEN`, Telegram, provider API keys); routed external HTTP MCP servers through `build_ssrf_safe_http_client()` when `allow_private_url` is false.
- **Git Staging, Memory Scrubbing & Subtree Ownership:** Expanded directory/glob pathspecs via `git status --porcelain -uall -z` before `is_sensitive_git_path` validation in `git_tool`; applied vault secret scrubbing in `update_memory_with_secrets`, `memory_tool(action="update_entry")`, and CLI/MCP standalone `memory add`/`update`; enforced `ensure_subtree_owner` in `SubagentSupervisor::wait_agents`.

### Fixed
- **Pipe Deadlocks, Bounded I/O & Graceful Shutdown:** Eliminated stdin/stdout pipe deadlocks in `run_hooks_with_context` (`policy.rs`) and `run_git_env` (`checkpoint.rs`) using `tokio::join!` inside bounded timeouts; capped stdio MCP lines (`MCP_MAX_RESPONSE_BYTES`), SSE lines (`1 MiB`), provider error bodies (`16 KiB`), and `read_file` / `hash_edit` / `apply_patch` line & file sizes with CRLF preservation; fixed `zenko web` shutdown (`main.rs`) to always cancel background tasks, kill subagents, stop jobs, and flush logs even on bind error.
- **SQLite Atomicity, Multi-Project Recall & Memory Leaks:** Made `redeem_pairing_code` single-use atomic inside an RAII `IMMEDIATE` transaction; wrapped FTS5 memory updates/archives and UI profile writes/activations in RAII transactions (and stopped clearing `is_active` on nonexistent profile IDs); pushed `m.scope` filtering into SQL before `LIMIT` in `search_memory_hybrid`; bounded `TriggerRateLimiter.buckets`, `workspace_locks`, `session_controls`, and `frozen_snapshot_for_session` maps; fixed stale `"Obsidian Zinc (Default)"` UI profile fallback name.
- **Provider / SSE Protocol Accuracy:** Terminated keep-alive SSE loops immediately on `data: [DONE]`; separated parallel tool calls omitting `index` by ID; skipped empty tool names; extracted DeepSeek `prompt_cache_hit_tokens` and Anthropic `cache_read_input_tokens`; preserved `name` on `EventRole::Tool` messages; restricted OpenRouter penultimate `cache_control` array conversion strictly to `role: "user"`; required valid JSON in `verify_key` / MiniMax probe.
- **Runtime, Subagents, LSP, CLI IPC & WebUI API Reliability:**
  - Fixed post-compaction Task DAG/memory recall firing only on iterations where compaction actually ran, wrote synthetic `ToolResult` errors on cancelled tool waves so SQLite never holds orphaned `tool_calls`, fixed wrap-up synthesis to use `effective_system_prompt` and skip persisting partial errored streams, and cleared `activity_logger` flags on `TurnError`.
  - Added monotonic generation guards (`run_gen`) on `ActiveSubagentHandle` so slow exit cleanup never clobbers killed/resumed runs, failed closed when `git worktree` creation errors for mutating subagents inside a git repo, and fixed MCP/LSP cold-start child process leaks and `sync_server` tool-cache preservation.
  - Made `lsp_tool` `format_locations_output` async with workspace jail & `256 KiB` size caps, filtered `WorkspaceSymbols` by `is_alive()`, and inferred `language_id` for custom `[[lsp.servers]]`.
  - Distinguished `DaemonOfflineError` from in-flight IPC timeouts/errors in `crates/zenko-cli/src/ipc.rs` so non-idempotent CLI/MCP commands never double-execute after a timeout; switched `run_chat`/`run_daemon_chat` to async `tokio::io::stdin()`; added timeouts to `run_doctor`; offloaded `api_dashboard` directory scans to `spawn_blocking`; bound `api_run_goal_now` to `state.shutdown.child_token()`; returned JSON `404` for missing `/api/*` and `/v1/*` routes instead of `200 index.html`.
- **WebUI Theme, Zero Idle CPU & React 19 Polish:**
  - Added Tailwind v4 `@custom-variant dark (&:where(.dark, .dark *));` and lightweight `animate-in`/`fade-in`/`zoom-in-*`/`slide-in-*` CSS utilities in `webui/src/index.css`; aligned Radix collapsible `data-[state=open|closed]` attributes across `surfaces.tsx`, `tool-call.tsx`, `tool-timeline.tsx`, and `run-activity.tsx`.
  - Switched `CompactScrollArea` Lenis scrolling to **on-demand rAF** (0% idle CPU) with rAF-coalesced `ResizeObserver`/`MutationObserver` updates and `hasPointerCapture` guards; batched `ZenkoChart` resizes in rAF with `isDisposed()` guard.
  - Made `App.tsx` `patchUi`/`selectPreset` state updaters pure with `saveUiTimer` unmount cleanup and `try/catch` error toasts on all API mutations; prevented `AppearanceStudio` live previews from unmounting on every slider/color tick; memoized `ChatStream` `ReactMarkdown` components and keyed `MessageScroller` by session; added `useLayoutEffect` deps and clipboard error guard in `tool-result.tsx`; memoized `ZenkoTooltipProvider`; wired `heroicons` in `icon-packs.ts` and forwarded SVG props through `ZenkoIcon`; added `inert={!open}` in `AgentDisclosure`, IME `isComposing` guard in `PromptComposer`, and `Escape` close in `ModelPicker`.

## [0.0.3] - 2026-11-23

Phase 1 (P0) security, crash, data-corruption, and WebUI reliability hardening across all 8 Rust crates and the embedded React frontend.

### Security
- **Admin Token Redaction:** Stopped logging the first-boot plaintext `WEBUI_ADMIN_TOKEN` into SQLite logs, JSONL files, in-memory ring buffers, and SSE (`crates/zenko-cli/src/main.rs`); token is printed once to operator stderr only.
- **Fail-Closed WebUI Login & Safe Cookies:** Hardened `POST /api/login` (`crates/zenko-webui/src/api.rs`) to fail closed on vault read/decryption errors and reject invalid/header-injecting characters before emitting `Set-Cookie`.
- **Dangling-Symlink Workspace Jail Guard:** Replaced `Path::exists()` checks in `resolve_jailed_path` (`crates/zenko-tools/src/jail.rs`) with a component-by-component `symlink_metadata` + bounded `read_link` walk so dangling symlinks pointing outside the workspace root are blocked even on non-existent writes, and fixed `~` expansion in `resolve_workspace_or_hub_path`.
- **ExecPolicy, Plan Mode & Read-Only Intent Gate Hardening:**
  - Split single `&` background separators (while preserving `2>&1` fd redirections) and `\r` in `orchestrator/guard.rs`, removed bare `env` from `READ_ONLY_SHELL_CMD_PREFIXES`, and blocked mutating scoped tools (`git_tool`, `skill_manage`, `goal_loop`, `cron_tool`, `memory_tool`, `spawn_agent`, `agent_worktree`, `agent_kill`) during read-only Intent Gate turns.
  - Added deterministic wrapper/flag stripping (`env -i/-u`, `command -p`, `nice -n`, `nohup`, `timeout`, `VAR=val`), `git` global flag parsing (`-C`, `-c`, `--git-dir`), and recursive `-c` / `$(...)` / backtick scanning across structural deny, dangerous approval, and pipe-to-shell checks in `policy.rs`; prevented `Balanced` + `always_allowed(shell_exec)` from bypassing dangerous shell confirmation.
- **Skill Draft Gate & CSS / Vault Hardening:**
  - Sanitized YAML frontmatter fields in `format_skill_document` (`crates/zenko-memory/src/snapshot.rs`) and `SecondaryViews.tsx` and emitted governance flags last to prevent newline injection bypasses of the draft approval gate.
  - Hardened `sanitize_css_value` (`crates/zenko-core/src/ui.rs`) and dynamic fox favicon color injection (`webui/src/lib/api.ts`) against `url(//...)` / `expression(...)`.
  - Fixed read-only `SecretVault` operations (`crates/zenko-core/src/vault.rs`) to fail closed via `load_key()` instead of writing a bogus `vault.key` to disk, verified envelope version `1`, and zeroized key material in memory.
  - Guarded `setByPath` against prototype pollution (`__proto__`, `constructor`, `prototype`), HTML-escaped ECharts tooltip formatters (`DashboardView.tsx`), and enforced `safeHref` on external links (`citations.tsx`, `activity-row.tsx`, `preview-rail.tsx`, `ChatStream.tsx`).

### Fixed
- **DuckDuckGo Parser Infinite Loop & Dropped Snippet:** Fixed `extract_html_attr` (`crates/zenko-tools/src/web_search.rs`) hanging at 100% CPU on `/` or self-closing HTML tags and fixed `parse_duckduckgo_html` dropping the `limit`-th result's snippet.
- **ULID Prefix Collisions & Atomic Writes:** Introduced `new_random_suffix` drawing from ULID's 80-bit random tail (`crates/zenko-core/src/types.rs`) for `new_session_task_id`, `new_goal_loop_id`, and `snapshot.rs` history IDs, and unified `snapshot.rs` onto `zenko_core::write_owner_only_atomic` (`0600` on creation).
- **UTF-8 Boundary, `Duration` Overflow & Small-Terminal Panics:** Fixed `parse_since_window` (`crates/zenko-memory/src/algorithms.rs`) against multi-byte UTF-8 and overflow panics; replaced raw `&s[..min]` byte slices with `zenko_core::truncate_utf8_prefix` across CLI, runtime, tools, and Telegram chunking; clamped `centered_box` and added cursor-relative scrolling in `zenko setup` (`crates/zenko-cli/src/setup/render.rs`).
- **SQLite Dream Lease RAII Safety:** Switched `try_claim_dream_lease` (`crates/zenko-memory/src/sqlite.rs`) to an RAII `IMMEDIATE` transaction so errors auto-rollback instead of wedging the shared SQLite mutex.
- **Trajectory Export Schema Alignment:** Aligned `export.rs` with canonical `EventKind::ToolCall` (`tool_calls` array + `text`/`thinking`) and `EventKind::ToolResult` (`tool_name`), prevented double-JSON-encoding of string arguments, and applied `verified_only`/`min_turns` filters before capping at `limit`.
- **Shadow-Git Checkpoint Durability:** Pinned shadow-git checkpoint trees under `refs/zenko/checkpoints/<id>/{pre,post}`, configured `gc.pruneExpire=never` & `gc.auto=0`, and unpinned refs only upon checkpoint expiry/pruning (`crates/zenko-runtime/src/checkpoint.rs`).
- **Provider State Atomicity & Custom Endpoint Routing:** Combined `provider_id`, `base_url`, and `api_key` into one atomic `RwLock<ProviderState>`, fixed `UnifiedLlmProvider::new` routing `"custom"` OpenAI-compatible endpoints to `"custom"` instead of `"openrouter"`, and added explicit `is_local_free` on `ProviderDescriptor` so paid custom proxies are not forced to `$0.0`.
- **Telegram Gateway Reliability:** Fixed `RateLimiter::wait_slot` concurrent scheduling math, fixed `send_message`/`edit_message_text` error checking and fallback delivery so chunk 0 is never silently dropped on `Ok(0)`, fixed `run_approval_forwarder` spinning on `RecvError::Closed`, and eliminated `Instant::now() - 2s` underflow risk.
- **WebUI Fetch, Session Races & Draft Protection:** Fixed `api()` header merge ordering (`lib/api.ts`), added `AbortController`/sequence guards + synchronous `activeSidRef` updates + SSE reconnect resync + approval/clarification deduplication in `App.tsx`, prevented `mergeLastIntoLive` from stealing the previous assistant turn (`ChatStream.tsx`), and added debounced/abortable search + `docDirty` protection in `MemoryView` (`SecondaryViews.tsx`).

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
