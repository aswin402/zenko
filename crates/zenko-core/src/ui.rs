use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    System,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorTokens {
    pub background: String,
    pub surface: String,
    pub card: String,
    pub border: String,
    pub foreground: String,
    pub muted: String,
    pub primary: String,
    pub primary_foreground: String,
    pub accent: String,
    pub destructive: String,
    pub success: String,
    pub warning: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonStyle {
    #[default]
    Solid,
    Soft,
    Outline,
    Ghost,
    Glass,
    Glow,
    Tactile3d,
    Shimmer,
}

impl ButtonStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Soft => "soft",
            Self::Outline => "outline",
            Self::Ghost => "ghost",
            Self::Glass => "glass",
            Self::Glow => "glow",
            Self::Tactile3d => "tactile3d",
            Self::Shimmer => "shimmer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonShape {
    Sharp,
    #[default]
    Rounded,
    Soft,
    Pill,
}

impl ButtonShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sharp => "sharp",
            Self::Rounded => "rounded",
            Self::Soft => "soft",
            Self::Pill => "pill",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComponentSize {
    Compact,
    #[default]
    Default,
    Comfortable,
}

impl ComponentSize {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Default => "default",
            Self::Comfortable => "comfortable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ButtonMotion {
    None,
    SubtleLift,
    #[default]
    ScalePress,
    MagneticGlow,
}

