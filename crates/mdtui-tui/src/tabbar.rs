use mdtui_render::Theme;
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};
use unicode_width::UnicodeWidthStr;

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    theme: &Theme,
    tabs: &[&str],
    active: usize,
    active_fill_bg: Color,
) {
    if area.width == 0 || area.height == 0 || tabs.is_empty() {
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(area, Style::default().bg(color(theme.bg)));

    let top_y = area.y;
    let label_y = area.y + if area.height > 1 { 1 } else { 0 };
    let join_y = area.y + area.height.saturating_sub(1);
    let has_roof = label_y > top_y;
    let has_join = area.height >= 3;

    let mut x = area.x;
    let right = area.x.saturating_add(area.width);

    if has_join {
        let underline = Style::default().fg(color(theme.accent)).bg(color(theme.bg));
        for col in area.x..right {
            buf.set_string(col, join_y, "─", underline);
        }
    }

    for (idx, label) in tabs.iter().enumerate() {
        if x >= right {
            break;
        }
        let is_active = idx == active;
        let border_style = Style::default()
            .fg(if is_active {
                color(theme.accent)
            } else {
                color(theme.line)
            })
            .bg(if is_active {
                active_fill_bg
            } else {
                color(theme.bg)
            });
        let base = if is_active {
            Style::default()
                .fg(color(theme.fg))
                .bg(active_fill_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(color(theme.fg_dim))
                .bg(color(theme.bg_soft))
        };
        let num_style = Style::default().fg(color(theme.fg_mute)).bg(if is_active {
            active_fill_bg
        } else {
            color(theme.bg_soft)
        });

        let sup = superscript_for(idx);
        let name_w = UnicodeWidthStr::width(*label) as u16;
        let content_width = 1 + name_w + 1 + 1;
        let tab_width = content_width + 2;
        let tab_x = x;

        if has_roof {
            let mut roof_x = tab_x;
            roof_x = put(buf, roof_x, top_y, right, "╭", border_style);
            if tab_width > 2 {
                roof_x = put(
                    buf,
                    roof_x,
                    top_y,
                    right,
                    &"─".repeat(tab_width.saturating_sub(2) as usize),
                    border_style,
                );
            }
            put(buf, roof_x, top_y, right, "╮", border_style);
        }

        let mut label_x = tab_x;
        label_x = put(buf, label_x, label_y, right, "│", border_style);
        label_x = put(buf, label_x, label_y, right, " ", base);
        label_x = put(buf, label_x, label_y, right, label, base);
        label_x = put(buf, label_x, label_y, right, sup, num_style);
        label_x = put(buf, label_x, label_y, right, " ", base);
        put(buf, label_x, label_y, right, "│", border_style);

        if is_active && has_join {
            let accent = Style::default().fg(color(theme.accent)).bg(active_fill_bg);
            let interior = Style::default().bg(active_fill_bg);
            put(buf, tab_x, join_y, right, "╯", accent);
            if tab_width > 2 {
                put(
                    buf,
                    tab_x.saturating_add(1),
                    join_y,
                    right,
                    &" ".repeat(tab_width.saturating_sub(2) as usize),
                    interior,
                );
            }
            put(
                buf,
                tab_x.saturating_add(tab_width.saturating_sub(1)),
                join_y,
                right,
                "╰",
                accent,
            );
        }

        x = tab_x.saturating_add(tab_width).min(right);
    }
}

pub fn hit_test(area: Rect, tabs: &[&str], x: u16, y: u16) -> Option<usize> {
    if area.width == 0
        || area.height == 0
        || y < area.y
        || y >= area.y.saturating_add(area.height)
        || x < area.x
        || x >= area.x.saturating_add(area.width)
    {
        return None;
    }
    let mut cursor = area.x;
    let right = area.x.saturating_add(area.width);
    for (idx, label) in tabs.iter().enumerate() {
        if cursor >= right {
            break;
        }
        let name_w = UnicodeWidthStr::width(*label) as u16;
        let tab_width = name_w + 5;
        let tab_end = cursor.saturating_add(tab_width).min(right);
        if x >= cursor && x < tab_end {
            return Some(idx);
        }
        cursor = tab_end;
    }
    None
}

fn superscript_for(idx: usize) -> &'static str {
    match idx {
        0 => "¹",
        1 => "²",
        2 => "³",
        3 => "⁴",
        4 => "⁵",
        5 => "⁶",
        6 => "⁷",
        7 => "⁸",
        8 => "⁹",
        _ => " ",
    }
}

fn put(buf: &mut Buffer, mut x: u16, y: u16, right: u16, text: &str, style: Style) -> u16 {
    for grapheme in unicode_segmentation::UnicodeSegmentation::graphemes(text, true) {
        if x >= right {
            break;
        }
        let width = UnicodeWidthStr::width(grapheme) as u16;
        if width == 0 {
            continue;
        }
        buf.set_string(x, y, grapheme, style);
        x = x.saturating_add(width);
    }
    x
}

fn color(rgb: mdtui_render::Rgb) -> Color {
    Color::Rgb(rgb.0, rgb.1, rgb.2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_buffer(active: usize, active_fill_bg: Color) -> Buffer {
        let backend = TestBackend::new(32, 3);
        let mut terminal =
            Terminal::new(backend).unwrap_or_else(|error| panic!("terminal init failed: {error}"));
        let theme = Theme::ghostty_default_dark();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    Rect {
                        x: 0,
                        y: 0,
                        width: 32,
                        height: 3,
                    },
                    &theme,
                    &["README.md", "roadmap.md", "spec.md"],
                    active,
                    active_fill_bg,
                );
            })
            .unwrap_or_else(|error| panic!("tabbar draw failed: {error}"));
        terminal.backend().buffer().clone()
    }

    #[test]
    fn active_tab_border_cells_use_active_fill_background() {
        let theme = Theme::ghostty_default_dark();
        let buffer = render_buffer(0, color(theme.bg_raised));

        assert_eq!(buffer[(0, 0)].bg, color(theme.bg_raised));
        assert_eq!(buffer[(0, 1)].bg, color(theme.bg_raised));
        assert_eq!(buffer[(0, 2)].bg, color(theme.bg_raised));
    }

    #[test]
    fn inactive_tab_border_cells_stay_on_base_background() {
        let theme = Theme::ghostty_default_dark();
        let buffer = render_buffer(0, color(theme.bg_raised));
        let inactive_x = UnicodeWidthStr::width("README.md") as u16 + 5;

        assert_eq!(buffer[(inactive_x, 0)].bg, color(theme.bg));
        assert_eq!(buffer[(inactive_x, 1)].bg, color(theme.bg));
    }
}
