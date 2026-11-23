# Zenko — WebUI Design System & Customization Specification (`DESIGN.md`)

**Design Philosophy:** *Obsidian Vermilion Minimal — Zero Topbar Chrome, Pure Matte Black (`#0a0a0b`), Burnt Vermilion Red-Orange (`#ea5829`), and Crisp White (`#f5f5f6`), powered by real BeUI, BeautifulUI, RareUI, and shadcn/Tailwind v4 primitives compiled into static assets and embedded inside the single native Rust binary via `rust-embed` (zero Node.js at runtime).*

Zenko's WebUI uses the exact minimal architecture of **Image 1** (`OpenZ Agent` — clean left session sidebar with geometric Zenko Fox emblem + vermilion-tinted `+ New Session` CTA + `Clear Active Session`, zero topbar chrome, borderless editorial assistant messages with `✦ Tool executed` pill & collapsible BeUI `Completed N steps ⌄` trace, warm vermilion inline path chips, sculpted floating bottom composer with BeautifulUI `ProgressRing`, and a signature **floating right vertical icon dock** with BeUI `layoutId` spring highlight) paired with **Image 2**'s calm editorial typography and **deep live customization over every UI component**.

---

## 1. UI Component Inspiration & Registry Mapping

Zenko adopts a clear hierarchy of design inspirations ported cleanly into **Pure Rust (`Axum + Maud`) + Tailwind CSS v4 (Standalone CLI, zero Node.js) + CSS Custom Properties + tiny (<15 KB) vanilla micro-interaction engine**:

