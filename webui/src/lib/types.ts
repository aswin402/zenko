export type PermissionMode = 'plan' | 'strict' | 'balanced' | 'autonomous'
export type SessionStatus = 'idle' | 'running' | 'waiting_approval' | 'paused' | 'error'

export interface Session {
  id: string
  title: string
  channel: string
  external_chat_id?: string | null
  external_thread_id?: string | null
  workspace_dir?: string | null
  parent_session_id?: string | null
  agent_role?: string | null
  agent_depth?: number
  worktree_branch?: string | null
  subagent_count?: number
  model_override?: string | null
  permission_mode: PermissionMode
  status: SessionStatus
  head_event_id?: string | null
  total_tokens: number
  total_cost_usd: number
  created_at: string
  updated_at: string
}

export interface SessionEvent {
  id: string
  session_id: string
  parent_id?: string | null
  branch_index: number
  role: 'system' | 'user' | 'assistant' | 'tool'
  kind: 'message' | 'thinking' | 'tool_call' | 'tool_result' | 'compaction_checkpoint' | 'system_note'
  content_json: Record<string, any>
  tokens_in: number
  tokens_out: number
  cache_read_tokens: number
  cost_usd: number
  created_at: string
}

export interface ApprovalRecord {
  id: string
  session_id: string
  call_id: string
  tool_name: string
  args_preview: string
  risk: 'safe' | 'moderate' | 'dangerous'
  status: string
  resolved_by?: string | null
  created_at: string
}

export type ExecutionTier = 'instant' | 'standard' | 'orchestrated'

export interface BrainstormOption {
  id: string
  title: string
  summary: string
  pros: string[]
  cons: string[]
  blast_radius: string
  reversibility: string
  safety_score: number
  testability_score: number
  touched_files?: string[]
  rollback_steps?: string[]
  risks?: string[]
  recommended: boolean
}

export type PlanStepStatus = 'pending' | 'in_progress' | 'completed' | 'blocked'

export interface PlanStep {
  id: string
  title: string
  status: PlanStepStatus
  depends_on: string[]
  verification?: string | null
}

export interface ExecutionPlan {
  id: string
  goal: string
  steps: PlanStep[]
  rubric_warnings?: string[]
  updated_at: string
}

export interface ClarificationDecisionNote {
  question: string
  context?: string
  selected_option_id: string
  custom_answer?: string | null
  resolved_by: string
  auto_resolved: boolean
}

export type BrainstormStage = 'exploring' | 'refined' | 'committed'

export interface BrainstormSpec {
  id: string
  stage?: BrainstormStage
  parent_spec_id?: string | null
  problem: string
  invariants: string[]
  assumptions?: string[]
  blind_spots?: string[]
  decision_criteria?: string[]
  options: BrainstormOption[]
  selected_option_id: string
  selection_rationale: string
  critique?: string[]
  open_questions: string[]
  pre_mortem?: string[]
  verification_plan: string[]
  rubric_warnings?: string[]
  created_at: string
}

export interface ClarificationOption {
  id: string
  label: string
  description: string
  recommended: boolean
}

export interface ClarificationRecord {
  id: string
  session_id: string
  question: string
  context: string
  options: ClarificationOption[]
  recommended_option_id: string
  status: string
  selected_option_id?: string | null
  custom_answer?: string | null
  resolved_by?: string | null
  created_at: string
}

export interface OrchestratorStatus {
  tier: ExecutionTier
  stage: string
  detail: string
  model?: string | null
}

export type TaskKind =
  | 'shell'
  | 'build'
  | 'dev'
  | 'debug'
  | 'script'
  | 'cron'
  | 'browser'
  | 'subagent'

export interface TaskKindMeta {
  id: TaskKind
  label: string
  subdir: string
  description: string
}

export interface HubCategoryMeta {
  id: string
  label: string
  subdir: string
  description: string
}

export interface HubEntry {
  category: string
  rel_path: string
  is_dir: boolean
  size_bytes: number
  modified_at: string
}

export interface BackgroundJob {
  id: string
  session_id: string
  kind?: TaskKind
  label?: string | null
  command: string
  workspace_dir: string
  pid?: number | null
  status: 'running' | 'completed' | 'failed' | 'stopped' | 'interrupted'
  exit_code?: number | null
  log_path: string
  bytes_written: number
  started_at: string
  finished_at?: string | null
}

export interface JobReadChunk {
  job: BackgroundJob
  offset: number
  next_offset: number
  total_bytes: number
  content: string
}

export interface QueuedPrompt {
  id: string
  session_id: string
  text: string
  status: string
  created_at: string
}

export interface SubagentRoleDescriptor {
  id: string
  label: string
  description: string
  model_tier: 'fast' | 'plan' | 'lead' | 'advisor'
  default_workspace: 'inherit' | 'worktree'
  max_iterations: number
  read_only: boolean
  can_spawn_children: boolean
}

export interface SubagentRecord {
  id: string
  parent_session_id: string
  role: string
  depth: number
  goal: string
  status: 'running' | 'completed' | 'failed' | 'killed' | 'interrupted'
  workspace_mode: 'inherit' | 'worktree'
  workspace_dir: string
  worktree_branch?: string | null
  model?: string | null
  summary?: string | null
  structured_output?: any
  diff_stat?: string | null
  changed_files: string[]
  tokens_used: number
  cost_usd: number
  tree_tokens?: number
  tree_cost_usd?: number
  descendant_count?: number
  resume_count?: number
  started_at: string
  finished_at?: string | null
}

