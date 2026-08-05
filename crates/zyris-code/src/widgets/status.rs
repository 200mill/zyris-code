//! The one line under the input field — mode and agent.
//!
//! **Connection status is not here.** There's no reason to always show "connected fine" in a screen
//! corner, so it was removed. What's happening right now is told by the `activity` line above the input.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::State;

use crate::theme;

pub fn draw(frame: &mut Frame, area: Rect, state: &State) {
    // Attach at the far left — aligns with the dot of the status line directly above.
    //
    // No spaces around the middle dot. Mode and agent are meant to be read as **one set**, so a gap
    // makes them look like two separate pieces of info — close together, the eye takes them in at once.
    let mut spans = vec![
        Span::styled(state.mode.label(state.lang), Style::default().fg(state.mode.color())),
        Span::styled("·", Style::default().fg(theme::BORDER_LIGHT)),
        Span::styled(
            if state.agent.is_empty() { "-" } else { state.agent.as_str() }.to_string(),
            Style::default().fg(theme::TEXT_MUTED),
        ),
    ];

    // **If there's unsent text, it must be said.** If it isn't announced, the user believes it was sent.
    if !state.queued.is_empty() {
        spans.push(Span::styled(" · ", Style::default().fg(theme::BORDER_LIGHT)));
        spans.push(Span::styled(
            state.lang.queued(state.queued.len()),
            Style::default().fg(theme::WARNING),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