impl ButtonMotion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SubtleLift => "subtle_lift",
            Self::ScalePress => "scale_press",
            Self::MagneticGlow => "magnetic_glow",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ButtonCustomization {
    #[serde(default)]
    pub style: ButtonStyle,
    #[serde(default)]
    pub shape: ButtonShape,
    #[serde(default)]
    pub size: ComponentSize,
    #[serde(default)]
    pub motion: ButtonMotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IconPack {
    #[default]
    Lucide,
    Pulsoicon,
    Iconsax,
    Hugeicons,
    Itshover,
    Tabler,
    Aicanvas,
    Humbleicons,
    Majesticons,
    Dooicons,
    Phosphor,
    Heroicons,
    Pixel,
}

impl IconPack {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lucide => "lucide",
            Self::Pulsoicon => "pulsoicon",
            Self::Iconsax => "iconsax",
            Self::Hugeicons => "hugeicons",
            Self::Itshover => "itshover",
            Self::Tabler => "tabler",
            Self::Aicanvas => "aicanvas",
            Self::Humbleicons => "humbleicons",
            Self::Majesticons => "majesticons",
            Self::Dooicons => "dooicons",
            Self::Phosphor => "phosphor",
            Self::Heroicons => "heroicons",
            Self::Pixel => "pixel",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IconStyle {
    #[default]
    Outline,
    Duotone,
    Solid,
}

impl IconStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Outline => "outline",
            Self::Duotone => "duotone",
            Self::Solid => "solid",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IconCustomization {
    #[serde(default)]
    pub pack: IconPack,
    #[serde(default = "default_icon_stroke")]
    pub stroke_width: f32,
    #[serde(default)]
    pub style: IconStyle,
    #[serde(default = "default_icon_size")]
    pub size_px: u16,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

impl Default for IconCustomization {
    fn default() -> Self {
        Self {
            pack: IconPack::Lucide,
            stroke_width: default_icon_stroke(),
            style: IconStyle::Outline,
            size_px: default_icon_size(),
            overrides: BTreeMap::new(),
        }
    }
}

fn default_icon_stroke() -> f32 {
    1.75
}
fn default_icon_size() -> u16 {
    16
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceStyle {
    #[default]
    MinimalTop,
    MinimalBottom,
    Borderless,
    GhostHairline,
    Flat,
    Bordered,
    Elevated,
    Glass,
    Spotlight,
}

impl SurfaceStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MinimalTop => "minimal_top",
            Self::MinimalBottom => "minimal_bottom",
            Self::Borderless => "borderless",
            Self::GhostHairline => "ghost_hairline",
            Self::Flat => "flat",
            Self::Bordered => "bordered",
            Self::Elevated => "elevated",
            Self::Glass => "glass",
            Self::Spotlight => "spotlight",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BgPattern {
    #[default]
    Clean,
    DotGrid,
    TechGrid,
    AmbientGlow,
}

impl BgPattern {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::DotGrid => "dot_grid",
            Self::TechGrid => "tech_grid",
            Self::AmbientGlow => "ambient_glow",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceCustomization {
    #[serde(default)]
    pub card_style: SurfaceStyle,
    #[serde(default)]
    pub bg_pattern: BgPattern,
    #[serde(default = "default_radius_px")]
    pub radius_px: u16,
    #[serde(default = "default_border_contrast")]
    pub border_contrast: String,
    #[serde(default = "default_scrollbar_style")]
    pub scrollbar_style: String,
    #[serde(default = "default_tabs_variant")]
    pub tabs_variant: String,
    #[serde(default = "default_accordion_style")]
    pub accordion_style: String,
    #[serde(default = "default_alert_style")]
    pub alert_style: String,
    #[serde(default = "default_modal_style")]
    pub modal_style: String,
    #[serde(default = "default_popover_style")]
    pub popover_style: String,
    #[serde(default = "default_element_chrome")]
    pub element_chrome: String,
    #[serde(default = "default_table_style")]
    pub table_style: String,
    #[serde(default = "default_tooltip_style")]
    pub tooltip_style: String,
    #[serde(default = "default_tooltip_shape")]
    pub tooltip_shape: String,
    #[serde(default = "default_tooltip_motion")]
    pub tooltip_motion: String,
    #[serde(default = "default_true_ui")]
    pub show_hover_hints: bool,
}

impl Default for SurfaceCustomization {
    fn default() -> Self {
        Self {
            card_style: SurfaceStyle::MinimalTop,
            bg_pattern: BgPattern::Clean,
            radius_px: default_radius_px(),
            border_contrast: default_border_contrast(),
            scrollbar_style: default_scrollbar_style(),
            tabs_variant: default_tabs_variant(),
            accordion_style: default_accordion_style(),
            alert_style: default_alert_style(),
            modal_style: default_modal_style(),
            popover_style: default_popover_style(),
            element_chrome: default_element_chrome(),
            table_style: default_table_style(),
            tooltip_style: default_tooltip_style(),
            tooltip_shape: default_tooltip_shape(),
            tooltip_motion: default_tooltip_motion(),
            show_hover_hints: true,
        }
    }
}

fn default_radius_px() -> u16 {
    10
}
fn default_border_contrast() -> String {
    "normal".to_string()
}
fn default_scrollbar_style() -> String {
    "accent_thin".to_string()
}
fn default_tabs_variant() -> String {
    "pill".to_string()
}
fn default_accordion_style() -> String {
    "bouncy".to_string()
}
fn default_alert_style() -> String {
    "beui_alert".to_string()
}
fn default_modal_style() -> String {
    "center_morph".to_string()
}
fn default_popover_style() -> String {
    "gooey".to_string()
}
fn default_element_chrome() -> String {
    "minimal".to_string()
}
fn default_table_style() -> String {
    "borderless".to_string()
}
fn default_tooltip_style() -> String {
    "accent_edge".to_string()
}
fn default_tooltip_shape() -> String {
    "pill".to_string()
}
fn default_tooltip_motion() -> String {
    "spring".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypographyConfig {
    #[serde(default = "default_ui_font")]
    pub ui_font: String,
    #[serde(default = "default_mono_font")]
    pub mono_font: String,
    #[serde(default = "default_base_size")]
    pub base_size_px: u16,
    #[serde(default)]
    pub custom_ui_font: String,
    #[serde(default)]
    pub custom_mono_font: String,
}

impl Default for TypographyConfig {
    fn default() -> Self {
        Self {
            ui_font: default_ui_font(),
            mono_font: default_mono_font(),
            base_size_px: default_base_size(),
            custom_ui_font: String::new(),
            custom_mono_font: String::new(),
        }
    }
}

fn default_ui_font() -> String {
    "inter".to_string()
}
fn default_mono_font() -> String {
    "jetbrains".to_string()
}
fn default_base_size() -> u16 {
    14
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DensityMode {
    Compact,
    #[default]
    Balanced,
    Spacious,
}

impl DensityMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Balanced => "balanced",
            Self::Spacious => "spacious",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidebarMode {
    #[default]
    Fixed,
    IconRail,
    FloatingDock,
}

impl SidebarMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::IconRail => "icon_rail",
            Self::FloatingDock => "floating_dock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatLayout {
    #[default]
    Editorial,
    BubbleSplit,
    TerminalStream,
}

impl ChatLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Editorial => "editorial",
            Self::BubbleSplit => "bubble_split",
            Self::TerminalStream => "terminal_stream",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LayoutCustomization {
    #[serde(default)]
    pub density: DensityMode,
    #[serde(default)]
    pub sidebar_mode: SidebarMode,
    #[serde(default)]
    pub chat_layout: ChatLayout,
    #[serde(default)]
    pub show_inspector: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MotionPreset {
    #[default]
    Snappy,
    FluidSpring,
    Reduced,
    Instant,
}

impl MotionPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Snappy => "snappy",
            Self::FluidSpring => "fluid_spring",
            Self::Reduced => "reduced",
            Self::Instant => "instant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OrbStyle {
    #[default]
    LiquidHalo,
    FoxGeometric,
    FoxLine,
    FoxCute,
    FoxOrigami,
    FoxKitsune,
    FoxSide,
    FoxSleeping,
    FoxCrest,
    FoxCyber,
    // Legacy variants kept deserializable for backwards compatibility with older profiles
    IconBot,
    IconBrain,
    IconCpu,
    IconTerminal,
    IconZap,
    IconShield,
    IconActivity,
    IconBraces,
    IconGlobe,
    PulseRing,
    MatrixDot,
    MinimalDot,
    Hidden,
}

impl OrbStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LiquidHalo => "liquid_halo",
            Self::FoxGeometric => "fox_geometric",
            Self::FoxLine => "fox_line",
            Self::FoxCute => "fox_cute",
            Self::FoxOrigami => "fox_origami",
            Self::FoxKitsune => "fox_kitsune",
            Self::FoxSide => "fox_side",
            Self::FoxSleeping => "fox_sleeping",
            Self::FoxCrest => "fox_crest",
            Self::FoxCyber => "fox_cyber",
            Self::IconBot => "icon_bot",
            Self::IconBrain => "icon_brain",
            Self::IconCpu => "icon_cpu",
            Self::IconTerminal => "icon_terminal",
            Self::IconZap => "icon_zap",
            Self::IconShield => "icon_shield",
            Self::IconActivity => "icon_activity",
            Self::IconBraces => "icon_braces",
            Self::IconGlobe => "icon_globe",
            Self::PulseRing => "pulse_ring",
            Self::MatrixDot => "matrix_dot",
            Self::MinimalDot => "minimal_dot",
            Self::Hidden => "hidden",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionCustomization {
    #[serde(default)]
    pub preset: MotionPreset,
    #[serde(default)]
    pub orb_style: OrbStyle,
    #[serde(default = "default_theme_transition")]
    pub theme_transition: String,
    #[serde(default = "default_true_ui")]
    pub smooth_scroll: bool,
}

impl Default for MotionCustomization {
    fn default() -> Self {
        Self {
            preset: MotionPreset::default(),
            orb_style: OrbStyle::default(),
            theme_transition: default_theme_transition(),
            smooth_scroll: true,
        }
    }
}

fn default_theme_transition() -> String {
    "circle-blur".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetToggles {
    #[serde(default = "default_true_ui")]
    pub show_thinking: bool,
    #[serde(default = "default_true_ui")]
    pub show_cost_badges: bool,
    #[serde(default = "default_true_ui")]
    pub show_timestamps: bool,
    #[serde(default = "default_true_ui")]
    pub show_line_numbers: bool,
    #[serde(default = "default_true_ui")]
    pub show_shortcut_hints: bool,
}

impl Default for WidgetToggles {
    fn default() -> Self {
        Self {
            show_thinking: true,
            show_cost_badges: true,
            show_timestamps: true,
            show_line_numbers: true,
            show_shortcut_hints: true,
        }
    }
}

fn default_true_ui() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposerSlotConfig {
    #[serde(default = "default_composer_shell")]
    pub shell_variant: String,
    #[serde(default = "default_model_picker_style")]
    pub model_picker_style: String,
    #[serde(default = "default_send_shape")]
    pub send_button_shape: String,
    #[serde(default = "default_send_icon")]
    pub send_button_icon: String,
    #[serde(default = "default_context_meter")]
    pub context_meter: String,
    #[serde(default = "default_action_pills")]
    pub action_pills: String,
    #[serde(default = "default_true_ui")]
    pub show_attach: bool,
    #[serde(default = "default_true_ui")]
    pub show_policy: bool,
    #[serde(default = "default_true_ui")]
    pub show_streaming_toggle: bool,
}

impl Default for ComposerSlotConfig {
    fn default() -> Self {
        Self {
            shell_variant: default_composer_shell(),
            model_picker_style: default_model_picker_style(),
            send_button_shape: default_send_shape(),
            send_button_icon: default_send_icon(),
            context_meter: default_context_meter(),
            action_pills: default_action_pills(),
            show_attach: true,
            show_policy: true,
            show_streaming_toggle: true,
        }
    }
}

fn default_composer_shell() -> String { "floating_card".into() }
fn default_model_picker_style() -> String { "drilldown".into() }
fn default_send_shape() -> String { "rounded_square".into() }
fn default_send_icon() -> String { "arrow_up".into() }
fn default_context_meter() -> String { "progress_ring".into() }
fn default_action_pills() -> String { "soft_tint".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeftMenubarSlotConfig {
    #[serde(default = "default_sidebar_frame")]
    pub frame_style: String,
    #[serde(default = "default_new_session_style")]
    pub new_session_style: String,
    #[serde(default = "default_session_item_style")]
    pub session_item_style: String,
    #[serde(default = "default_true_ui")]
    pub show_telemetry_badge: bool,
}

impl Default for LeftMenubarSlotConfig {
    fn default() -> Self {
        Self {
            frame_style: default_sidebar_frame(),
            new_session_style: default_new_session_style(),
            session_item_style: default_session_item_style(),
            show_telemetry_badge: true,
        }
    }
}

fn default_sidebar_frame() -> String { "flush_border".into() }
fn default_new_session_style() -> String { "vermilion_soft".into() }
fn default_session_item_style() -> String { "soft_fill".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RightMenubarSlotConfig {
    #[serde(default = "default_dock_style")]
    pub dock_style: String,
    #[serde(default = "default_dock_indicator")]
    pub active_indicator: String,
}

impl Default for RightMenubarSlotConfig {
    fn default() -> Self {
        Self {
            dock_style: default_dock_style(),
            active_indicator: default_dock_indicator(),
        }
    }
}

fn default_dock_style() -> String { "floating_capsule".into() }
fn default_dock_indicator() -> String { "vermilion_pill".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatPageSlotConfig {
    #[serde(default = "default_chat_background")]
    pub background: String,
    #[serde(default = "default_content_width")]
    pub content_width: String,
    #[serde(default = "default_user_bubble")]
    pub user_bubble: String,
    #[serde(default = "default_thinking_style")]
    pub thinking_style: String,
    #[serde(default = "default_tool_card_style")]
    pub tool_card_style: String,
    #[serde(default = "default_true_ui")]
    pub show_avatars: bool,
    #[serde(default = "default_agent_avatar_frame")]
    pub agent_avatar_frame: String,
    #[serde(default)]
    pub agent_avatar_color: String,
    #[serde(default = "default_user_avatar")]
    pub user_avatar: String,
    #[serde(default = "default_user_avatar_frame")]
    pub user_avatar_frame: String,
    #[serde(default)]
    pub user_avatar_color: String,
    #[serde(default = "default_scroller_style")]
    pub scroller_style: String,
    #[serde(default = "default_streaming_style")]
    pub streaming_style: String,
    #[serde(default = "default_approval_style")]
    pub approval_style: String,
    #[serde(default = "default_activity_style")]
    pub activity_style: String,
    #[serde(default = "default_loader_variant")]
    pub loader_variant: String,
    #[serde(default = "default_loading_style")]
    pub loading_style: String,
    #[serde(default = "default_message_actions_style")]
    pub message_actions_style: String,
    #[serde(default = "default_diff_style")]
    pub diff_style: String,
    #[serde(default = "default_shell_style")]
    pub shell_style: String,
    #[serde(default = "default_code_block_style")]
    pub code_block_style: String,
    #[serde(default = "default_true_ui")]
    pub show_day_separators: bool,
    #[serde(default = "default_true_ui")]
    pub show_citations: bool,
    #[serde(default = "default_skill_badge_style")]
    pub skill_badge_style: String,
}

impl Default for ChatPageSlotConfig {
    fn default() -> Self {
        Self {
            background: default_chat_background(),
            content_width: default_content_width(),
            user_bubble: default_user_bubble(),
            thinking_style: default_thinking_style(),
            tool_card_style: default_tool_card_style(),
            show_avatars: true,
            agent_avatar_frame: default_agent_avatar_frame(),
            agent_avatar_color: String::new(),
            user_avatar: default_user_avatar(),
            user_avatar_frame: default_user_avatar_frame(),
            user_avatar_color: String::new(),
            scroller_style: default_scroller_style(),
            streaming_style: default_streaming_style(),
            approval_style: default_approval_style(),
            activity_style: default_activity_style(),
            loader_variant: default_loader_variant(),
            loading_style: default_loading_style(),
            message_actions_style: default_message_actions_style(),
            diff_style: default_diff_style(),
            shell_style: default_shell_style(),
            code_block_style: default_code_block_style(),
            show_day_separators: true,
            show_citations: true,
            skill_badge_style: default_skill_badge_style(),
        }
    }
}

fn default_chat_background() -> String { "clean".into() }
fn default_content_width() -> String { "balanced".into() }
fn default_user_bubble() -> String { "soft_card".into() }
fn default_thinking_style() -> String { "collapsible_steps".into() }
fn default_tool_card_style() -> String { "aui_smart".into() }
fn default_scroller_style() -> String { "beui_rail".into() }
fn default_streaming_style() -> String { "beui_stream".into() }
fn default_approval_style() -> String { "aui_permission_grant".into() }
fn default_activity_style() -> String { "aui_tool_call_icon".into() }
fn default_loader_variant() -> String { "dot-matrix".into() }
fn default_loading_style() -> String { "beui_loader".into() }
fn default_message_actions_style() -> String { "aui_message_actions".into() }
fn default_diff_style() -> String { "aui_code_diff".into() }
fn default_shell_style() -> String { "aui_terminal_block".into() }
fn default_code_block_style() -> String { "aui_code_runner".into() }
fn default_skill_badge_style() -> String { "aui_memory_chips".into() }
fn default_agent_avatar_frame() -> String { "squircle".into() }
fn default_user_avatar() -> String { "icon_user".into() }
fn default_user_avatar_frame() -> String { "circle".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSlotConfig {
    #[serde(default = "default_dashboard_layout")]
    pub layout: String,
    #[serde(default = "default_dashboard_chart_theme")]
    pub chart_theme: String,
    #[serde(default = "default_dashboard_heatmap_shape")]
    pub heatmap_shape: String,
    #[serde(default = "default_dashboard_heatmap_range")]
    pub heatmap_range: String,
    #[serde(default = "default_true_ui")]
    pub show_heatmap: bool,
    #[serde(default = "default_true_ui")]
    pub show_storage_vault: bool,
    #[serde(default = "default_true_ui")]
    pub show_token_trends: bool,
    #[serde(default = "default_true_ui")]
    pub show_tools_table: bool,
    #[serde(default = "default_true_ui")]
    pub show_subagents_jobs: bool,
    #[serde(default = "default_true_ui")]
    pub show_logs: bool,
}

impl Default for DashboardSlotConfig {
    fn default() -> Self {
        Self {
            layout: default_dashboard_layout(),
            chart_theme: default_dashboard_chart_theme(),
            heatmap_shape: default_dashboard_heatmap_shape(),
            heatmap_range: default_dashboard_heatmap_range(),
            show_heatmap: true,
            show_storage_vault: true,
            show_token_trends: true,
            show_tools_table: true,
            show_subagents_jobs: true,
            show_logs: true,
        }
    }
}

fn default_dashboard_layout() -> String { "bento_grid".into() }
fn default_dashboard_chart_theme() -> String { "monochrome_accent".into() }
fn default_dashboard_heatmap_shape() -> String { "rounded".into() }
fn default_dashboard_heatmap_range() -> String { "year".into() }

/// Per-component structural customization (colors & dark/light strictly inherit global theme).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComponentSlots {
    #[serde(default)]
    pub composer: ComposerSlotConfig,
    #[serde(default)]
    pub left_menubar: LeftMenubarSlotConfig,
    #[serde(default)]
    pub right_menubar: RightMenubarSlotConfig,
    #[serde(default)]
    pub chat_page: ChatPageSlotConfig,
    #[serde(default)]
    pub dashboard: DashboardSlotConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiCustomization {
    #[serde(default)]
    pub mode: ThemeMode,
    #[serde(default = "default_preset_id")]
    pub preset_id: String,
    pub colors: ColorTokens,
    #[serde(default)]
    pub buttons: ButtonCustomization,
    #[serde(default)]
    pub icons: IconCustomization,
    #[serde(default)]
    pub surface: SurfaceCustomization,
    #[serde(default)]
    pub typography: TypographyConfig,
    #[serde(default)]
    pub layout: LayoutCustomization,
    #[serde(default)]
    pub motion: MotionCustomization,
    #[serde(default)]
    pub widgets: WidgetToggles,
    #[serde(default)]
    pub components: ComponentSlots,
}

fn default_preset_id() -> String {
    "zenko_vermilion".to_string()
}

impl Default for UiCustomization {
    fn default() -> Self {
        Self::preset("zenko_vermilion")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiProfile {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub config: UiCustomization,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ThemeSwatch {
    pub bg: &'static str,
    pub surface: &'static str,
    pub card: &'static str,
    pub border: &'static str,
    pub fg: &'static str,
    pub muted: &'static str,
    pub primary: &'static str,
    pub primary_fg: &'static str,
}

impl ThemeSwatch {
    fn to_color_tokens(self, mode: ThemeMode) -> ColorTokens {
        let is_light = matches!(mode, ThemeMode::Light);
        ColorTokens {
            background: self.bg.into(),
            surface: self.surface.into(),
            card: self.card.into(),
            border: self.border.into(),
            foreground: self.fg.into(),
            muted: self.muted.into(),
            primary: self.primary.into(),
            primary_foreground: self.primary_fg.into(),
            accent: self.primary.into(),
            destructive: if is_light { "#dc2626" } else { "#ef4444" }.into(),
            success: if is_light { "#16a34a" } else { "#22c55e" }.into(),
            warning: if is_light { "#d97706" } else { "#f59e0b" }.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ThemePresetMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub dark: ThemeSwatch,
    pub light: ThemeSwatch,
}

/// Declarative catalog of Zenko color themes.
/// Each theme family defines both Dark and Light palettes following strict 60/30/10 contrast.
pub const THEME_FAMILIES: &[ThemePresetMeta] = &[
    ThemePresetMeta {
        id: "zenko_vermilion",
        label: "Zenko Vermilion",
        dark: ThemeSwatch {
            bg: "#0a0a0b",
            surface: "#0f0f11",
            card: "#141417",
            border: "#222227",
            fg: "#f5f5f6",
            muted: "#8a8a93",
            primary: "#ea5829",
            primary_fg: "#ffffff",
        },
        light: ThemeSwatch {
            bg: "#fafafa",
            surface: "#f1f1f4",
            card: "#ffffff",
            border: "#e2e2e7",
            fg: "#0b0b0d",
            muted: "#6b6b76",
            primary: "#ea5829",
            primary_fg: "#ffffff",
        },
    },
    ThemePresetMeta {
        id: "pure_oled",
        label: "Pure OLED Flame",
        dark: ThemeSwatch {
            bg: "#000000",
            surface: "#08080a",
            card: "#0e0e11",
            border: "#1e1e24",
            fg: "#ffffff",
            muted: "#8e8e97",
            primary: "#ff5a26",
            primary_fg: "#ffffff",
        },
        light: ThemeSwatch {
            bg: "#ffffff",
            surface: "#f5f5f7",
            card: "#ffffff",
            border: "#e5e5ea",
            fg: "#050506",
            muted: "#6e6e77",
            primary: "#ff5a26",
            primary_fg: "#ffffff",
        },
    },
    ThemePresetMeta {
        id: "obsidian_zinc",
        label: "Obsidian Mono",
        dark: ThemeSwatch {
            bg: "#09090b",
            surface: "#0e0e11",
            card: "#141418",
            border: "#232329",
            fg: "#fafafa",
            muted: "#8e8e97",
            primary: "#e4e4e7",
            primary_fg: "#09090b",
        },
        light: ThemeSwatch {
            bg: "#f8fafc",
            surface: "#eef2f6",
            card: "#ffffff",
            border: "#dfe5ed",
            fg: "#0f172a",
            muted: "#64748b",
            primary: "#0f172a",
            primary_fg: "#ffffff",
        },
    },
    ThemePresetMeta {
        id: "warm_terracotta",
        label: "Warm Terracotta",
        dark: ThemeSwatch {
            bg: "#0e0c0a",
            surface: "#15120f",
            card: "#1c1814",
            border: "#2e2721",
            fg: "#f8f4ef",
            muted: "#9d9287",
            primary: "#d95b26",
            primary_fg: "#ffffff",
        },
        light: ThemeSwatch {
            bg: "#f7f5f0",
            surface: "#ede9e0",
            card: "#ffffff",
            border: "#dfdad0",
            fg: "#1c1917",
            muted: "#78716c",
            primary: "#c2410c",
            primary_fg: "#ffffff",
        },
    },
    ThemePresetMeta {
        id: "amber_forge",
        label: "Amber Forge",
        dark: ThemeSwatch {
            bg: "#0d0b08",
            surface: "#13100b",
            card: "#19150f",
            border: "#2c2419",
            fg: "#faf5eb",
            muted: "#a69882",
            primary: "#f59e0b",
            primary_fg: "#0d0b08",
        },
        light: ThemeSwatch {
            bg: "#fbf9f4",
            surface: "#f3eee3",
            card: "#ffffff",
            border: "#e5dccb",
            fg: "#1c160b",
            muted: "#7c6f59",
            primary: "#d97706",
            primary_fg: "#ffffff",
        },
    },
    ThemePresetMeta {
        id: "crimson_noir",
        label: "Crimson Noir",
        dark: ThemeSwatch {
            bg: "#0b090a",
            surface: "#120e10",
            card: "#171214",
            border: "#2a1f24",
            fg: "#fafafa",
            muted: "#a08b94",
            primary: "#e11d48",
            primary_fg: "#ffffff",
        },
        light: ThemeSwatch {
            bg: "#fbf8f9",
            surface: "#f4ecef",
            card: "#ffffff",
            border: "#e6d8de",
            fg: "#190c11",
            muted: "#7d646f",
            primary: "#e11d48",
            primary_fg: "#ffffff",
        },
    },
    ThemePresetMeta {
        id: "emerald_terminal",
        label: "Emerald Term",
        dark: ThemeSwatch {
            bg: "#070c0a",
            surface: "#0b120f",
            card: "#101915",
            border: "#1c2d25",
            fg: "#f0fdf4",
            muted: "#7f9c8d",
            primary: "#10b981",
            primary_fg: "#052016",
        },
        light: ThemeSwatch {
            bg: "#f6f8f6",
            surface: "#eaefea",
            card: "#ffffff",
            border: "#d9e2da",
            fg: "#101914",
            muted: "#5b6b61",
            primary: "#059669",
            primary_fg: "#ffffff",
        },
    },
    ThemePresetMeta {
        id: "midnight_indigo",
        label: "Midnight Slate",
        dark: ThemeSwatch {
            bg: "#0a0c14",
            surface: "#0f121d",
            card: "#141826",
            border: "#242a40",
            fg: "#f1f5f9",
            muted: "#8c96b0",
            primary: "#818cf8",
            primary_fg: "#0a0c14",
        },
        light: ThemeSwatch {
            bg: "#f7f8fc",
            surface: "#edf0f9",
            card: "#ffffff",
            border: "#dce1f1",
            fg: "#0f1426",
            muted: "#646d8a",
            primary: "#4f46e5",
            primary_fg: "#ffffff",
        },
    },
];

impl UiCustomization {
    pub fn builtin_presets() -> Vec<ThemePresetMeta> {
        THEME_FAMILIES.to_vec()
    }

    /// Resolve a preset identifier (including legacy split dark/light aliases) into a
    /// canonical family id and an optional implied mode override.
    pub fn resolve_preset_alias(id: &str) -> (&'static str, Option<ThemeMode>) {
        match id {
            "zenko_daylight" | "paper_daylight" => ("zenko_vermilion", Some(ThemeMode::Light)),
            "warm_alabaster" => ("warm_terracotta", Some(ThemeMode::Light)),
            "slate_daylight" => ("obsidian_zinc", Some(ThemeMode::Light)),
            "sage_daylight" => ("emerald_terminal", Some(ThemeMode::Light)),
            "rose_quartz" => ("crimson_noir", Some(ThemeMode::Dark)),
            "nordic_frost" => ("midnight_indigo", Some(ThemeMode::Dark)),
            other => {
                let family = THEME_FAMILIES
                    .iter()
                    .find(|f| f.id == other)
                    .unwrap_or(&THEME_FAMILIES[0]);
                (family.id, None)
            }
        }
    }

    pub fn preset_with_mode(id: &str, requested_mode: Option<ThemeMode>) -> Self {
        let (canonical_id, legacy_mode) = Self::resolve_preset_alias(id);
        let family = THEME_FAMILIES
            .iter()
            .find(|f| f.id == canonical_id)
            .unwrap_or(&THEME_FAMILIES[0]);
        let mode = requested_mode.or(legacy_mode).unwrap_or(ThemeMode::Dark);
        let swatch = match mode {
            ThemeMode::Light => family.light,
            ThemeMode::Dark | ThemeMode::System => family.dark,
        };

        Self {
            mode,
            preset_id: family.id.to_string(),
            colors: swatch.to_color_tokens(mode),
            buttons: ButtonCustomization::default(),
            icons: IconCustomization::default(),
            surface: SurfaceCustomization::default(),
            typography: TypographyConfig::default(),
            layout: LayoutCustomization::default(),
            motion: MotionCustomization::default(),
            widgets: WidgetToggles::default(),
            components: ComponentSlots::default(),
        }
    }

    pub fn preset(id: &str) -> Self {
        Self::preset_with_mode(id, None)
    }

    pub fn ui_font_stack(&self) -> &'static str {
        match self.typography.ui_font.as_str() {
            "geist" => "'Space Grotesk', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
            "jakarta" => "'Plus Jakarta Sans', 'Inter', ui-sans-serif, system-ui, sans-serif",
            "halden" => "'Halden', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
            "rondel" => "'Rondel', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
            "mallow" => "'Mallow', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
            "strata" => "'Strata', 'Plus Jakarta Sans', ui-sans-serif, system-ui, sans-serif",
            "vector" => "'Vector', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
            "folio" => "'Folio', ui-serif, Georgia, Cambria, serif",
            "meridian" => "'Meridian', ui-serif, Georgia, Cambria, serif",
            "ledger" => "'Ledger', ui-serif, Georgia, Cambria, serif",
            "opaline" => "'Opaline', ui-serif, Georgia, Cambria, serif",
            "vesper" => "'Vesper', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
            "orbit" => "'Orbit', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
            "diode" => "'Diode', 'Space Grotesk', ui-sans-serif, system-ui, sans-serif",
            "serif" => "ui-serif, Georgia, Cambria, 'Times New Roman', serif",
            "system" => "system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
            "obel_mono" | "mono" => "'Obel Mono', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
            _ => "'Plus Jakarta Sans', 'Inter', ui-sans-serif, system-ui, -apple-system, sans-serif",
        }
    }

    pub fn mono_font_stack(&self) -> &'static str {
        match self.typography.mono_font.as_str() {
            "obel_mono" => "'Obel Mono', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
            "signal" => "'Signal', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
            "tally" => "'Tally', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
            "fathom" => "'Fathom', 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace",
            "fira" => "'Fira Code', ui-monospace, SFMono-Regular, Menlo, monospace",
            "cascadia" => "'Cascadia Code', ui-monospace, SFMono-Regular, Menlo, monospace",
            "system" => "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
            _ => "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
        }
    }

    pub fn to_css_vars(&self) -> String {
        let radius = self.surface.radius_px.min(24);
        let radius_sm = radius.saturating_sub(4).max(2);
        let radius_lg = (radius + 4).min(28);
        let base_size = self.typography.base_size_px.clamp(12, 18);
        let icon_size = self.icons.size_px.clamp(12, 24);
        let icon_stroke = self.icons.stroke_width.clamp(1.0, 2.5);
        let (motion_ms, motion_ease) = match self.motion.preset {
            MotionPreset::Snappy => (140, "cubic-bezier(0.2, 0.8, 0.2, 1)"),
            MotionPreset::FluidSpring => (260, "cubic-bezier(0.34, 1.4, 0.64, 1)"),
            MotionPreset::Reduced => (80, "linear"),
            MotionPreset::Instant => (0, "linear"),
        };

        format!(
            "--bg:{bg};--surface:{surface};--card:{card};--border:{border};--fg:{fg};--muted:{muted};\
             --primary:{primary};--primary-fg:{primary_fg};--accent:{accent};\
             --destructive:{destructive};--success:{success};--warning:{warning};\
             --radius:{radius}px;--radius-sm:{radius_sm}px;--radius-lg:{radius_lg}px;\
             --font-sans:{font_sans};--font-mono:{font_mono};--text-base:{base_size}px;\
             --icon-size:{icon_size}px;--icon-stroke:{icon_stroke:.2};\
             --motion-ms:{motion_ms}ms;--motion-ease:{motion_ease};",
            bg = sanitize_css_value(&self.colors.background),
            surface = sanitize_css_value(&self.colors.surface),
            card = sanitize_css_value(&self.colors.card),
            border = sanitize_css_value(&self.colors.border),
            fg = sanitize_css_value(&self.colors.foreground),
            muted = sanitize_css_value(&self.colors.muted),
            primary = sanitize_css_value(&self.colors.primary),
            primary_fg = sanitize_css_value(&self.colors.primary_foreground),
            accent = sanitize_css_value(&self.colors.accent),
            destructive = sanitize_css_value(&self.colors.destructive),
            success = sanitize_css_value(&self.colors.success),
            warning = sanitize_css_value(&self.colors.warning),
            font_sans = self.ui_font_stack(),
            font_mono = self.mono_font_stack(),
        )
    }
}

fn sanitize_css_value(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    // Strictly allow only safe CSS color/value syntax; reject url(...), expression(...), @import, or escapes.
    let forbidden = lower.contains("url(")
        || lower.contains("expression(")
        || lower.contains("javascript:")
        || lower.contains("data:")
        || lower.contains("//")
        || lower.contains("/*");
    if forbidden {
        return String::from("transparent");
    }
    let cleaned: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '#' | '(' | ')' | '%' | '.' | ',' | ' ' | '-' | '/'))
        .collect();
    if cleaned.is_empty() {
        String::from("transparent")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_ui_customization_deserializes_with_beui_defaults() {
        let legacy_json = serde_json::json!({
            "mode": "dark",
            "preset_id": "zenko_vermilion",
            "colors": UiCustomization::default().colors,
            "surface": { "card_style": "bordered", "bg_pattern": "clean", "radius_px": 10 },
            "motion": { "preset": "snappy", "orb_style": "liquid_halo" },
            "components": {
                "chat_page": {
                    "background": "clean",
                    "content_width": "balanced",
                    "user_bubble": "soft_card",
                    "thinking_style": "collapsible_steps",
                    "tool_card_style": "pill_chips",
                    "show_avatars": true
                }
            }
        });

        let parsed: UiCustomization = serde_json::from_value(legacy_json).expect("legacy profile");
        assert_eq!(parsed.components.chat_page.scroller_style, "beui_rail");
        assert_eq!(parsed.components.chat_page.streaming_style, "beui_stream");
        assert_eq!(parsed.components.chat_page.approval_style, "aui_permission_grant");
        assert_eq!(parsed.components.chat_page.activity_style, "aui_tool_call_icon");
        assert_eq!(parsed.components.chat_page.loader_variant, "dot-matrix");
        assert_eq!(parsed.components.dashboard.layout, "bento_grid");
        assert_eq!(parsed.components.dashboard.heatmap_shape, "rounded");
        assert!(parsed.components.dashboard.show_heatmap);
        assert_eq!(parsed.surface.tooltip_style, "accent_edge");
        assert_eq!(parsed.surface.tooltip_shape, "pill");
        assert_eq!(parsed.surface.tooltip_motion, "spring");
        assert!(parsed.surface.show_hover_hints);
        assert_eq!(parsed.components.chat_page.loading_style, "beui_loader");
        assert_eq!(parsed.components.chat_page.message_actions_style, "aui_message_actions");
        assert_eq!(parsed.components.chat_page.diff_style, "aui_code_diff");
        assert_eq!(parsed.components.chat_page.shell_style, "aui_terminal_block");
        assert_eq!(parsed.components.chat_page.code_block_style, "aui_code_runner");
        assert_eq!(parsed.components.chat_page.skill_badge_style, "aui_memory_chips");
        assert_eq!(parsed.components.chat_page.agent_avatar_frame, "squircle");
        assert_eq!(parsed.components.chat_page.agent_avatar_color, "");
        assert_eq!(parsed.components.chat_page.user_avatar, "icon_user");
        assert_eq!(parsed.components.chat_page.user_avatar_frame, "circle");
        assert_eq!(parsed.components.chat_page.user_avatar_color, "");
        assert!(parsed.components.chat_page.show_day_separators);
        assert!(parsed.components.chat_page.show_citations);
        assert_eq!(parsed.motion.theme_transition, "circle-blur");
        assert!(parsed.motion.smooth_scroll);
        assert_eq!(parsed.surface.scrollbar_style, "accent_thin");
        assert_eq!(parsed.surface.tabs_variant, "pill");
        assert_eq!(parsed.surface.accordion_style, "bouncy");
        assert_eq!(parsed.surface.modal_style, "center_morph");
        assert_eq!(parsed.surface.popover_style, "gooey");
        assert_eq!(parsed.surface.element_chrome, "minimal");
        assert_eq!(parsed.surface.table_style, "borderless");
    }
}