export interface WorktreeDiffReport {
  agent_id: string
  branch: string
  worktree_path: string
  repo_root: string
  base_commit?: string | null
  dirty_baseline?: boolean
  has_changes: boolean
  changed_files: string[]
  diff_stat: string
  patch_preview: string
  merged: boolean
  cleaned_up: boolean
}

export type SessionTaskStatus = 'pending' | 'in_progress' | 'blocked' | 'done' | 'failed'

export interface SessionTask {
  id: string
  session_id: string
  branch_event_id?: string | null
  title: string
  description: string
  active_label?: string | null
  status: SessionTaskStatus
  depends_on: string[]
  owner_agent?: string | null
  verification_cmd?: string | null
  verification_evidence?: string | null
  notes?: string | null
  priority: number
  created_at: string
  updated_at: string
}

export type RewindMode = 'code_and_conversation' | 'code_only' | 'conversation_only'

export interface WorkspaceCheckpoint {
  id: string
  session_id: string
  turn_id: string
  event_id?: string | null
  workspace_dir: string
  backend: 'shadow_git' | 'content_blobs'
  pre_ref: string
  post_ref?: string | null
  trigger_tool: string
  label: string
  changed_files: string[]
  is_safety_backup: boolean
  created_at: string
}

export interface CheckpointDiffReport {
  checkpoint_id: string
  session_id: string
  workspace_dir: string
  changed_files: string[]
  manual_edit_conflicts: string[]
  diff_stat: string
  patch_preview: string
}

export interface RewindReport {
  checkpoint_id: string
  session_id: string
  mode: RewindMode
  restored_files: string[]
  deleted_files: string[]
  preserved_manual_edits: string[]
  skipped_symlinks: number
  safety_checkpoint_id?: string | null
  new_head_event_id?: string | null
  rewind_summary?: string | null
}

export interface DaemonTelemetry {
  pid: number
  uptime_secs: number
  rss_bytes: number
  db_size_bytes: number
  active_sessions: number
  active_jobs?: number
  pending_approvals: number
  daily_cost_usd: number
  total_cost_usd: number
  cache_hit_pct: number
  telegram_connected: boolean
  telegram_bot_username?: string | null
}

export interface VaultStats {
  exists: boolean
  encrypted: boolean
  cipher: string
  path: string
  size_bytes: number
  key_count: number
  key_names: string[]
}

export interface DailyActivityPoint {
  date: string
  events: number
  turns: number
  tool_calls: number
  tokens: number
  cache_read_tokens: number
  cost_usd: number
}

export interface ToolUsageStat {
  name: string
  calls: number
  errors: number
  avg_duration_ms: number
}

export interface ModelUsageStat {
  provider: string
  model: string
  calls: number
  tokens: number
  cache_read_tokens: number
  cost_usd: number
}

export interface StorageSlice {
  id: string
  label: string
  path: string
  bytes: number
  files: number
}

export interface DashboardCounts {
  sessions: number
  messages: number
  tool_calls: number
  memories: number
  skills_enabled: number
  skills_total: number
  cron_jobs: number
  goal_loops: number
  background_jobs: number
  subagents: number
  checkpoints: number
  mcp_servers: number
  paired_peers: number
}

export interface DashboardAnalytics {
  telemetry: DaemonTelemetry
  vault: VaultStats
  counts: DashboardCounts
  total_storage_bytes: number
  storage: StorageSlice[]
  daily_activity: DailyActivityPoint[]
  top_tools: ToolUsageStat[]
  top_models: ModelUsageStat[]
  channels: Record<string, number>
  streak_days: number
  active_days: number
  total_tokens: number
  total_cache_read_tokens: number
  generated_at: string
}

export type AvatarFrameStyle =
  | 'squircle'
  | 'bare'
  | 'circle'
  | 'soft_tint'
  | 'accent_solid'
  | 'ring'

export type AgentAvatarStyle =
  | 'liquid_halo'
  | 'fox_geometric'
  | 'fox_line'
  | 'fox_cute'
  | 'fox_origami'
  | 'fox_kitsune'
  | 'fox_side'
  | 'fox_sleeping'
  | 'fox_crest'
  | 'fox_cyber'
  | 'icon_bot'
  | 'icon_brain'
  | 'icon_cpu'
  | 'icon_terminal'
  | 'icon_zap'
  | 'icon_shield'
  | 'icon_activity'
  | 'icon_braces'
  | 'icon_globe'
  | 'pulse_ring'
  | 'matrix_dot'
  | 'minimal_dot'
  | 'hidden'

export type UserAvatarStyle =
  | 'icon_user'
  | 'user_circle'
  | 'user_round'
  | 'user_check'
  | 'user_code'
  | 'user_crown'
  | 'user_glasses'
  | 'user_smile'
  | 'user_hoodie'
  | 'monogram'
  | 'hidden'

