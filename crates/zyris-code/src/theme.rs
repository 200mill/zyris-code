//! The Zyris brand palette — warm dark.
//!
//! **Only one place gets a background** — the user message (`USER_BG`). The whole screen is not
//! painted — letting the terminal use its own background is this app's policy. On 2026-08-03 a
//! full-screen background was turned on to stop ghosting, then reverted when it turned out the
//! person had cleared it. Ghosting is cleaned up by `app::heal_interval` (full redraw every 2s by default).
//!
//! **Don't create text without a color.** Unspecified, the terminal's own default foreground leaks
//! out — on a terminal with a changed default foreground, everything "that should be white" shows
//! in that color.

use ratatui::style::Color;

/// `--zyris-bg`(#0f0d0a) of the Zyris web palette. **Not painted by default** — see `page_bg`.
pub const BG: Color = Color::Rgb(0x0f, 0x0d, 0x0a);

/// The background laid over the whole screen. **Default is none — the terminal uses its own.**
///
/// It used to lay `BG` over every leftover cell. There was a reason: the ratatui diff doesn't
/// send a full-width character's right cell to the terminal (trusting the terminal to paint both
/// cells), and the protection that force-clears that cell when a full-width character turns narrow
/// only fires when **`previous.bg != Reset`**. Without a background, the right half of a full-width character ghosts on remote terminals.
///
/// **Still, the default stays off** — because there are places the app can't paint. The terminal
/// window leaves pixels that don't fit the grid as margin at the right and bottom, and the window
/// itself has padding. Those spots stay terminal background, so the moment the app paints its own,
/// **a differently colored band appears at the edges.** That ruins the screen border for the sake
/// of the screen — the terminal background the person chose is simply better everywhere.
///
/// Ghosting is cleared by redrawing — `Ctrl+L` and `app::heal_interval` (2 seconds by default) do it.
/// On terminals where that isn't enough, turn it back on with **`ZYRIS_CODE_BG`**: `zyris` gives
/// the brand color above, `#rrggbb` gives that color.
pub fn page_bg() -> Option<Color> {
    static PICKED: std::sync::OnceLock<Option<Color>> = std::sync::OnceLock::new();
    *PICKED.get_or_init(|| page_bg_from(std::env::var("ZYRIS_CODE_BG").ok().as_deref()))
}

/// Turns `$ZYRIS_CODE_BG` into a color. **Pure** — the decision must live here so tests don't shake the env.
pub fn page_bg_from(given: Option<&str>) -> Option<Color> {
    let given = given.map(str::trim).filter(|v| !v.is_empty())?;
    match given.to_ascii_lowercase().as_str() {
        // The off side must be expressible too — it's the way back when it was left on and forgotten.
        "none" | "off" | "0" | "terminal" => None,
        "zyris" | "on" | "1" | "default" => Some(BG),
        _ => hex(given),
    }
}

/// `#rrggbb` or `rrggbb`. Unreadable gives `None` — no reason for the app to die on a typo.
fn hex(text: &str) -> Option<Color> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?))
}

/// The line dividing areas. Kept at mid brightness so it shows subtly on any terminal background.
pub const BORDER: Color = Color::Rgb(0x3a, 0x30, 0x29);
pub const BORDER_LIGHT: Color = Color::Rgb(0x4a, 0x3e, 0x36);
pub const TEXT: Color = Color::Rgb(0xe8, 0xe2, 0xdc);
pub const TEXT_MUTED: Color = Color::Rgb(0x9c, 0x94, 0x8d);
pub const TEXT_HEADING: Color = Color::Rgb(0xf1, 0xed, 0xe8);
pub const ACCENT: Color = Color::Rgb(0xc9, 0x73, 0x4d);
pub const ACCENT_HOVER: Color = Color::Rgb(0xb5, 0x62, 0x3e);
pub const ACCENT_MUTED: Color = Color::Rgb(0xa3, 0x53, 0x32);
pub const SUCCESS: Color = Color::Rgb(0x8f, 0xae, 0x5c);
pub const WARNING: Color = Color::Rgb(0xd9, 0xa4, 0x41);
pub const DANGER: Color = Color::Rgb(0xc1, 0x50, 0x3f);

/// The background where the user spoke. ACCENT (0xc9734d) lowered enough to work as a background.
///
/// **This is the only place that uses a background.** The "don't paint backgrounds" rule at the
/// top of the file is flipped here and only here — laid over the whole screen it would jump like
/// a stain, and painting everything makes nothing distinguishable. This one line is the "where I spoke" signal.
pub const USER_BG: Color = Color::Rgb(0x2a, 0x20, 0x1a);

/// The tool line's name. **Must not be the same muted color as reasoning.**
///
/// Inside an expanded card reasoning fills the screen; if tools were also `TEXT_MUTED`, the actual
/// "what was done" would be buried. What the reader scans is the tool line, so that side must stand out.
pub const TOOL: Color = Color::Rgb(0x7f, 0xb0, 0xd4);
/// The tool line's argument summary. One step below the name.
pub const TOOL_ARG: Color = Color::Rgb(0x6b, 0x8a, 0xa0);

/// The added line in a diff. Green.
///
/// `SUCCESS`·`DANGER` aren't used as-is. Those two say "tool worked / didn't work", so if a
/// deletion line in a successful edit were the same red as a failed tool, the eye would misread.
pub const DIFF_ADD: Color = Color::Rgb(0x7e, 0xc0, 0x50);
/// The removed line in a diff. Red.
pub const DIFF_DEL: Color = Color::Rgb(0xe0, 0x6c, 0x75);

#[cfg(test)]
mod tests {
    use super::*;

    /// The palette must match the Zyris web values. Misaligned, the same product shows in different colors.
    #[test]
    fn the_palette_matches_the_brand_values() {
        assert_eq!(BG, Color::Rgb(0x0f, 0x0d, 0x0a));
        assert_eq!(ACCENT, Color::Rgb(0xc9, 0x73, 0x4d));
        assert_eq!(TEXT, Color::Rgb(0xe8, 0xe2, 0xdc));
        assert_eq!(TEXT_MUTED, Color::Rgb(0x9c, 0x94, 0x8d));
        assert_eq!(TEXT_HEADING, Color::Rgb(0xf1, 0xed, 0xe8));
        assert_eq!(BORDER, Color::Rgb(0x3a, 0x30, 0x29));
        assert_eq!(DANGER, Color::Rgb(0xc1, 0x50, 0x3f));
    }
}
