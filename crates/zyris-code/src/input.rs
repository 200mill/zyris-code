//! The input field's editing state. Fixed at the bottom of the screen; its height grows with the content.
//!
//! **The cursor is a character (char) index.** Handled as a byte index, it would leave character
//! boundaries in Korean and panic.

use crate::markdown::display_width;

#[derive(Debug, Default, Clone)]
pub struct Input {
    pub text: String,
    /// Position in characters.
    pub cursor: usize,
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, c: char) {
        let byte = self.byte_at(self.cursor);
        self.text.insert(byte, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let from = self.byte_at(self.cursor - 1);
        let to = self.byte_at(self.cursor);
        self.text.replace_range(from..to, "");
        self.cursor -= 1;
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    /// Number of characters before the cursor.
    pub fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len_chars());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len_chars();
    }

    /// Deletes the character under the cursor (`Delete`). At the end, does nothing.
    pub fn delete(&mut self) {
        if self.cursor >= self.len_chars() {
            return;
        }
        let from = self.byte_at(self.cursor);
        let to = self.byte_at(self.cursor + 1);
        self.text.replace_range(from..to, "");
    }

    /// Deletes the previous word (`Ctrl+W`). Like readline, it's a **delete**.
    pub fn delete_word(&mut self) {
        // First swallow the whitespace right before the cursor, then the run of non-whitespace.
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        let from = self.byte_at(i);
        let to = self.byte_at(self.cursor);
        self.text.replace_range(from..to, "");
        self.cursor = i;
    }

    /// Paste. Even multi-line text goes in as-is.
    pub fn insert_str(&mut self, s: &str) {
        let byte = self.byte_at(self.cursor);
        self.text.insert_str(byte, s);
        self.cursor += s.chars().count();
    }

    /// Columns taken by the text left of the cursor. Used to draw the cursor **in place by wide-character width**.
    ///
    /// Drawn by character count, the cursor shifts left in front of Korean.
    pub fn cursor_col(&self) -> usize {
        display_width(&self.text.chars().take(self.cursor).collect::<String>())
    }

    /// Rows the input occupies at this width. At least one.
    pub fn height(&self, width: u16) -> u16 {
        self.wrapped(width).0.len() as u16
    }

    /// The wrapped result at this width and **the (row, column) where the cursor sits**.
    ///
    /// If the wrapping and the cursor placement disagree, the cursor lands in the wrong spot on long
    /// text. So both come back together — `height` uses this too.
    ///
    /// **Wraps per character.** Wrapping per word would let one long URL push an entire line out, and
    /// cursor placement math would get far trickier. It's also how terminals do it.
    /// The width passed in is the **inner width**, minus the prompt (`"> "`).
    pub fn wrapped(&self, width: u16) -> (Vec<String>, (u16, u16)) {
        let limit = width.max(1) as usize;
        let mut lines = vec![String::new()];
        let (mut row, mut col) = (0u16, 0usize);
        let mut at = (0u16, 0u16);

        for (i, ch) in self.text.chars().enumerate() {
            // Pasting can bring in newlines. Break the line right there.
            if ch == '\n' {
                if i == self.cursor {
                    at = (row, col as u16);
                }
                lines.push(String::new());
                row += 1;
                col = 0;
                continue;
            }
            let w = display_width(&ch.to_string()).max(1);
            if col + w > limit {
                lines.push(String::new());
                row += 1;
                col = 0;
            }
            if i == self.cursor {
                at = (row, col as u16);
            }
            lines.last_mut().expect("there is always at least one line").push(ch);
            col += w;
        }
        // If the cursor is at the very end, it's the end of the last line.
        if self.cursor >= self.text.chars().count() {
            at = (row, col as u16);
        }
        (lines, at)
    }

    fn byte_at(&self, char_index: usize) -> usize {
        self.text.char_indices().nth(char_index).map(|(b, _)| b).unwrap_or(self.text.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cursor is in characters (char). Counted in byte indices, it panics on Korean.
    #[test]
    fn backspace_removes_one_korean_character_not_one_byte() {
        let mut i = Input::new();
        for c in "한글".chars() {
            i.insert(c);
        }
        i.backspace();
        assert_eq!(i.text, "한");
        assert_eq!(i.cursor, 1);
    }

    #[test]
    fn backspace_on_an_empty_input_does_nothing() {
        let mut i = Input::new();
        i.backspace();
        assert_eq!(i.text, "");
        assert_eq!(i.cursor, 0);
    }

    #[test]
    fn taking_the_text_clears_the_input() {
        let mut i = Input::new();
        for c in "안녕".chars() {
            i.insert(c);
        }
        assert_eq!(i.take(), "안녕");
        assert_eq!(i.text, "");
        assert_eq!(i.cursor, 0);
    }

    /// A long input grows the field taller and shrinks the conversation area by the same amount.
    #[test]
    fn a_long_input_grows_taller() {
        let mut i = Input::new();
        for c in "가나다라마바사아자차".chars() {
            i.insert(c);
        }
        assert!(
            i.height(10) >= 2,
            "ten wide glyphs (20 columns) in a width of 10 take more than one line"
        );
    }

    #[test]
    fn an_empty_input_is_one_line_tall() {
        assert_eq!(Input::new().height(40), 1);
    }

    /// **A long input wraps onto the next line.** Cut off, you can't tell what you're typing.
    #[test]
    fn a_long_input_wraps_onto_the_next_line() {
        let mut i = Input::new();
        for c in "abcdefghij".chars() {
            i.insert(c);
        }
        let (lines, at) = i.wrapped(4);
        assert_eq!(lines, vec!["abcd", "efgh", "ij"]);
        assert_eq!(at, (2, 2), "the cursor is at the end of the last line");
    }

    /// If the wrap point and the cursor spot disagree, the cursor stands in the wrong place on long text.
    #[test]
    fn the_cursor_follows_the_line_it_wrapped_onto() {
        let mut i = Input::new();
        for c in "abcdefgh".chars() {
            i.insert(c);
        }
        i.home();
        for _ in 0..5 {
            i.right();
        }
        // Five characters in, at width 4 that's the second column of the second line.
        assert_eq!(i.wrapped(4).1, (1, 1));
    }

    /// A wide character takes two columns, so only half fits inside the width. Wrapping by character count overflows on the right.
    #[test]
    fn wide_characters_wrap_by_columns_not_by_character_count() {
        let mut i = Input::new();
        for c in "가나다라".chars() {
            i.insert(c);
        }
        assert_eq!(i.wrapped(4).0, vec!["가나", "다라"]);
    }

    /// A newline that came in through paste breaks the line right there.
    #[test]
    fn a_pasted_newline_breaks_the_line_there() {
        let mut i = Input::new();
        i.insert_str("한 줄\n두 줄");
        assert_eq!(i.wrapped(40).0, vec!["한 줄", "두 줄"]);
        assert_eq!(i.height(40), 2);
    }

    fn typed(s: &str) -> Input {
        let mut i = Input::new();
        for c in s.chars() {
            i.insert(c);
        }
        i
    }

    /// You must be able to fix a character in the middle. Without this, fixing a typo means deleting everything after it.
    #[test]
    fn text_can_be_edited_in_the_middle() {
        let mut i = typed("안녕하세요");
        i.left();
        i.left();
        i.backspace();
        assert_eq!(i.text, "안녕세요");
        assert_eq!(i.cursor, 2);
    }

    #[test]
    fn the_cursor_stops_at_both_ends() {
        let mut i = typed("가나");
        i.left();
        i.left();
        i.left();
        assert_eq!(i.cursor, 0);
        i.end();
        i.right();
        assert_eq!(i.cursor, 2);
    }

    #[test]
    fn delete_removes_the_character_under_the_cursor() {
        let mut i = typed("한글");
        i.home();
        i.delete();
        assert_eq!(i.text, "글");
        assert_eq!(i.cursor, 0);
    }

    #[test]
    fn delete_at_the_end_does_nothing() {
        let mut i = typed("한");
        i.delete();
        assert_eq!(i.text, "한");
    }

    /// Ctrl+W is a **delete**, per the readline standard.
    #[test]
    fn ctrl_w_deletes_the_previous_word() {
        let mut i = typed("hello world");
        i.delete_word();
        assert_eq!(i.text, "hello ");
        i.delete_word();
        assert_eq!(i.text, "");
    }

    #[test]
    fn pasting_inserts_at_the_cursor() {
        let mut i = typed("가다");
        i.left();
        i.insert_str("나");
        assert_eq!(i.text, "가나다");
        assert_eq!(i.cursor, 2);
    }

    /// The cursor column counts wide characters as two. Drawn by character count, it shifts left in front of Korean.
    #[test]
    fn the_cursor_column_counts_wide_characters_as_two() {
        let mut i = typed("한a");
        assert_eq!(i.cursor_col(), 3);
        i.home();
        assert_eq!(i.cursor_col(), 0);
        i.right();
        assert_eq!(i.cursor_col(), 2);
    }
}
