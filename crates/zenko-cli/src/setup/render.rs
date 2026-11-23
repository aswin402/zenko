use crate::setup::state::{ScreenView, SetupMachine};
use crate::setup::theme::SetupTheme;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap};

pub fn draw_setup(frame: &mut Frame, machine: &SetupMachine, theme: &SetupTheme) {
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.base()), area);

    // Centered minimal container (max 74 cols wide for calm, editorial proportions)
    let container = centered_box(area, 76, 24);
    frame.render_widget(Clear, container);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Brand + Breadcrumb header
            Constraint::Min(10),   // Main Menu or Step Input
            Constraint::Length(2), // Optional status notice
            Constraint::Length(2), // Minimal keybind footer
        ])
        .split(container);

    render_header(frame, chunks[0], machine, theme);

    match machine.current_view() {
        ScreenView::Menu {
            title,
            subtitle,
            cursor,
            items,
        } => {
            render_menu(frame, chunks[1], &title, &subtitle, cursor, &items, theme);
        }
        ScreenView::Input(prompt) => {
            render_input(frame, chunks[1], &prompt, theme);
        }
    }

    render_notice(frame, chunks[2], machine.draft.status_notice.as_deref(), theme);
    render_footer(frame, chunks[3], machine.is_input_mode(), theme);
}

fn render_header(frame: &mut Frame, area: Rect, machine: &SetupMachine, theme: &SetupTheme) {
    let crumbs = machine.breadcrumbs();
    let mut spans = vec![
        Span::styled("▲ ", theme.accent_bold()),
        Span::styled(
            "Zenko",
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", theme.muted_style()),
    ];

    for (idx, crumb) in crumbs.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" › ", theme.muted_style()));
        }
        let style = match idx + 1 == crumbs.len() {
            true => Style::default().fg(theme.accent),
            false => theme.muted_style(),
        };
        spans.push(Span::styled(crumb.clone(), style));
    }

    let header = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.border_style())
            .padding(Padding::new(1, 1, 1, 0)),
    );
    frame.render_widget(header, area);
}

fn render_menu(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    subtitle: &str,
    cursor: usize,
    items: &[crate::setup::state::MenuEntry],
    theme: &SetupTheme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style())
        .style(theme.card())
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(4)])
        .split(inner);

    let heading = Paragraph::new(vec![
        Line::from(Span::styled(
            title,
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(subtitle, theme.muted_style())),
    ]);
    frame.render_widget(heading, sections[0]);

    let inner_w = sections[1].width as usize;
    let visible_items = ((sections[1].height as usize) / 2).max(1);
    let scroll_start = if items.len() <= visible_items {
        0
    } else {
        let half = visible_items / 2;
        cursor
            .saturating_sub(half)
            .min(items.len().saturating_sub(visible_items))
    };
    let scroll_end = (scroll_start + visible_items).min(items.len());
    let mut lines = Vec::with_capacity((scroll_end - scroll_start) * 2);

    for (idx, item) in items.iter().enumerate().skip(scroll_start).take(scroll_end - scroll_start) {
        let selected = idx == cursor;
        let pointer = match selected {
            true => Span::styled("▸ ", theme.accent_bold()),
            false => Span::styled("  ", theme.muted_style()),
        };
        let label_style = match selected {
            true => theme.selected_row(),
            false => theme.normal_row(),
        };
        let badge_style = match (selected, item.badge_active) {
            (_, true) => Style::default().fg(theme.accent),
            (true, false) => Style::default().fg(theme.fg),
            (false, false) => theme.muted_style(),
        };

        let left_len = 2 + item.label.chars().count();
        let right_len = item.badge.chars().count();
        let pad = inner_w.saturating_sub(left_len + right_len + 1).max(2);

        lines.push(Line::from(vec![
            pointer,
            Span::styled(item.label.clone(), label_style),
            Span::raw(" ".repeat(pad)),
            Span::styled(item.badge.clone(), badge_style),
        ]));

        let detail_style = match selected {
            true => Style::default().fg(theme.muted),
            false => Style::default().fg(theme.border),
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(item.detail.clone(), detail_style),
        ]));
    }

    let list_para = Paragraph::new(lines);
    frame.render_widget(list_para, sections[1]);
}