export interface ColorTokens {
  background: string
  surface: string
  card: string
  border: string
  foreground: string
  muted: string
  primary: string
  primary_foreground: string
  accent: string
  destructive: string
  success: string
  warning: string
}

export interface ComponentSlots {
  composer: {
    shell_variant:
      | 'floating_card'
      | 'pill_capsule'
      | 'minimal_bar'
      | 'glass_dock'
      | 'motion_brew_prompt'
    model_picker_style: 'drilldown' | 'split_pane' | 'compact_inline' | 'rareui_picker'
    model_trigger_chrome?:
      | 'bordered_filled'
      | 'border_only'
      | 'background_only'
      | 'accent_tint'
      | 'minimal_bare'
    send_button_shape: 'rounded_square' | 'circle' | 'pill_label'
    send_button_icon: 'arrow_up' | 'send' | 'sparkles' | 'corner_up'
    context_meter:
      | 'progress_ring'
      | 'smoothui_meter'
      | 'kokonut_meter'
      | 'motion_brew_ring'
      | 'linear_bar'
      | 'compact_text'
      | 'hidden'
    context_meter_chrome?: 'bordered_filled' | 'border_only' | 'background_only' | 'minimal_bare'
    action_pills: 'soft_tint' | 'outline' | 'ghost' | 'solid_chip'
    border_animation?:
      | 'border_beam_md'
      | 'border_beam_sm'
      | 'border_beam_line'
      | 'border_beam_pulse_out'
      | 'border_beam_pulse_in'
      | 'accent_orbit'
      | 'accent_shimmer_edge'
      | 'accent_breathe_halo'
      | 'none'
    animation_trigger?: 'while_working' | 'always' | 'on_focus' | 'off'
    beam_color_variant?:
      | 'theme_accent'
      | 'colorful'
      | 'mono'
      | 'ocean'
      | 'sunset'
      | 'forest'
      | 'candy'
      | 'ice'
      | 'gold'
    beam_strength?: number
    show_attach: boolean
    show_policy: boolean
    show_streaming_toggle: boolean
    show_concise_toggle?: boolean
    concise_button_style?:
      | 'split_pill'
      | 'accent_solid'
      | 'soft_tint'
      | 'outline_mono'
      | 'minimal_ghost'
      | 'segmented_bar'
    concise_button_shape?: 'pill' | 'rounded' | 'square'
    concise_button_chrome?: 'bordered_filled' | 'border_only' | 'background_only' | 'minimal_bare'
    concise_button_accent?: string
  }
  left_menubar: {
    frame_style:
      | 'flush_border'
      | 'floating_card'
      | 'compact_rail'
      | 'motion_brew_nav'
      | 'responsive_sidebar'
    new_session_style: 'vermilion_soft' | 'solid_primary' | 'outline_minimal' | 'pill_soft'
    session_item_style: 'soft_fill' | 'left_accent_bar' | 'bordered_card' | 'minimal_text'
    show_telemetry_badge: boolean
  }
  right_menubar: {
    dock_style:
      | 'floating_capsule'
      | 'attached_rail'
      | 'glass_pill'
      | 'vengeance_glass_dock'
      | 'skiper98_vtooltip'
      | 'minimal_strip'
    active_indicator: 'vermilion_pill' | 'solid_accent' | 'side_bar' | 'dot_indicator'
  }
  chat_page: {
    background:
      | 'clean'
      | 'dot_grid'
      | 'fine_grid'
      | 'cross_grid'
      | 'top_glow'
      | 'bottom_glow'
      | 'vignette'
      | 'diagonal_hatch'
    content_width: 'narrow' | 'balanced' | 'wide' | 'full'
    user_bubble:
      | 'soft_card'
      | 'easeui_bubble'
      | 'accent_tint'
      | 'solid_accent'
      | 'minimal_border'
      | 'background_only'
      | 'borderless_minimal'
      | 'pill'
    model_badge_style?:
      | 'bordered_chip'
      | 'border_only'
      | 'soft_fill'
      | 'accent_tint'
      | 'minimal_text'
      | 'hidden'
    thinking_style:
      | 'collapsible_steps'
      | 'aimran_stretchtext'
      | 'inline_quote'
      | 'minimal_bar'
      | 'hidden'
    tool_card_style:
      | 'aui_smart'
      | 'aui_tool_call'
      | 'aui_tool_call_icon'
      | 'aui_code_runner'
      | 'aui_terminal_block'
      | 'beui_tool_result'
      | 'pill_chips'
      | 'detailed_card'
      | 'terminal_line'
    show_avatars: boolean
    agent_avatar_frame?: AvatarFrameStyle
    agent_avatar_color?: string
    user_avatar?: UserAvatarStyle
    user_avatar_frame?: AvatarFrameStyle
    user_avatar_color?: string
    scroller_style: 'beui_rail' | 'beui_scroller' | 'smooth' | 'native'
    streaming_style:
      | 'beui_stream'
      | 'easeui_stream'
      | 'rareui_stream'
      | 'aui_streaming_text'
      | 'editorial'
    emphasis_style?: 'accent' | 'accent_glow' | 'marker_stroke' | 'neutral_bold'
    heading_style?: 'accent' | 'accent_bar' | 'accent_underline' | 'soft_tint' | 'neutral'
    message_actions_style?: 'aui_message_actions' | 'beui_inline' | 'minimal'
    approval_style:
      | 'aui_permission_grant'
      | 'kokonut_approval'
      | 'beui_approval'
      | 'easeui_approval'
      | 'compact_card'
    activity_style:
      | 'aui_tool_call_icon'
      | 'kokonut_tool_calls'
      | 'aui_tool_call'
      | 'aui_smart'
      | 'beui_tool_result'
      | 'beui_activity'
      | 'easeui_tool_chip'
      | 'aui_run_activity'
      | 'aui_tool_timeline'
      | 'aui_terminal_block'
      | 'aui_code_runner'
      | 'detailed_card'
      | 'terminal_line'
      | 'minimal_pills'
      | 'hidden'
    loading_style?:
      | 'beui_loader'
      | 'kokonut_loading'
      | 'easeui_pixel_loader'
      | 'rareui_ai_status'
      | 'aui_thinking_indicator'
      | 'aui_loading_dots'
      | 'aui_loading_squares'
      | 'aui_loading_rounded'
    diff_style?: 'aui_code_diff' | 'kokonut_diff' | 'aui_reviewable_diff' | 'minimal_inline'
    shell_style?: 'aui_terminal_block' | 'aimran_copy_cmd' | 'aui_code_runner' | 'minimal_mono'
    code_block_style?:
      | 'aui_code_runner'
      | 'easeui_code'
      | 'rareui_code'
      | 'aimran_copy_cmd'
      | 'minimal_code'
      | 'bordered_code'
    error_state_style?:
      | 'aui_error_card'
      | 'easeui_alert'
      | 'beui_alert'
      | 'compact_bar'
      | 'minimal_text'
    stopped_run_style?: 'aui_stopped_run' | 'compact_banner' | 'accent_rail' | 'minimal_notice'
    file_tree_style?: 'aui_file_tree' | 'compact_list' | 'minimal_flat' | 'bordered_panel'
    web_search_style?: 'aui_web_search' | 'rareui_web_search' | 'compact_cards' | 'minimal_links'
    show_day_separators?: boolean
    loader_variant:
      | 'dot-matrix'
      | 'dots'
      | 'bars'
      | 'spinner'
      | 'comet'
      | 'morph'
      | 'metaballs'
      | 'newton'
      | 'helix'
      | 'dither'
      | 'ascii'
      | 'ascii-line'
      | 'ascii-braille'
      | 'ascii-blocks'
      | 'ascii-bounce'
      | 'scramble'
      | 'percent'
    show_citations: boolean
    citations_style?: 'beui_citations' | 'kokonut_citations' | 'minimal_pills'
    skill_badge_style?:
      | 'aui_memory_chips'
      | 'pill_glow'
      | 'minimal_outline'
      | 'accent_solid'
      | 'compact_chip'
      | 'hidden'
  }
  dashboard?: {
    layout: 'bento_grid' | 'editorial_stack' | 'compact_ops'
    widget_kit?: 'echarts' | 'motion_brew' | 'hybrid'
    recap_style?: 'motion_brew_recap' | 'aimran_marker_highlight' | 'aimran_stretchtext'
    chart_theme: 'monochrome_accent' | 'soft_gradient' | 'crisp_outline'
    heatmap_shape: 'rounded' | 'square' | 'circle'
    heatmap_range: 'year' | 'half_year' | 'quarter'
    show_recap_card?: boolean
    show_quota_gauges?: boolean
    show_heatmap: boolean
    show_storage_vault: boolean
    show_token_trends: boolean
    show_tools_table: boolean
    show_subagents_jobs: boolean
    show_logs: boolean
    show_faq?: boolean
  }
}

