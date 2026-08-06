//! Text selection anywhere on the screen.
//!
//! Coordinates are **(row, screen column)**. A column is cells, not characters, so a full-width
//! glyph takes 2 — because the mouse reports in cells. Handling characters instead would skew
//! the selection on lines that contain Hangul.

use crate::markdown::display_width;

/// The range being dragged. The start and the current point; their order can be reversed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drag {
    pub from: (usize, usize),
    pub to: (usize, usize),
}

impl Drag {
    pub fn new(at: (usize, usize)) -> Self {
        Self { from: at, to: at }
    }

    /// (Start, end) sorted top-to-bottom.
    pub fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.from <= self.to {
            (self.from, self.to)
        } else {
            (self.to, self.from)
        }
    }

    /// Has it not moved a single cell? Then it is a click, not a selection.
    pub fn is_click(&self) -> bool {
        self.from == self.to
    }
}

/// Extracts the selected text. Lines are joined with `\n`.
pub fn extract(rows: &[String], drag: &Drag) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let ((r0, c0), (r1, c1)) = drag.ordered();
    let last = rows.len() - 1;
    let (r0, r1) = (r0.min(last), r1.min(last));

    if r0 == r1 {
        return slice_cols(&rows[r0], c0.min(c1), c0.max(c1));
    }

    let mut out = vec![slice_cols(&rows[r0], c0, usize::MAX)];
    for row in &rows[r0 + 1..r1] {
        out.push(row.trim_end().to_string());
    }
    out.push(slice_cols(&rows[r1], 0, c1));
    out.join("\n")
}

/// The screen rectangle `(top, left, bottom, right)` the drag covers, clipped to the
/// terminal size. `None` when the drag never moved — that is a click, not a selection.
pub fn rect(drag: &Drag, width: u16, height: u16) -> Option<(u16, u16, u16, u16)> {
    if drag.is_click() {
        return None;
    }
    let ((r0, c0), (r1, c1)) = drag.ordered();
    let h = height as usize;
    let w = width as usize;
    let (r0, r1) = (r0.min(h.saturating_sub(1)), r1.min(h.saturating_sub(1)));
    let (c0, c1) = (c0.min(w.saturating_sub(1)), c1.min(w.saturating_sub(1)));
    Some((r0 as u16, c0 as u16, r1 as u16, c1 as u16))
}

/// The characters in screen columns `[from, to)`. Full-width glyphs count as 2 cells.
fn slice_cols(row: &str, from: usize, to: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for ch in row.chars() {
        let w = display_width(&ch.to_string()).max(1);
        if col >= to {
            break;
        }
        if col >= from {
            out.push(ch);
        }
        col += w;
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<String> {
        vec![
            "안녕하세요 반갑습니다".to_string(),
            "second line".to_string(),
            "세 번째 줄".to_string(),
        ]
    }

    #[test]
    fn a_drag_that_never_moved_is_a_click() {
        assert!(Drag::new((2, 3)).is_click());
        let mut d = Drag::new((2, 3));
        d.to = (2, 4);
        assert!(!d.is_click());
    }

    /// Selecting within one line yields just that span. Full-width is 2 cells, so column 4 is the third character.
    #[test]
    fn selecting_within_one_line_takes_that_span() {
        let d = Drag { from: (0, 0), to: (0, 4) };
        assert_eq!(extract(&rows(), &d), "안녕");
    }

    #[test]
    fn selecting_backwards_gives_the_same_text() {
        let forward = Drag { from: (0, 0), to: (0, 4) };
        let backward = Drag { from: (0, 4), to: (0, 0) };
        assert_eq!(extract(&rows(), &forward), extract(&rows(), &backward));
    }

    /// Selecting across lines keeps the middle rows whole and clips the two ends.
    #[test]
    fn selecting_across_lines_keeps_the_middle_whole() {
        let d = Drag { from: (0, 10), to: (2, 4) };
        let got = extract(&rows(), &d);
        let lines: Vec<&str> = got.lines().collect();
        assert_eq!(lines.len(), 3, "{got:?}");
        assert_eq!(lines[1], "second line", "a middle line must be selected whole");
        // That glyph is full-width (columns 3–4), so dragging to column 4 covers it and includes it.
        assert_eq!(lines[2], "세 번", "the last line runs to the glyph covering column 4");
    }

    /// Pointing off-screen must not crash — the mouse can go anywhere.
    #[test]
    fn dragging_past_the_end_is_clamped() {
        let d = Drag { from: (0, 0), to: (99, 999) };
        let got = extract(&rows(), &d);
        assert!(got.ends_with("세 번째 줄"), "{got:?}");
    }

    /// A drag that moved covers the rectangle between the two points, in either direction.
    #[test]
    fn rect_covers_the_drag_either_way_around() {
        let down = Drag { from: (1, 3), to: (3, 7) };
        let up = Drag { from: (3, 7), to: (1, 3) };
        assert_eq!(rect(&down, 80, 24), Some((1, 3, 3, 7)));
        assert_eq!(rect(&up, 80, 24), Some((1, 3, 3, 7)));
    }

    /// A drag that never moved is a click and highlights nothing.
    #[test]
    fn rect_is_none_for_a_click() {
        assert_eq!(rect(&Drag::new((2, 3)), 80, 24), None);
    }

    /// The mouse can go past the edge of the terminal; the rectangle is clamped to it.
    #[test]
    fn rect_is_clamped_to_the_terminal() {
        let d = Drag { from: (0, 0), to: (99, 999) };
        assert_eq!(rect(&d, 80, 24), Some((0, 0, 23, 79)));
    }

    #[test]
    fn selecting_nothing_gives_an_empty_string() {
        assert_eq!(extract(&[], &Drag::new((0, 0))), "");
    }
}
