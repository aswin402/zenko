# Changelog

All notable changes to **Zenko** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