export interface StudioOption {
  value: string
  label: string
  hint: string
}

export interface StudioControl {
  path: string
  label: string
  description: string
  kind: 'cards' | 'toggle' | 'range' | 'icon_overrides' | 'color'
  options: StudioOption[]
  min?: number | null
  max?: number | null
  step?: number | null
  unit: string
}

export interface StudioSubComponent {
  id: string
  group?: string
  label: string
  description: string
  preview: string
  controls: StudioControl[]
}

export interface StudioCategory {
  id: string
  label: string
  description: string
  preview: string
  sub_components: StudioSubComponent[]
}

export interface UiCustomization {
  mode: 'dark' | 'light' | 'system'
  preset_id: string
  colors: ColorTokens
  buttons: {
    style:
      | 'solid'
      | 'soft'
      | 'outline'
      | 'ghost'
      | 'glass'
      | 'glow'
      | 'tactile3d'
      | 'shimmer'
      | 'vengeance_liquid'
      | 'vengeance_animated'
    shape: 'sharp' | 'rounded' | 'soft' | 'pill'
    size: 'compact' | 'default' | 'comfortable'
    motion: 'none' | 'subtle_lift' | 'scale_press' | 'magnetic_glow'
  }
  icons: {
    pack:
      | 'lucide'
      | 'pulsoicon'
      | 'iconsax'
      | 'hugeicons'
      | 'itshover'
      | 'tabler'
      | 'aicanvas'
      | 'humbleicons'
      | 'majesticons'
      | 'dooicons'
      | 'phosphor'
      | 'heroicons'
      | 'pixel'
    stroke_width: number
    style: 'outline' | 'duotone' | 'solid'
    size_px: number
    overrides: Record<string, string>
  }
  surface: {
    card_style:
      | 'minimal_top'
      | 'minimal_bottom'
      | 'borderless'
      | 'ghost_hairline'
      | 'flat'
      | 'bordered'
      | 'elevated'
      | 'glass'
      | 'spotlight'
    bg_pattern: 'clean' | 'dot_grid' | 'tech_grid' | 'ambient_glow'
    radius_px: number
    border_contrast: string
    scrollbar_style?: 'accent_thin' | 'muted_thin' | 'minimal'
    scroll_indicator?:
      | 'none'
      | 'skiper87_fade'
      | 'skiper89_bar'
      | 'skiper94_ring'
      | 'skiper87_and_89'
    input_caret_style?: 'skiper106_smooth' | 'native_accent' | 'native_default'
    tabs_variant?: 'pill' | 'segment' | 'underline' | 'skiper96'
    accordion_style?: 'bouncy' | 'compact' | 'rareui_faq'
    alert_style?: 'beui_alert' | 'easeui_alert' | 'compact'
    command_palette_style?: 'default' | 'easeui_palette'
    toast_style?: 'default' | 'easeui_toast' | 'minimal_pill' | 'accent_bar' | 'glass_toast'
    toast_position?:
      | 'top_center'
      | 'top_right'
      | 'top_left'
      | 'bottom_center'
      | 'bottom_right'
      | 'bottom_left'
    link_hover_style?:
      | 'zenko_underline'
      | 'skiper40_bracket'
      | 'skiper40_arrow_shift'
      | 'skiper40_highlight_fill'
      | 'slide'
      | 'double'
      | 'grow'
      | 'strike'
      | 'fade'
      | 'pulse'
      | 'swap'
      | 'sweep'
      | 'bounce'
      | 'arc'
      | 'scribble'
    modal_style?: 'center_morph' | 'scale'
    popover_style?: 'gooey' | 'crisp'
    element_chrome?: 'borderless' | 'minimal' | 'bordered'
    table_style?: 'borderless' | 'minimal' | 'striped' | 'bordered' | 'easeui_table'
    tooltip_style?:
      | 'accent_edge'
      | 'skiper101_arrow'
      | 'skiper43_vercel'
      | 'skecher_gooey'
      | 'minimal_glass'
      | 'solid_contrast'
      | 'accent_solid'
      | 'quiet_matte'
    tooltip_shape?: 'pill' | 'rounded' | 'sharp'
    tooltip_motion?: 'spring' | 'fade' | 'instant'
    show_hover_hints?: boolean
  }
  typography: {
    ui_font: string
    mono_font: string
    base_size_px: number
    custom_ui_font?: string
    custom_mono_font?: string
  }
  layout: {
    density: 'compact' | 'balanced' | 'spacious'
    sidebar_mode: 'fixed' | 'icon_rail' | 'floating_dock'
    chat_layout: 'editorial' | 'bubble_split' | 'terminal_stream'
    show_inspector: boolean
  }
  motion: {
    preset: 'snappy' | 'fluid_spring' | 'reduced' | 'instant'
    orb_style: AgentAvatarStyle
    theme_transition?:
      | 'circle-blur'
      | 'circle'
      | 'skiper26_bottom_up'
      | 'skiper26_top_down'
      | 'skiper26_diagonal'
      | 'rectangle'
      | 'blinds'
    theme_toggle_icon?:
      | 'default'
      | 'skiper4_eclipse'
      | 'skiper4_rays'
      | 'skiper4_horizon'
      | 'skiper4_classic_fill'
      | 'skiper4_minimal_dot'
    smooth_scroll?: boolean
  }
  widgets: {
    show_thinking: boolean
    show_cost_badges: boolean
    show_timestamps: boolean
    show_line_numbers: boolean
    show_shortcut_hints: boolean
  }
  components: ComponentSlots
}

