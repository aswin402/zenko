use ratatui::style::{Color, Modifier, Style};
use zenko_core::{DEFAULT_THEME_PRESET, UiCustomization, parse_hex_rgb};

/// TUI color palette derived dynamically from Zenko's active default theme preset
/// (`zenko_vermilion` — Obsidian Black, Burnt Vermilion Red-Orange, Pure Crisp White).
#[derive(Debug, Clone, Copy)]
pub struct SetupTheme {
    pub bg: Color,
    pub surface: Color,
    pub border: Color,
    pub fg: Color,
    pub muted: Color,
    pub accent: Color,
    pub success: Color,
}

impl SetupTheme {
    pub fn from_customization(ui: &UiCustomization) -> Self {
        let c = &ui.colors;
        Self {
            bg: rgb_from_hex(&c.background, (10, 10, 11)),
            surface: rgb_from_hex(&c.surface, (14, 14, 16)),
            border: rgb_from_hex(&c.border, (31, 31, 36)),
            fg: rgb_from_hex(&c.foreground, (245, 245, 246)),
            muted: rgb_from_hex(&c.muted, (138, 138, 147)),
            accent: rgb_from_hex(&c.accent, (234, 88, 41)),
            success: rgb_from_hex(&c.success, (34, 197, 94)),
        }
    }

    pub fn default_zenko() -> Self {
        let ui = UiCustomization::preset(DEFAULT_THEME_PRESET);
        Self::from_customization(&ui)
    }

    pub fn base(&self) -> Style {
        Style::default().bg(self.bg).fg(self.fg)
    }

    pub fn card(&self) -> Style {
        Style::default().bg(self.surface).fg(self.fg)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn active_border_style(&self) -> Style {
        Style::default().fg(self.accent)
    }

    pub fn selected_row(&self) -> Style {
        Style::default()
            .bg(self.surface)
            .fg(self.accent)
            .add_modifier(Modifier::BOLD)
    }

    pub fn normal_row(&self) -> Style {
        Style::default().fg(self.fg)
    }

    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn accent_bold(&self) -> Style {
        Style::default().fg(self.accent).add_modifier(Modifier::BOLD)
    }
}

fn rgb_from_hex(hex: &str, fallback: (u8, u8, u8)) -> Color {
    let (r, g, b) = parse_hex_rgb(hex).unwrap_or(fallback);
    Color::Rgb(r, g, b)
}
