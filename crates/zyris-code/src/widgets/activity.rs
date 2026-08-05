//! The one line right above the input — **what is happening right now.**
//!
//! This beats keeping a connection status pinned in a corner of the screen. What people want
//! to know is not "am I connected" but "is it my turn, or do I wait". The connection is only
//! mentioned when it drops — no reason to keep announcing what works.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::State;
use crate::markdown::display_width;
use crate::theme;

/// Whether the dot is lit. Flips every eight frames (0.4s) at 20fps.
///
/// The blink is set by frame count rather than a clock so that **tests do not have to wait
/// on time**. The drawing side stays pure.
pub fn blink_on(tick: u64) -> bool {
    (tick / 8).is_multiple_of(2)
}

/// What appears on this line: (dot color, text, hint). Pure — tests look at this.
pub fn parts(state: &State) -> (ratatui::style::Color, String, &'static str) {
    parts_at(state, std::time::Instant::now())
}

/// The variant that takes a time. Tests set the elapsed time and inspect it.
pub fn parts_at(
    state: &State,
    now: std::time::Instant,
) -> (ratatui::style::Color, String, &'static str) {
    let lang = state.lang;
    // What you need to know now goes on top. The quit notice comes before anything else.
    if state.quit_pending() {
        return (theme::WARNING, lang.quit_armed().to_string(), "");
    }
    // Notices disappear on their own after a while — `State::status` makes that call.
    if let Some(s) = state.status() {
        return (theme::WARNING, s.to_string(), "");
    }
    if !state.connected {
        return (theme::WARNING, lang.connecting().to_string(), "");
    }
    // **More specific than "working…".** A command gives its result once, when done, so unless
    // we say here what is running, people wait up to 55 seconds blind.
    // **Saying you asked to stop comes first.** Until the server answers, "working" stays up,
    // and while it keeps showing, people think Ctrl+C did not work and press again.
    if state.running && state.stopping {
        return (theme::WARNING, lang.stopping().to_string(), lang.ctrl_c_quits());
    }
    if let Some((_, command, since)) = &state.running_exec {
        let secs = now.saturating_duration_since(*since).as_secs();
        return (theme::ACCENT, lang.running_command(command, secs), lang.esc_stops());
    }
    if state.running {
        return (theme::ACCENT, lang.working().to_string(), lang.esc_stops());
    }
    if state.asking.is_some() {
        return (
            theme::WARNING,
            lang.waiting_answer().to_string(),
            lang.waiting_answer_hint(),
        );
    }
    // No hint when idle. An always-on hint stops getting read.
    (theme::TEXT_MUTED, lang.idle().to_string(), "")
}

pub fn draw(frame: &mut Frame, area: Rect, state: &State) {
    let (colour, label, hint) = parts(state);

    // The dot blinks only while working. A still dot does not say "it is running".
    let lit = !state.running || blink_on(state.tick);
    let dot = Style::default().fg(if lit { colour } else { theme::BORDER_LIGHT });

    // **The dot goes at the far left.** It does not align with the conversation's margin —
    // this line is not the conversation but the screen's own status, and at the left edge the eye always finds it in the same place.
    let mut spans = vec![
        Span::styled("● ", dot),
        Span::styled(label.clone(), Style::default().fg(colour).add_modifier(Modifier::BOLD)),
    ];

    // The hint goes at the right edge. If narrow, drop it entirely — the status comes first.
    // The sidebar-side margin is not subtracted again here because this area already arrives narrowed.
    let used = 2 + display_width(&label);
    let room = area.width as usize;
    if !hint.is_empty() && used + display_width(hint) + 2 <= room {
        let gap = room - used - display_width(hint);
        spans.push(Span::styled(" ".repeat(gap), Style::default().fg(theme::TEXT)));
        spans.push(Span::styled(hint, Style::default().fg(theme::BORDER_LIGHT)));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