export interface UiProfile {
  id: string
  name: string
  is_active: boolean
  config: UiCustomization
  updated_at: string
}

export interface ThemeSwatch {
  bg: string
  surface: string
  card: string
  border: string
  fg: string
  muted: string
  primary: string
  primary_fg: string
}

export interface ThemePresetMeta {
  id: string
  label: string
  dark: ThemeSwatch
  light: ThemeSwatch
}

export interface ModelInfo {
  id: string
  name: string
  context_length?: number | null
  max_output_tokens?: number | null
  prompt_price_per_m?: number | null
  completion_price_per_m?: number | null
  is_free: boolean
  supports_tools: boolean
  supports_reasoning: boolean
  source?: string
}

export interface GatewayPeer {
  channel: string
  peer_id: string
  username?: string | null
  display_name?: string | null
  role: 'owner' | 'trusted' | 'readonly'
  paired_at: string
}

export interface PairingCode {
  code: string
  channel: string
  role: string
  expires_at: string
}

export interface SkillDoc {
  slug: string
  name: string
  description: string
  content: string
  enabled: boolean
  status?: 'active' | 'draft'
  author?: 'user' | 'agent' | 'dream'
  triggers?: string[]
  allowed_tools?: string[]
  token_estimate: number
  subfiles?: string[]
  source_dir?: string | null
  file_path?: string | null
}