fn render_input(
    frame: &mut Frame,
    area: Rect,
    prompt: &crate::setup::state::InputPromptView,
    theme: &SetupTheme,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style())
        .style(theme.card())
        .padding(Padding::new(2, 2, 1, 1));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Step label
            Constraint::Length(2), // Title
            Constraint::Length(2), // Subtitle
            Constraint::Length(3), // Input Box
            Constraint::Min(1),    // Hint
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Span::styled(prompt.step_label.clone(), theme.accent_bold())),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            prompt.title.clone(),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        )),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(prompt.subtitle.clone(), theme.muted_style()))
            .wrap(Wrap { trim: true }),
        rows[2],
    );

    let shown_value = match (prompt.value.is_empty(), prompt.masked) {
        (true, _) => Span::styled(prompt.placeholder.clone(), theme.muted_style()),
        (false, true) => {
            let len = prompt.value.chars().count();
            let dots = "•".repeat(len.min(36));
            Span::styled(format!("{dots}▌"), Style::default().fg(theme.fg))
        }
        (false, false) => Span::styled(format!("{}▌", prompt.value), Style::default().fg(theme.fg)),
    };

    let input_box = Paragraph::new(Line::from(shown_value)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.active_border_style())
            .style(Style::default().bg(theme.bg))
            .padding(Padding::horizontal(1)),
    );
    frame.render_widget(input_box, rows[3]);

    let hint = Paragraph::new(Line::from(vec![
        Span::styled("Enter", theme.accent_bold()),
        Span::styled(" confirm & next   ", theme.muted_style()),
        Span::styled("Esc", theme.accent_bold()),
        Span::styled(" go back   ", theme.muted_style()),
        Span::styled("Ctrl+V", theme.accent_bold()),
        Span::styled(" paste", theme.muted_style()),
    ]));
    frame.render_widget(hint, rows[4]);
}

fn render_notice(frame: &mut Frame, area: Rect, notice: Option<&str>, theme: &SetupTheme) {
    if let Some(msg) = notice {
        let p = Paragraph::new(Line::from(vec![
            Span::styled("✓ ", Style::default().fg(theme.success)),
            Span::styled(msg.to_string(), Style::default().fg(theme.fg)),
        ]))
        .alignment(Alignment::Center);
        frame.render_widget(p, area);
    }
}

fn render_footer(frame: &mut Frame, area: Rect, is_input: bool, theme: &SetupTheme) {
    let line = match is_input {
        true => Line::from(vec![
            Span::styled("Type / Paste", theme.normal_row()),
            Span::styled(" · ", theme.muted_style()),
            Span::styled("Enter", theme.accent_bold()),
            Span::styled(" Next · ", theme.muted_style()),
            Span::styled("Esc", theme.accent_bold()),
            Span::styled(" Go Back", theme.muted_style()),
        ]),
        false => Line::from(vec![
            Span::styled("↑/↓", theme.accent_bold()),
            Span::styled(" Navigate · ", theme.muted_style()),
            Span::styled("Enter", theme.accent_bold()),
            Span::styled(" Select · ", theme.muted_style()),
            Span::styled("Esc", theme.accent_bold()),
            Span::styled(" Go Back · ", theme.muted_style()),
            Span::styled("q", theme.accent_bold()),
            Span::styled(" Quit", theme.muted_style()),
        ]),
    };
    let footer = Paragraph::new(line).alignment(Alignment::Center);
    frame.render_widget(footer, area);
}

fn centered_box(area: Rect, max_w: u16, max_h: u16) -> Rect {
    let w = area.width.min(max_w);
    let h = area.height.min(max_h);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect::new(x, y, w, h)
}
