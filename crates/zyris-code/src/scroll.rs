//! Viewport. Moves in units of lines.
//!
//! "Smooth" here is not pixel scrolling but **no delay** — as many notches as arrive are applied
//! immediately, and drawing is batched once per frame.

pub const LINES_PER_NOTCH: usize = 3;

#[derive(Debug, Clone, Copy)]
pub struct Scroll {
    /// Index of the line visible at the top of the screen.
    pub top: usize,
    /// Whether it sticks to the bottom. When stuck, new lines keep it at the bottom.
    pub stick: bool,
}

impl Default for Scroll {
    fn default() -> Self {
        Self { top: 0, stick: true }
    }
}

impl Scroll {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wheel. Positive goes up.
    pub fn wheel(&mut self, notches: i32, total: usize, height: usize) {
        let max_top = total.saturating_sub(height);
        let delta = notches.unsigned_abs() as usize * LINES_PER_NOTCH;
        self.top = if notches > 0 {
            self.top.saturating_sub(delta)
        } else {
            (self.top + delta).min(max_top)
        };
        // Reaching the bottom revives stickiness.
        self.stick = self.top >= max_top;
    }

    /// Called when the content changes.
    pub fn on_content(&mut self, total: usize, height: usize) {
        if self.stick {
            self.top = total.saturating_sub(height);
        } else {
            self.top = self.top.min(total.saturating_sub(height));
        }
    }

    /// The line range `[start, end)` to draw now.
    pub fn window(&self, total: usize, height: usize) -> (usize, usize) {
        let start = self.top.min(total.saturating_sub(height));
        (start, (start + height).min(total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_scroll_sticks_to_the_bottom() {
        let mut s = Scroll::new();
        s.on_content(100, 10);
        assert_eq!(s.window(100, 10), (90, 100));
    }

    /// Stuck to the bottom, new lines keep it at the bottom.
    #[test]
    fn new_content_keeps_a_stuck_view_at_the_bottom() {
        let mut s = Scroll::new();
        s.on_content(100, 10);
        s.on_content(120, 10);
        assert_eq!(s.window(120, 10), (110, 120));
    }

    /// If scrolled up, new lines keep the position — the screen must not jump while reading.
    #[test]
    fn new_content_does_not_move_a_scrolled_up_view() {
        let mut s = Scroll::new();
        s.on_content(100, 10);
        s.wheel(3, 100, 10); // 3 notches up = 9 lines
        let before = s.window(100, 10);
        s.on_content(120, 10);
        assert_eq!(s.window(120, 10), before, "위로 올려 둔 자리를 지켜야 한다");
    }

    #[test]
    fn one_notch_moves_three_lines() {
        let mut s = Scroll::new();
        s.on_content(100, 10);
        s.wheel(1, 100, 10);
        assert_eq!(s.top, 87);
    }

    #[test]
    fn scrolling_past_the_top_stops_at_zero() {
        let mut s = Scroll::new();
        s.on_content(100, 10);
        s.wheel(1000, 100, 10);
        assert_eq!(s.top, 0);
    }

    /// Scrolling back down to the bottom revives stickiness.
    #[test]
    fn scrolling_back_to_the_bottom_restores_stickiness() {
        let mut s = Scroll::new();
        s.on_content(100, 10);
        s.wheel(3, 100, 10);
        assert!(!s.stick);
        s.wheel(-3, 100, 10);
        assert!(s.stick, "바닥에 닿으면 다시 붙어야 한다");
    }

    #[test]
    fn content_shorter_than_the_view_starts_at_zero() {
        let mut s = Scroll::new();
        s.on_content(4, 10);
        assert_eq!(s.window(4, 10), (0, 4));
    }
}