export interface SkillVersionSnapshot {
  id: string
  slug: string
  reason: string
  bytes: number
  created_at: string
}

export type GoalLoopStatus =
  | 'active'
  | 'paused'
  | 'succeeded'
  | 'failed'
  | 'budget_exhausted'
  | 'stalled'

export interface GoalLoopRecord {
  id: string
  name: string
  session_id: string
  workspace_dir: string
  goal_text: string
  verify_cmd: string[]
  interval_secs: number
  max_iterations: number
  max_budget_usd: number
  max_consecutive_stalls: number
  checkpoint_each_iter: boolean
  auto_rewind_on_stall: boolean
  model?: string | null
  notify_channel: string
  status: GoalLoopStatus
  iterations_done: number
  consecutive_stalls: number
  cost_spent_usd: number
  last_verify_exit?: number | null
  last_verify_output?: string | null
  next_run_at?: string | null
  created_at: string
  updated_at: string
}

export interface GoalLoopRun {
  id: string
  goal_id: string
  iteration: number
  checkpoint_id?: string | null
  verify_exit_code: number
  verify_output_excerpt: string
  verify_hash: string
  tokens_used: number
  cost_usd: number
  duration_ms: number
  outcome: string
  reward: number
  created_at: string
}

export interface WebhookRuleConfig {
  id: string
  enabled: boolean
  source: string
  match_pointer?: string | null
  match_values: string[]
  prompt_template: string
  session_key?: string | null
  workspace_dir?: string | null
  model?: string | null
  goal_loop_id?: string | null
  cooldown_secs: number
  max_per_hour: number
}

export interface WorkspaceWatcherConfig {
  id: string
  enabled: boolean
  path: string
  glob: string
  poll_interval_secs: number
  debounce_secs: number
  cooldown_secs: number
  max_per_hour: number
  action: 'prompt' | 'script' | 'goal_loop'
  prompt: string
  goal_loop_id?: string | null
  script_path?: string | null
  model?: string | null
}

export interface TriggerStatusEntry {
  id: string
  kind: string
  enabled: boolean
  target: string
  fires_last_hour: number
  max_per_hour: number
  cooldown_secs: number
  last_fired_at?: string | null
  detail: string
}

export interface WorkspaceSnapshot {
  soul_md: string
  user_md: string
  memory_md: string
  project_memory_md?: string
  zenko_md: string
  skills: SkillDoc[]
  repo_rules_loaded?: string[]
  tier1_tokens?: number
  tier1_hash?: string
  tier2_tokens?: number
  total_tokens_estimate: number
}

export type ConciseMode = 'off' | 'lite' | 'full' | 'ultra'

export interface ConciseModeDescriptor {
  mode: ConciseMode
  id: ConciseMode
  label: string
  description: string
  target_reduction_pct: number
  directive: string
}

export type ZMode = 'lite' | 'balanced' | 'high' | 'ultra'

export interface ZModeDescriptor {
  mode: ZMode
  id: ZMode
  label: string
  tagline: string
  description: string
  concise_mode: ConciseMode
  minimalism_mode: CodeMinimalismMode
  auto_bloat_hint: boolean
  read_less: boolean
  read_less_min_bytes: number
  microcompact_trigger_ratio: number
  compaction_trigger_ratio: number
  auto_micro_skills: boolean
  auto_tool_prune: boolean
  tool_prune_min_prompts: number
  tool_prune_min_sessions: number
  p90_loop_guard: boolean
  p90_multiplier: number
}

export interface ContextBreakdown {
  session_id: string
  model: string
  context_window: number
  total_tokens: number
  utilization_pct: number
  tier1_tokens: number
  tier1_hash: string
  tier2_tokens: number
  repo_rules_loaded: string[]
  tool_schema_tokens: number
  eager_tools_count: number
  deferred_active_count: number
  deferred_available_count: number
  history_tokens: number
  microcompacted_tools_count: number
  microcompacted_tokens_saved: number
  active_checkpoint_id?: string | null
  checkpoints_count: number
  spilled_artifacts_count: number
  cache_hit_pct: number
  zmode?: ZMode
  zmode_customized?: boolean
  concise_mode?: ConciseMode
  auto_clarity?: boolean
  read_less_enabled?: boolean
  read_less_calls_compressed?: number
  read_less_bytes_saved?: number
  read_less_tokens_saved?: number
  economy_net_tokens_saved?: number
  economy_active_micro_skills?: number
  economy_pruned_tools_count?: number
  economy_p90_turn_calls?: number
  minimalism_mode?: CodeMinimalismMode
  economy_lines_added?: number
  economy_lines_deleted?: number
  cache_stability_warnings?: string[]
  scoped_rules_loaded?: string[]
  nested_repo_docs_count?: number
  project_brain_pages_count?: number
}

export type CodeMinimalismMode = 'off' | 'lite' | 'full' | 'ultra'

export interface CodeMinimalismDescriptor {
  mode: CodeMinimalismMode
  id: CodeMinimalismMode
  label: string
  description: string
  directive: string
  bloat_hint_net_loc: number
  bloat_hint_new_files: number
}

