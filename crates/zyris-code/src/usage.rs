//! What one session has cost so far — the numbers the bottom bar shows.
//!
//! Usage comes from `session_usage` (`conn::usage`), polled off the draw loop so a dead
//! connection never stalls keys and drawing.

/// The session's usage. A missing value means no turn has run yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub model: Option<String>,
    pub context_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub credits_used: Option<String>,
}

impl Usage {
    /// Cleared when switching sessions — stale numbers from the previous session would lie.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// How much context this model fits at once. **`None` when unknown, and then no maximum is shown.**
///
/// The server does not provide this — `ZUsage` only has what was used so far. So the maximum is
/// **loaded, never hardcoded**:
///
/// - `ZYRIS_CODE_CONTEXT_MAX` overrides whatever the model says.
/// - Otherwise a window spelled in the model's own name (`claude-opus-5-1m`, `gpt-4o-200k`) is
///   read from that name.
///
/// A model that names no window has no known limit. Inventing one would make it look true, so
/// none is shown.
pub fn context_limit(model: Option<&str>) -> Option<i64> {
    if let Some(n) = std::env::var("ZYRIS_CODE_CONTEXT_MAX").ok().and_then(|v| v.parse().ok()) {
        return Some(n);
    }
    window_in_name(model?)
}

/// A context window spelled in the model name itself: `1m`, `200k`, `1.5m`…
///
/// Scans for the first `<number><k|m>` run — a version number like the `5` in `claude-opus-5` has
/// no suffix and is skipped. The window is the model's own data, not a table of guesses.
fn window_in_name(model: &str) -> Option<i64> {
    let lower = model.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            let mult = match bytes.get(i) {
                Some(b'k') => 1_000.0,
                Some(b'm') => 1_000_000.0,
                _ => continue,
            };
            let num: f64 = lower[start..i].parse().ok()?;
            return Some((num * mult) as i64);
        }
        i += 1;
    }
    None
}

/// Big numbers, short. The bottom bar is one line.
///
/// When it lands evenly, drop the decimal and write `200k` — `200.0k` looks precise but reads badly.
pub fn compact(n: i64) -> String {
    let short = match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1_000.0),
        n => return n.to_string(),
    };
    short.replace(".0", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_numbers_get_short() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_500), "1.5k");
        assert_eq!(compact(2_400_000), "2.4M");
        assert_eq!(compact(200_000), "200k", "a round number drops the decimal");
    }

    /// The window must come out of the model's own name, not a table.
    #[test]
    fn a_window_spelled_in_the_model_name_is_read_from_it() {
        assert_eq!(window_in_name("claude-opus-5-1m"), Some(1_000_000));
        assert_eq!(window_in_name("gpt-4o-128k"), Some(128_000));
        assert_eq!(window_in_name("claude-sonnet-4-5-200k"), Some(200_000));
        assert_eq!(window_in_name("gemini-2.5-pro-1.5m"), Some(1_500_000));
    }

    /// A plain version number is not a window — `claude-opus-5` must not read as 5 tokens.
    #[test]
    fn a_version_number_without_a_suffix_is_not_a_window() {
        assert_eq!(window_in_name("claude-opus-5"), None);
        assert_eq!(window_in_name("gpt-4o"), None);
        assert_eq!(window_in_name("llama-3.1-70b"), None);
        assert_eq!(window_in_name(""), None);
    }

    /// Unknown model, unknown limit — that is the honest answer.
    #[test]
    fn an_unknown_model_has_no_limit() {
        assert_eq!(context_limit(Some("어느-새-모델")), None);
        assert_eq!(context_limit(None), None);
    }
}