> [!NOTE]
> **Why Pure-Rust Ported Components Instead of Runtime React/Node.js?**  
> All referenced registries (`beui.dev`, `beautifului.dev`, `rareui.com`, `assistant-ui`, `shadcn/ui`, `boardui`, `uiarc.dev`, `transitions.dev`) publish React + Tailwind + Framer Motion snippets. Porting their exact visual geometry, OKLCH tokens, component anatomy, and GPU-accelerated CSS transitions into type-safe Rust `maud` components gives Zenko **identical visual polish** with **0 Node.js runtime**, **<15 KB client JS**, **instant localhost renders (<30 ms)**, and **~5–15 MB server RAM**.  
> *(Note: Because Zenko's daemon exposes a clean REST + SSE/WebSocket API, an optional static React/Bun frontend build embedded via `rust-embed` can also be swapped in later without changing a single line of backend code.)*

### Primary Design Sources (Main Visual Language)

| Source | Role in Zenko WebUI | Components & Patterns Adopted |
| :--- | :--- | :--- |
| **`beui.dev` (BeUI Agents)** | **Primary AI Agent Primitives** | • Collapsible **Reasoning / Thinking Trace** accordion with elapsed timer & live shimmer<br>• **Tool Execution Timeline Cards** (status spinner -> check/error, expandable JSON input, syntax-highlighted output, unified file diff view)<br>• **Human-in-the-Loop (HITL) Approval Bar & Modal** (risk badge, command preview, Approve / Deny / Always Allow)<br>• **Streaming Response Block** with inline source/file citation pills<br>• **Agent Prompt Composer** (multiline auto-grow, `/` slash menu, `@` file/skill mention picker, model selector, mode pill, Stop button)<br>• **Command Palette (`Cmd/Ctrl+K`)** & animated breadcrumbs |
| **`beautifului.dev` (BeautifulUI)** | **Primary Layout & SaaS Blocks** | • Multi-pane workbench shells & split-view inspectors<br>• Polished metric/telemetry stat cards (RSS RAM, CPU %, active sessions, token/USD burn sparklines)<br>• Sleek settings forms, segmented controls, interactive color/theme pickers, file dropzones, and onboarding wizard stepper |
| **`rareui.com` (RareUI)** | **Primary Signature Micro-Interactions** | • **Interactive Zenko Status Orb** (living CSS/SVG orb reflecting daemon state: `Idle` calm breath, `Thinking` orbiting ring, `Executing Tool` pulse lattice, `Needs Approval` amber beacon, `Error` crimson ripple)<br>• **Animated Rolling Counters** (live token count, cost `$0.0042`, uptime, RAM MB)<br>• **Magnetic / Shimmer / Glow Button Styles** (selectable in Customizer!)<br>• **Floating Action Dock & Reaction/Feedback Pill Bars** (copy, regenerate, branch, refine, thumbs/bookmark) |

### Secondary / Fallback Component Inspirations (Used Where Needed)

| Source | Specific Gap Filled in Zenko |
| :--- | :--- |
| **`assistant-ui`** | Conversation **Thread Branching** (`< 1 / 3 >` switcher), inline user message **Edit & Resend**, smart stick-to-bottom auto-scroll during token streaming, attachment chips, and **Generative Tool UI Slots**. |
| **`shadcn/ui`** | Accessible foundational primitives & semantic OKLCH token architecture: Dialog, Sheet/Drawer, Tabs, Select, Switch, Checkbox, Tooltip, Popover, Toast, Badge, Table, Skeleton, Separator, ScrollArea. |
| **`boardui`** | High-density **Operations Dashboard** cards, Cron schedule calendar/list tables, Gateway health grid, and subagent hierarchy cards. |
| **`uiarc.dev`** | Crisp 1px gradient hairline borders, subtle spotlight card hover effects, and refined dark-mode depth layering. |
| **`transitions.dev`** | GPU-only CSS transition recipes (`transform`, `opacity`, `@starting-style`, `cubic-bezier` spring curves), smooth view transitions, and strict `prefers-reduced-motion` compliance. |

---

## 2. Deep Component Customization System (First-Class Feature)

Every visual aspect of Zenko's WebUI is controlled through a typed Rust `UiCustomization` profile stored in SQLite (`ui_profiles` table), exported/imported as portable JSON, and applied instantaneously via CSS custom properties (`:root`) + root `data-*` attributes + server-rendered component variants.

### 2.1 Customizable Dimensions Matrix

| Category | Customizable Options (Live Preview & Instant Apply) | Implementation Mechanism |
| :--- | :--- | :--- |
| **1. Color Themes & Mode** | • **Mode:** `Dark`, `Light`, `System`<br>• **Built-in Presets (8):** `Obsidian Zinc` *(default)*, `Midnight Indigo`, `Emerald Terminal`, `Amber Forge`, `Nordic Frost`, `Rose Quartz`, `Pure OLED Black`, `Paper Daylight`<br>• **Custom Token Editor:** Live OKLCH/Hex pickers for `--background`, `--surface`, `--card`, `--border`, `--foreground`, `--muted`, `--primary`, `--accent`, `--destructive`, `--success`, `--warning` | CSS Custom Properties on `:root` (`oklch(...)` tokens) with automatic WCAG AA contrast checker warning in the Customizer! |
| **2. Buttons (All Variants)** | • **Style Variant:** `Solid`, `Soft/Tinted`, `Outline`, `Ghost`, `Glass`, `Glow (RareUI)`, `3D Tactile (Press-down)`, `Shimmer Border`<br>• **Shape:** `Sharp (2px)`, `Rounded (8px)`, `Soft (12px)`, `Pill (9999px)`<br>• **Size Scale:** `Compact`, `Default`, `Comfortable`<br>• **Hover/Active Motion:** `None`, `Subtle Lift`, `Scale Press`, `Magnetic/Glow` | `data-btn-style`, `data-btn-shape`, `data-btn-size`, `data-btn-motion` + unified Rust `Button` component macro. |
| **3. Icons (Global & Per-Section)** | • **Icon Pack (Embedded SVG Sprites — 0 external requests!):**<br>  1. `Lucide` *(clean geometric default)*<br>  2. `Phosphor` *(duotone/bold expressive)*<br>  3. `Tabler` *(crisp technical)*<br>  4. `Heroicons` *(friendly solid/outline)*<br>  5. `Minimal Pixel / Mono` *(retro terminal vibe)*<br>• **Stroke Width:** `1.25px` (Thin), `1.5px` (Regular), `2.0px` (Bold)<br>• **Icon Style:** `Outline`, `Duotone (tinted fill)`, `Solid`<br>• **Icon Size Scale:** `SM (14px)`, `MD (16px)`, `LG (18px)`<br>• **Custom Icon Overrides:** Map custom icons for Sidebar items, Agent avatar, Tool badges, and Gateway badges | Compile-time embedded SVG sprite sheets per icon pack (`crates/zenko-webui/assets/icons/*.svg`) + CSS variables `--icon-stroke`, `--icon-size`. |
| **4. Surfaces & Depth** | • **Surface Style:** `Flat Minimal`, `Crisp Bordered (shadcn)`, `Subtle Elevated`, `Frosted Glassmorphism`, `Spotlight Border (UIArc)`<br>• **Background Texture:** `Clean`, `Subtle Dot Grid`, `Fine Technical Grid`, `Ambient Radial Glow` | `data-surface`, `data-bg-pattern` with hardware-accelerated backdrop filters & CSS radial gradients. |
| **5. Border Radius & Borders** | • **Global Radius Scale:** `0px` (Brutalist), `4px` (Sharp), `8px` (Modern), `12px` (Soft), `16px` (Plush)<br>• **Border Contrast:** `Subtle`, `Normal`, `High Contrast` | `--radius-sm`, `--radius-md`, `--radius-lg`, `--border-alpha`. |
| **6. Typography** | • **UI Font Stack:** `Inter / System Sans`, `Geist-like Geometric`, `Plus Jakarta`, `JetBrains / Mono UI`<br>• **Code/Tool Font:** `JetBrains Mono`, `Fira Code`, `Cascadia`, `System Monospace`<br>• **Base Font Size:** `13px` (Dense), `14px` (Default), `15px` (Relaxed), `16px` (Large) | `--font-sans`, `--font-mono`, `--text-base` (uses local system/embedded WOFF2 subset—zero Google Fonts tracking!). |
| **7. Layout & Density** | • **Density:** `Compact` (power user), `Balanced` (default), `Spacious`<br>• **Sidebar Mode:** `Fixed Left`, `Collapsible Icon Rail`, `Floating Glass Dock (RareUI)`, `Hidden (Cmd+K driven)`<br>• **Chat Layout:** `Centered Editorial (Claude/BeUI)`, `Bubble Split`, `Terminal Log Stream (Codex/OMP)`<br>• **Inspector Panel:** `Right Split Drawer`, `Inline Expandable`, `Bottom Console` | `data-density`, `data-sidebar`, `data-chat-layout`. |
| **8. Motion & AI Status Orb** | • **Animation Preset:** `Snappy (Transitions.dev)`, `Fluid Spring`, `Reduced / Minimal`, `Instant (0ms)`<br>• **Zenko Status Orb Style:** `Liquid Halo (RareUI)`, `Pulse Ring`, `Matrix Dot`, `Minimal Dot`, `Hidden` | `--motion-duration`, `--motion-ease`, `data-orb-style`, auto-respects OS `prefers-reduced-motion`. |
| **9. Chat & Component Toggles** | • Toggle visibility/position of: Thinking Traces (auto-collapse vs expanded), Inline Token & USD Cost Badges, Timestamps, Tool JSON Raw View, Code Line Numbers, Avatars, Keyboard Shortcut Hints | Persisted in `UiCustomization.widgets` map. |

### 2.2 Customization Studio UX (`/settings/appearance` + Quick Theme Drawer)
- **Quick Customizer Slide-Over (`Sheet`):** Accessible from ANY page via the palette icon in the top bar (or `Cmd/Ctrl + .`). Lets you tweak themes, buttons, icons, radius, surface, and orb live while watching the actual Chat / Dashboard update behind it in real time!
- **Full Component Playground (`/settings/appearance`):** Live interactive preview stage showing a miniature Agent Chat card, Tool Approval card, Button matrix, Icon grid, Metrics card, and Inputs updating synchronously as you change settings.
- **Presets & Sharing:**
  - Save unlimited custom profiles (`"Late Night Ops"`, `"High-Contrast Daylight"`, `"Minimal Terminal"`).
  - One-click **Export JSON / Import JSON** and **Reset to Default**.

---

## 3. Core Screen Architecture & Information Hierarchy

```text
┌─────────────────────────────────────────────────────────────────────────┐
│ [◉ Zenko Orb]  Session: "Refactor auth..."   [Model: Claude/DeepSeek ▾] │
│                [Mode: Balanced ▾]   [RAM 9.4MB | $0.04]  [⌘K] [🎨] [⚙] │
├──────────────────┬───────────────────────────────────┬──────────────────┤
│ SIDEBAR          │ MAIN WORKSPACE                    │ INSPECTOR (opt.) │
│                  │                                   │                  │
│ + New Session    │ ┌─ Thinking Trace (1.4s) ──── ▾ ─┐│ • Active Tools   │
│                  │ │ Checking workspace files...    ││ • Modified Files │
│ ▸ Active Tasks   │ └────────────────────────────────┘│ • Session Cost   │
│   • Auth audit   │ ┌─ Tool: hash_edit ────── [✓ 4ms]┐│ • Subagents (2)  │
│   • TG: Daily... │ │ src/auth.rs  +12 -4  [View Diff]││ • Memory Context │
│                  │ └────────────────────────────────┘│   USER.md (420t) │
│ ▸ Navigation     │ ┌─ ⚠ Approval Required ──────────┐│   MEMORY.md(680t)│
│   Chat Workbench │ │ shell: `git push origin main`  ││                  │
│   Gateways (TG)  │ │ [✓ Approve] [✗ Deny] [Always]  ││                  │
│   Memory & Files │ └────────────────────────────────┘│                  │
│   Skills & Hooks │                                   │                  │
│   Cron & Jobs    │ Streaming Markdown response...    │                  │
│   Telemetry/Logs │ [Copy] [Edit] [Branch 1/2] [↻]    │                  │
│   Appearance 🎨  ├───────────────────────────────────┤                  │
│                  │ [+] Ask Zenko... (/cmd, @skill)   │                  │
│ ● Telegram OK    │ [Attach] [Thinking:Med] [■ Stop]  │                  │
└──────────────────┴───────────────────────────────────┴──────────────────┘
```

### Key Screen Specifications

1. **First-Run Setup Wizard (`/setup`):**
   - 4 crisp steps with progress stepper (BeautifulUI style):
     1. **Welcome & Security:** Set WebUI Admin Token / Passcode + choose workspace directory.
     2. **OpenRouter Connection:** Paste API key (stored encrypted in Vault) -> instant test button -> pick Lead Model, Fast/Subagent Model, Fallback Model, and optional daily spend cap.
     3. **Telegram Gateway (Optional, skippable):** Paste `@BotFather` token -> instant bot verification (`@your_zenko_bot`) -> shows 6-char pairing code & instructions (`Send /pair X7K9P2 to your bot`) -> live confirmation checkmark when paired!
     4. **Theme & Execution Policy:** Pick initial theme preset (`Obsidian Zinc`, `Midnight Indigo`, `Paper Daylight`, etc.) and permission mode (`Strict`, `Balanced`, `Autonomous`) -> **Launch Zenko**.
2. **Agent Workbench (`/` & `/sessions/:id`):**
   - Real-time SSE/WebSocket streaming with zero layout shift.
   - Collapsible Reasoning Traces (BeUI), inline Tool Execution Cards with unified colorized diffs, HITL Approval Banners synchronized live with Telegram (approving on Telegram instantly resolves the WebUI banner and vice versa!).
   - Message Branching & Edit-and-Resend (assistant-ui pattern) backed by Zenko's SQLite event tree (`parent_id`).
3. **Gateways Console (`/gateways`):**
   - Card grid for Telegram (active in Phase 1) and upcoming gateways (Discord, Slack, Webhooks).
   - Live Telegram connection badge, active Forum Topics -> Session mapping table, Pairing Code generator, Allowlist CRUD, per-chat permission mode selector, and rate-limiter queue telemetry.
4. **Memory, Rules & Skills Studio (`/memory` & `/skills`):**
   - Clean tabbed editor for `SOUL.md`, `USER.md`, `MEMORY.md`, `ZENKO.md` with live token count gauge (so users keep frozen prompt snapshots lean for KV-cache efficiency!).
   - Skills gallery (`skills/*.md`) with toggle enable/disable, trigger preview, and `/refine` history.
5. **Automation, Hooks & Security (`/automation` & `/security`):**
   - Cron & Heartbeat manager (visual expression builder, next fire countdown, target gateway selector, run-now button, run history).
   - Declarative `execpolicy` rules & Lifecycle Hooks (`PreToolUse`, `PostToolUse`, `Stop`) inspector + live sandbox test runner.
6. **Daemon Telemetry & Live Logs (`/telemetry`):**
   - RareUI animated counters for Daemon RSS (MB), CPU %, Uptime, SQLite DB size, OpenRouter Cache Hit %, Total Tokens, and Daily Cost ($).
   - Filterable live `tracing` log stream over SSE (level filter, crate filter, pause/resume, download).

---

## 4. Accessibility, Responsiveness & Performance Standards

- **Keyboard-First:** Global `Cmd/Ctrl + K` Command Palette, `Cmd/Ctrl + .` Quick Appearance Drawer, `Escape` to cancel active generation (with confirmation guard or double-Esc), `J/K` session navigation, `A/D` when an approval card is focused.
- **Responsive Mobile Support:** Works cleanly on phone browsers (collapses sidebar to a slide-over Sheet and inspector to a bottom Drawer) for quick remote management over Tailscale/VPN.
- **Zero External CDN Dependencies:** All CSS, SVG icon packs, and tiny client scripts are compiled and embedded directly into the single `zenko` binary via `include_bytes!` — works 100% air-gapped or on LAN.
- **Performance Targets:**
  - Embedded WebUI asset payload: **< 120 KB total gzipped** (compiled Tailwind CSS + embedded SVG icon packs + tiny vanilla micro-interaction script).
  - Zero virtual DOM diffing overhead; SSE patches target exact DOM containers.
  - Strict `prefers-reduced-motion` support across all RareUI/Transitions.dev animations.
