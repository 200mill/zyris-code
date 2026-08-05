//! Drawing. Widgets receive state like props and only draw — no logic is run here.
//!
//! ```text
//! │   (conversation)                    │ sidebar
//! │ ● working…                 Esc stop │ what's happening now
//! ├─────────────────────────────────────┤
//! │ > input                             │ input box (grows with content)
//! ├─────────────────────────────────────┤
//! │ base·Main Agent                     │ bottom bar
//! ```
//!
//! **The input box is clamped between lines above and below.** If a blank line separated them, the
//! status line would read as the box's heading and the bottom bar as its footer — the drawn lines show at a glance how far the box extends.
//!
//! **No blank lines.** One line was left between the conversation and the activity line, but it
//! only showed as unused space at the bottom of the screen. That one line goes to the conversation.
//!
//! There is no header. The app name and directory only need to be seen once, so there was no
//! reason for them to keep taking space — that is one more line for the conversation.

mod activity;
mod approve;
mod ask;
mod enroll;
mod input;
mod newproject;
mod picker;
mod sidebar;
mod status;
mod transcript;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

/// One column keeping text off the sidebar edge. Unlike the left margin (`rows::PAD`), no marker
/// stands here, so one column suffices.
const SIDE_GAP: u16 = 1;

use crate::app::State;

pub fn draw(frame: &mut Frame, state: &mut State) {
    let full = frame.area();

    // Carve the sidebar off to the right. On narrow screens it folds away — the conversation comes first.
    let show_side = state.sidebar_on && full.width > sidebar::WIDTH + 40;
    let (area, side) = if show_side {
        let cut = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(20), Constraint::Length(sidebar::WIDTH)])
            .split(full);
        // **The right margin is given across the whole left column, and only when the sidebar is up.**
        //
        // Carving out only the conversation leaves the input box and divider lines touching the sidebar
        // edge. The margin's purpose is "no text in the left column touches the boundary", so it is
        // given here in one place — the widgets below just use the narrowed width.
        // When the sidebar is folded there is nothing to touch, so the screen edge is used.
        //
        // **One column suffices.** The left margin (`rows::PAD`) is where the markers (`▌`·`●`·`▸`) stand, so
        // it needs two columns, but on the right we only need to keep text off the boundary, and one
        // column is enough. Two columns would narrow the conversation by that much.
        let body = Rect { width: cut[0].width.saturating_sub(SIDE_GAP), ..cut[0] };
        (body, Some(cut[1]))
    } else {
        (full, None)
    };
    // The input box grows with its content. It never exceeds half the screen.
    //
    // **There is only one input slot.** When a question is open the question takes it; when a tool wants to go
    // out, the approval window takes it — if both showed at once, a person couldn't tell where to answer.
    // Approval comes first: a tool is stopped waiting for an answer, and over there a deadline is running.
    let input_h = if state.pending.is_some() {
        approve::height(state, area.height.saturating_sub(3)).saturating_sub(1)
    } else {
        match &state.asking {
            Some((_, a)) => ask::height(a, area.height.saturating_sub(3)).saturating_sub(1),
            None => state
                .input
                .height(area.width.saturating_sub(2))
                .min((area.height / 2).max(1))
                .max(1),
        }
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),              // conversation
            Constraint::Length(1),           // what's happening now
            Constraint::Length(input_h + 1), // divider + input box
            Constraint::Length(1),           // divider
            Constraint::Length(1),           // bottom bar
        ])
        .split(area);

    transcript::draw(frame, chunks[0], state);
    activity::draw(frame, chunks[1], state);
    if state.pending.is_some() {
        // The approval window is not operated by clicking — the only answers are the three keys y·n·a.
        state.ask_area = None;
        approve::draw(frame, chunks[2], state);
    } else {
        match &state.asking {
            Some((_, a)) => {
                // Moving a click to a row requires knowing this area.
                state.ask_area = Some(chunks[2]);
                ask::draw(frame, chunks[2], a, state.lang);
            }
            None => {
                state.ask_area = None;
                input::draw(frame, chunks[2], state);
            }
        }
    }
    input::rule(frame, chunks[3]);
    status::draw(frame, chunks[4], state);

    if let Some(side) = side {
        sidebar::draw_divider(frame, Rect { width: 1, ..side });
        sidebar::draw(frame, side, state);
    }

    // The picker overlaps at the very top — while it is open, that is the current task.
    if let Some(p) = &state.picker {
        picker::draw(frame, full, p, state.lang);
    }

    // **The new-project form is laid on top of the picker.** The picker stays below, so pressing Esc
    // to close returns to the same spot.
    if let Some(form) = &state.new_project {
        newproject::draw(frame, full, form, state.lang);
    }

    // **The enrollment code window overlaps on top of that.** Nothing else may be done while viewing
    // the code — key handling also gives it top priority (`on_key`).
    if let Some(view) = &state.enroll {
        enroll::draw(frame, full, view, state.lang);
    }

    // **By default no background is painted** — the terminal uses its own. If the app painted, only the area
    // outside the grid (window padding, leftover pixels) would differ in color, making a band at the edges. See `theme::page_bg`.
    //
    // For someone who turned it back on, it is laid over all remaining cells. Every cell must have a background
    // so the ratatui diff clears trailing cells when full-width characters narrow.
    if let Some(bg) = crate::theme::page_bg() {
        for cell in frame.buffer_mut().content.iter_mut() {
            if cell.bg == ratatui::style::Color::Reset {
                cell.bg = bg;
            }
        }
    }

    // **Self-healing frame: force every cell out again.** Overwriting without clearing removes the ghost of
    // trailing cells behind full-width characters — there is no clear, so nothing flickers.
    // `AlwaysUpdate` bypasses the diff's equality comparison and puts every cell of this one frame on
    // the wire. The next draw returns to a fresh buffer (option `None`) and becomes a normal diff.
    if std::mem::take(&mut state.force_update) {
        use ratatui::buffer::CellDiffOption;
        for cell in frame.buffer_mut().content.iter_mut() {
            cell.set_diff_option(CellDiffOption::AlwaysUpdate);
        }
    }
}

/// What appears on the activity line. It takes a time so tests can pin the elapsed time and inspect.
pub fn activity_parts_at(
    state: &State,
    now: std::time::Instant,
) -> (ratatui::style::Color, String, &'static str) {
    activity::parts_at(state, now)
}

/// Which row this y coordinate is on in the question screen. Used by click handling.
pub fn ask_row_at(
    a: &crate::question::Answering,
    area: ratatui::layout::Rect,
    y: u16,
) -> Option<usize> {
    ask::row_at(a, area, y)
}