export interface RepoCodeMetrics {
  repo_scope: string
  lines_added: number
  lines_deleted: number
  net_loc_delta: number
  turns_with_edits: number
  new_files_created: number
  bloat_hints_issued: number
  debt_markers_last_seen: number
  updated_at?: string | null
}

export interface DebtItem {
  path: string
  line: number
  marker: string
  text: string
}

export interface DebtScanReport {
  files_scanned: number
  total_found: number
  truncated: boolean
  items: DebtItem[]
}

export interface LargeFileFinding {
  path: string
  lines: number
}

export interface LongFunctionFinding {
  path: string
  line: number
  span_lines: number
  signature: string
}

export interface DuplicateSymbolFinding {
  symbol: string
  locations: string[]
}

export interface ThinWrapperFinding {
  path: string
  line: number
  span_lines: number
  kind: string
  name: string
}

export interface DormantDependencyFinding {
  manifest: string
  package: string
}

export interface OversizedHunkFinding {
  path: string
  start_line: number
  added_lines: number
}

export interface DoxDriftFinding {
  doc_path: string
  changed_files: string[]
  new_files_count: number
}

export interface DiffAuditReport {
  files_changed: string[]
  new_files: string[]
  lines_added: number
  lines_deleted: number
  net_loc_delta: number
  manifest_changes: string[]
  oversized_hunks: OversizedHunkFinding[]
  duplicate_symbols: DuplicateSymbolFinding[]
  dox_drift?: DoxDriftFinding[]
  findings: string[]
}

export interface SingleImplAbstractionFinding {
  path: string
  line: number
  kind: string
  name: string
  impl_count: number
}

export interface CommentBloatFinding {
  path: string
  comment_lines: number
  code_lines: number
  comment_ratio_pct: number
  longest_block_lines: number
  longest_block_line: number
}

export interface RepoAuditReport {
  files_scanned: number
  total_lines: number
  large_files: LargeFileFinding[]
  long_functions: LongFunctionFinding[]
  duplicate_symbols: DuplicateSymbolFinding[]
  thin_wrappers: ThinWrapperFinding[]
  single_impl_abstractions?: SingleImplAbstractionFinding[]
  comment_bloat?: CommentBloatFinding[]
  dormant_dependencies: DormantDependencyFinding[]
  debt_count: number
}

export interface MinimalismAuditPayload {
  diff: DiffAuditReport
  repo: RepoAuditReport
}

export interface EconomyConfig {
  enabled: boolean
  auto_micro_skills: boolean
  auto_tool_prune: boolean
  p90_loop_guard: boolean
  optional_fast_judge: boolean
  micro_skill_min_sessions: number
  tool_prune_min_prompts: number
  tool_prune_min_sessions: number
  tool_prune_reaudit_tasks: number
  p90_multiplier: number
  minimalism_mode: CodeMinimalismMode
  auto_bloat_hint: boolean
}

export interface RepoMicroSkill {
  id: string
  repo_scope: string
  signature: string
  fact: string
  failed_cmd: string
  fixed_cmd: string
  corroborations: number
  session_ids: string[]
  need_hits: number
  exposures: number
  contradictions: number
  status: 'candidate' | 'active' | 'retired'
  created_at: string
  updated_at: string
}

export interface RepoToolUsage {
  repo_scope: string
  tool_name: string
  call_count: number
  prompts_since_used: number
  sessions_observed: number
  estimated_schema_tokens: number
  pruned_to_deferred: boolean
  manually_pinned: boolean
  last_used_at?: string | null
  updated_at: string
}

export interface CommandSavingsStat {
  command_family: string
  calls: number
  tokens_saved: number
  raw_bypasses?: number
}

export interface MissedSavingsOpportunity {
  command_prefix: string
  calls: number
  total_bytes: number
  estimated_missed_tokens: number
  suggested_filter_toml: string
}

export interface CustomFilterTestFailure {
  filter_name: string
  test_name: string
  expected: string
  actual: string
}

export interface CustomFilterVerifyReport {
  file_path: string
  exists: boolean
  valid: boolean
  rule_count: number
  tests_passed: number
  tests_failed: number
  errors?: string[]
  test_failures?: CustomFilterTestFailure[]
}

export interface CustomFilterTrustStatus {
  project_file_path: string
  project_file_exists: boolean
  project_sha256?: string | null
  trusted: boolean
  trusted_sha256?: string | null
  rule_count: number
  user_rule_count: number
}

export interface EconomyLedgerSummary {
  repo_scope: string
  read_less_tokens_saved: number
  output_dedup_tokens_saved?: number
  output_dedup_hits?: number
  shunt_hits?: number
  shunt_tokens_saved?: number
  ccr_recoveries?: number
  raw_bypasses?: number
  tool_prune_tokens_saved: number
  micro_skill_injections: number
  loop_guard_interventions: number
  overhead_tokens: number
  net_tokens_saved: number
  estimated_usd_saved: number
  p90_turn_calls: number
  envelope_calls: number
  baseline_samples: number
  active_micro_skills: number
  candidate_micro_skills: number
  retired_micro_skills: number
  pruned_tools_count: number
  completed_tasks: number
}

