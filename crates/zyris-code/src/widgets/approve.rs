//! Approval window — **only appears when going outside the working directory.**
//!
//! Takes the input field's spot. Same place as the question screen — if the two overlapped, nobody
//! could tell where to answer, so approval comes first; one tool is stalled waiting and there's a deadline.
//!
//! **What it asks is not "what are you doing" but "where are you touching".** Inside work asks
//! nothing, so the very fact this window appeared means "this is outside". That's why the middle of
//! the screen is taken not by the tool name but by **that path**.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{State, ToolAsk};
use crate::theme;

/// Header, path, blank line, key hints. These four are always there.
const CHROME: u16 = 4;

pub fn height(state: &State, cap: u16) -> u16 {
    let Some(ask) = &state.pending else { return 0 };
    // If the deadline passed, say so in two lines — crammed into one, it gets cut off on narrow screens.
    let want = CHROME + 2 * ask.expired as u16 + !state.ask_queue.is_empty() as u16;
    want.min(cap.max(3))
}

pub fn draw(frame: &mut Frame, area: Rect, state: &State) {
    let Some(ask) = &state.pending else { return };
    let mut lines = head(ask, state.lang);

    lines.push(Line::from(vec![
        Span::styled("  ", Style::default().fg(theme::TEXT)),
        Span::styled(
            ask.summary.clone(),
            Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default().fg(theme::TEXT)),
        Span::styled(
            format!("{} · ", short(&ask.call.capability, &ask.call.tool)),
            Style::default().fg(theme::TOOL),
        ),
        Span::styled(
            state.lang.approve_root(&state.cwd.display().to_string()),
            Style::default().fg(theme::TEXT_MUTED),
        ),
    ]));

    if !state.ask_queue.is_empty() {
        lines.push(Line::from(Span::styled(
            state.lang.approve_more_waiting(state.ask_queue.len()),
            Style::default().fg(theme::TEXT_MUTED),
        )));
    }
    lines.push(keys(state.lang));
    frame.render_widget(Paragraph::new(lines), area);
}

/// Header. If the deadline passed, **don't dismiss the window, just change what it says** —
/// dismissing it would leave the person not knowing what they missed.
fn head(ask: &ToolAsk, lang: crate::lang::Lang) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        lang.approve_head(),
        Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD),
    ))];
    if ask.expired {
        out.push(Line::from(Span::styled(
            lang.approve_gave_up(),
            Style::default().fg(theme::TEXT_MUTED),
        )));
        out.push(Line::from(Span::styled(
            lang.approve_next_time(),
            Style::default().fg(theme::TEXT_MUTED),
        )));
    }
    out
}

fn keys(lang: crate::lang::Lang) -> Line<'static> {
    Line::from(Span::styled(lang.approve_keys(), Style::default().fg(theme::BORDER_LIGHT)))
}

/// The tool name shown on screen. Same shape as the tool line in the conversation.
fn short(capability: &str, tool: &str) -> String {
    format!("{capability}.{tool}")
}
