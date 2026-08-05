//! Clipboard.
//!
//! **Reading and writing are not symmetric.** Terminals usually allow *writing* to the clipboard
//! via OSC 52 but block *reading* queries (understandably — other apps could snoop the clipboard). So:
//!
//! - Copy: held in the app and also put on the system clipboard via OSC 52 — pastable into other windows.
//! - Paste: **only what this app copied** pastes. The system clipboard cannot be read.
//!
//! The terminal's own paste (usually `Ctrl+Shift+V` or middle click) arrives as key input, so
//! that path still works — with bracketed paste on, it comes as one chunk.

use std::io::Write;

use base64::Engine;

/// The clipboard the app holds.
#[derive(Debug, Default, Clone)]
pub struct Clipboard {
    text: String,
}

impl Clipboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Puts it in the app. Exporting to the system clipboard is done by `osc52_sequence`.
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

/// OSC 52 sequence that puts text on the system clipboard.
///
/// Even if the terminal ignores it, the in-app clipboard is already filled, so nothing is lost.
pub fn osc52_sequence(text: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    format!("\x1b]52;c;{encoded}\x07")
}

/// Exports to the system clipboard. Failures are ignored — the in-app clipboard is already the source of truth.
pub fn export(text: &str) {
    let mut out = std::io::stdout();
    let _ = out.write_all(osc52_sequence(text).as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_goes_in_comes_out() {
        let mut c = Clipboard::new();
        assert!(c.is_empty());
        c.set("안녕하세요");
        assert_eq!(c.get(), "안녕하세요");
        assert!(!c.is_empty());
    }

    /// OSC 52 takes the form `ESC ] 52 ; c ; <base64> BEL`. Even mixed with Hangul, base64 keeps it safe.
    #[test]
    fn the_osc52_sequence_is_well_formed() {
        let seq = osc52_sequence("한글 test");
        assert!(seq.starts_with("\x1b]52;c;"), "{seq:?}");
        assert!(seq.ends_with('\x07'), "{seq:?}");

        let body = seq.trim_start_matches("\x1b]52;c;").trim_end_matches('\x07');
        let decoded = base64::engine::general_purpose::STANDARD.decode(body).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "한글 test");
    }
}