export interface TokenEconomyReport {
  repo_scope: string
  config: EconomyConfig
  summary: EconomyLedgerSummary
  code_metrics?: RepoCodeMetrics
  by_command?: CommandSavingsStat[]
  filter_status?: CustomFilterTrustStatus
  minimalism_modes?: CodeMinimalismDescriptor[]
  micro_skills: RepoMicroSkill[]
  tool_usage: RepoToolUsage[]
  pruned_tools: string[]
  prune_candidates: string[]
}

export interface MemoryKindMeta {
  id: string
  label: string
  description: string
  half_life_days: number
}

export interface MemoryEntry {
  id: string
  scope: string
  kind: string
  title: string
  content: string
  salience?: number
  access_count?: number
  last_accessed_at?: string | null
  updated_at?: string | null
  source_session_id?: string | null
  archived?: boolean
  created_at: string
}

export interface ScoredMemoryHit {
  entry: MemoryEntry
  score: number
  stale_paths: string[]
}

export interface MemoryDocSnapshot {
  id: string
  doc_name: string
  reason: string
  bytes: number
  created_at: string
}

export interface DreamRunRecord {
  id: string
  trigger: string
  status: string
  model?: string | null
  sessions_scanned: number
  memories_added: number
  memories_merged: number
  memories_archived: number
  docs_patched: number
  skills_proposed?: number
  diff_preview?: string | null
  detail?: string | null
  started_at: string
  finished_at?: string | null
}

export interface DreamStatus {
  enabled: boolean
  running: boolean
  watermark_ts?: string | null
  last_run_ts?: string | null
  pending_sessions: number
  recent_runs: DreamRunRecord[]
}

export interface SessionSearchHit {
  session_id: string
  session_title: string
  channel: string
  workspace_dir?: string | null
  event_id: string
  role: string
  snippet: string
  rank: number
  created_at: string
}

export interface CronJob {
  id: string
  name: string
  cron_expr: string
  kind: 'cron' | 'heartbeat' | 'script'
  prompt: string
  script_path?: string | null
  target_channel: string
  enabled: boolean
  next_run_at?: string | null
  last_run_at?: string | null
  last_status?: string | null
}

export interface LogCategoryMeta {
  id: string
  label: string
  file: string
  description: string
}

export interface LogEntry {
  timestamp: string
  level: string
  category?: string
  target: string
  session_id?: string | null
  turn_id?: string | null
  call_id?: string | null
  message: string
  payload?: Record<string, any> | null
}

export interface ProviderDescriptor {
  id: string
  label: string
  description: string
  default_base_url: string
  metadata_keys: string[]
  requires_api_key: boolean
  vault_secret_key: string
  supports_prompt_caching: boolean
  supports_sticky_routing: boolean
  dialect: 'open_router' | 'mini_max' | 'open_ai_compat'
}

export interface WebSearchProviderDescriptor {
  id: string
  label: string
  description: string
  requires_api_key: boolean
  vault_secret_key: string
}

export interface LspServerDescriptor {
  id: string
  label: string
  language_id: string
  command: string
  args: string[]
  extensions: string[]
  root_markers: string[]
}

export interface LspServerRuntimeStatus {
  server_id: string
  language_id: string
  workspace_root: string
  running: boolean
  open_documents: number
  cached_diagnostics_files: number
  last_used_at: string
}

export interface McpServerStatus {
  name: string
  transport: 'stdio' | 'http' | string
  enabled: boolean
  connected: boolean
  tool_count: number
  tools: string[]
  catalog_dirty?: boolean
  last_error?: string | null
  last_synced_at?: string | null
}

export interface BootstrapResponse {
  authorized: boolean
  config?: any
  telemetry?: DaemonTelemetry
  sessions?: Session[]
  active_profile: UiProfile
  profiles?: UiProfile[]
  presets: ThemePresetMeta[]
  studio_schema?: StudioCategory[]
  providers?: ProviderDescriptor[]
  concise_modes?: ConciseModeDescriptor[]
  zmodes?: ZModeDescriptor[]
  zmode_customized?: boolean
  configured_providers?: Record<string, boolean>
  pending_approvals?: ApprovalRecord[]
  pending_clarifications?: ClarificationRecord[]
  has_provider_key?: boolean
  has_openrouter_key?: boolean
  has_telegram_token?: boolean
  has_brave_search_key?: boolean
  has_tavily_search_key?: boolean
  web_search_providers?: WebSearchProviderDescriptor[]
  mcp_servers?: McpServerStatus[]
  lsp_registry?: LspServerDescriptor[]
  lsp_active?: LspServerRuntimeStatus[]
  workspace_dir?: string
  hub_dir?: string
  hub_categories?: HubCategoryMeta[]
  task_kinds?: TaskKindMeta[]
  snapshot_tokens?: number
  skills_count?: number
  log_categories?: LogCategoryMeta[]
  log_counts?: Record<string, number>
  logs_dir?: string
  jobs?: BackgroundJob[]
  subagent_roles?: SubagentRoleDescriptor[]
  subagents?: SubagentRecord[]
}

export type ActiveView =
  | 'chat'
  | 'telemetry'
  | 'gateways'
  | 'memory'
  | 'automation'
  | 'appearance'
  | 'setup'
