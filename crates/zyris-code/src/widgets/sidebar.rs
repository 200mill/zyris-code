//! The right sidebar — usage on top, tasks below.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::State;
use crate::markdown::display_width;
use crate::sidebar::compact;
use crate::theme;

/// Sidebar width. Narrower truncates numbers; wider squeezes the conversation.
pub const WIDTH: u16 = 30;

pub fn draw(frame: &mut Frame, area: Rect, state: &State) {
    let inner = Rect { x: area.x + 2, width: area.width.saturating_sub(2), ..area };
    let mut lines: Vec<Line<'static>> = Vec::new();

    // **Put where the tools run at the very top.** Which repo a tool line's `src/app.rs`
    // belongs to is only knowable from this. No label is added — a path reads as a path.
    let here = short_path(&state.cwd, inner.width as usize);
    if !here.is_empty() {
        lines.push(Line::from(Span::styled(here, Style::default().fg(theme::TEXT_MUTED))));
        lines.push(Line::from(""));
    }

    lines.push(head(state.lang.usage()));
    let u = &state.sidebar.usage;
    // **Align the values' left edges.** Label lengths differ (credits 6 cells, context 8), so
    // just appending would stair-step the numbers and make them hard to compare at a glance.
    let rows = [
        (state.lang.credits(), u.credits_used.clone().unwrap_or_else(|| "-".into())),
        (state.lang.context(), context_text(u)),
        (state.lang.total_tokens(), u.total_tokens.map(compact).unwrap_or_else(|| "-".into())),
    ];
    let col = rows.iter().map(|(k, _)| display_width(k)).max().unwrap_or(0) + 2;
    for (key, value) in rows {
        lines.push(kv(key, value, col));
    }
    // **The shells section opens only when there are shells.** In a narrow sidebar an empty section wastes space.
    if !state.shells.is_empty() {
        lines.push(Line::from(""));
        lines.push(head(state.lang.shells()));
        for shell in &state.shells {
            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(theme::SUCCESS)),
                Span::styled(
                    truncate(&shell.name, inner.width.saturating_sub(2) as usize),
                    Style::default().fg(theme::TEXT),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(head(state.lang.tasks()));

    // Show only as many tasks as the leftover height fits.
    let room = (inner.height as usize).saturating_sub(lines.len()).max(1);
    let tasks = state.sidebar.visible_tasks(room);
    if tasks.is_empty() {
        lines.push(Line::from(Span::styled(
            state.lang.none(),
            Style::default().fg(theme::TEXT_MUTED),
        )));
    }
    for t in tasks {
        let colour = match t.state {
            crate::sidebar::TaskState::Done => theme::TEXT_MUTED,
            crate::sidebar::TaskState::Running => theme::ACCENT,
            crate::sidebar::TaskState::Pending => theme::TEXT,
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", t.state.mark()), Style::default().fg(colour)),
            Span::styled(
                truncate(&t.text, inner.width.saturating_sub(2) as usize),
                Style::default().fg(colour),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// The vertical divider standing between the conversation area and the sidebar.
pub fn draw_divider(frame: &mut Frame, area: Rect) {
    let lines: Vec<Line<'static>> = (0..area.height)
        .map(|_| Line::from(Span::styled("│", Style::default().fg(theme::BORDER))))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn head(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
    ))
}

/// Context shows as **used / capacity**. A single number cannot tell whether it is comfortable
/// or full. For a model with unknown limits, only the used amount is written.
fn context_text(u: &crate::sidebar::Usage) -> String {
    let Some(used) = u.context_tokens else { return "-".into() };
    match crate::sidebar::context_limit(u.model.as_deref()) {
        Some(max) => format!("{} / {}", compact(used), compact(max)),
        None => compact(used),
    }
}

/// Pads the label to `col` cells to align the values' left edges. Full-width glyphs mean cells, not characters.
fn kv(key: &str, value: String, col: usize) -> Line<'static> {
    let pad = col.saturating_sub(display_width(key));
    Line::from(vec![
        Span::styled(format!("{key}{}", " ".repeat(pad)), Style::default().fg(theme::TEXT_MUTED)),
        Span::styled(value, Style::default().fg(theme::TEXT)),
    ])
}

/// Fits a path into the narrow sidebar.
///
/// Keep it as-is if it fits; if it overflows, keep **only the last two pieces** — cutting the
/// front would remove the final name that tells which repo it is, first. That is the side people need.
fn short_path(p: &std::path::Path, limit: usize) -> String {
    let full = p.to_string_lossy();
    if display_width(&full) <= limit {
        return full.into_owned();
    }
    let tail: Vec<String> = p
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let joined = tail.into_iter().rev().collect::<Vec<_>>().join("/");
    truncate(&format!("…/{joined}"), limit)
}

fn truncate(s: &str, limit: usize) -> String {
    if display_width(s) <= limit {
        return s.to_string();
    }
    let mut out = String::new();
    for ch in s.chars() {
        if display_width(&out) + display_width(&ch.to_string()) > limit.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}
