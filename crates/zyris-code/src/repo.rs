//! The context strip on the divider above the input — working directory and git state.

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::Span;

use crate::markdown::display_width;
use crate::theme;

/// What one `git status --porcelain=v2 --branch` call said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repo {
    /// Branch name, or a short oid when HEAD is detached.
    pub branch: String,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub conflicts: usize,
    pub ahead: usize,
    pub behind: usize,
}

impl Repo {
    /// Nothing to commit and nothing to push.
    pub fn is_clean(&self) -> bool {
        self.staged == 0
            && self.unstaged == 0
            && self.untracked == 0
            && self.conflicts == 0
            && self.ahead == 0
            && self.behind == 0
    }
}

/// Reads what `git status --porcelain=v2 --branch` printed.
///
/// `None` means **this is not a repository we can describe** — no branch header, or a HEAD we
/// cannot name. Every failure funnels here so the caller has exactly one case to handle.
///
/// ```text
/// # branch.oid <sha>          short 7 characters, used only when detached
/// # branch.head <name>        the literal "(detached)" sends us to the oid
/// # branch.ab +N -M           ahead N, behind M. Absent without an upstream.
/// 1 XY ... / 2 XY ...         X is the index column, Y the worktree column; '.' is unchanged
/// u ...                       unmerged
/// ? ...                       untracked
/// ```
///
/// **A path staged and then edited again counts in both columns.** That is what every shell
/// prompt does, and collapsing it would hide the edit that is not staged yet.
pub fn parse(out: &str) -> Option<Repo> {
    let mut repo = Repo::default();
    let mut oid = None;
    let mut saw_header = false;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# branch.") {
            saw_header = true;
            if let Some(v) = rest.strip_prefix("oid ") {
                oid = Some(v.trim().to_string());
            } else if let Some(v) = rest.strip_prefix("head ") {
                repo.branch = v.trim().to_string();
            } else if let Some(v) = rest.strip_prefix("ab ") {
                (repo.ahead, repo.behind) = ahead_behind(v);
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            // Field two is the two-letter status. Anything shorter is not a line git wrote.
            let mut xy = line.split(' ').nth(1).unwrap_or("..").chars();
            repo.staged += usize::from(xy.next().is_some_and(|c| c != '.'));
            repo.unstaged += usize::from(xy.next().is_some_and(|c| c != '.'));
        } else if line.starts_with("u ") {
            repo.conflicts += 1;
        } else if line.starts_with("? ") {
            repo.untracked += 1;
        }
    }
    if !saw_header {
        return None;
    }
    // A detached HEAD has no name, so the short oid stands in — otherwise the strip would say
    // "(detached)", which is longer and says less.
    if repo.branch.is_empty() || repo.branch == "(detached)" {
        repo.branch = oid.map(|o| o.chars().take(7).collect()).unwrap_or_default();
    }
    (!repo.branch.is_empty()).then_some(repo)
}

/// `+2 -3` from the `branch.ab` header. Unreadable counts as zero — a wrong number here would be
/// worse than none.
fn ahead_behind(text: &str) -> (usize, usize) {
    let mut out = (0, 0);
    for part in text.split_whitespace() {
        match part.split_at_checked(1) {
            Some(("+", n)) => out.0 = n.parse().unwrap_or(0),
            Some(("-", n)) => out.1 = n.parse().unwrap_or(0),
            _ => {}
        }
    }
    out
}

/// The lead-in before the first piece, and the gap before the rule resumes.
const LEAD: &str = "─ ";
/// Between two pieces that are **both** there.
const SEP: &str = " · ";
/// Below this many trailing rule columns the row stops reading as a divider, so the strip is
/// dropped entirely and the plain rule comes back.
const MIN_RULE: usize = 4;

/// How much of the git piece survives at a given width. **Least urgent first** — walking this
/// list in order is the drop order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Full,
    NoUntracked,
    NoAhead,
    NoCounts,
    NoGit,
}

const LEVELS: [Level; 5] =
    [Level::Full, Level::NoUntracked, Level::NoAhead, Level::NoCounts, Level::NoGit];

/// The whole divider row: `─ ~/zyris-code · ⎇ main +2 ~1 ────────`.
///
/// **Pure**, and it always returns exactly `width` columns, so the caller hands it straight to a
/// `Line` without measuring anything.
///
/// A piece that is not there contributes **nothing — not even a separator.** Building this by
/// concatenating optional fragments is how a machine without git ends up showing
/// `~/zyris-code ·` with a dangling join. Pieces are collected, empties are dropped, and only
/// the survivors are joined — which is also what lets a piece be added later without leaving a
/// hole on the days its neighbours are missing.
pub fn spans(
    width: u16,
    cwd: &Path,
    home: Option<&Path>,
    repo: Option<&Repo>,
) -> Vec<Span<'static>> {
    let width = width as usize;
    let bare = || vec![Span::styled("─".repeat(width), Style::default().fg(theme::BORDER))];
    // Lead-in, one space, and a rule long enough to still look like a rule.
    let overhead = display_width(LEAD) + 1 + MIN_RULE;
    if width <= overhead {
        return bare();
    }
    let budget = width - overhead;
    let path = path_text(cwd, home);
    for level in LEVELS {
        if let Some(out) = assemble(width, &pieces(&path, repo, level), budget) {
            return out;
        }
    }
    // Even the path alone did not fit — take it from the head, never the tail.
    match shorten(&path, budget) {
        Some(short) => {
            assemble(width, &pieces(&short, None, Level::NoGit), budget).unwrap_or_else(bare)
        }
        None => bare(),
    }
}

/// The pieces, in order, at this level of detail. **Empty inner vectors are the whole point** —
/// `assemble` drops them, and with them their separator.
fn pieces(path: &str, repo: Option<&Repo>, level: Level) -> Vec<Vec<Span<'static>>> {
    let muted = Style::default().fg(theme::TEXT_MUTED);
    let mut git: Vec<Span<'static>> = Vec::new();
    if let (Some(r), false) = (repo, level == Level::NoGit) {
        git.push(Span::styled(format!("⎇ {}", r.branch), muted));
        if level != Level::NoCounts {
            let mut count = |n: usize, mark: char, style: Style| {
                if n > 0 {
                    git.push(Span::styled(format!(" {mark}{n}"), style));
                }
            };
            let warn = Style::default().fg(theme::WARNING);
            // A conflict is the one thing here that must not be missed, so it comes first and
            // wears the only alarming colour on the row.
            count(r.conflicts, '!', Style::default().fg(theme::DANGER));
            count(r.staged, '+', warn);
            count(r.unstaged, '~', warn);
            if level == Level::Full {
                // Files git does not know are usually build litter. In a warning colour this
                // would be lit permanently and stop being read.
                count(r.untracked, '?', muted);
            }
            if matches!(level, Level::Full | Level::NoUntracked) {
                // Pushing is less urgent than committing, so these stay quiet too.
                count(r.ahead, '↑', muted);
                count(r.behind, '↓', muted);
            }
        }
    }
    vec![vec![Span::styled(path.to_string(), muted)], git]
}

/// Joins the non-empty pieces and pads out to exactly `width`. `None` when the body is over
/// budget, which is the signal to try a lower level.
fn assemble(
    width: usize,
    parts: &[Vec<Span<'static>>],
    budget: usize,
) -> Option<Vec<Span<'static>>> {
    let live: Vec<&Vec<Span<'static>>> = parts.iter().filter(|p| !p.is_empty()).collect();
    let body: usize = live.iter().flat_map(|p| p.iter()).map(|s| display_width(&s.content)).sum();
    let joins = live.len().saturating_sub(1) * display_width(SEP);
    if body + joins > budget {
        return None;
    }
    let border = Style::default().fg(theme::BORDER);
    let mut out = vec![Span::styled(LEAD, border)];
    for (i, piece) in live.iter().enumerate() {
        if i > 0 {
            out.push(Span::styled(SEP, Style::default().fg(theme::BORDER_LIGHT)));
        }
        out.extend(piece.iter().cloned());
    }
    let rule = width - display_width(LEAD) - body - joins - 1;
    out.push(Span::styled(format!(" {}", "─".repeat(rule)), border));
    Some(out)
}

/// `/home/ruma/zyris-code` under `/home/ruma` becomes `~/zyris-code`.
///
/// **Not under home is shown as it is** — `/etc/nginx` cut down to a name would say less than
/// the path does.
fn path_text(cwd: &Path, home: Option<&Path>) -> String {
    let full = cwd.display().to_string();
    let Some(home) = home.map(|h| h.display().to_string()).filter(|h| !h.is_empty()) else {
        return full;
    };
    if full == home {
        return "~".into();
    }
    match full.strip_prefix(&format!("{home}/")) {
        Some(rest) => format!("~/{rest}"),
        None => full,
    }
}

/// Drops leading components until it fits, marking the cut with `…/`.
///
/// **The tail is what identifies a directory**, so the head is what goes. `None` when even that
/// does not fit, and then the strip is not drawn at all.
fn shorten(path: &str, budget: usize) -> Option<String> {
    if display_width(path) <= budget {
        return Some(path.to_string());
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    (1..parts.len())
        .map(|cut| format!("…/{}", parts[cut..].join("/")))
        .find(|candidate| display_width(candidate) <= budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strip as plain text, the way a terminal would show it.
    fn strip(width: u16, cwd: &str, home: Option<&str>, repo: Option<&Repo>) -> String {
        super::spans(width, Path::new(cwd), home.map(Path::new), repo)
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    fn dirty() -> Repo {
        Repo {
            branch: "main".into(),
            staged: 2,
            unstaged: 1,
            untracked: 3,
            conflicts: 0,
            ahead: 1,
            behind: 0,
        }
    }

    #[test]
    fn a_missing_piece_leaves_no_separator_behind() {
        // **The point of the whole design.** Without git the path must be followed by the rule
        // directly — no separator, no reserved gap, nothing for a later piece to trip over.
        let out = strip(60, "/home/ruma/zyris-code", Some("/home/ruma"), None);
        assert!(out.starts_with("─ ~/zyris-code ─"), "no residue where git would be: {out:?}");
        assert!(!out.contains('·'), "a separator with nothing on one side: {out:?}");
    }

    #[test]
    fn the_separator_appears_only_between_two_present_pieces() {
        let out = strip(60, "/home/ruma/zyris-code", Some("/home/ruma"), Some(&dirty()));
        assert_eq!(out.matches('·').count(), 1, "exactly one join: {out:?}");
        assert!(out.contains("~/zyris-code · ⎇ main"), "{out:?}");
    }

    #[test]
    fn a_clean_repository_shows_no_counts() {
        let clean = Repo { branch: "main".into(), ..Repo::default() };
        let out = strip(60, "/home/ruma/zyris-code", Some("/home/ruma"), Some(&clean));
        assert!(out.contains("⎇ main"), "{out:?}");
        // Every count carries a digit, so no digit means no count. Checking the markers
        // themselves would trip over the `~` of the home directory.
        assert!(!out.contains(|c: char| c.is_ascii_digit()), "clean must be quiet: {out:?}");
        for noise in ['+', '?', '↑', '↓', '!'] {
            assert!(!out.contains(noise), "clean must be quiet, saw {noise:?}: {out:?}");
        }
    }

    #[test]
    fn every_count_shows_when_there_is_room() {
        let mut r = dirty();
        r.behind = 4;
        r.conflicts = 5;
        let out = strip(80, "/home/ruma/zyris-code", Some("/home/ruma"), Some(&r));
        for want in ["!5", "+2", "~1", "?3", "↑1", "↓4"] {
            assert!(out.contains(want), "missing {want}: {out:?}");
        }
    }

    #[test]
    fn narrowing_drops_the_least_urgent_piece_first() {
        let cwd = "/home/ruma/zyris-code";
        let home = Some("/home/ruma");
        let r = dirty();
        // Untracked goes before ahead, ahead before the counts, the counts before git itself.
        let order = ["?3", "↑1", "+2", "⎇ main", "~/zyris-code"];
        let mut gone: Vec<&str> = Vec::new();
        for width in (8..=60u16).rev() {
            let out = strip(width, cwd, home, Some(&r));
            for want in order {
                if !out.contains(want) && !gone.contains(&want) {
                    gone.push(want);
                }
            }
        }
        assert_eq!(gone, order, "things must fall off least-urgent first");
    }

    #[test]
    fn the_strip_never_exceeds_the_width_it_was_given() {
        let r = dirty();
        for width in 0..=120u16 {
            for cwd in ["/", "/home/ruma/zyris-code", "/home/ruma/a/very/deep/tree/of/directories"]
            {
                for repo in [None, Some(&r)] {
                    let out = strip(width, cwd, Some("/home/ruma"), repo);
                    let w = crate::markdown::display_width(&out);
                    assert_eq!(w, width as usize, "width {width} cwd {cwd} gave {w}: {out:?}");
                }
            }
        }
    }

    #[test]
    fn a_rule_too_short_to_read_as_a_rule_is_not_drawn_at_all() {
        // Under the floor the row goes back to what it draws today: a bare rule.
        let out = strip(7, "/home/ruma/zyris-code", Some("/home/ruma"), None);
        assert_eq!(out, "───────");
    }

    #[test]
    fn home_becomes_a_tilde_and_a_long_path_loses_its_head() {
        assert!(
            strip(60, "/home/ruma/zyris-code", Some("/home/ruma"), None).contains("~/zyris-code")
        );
        // Not under home: shown as it is.
        assert!(strip(60, "/etc/nginx", Some("/home/ruma"), None).contains("/etc/nginx"));
        // Too long for the room: the head goes, never the tail.
        let deep = "/home/ruma/a/very/deep/tree/of/directories/indeed";
        let out = strip(28, deep, Some("/home/ruma"), None);
        assert!(out.contains('…'), "{out:?}");
        assert!(out.contains("indeed"), "the tail is the part that identifies it: {out:?}");
    }

    #[test]
    fn every_span_carries_a_colour() {
        // An unstyled span leaks the terminal's own default foreground (`theme` header rule).
        for s in super::spans(60, Path::new("/home/ruma/zyris-code"), None, Some(&dirty())) {
            assert!(s.style.fg.is_some(), "uncoloured span: {:?}", s.content);
        }
    }

    #[test]
    fn a_clean_checkout_parses_to_a_clean_repo() {
        let out = "\
# branch.oid d192a603f6d083519b8bf785bcd41061c97e0cb8
# branch.head main
# branch.upstream origin/main
# branch.ab +0 -0
";
        let r = parse(out).expect("a branch header is enough to be a repository");
        assert_eq!(r.branch, "main");
        assert!(r.is_clean());
    }

    #[test]
    fn a_path_counts_in_both_columns_when_it_is_staged_and_edited_again() {
        // XY = "MM": staged in the index and changed again in the worktree.
        let out = "\
# branch.oid abc1234def5678
# branch.head main
1 MM N... 100644 100644 100644 aaa bbb src/app.rs
1 M. N... 100644 100644 100644 ccc ddd src/lib.rs
1 .M N... 100644 100644 100644 eee fff src/rows.rs
";
        let r = parse(out).unwrap();
        assert_eq!((r.staged, r.unstaged), (2, 2));
    }

    #[test]
    fn untracked_and_unmerged_lines_are_counted_apart() {
        let out = "\
# branch.oid abc1234def5678
# branch.head main
? target/
? notes.txt
u UU N... 100644 100644 100644 100644 aaa bbb ccc src/conflict.rs
";
        let r = parse(out).unwrap();
        assert_eq!((r.untracked, r.conflicts), (2, 1));
        assert_eq!((r.staged, r.unstaged), (0, 0));
    }

    #[test]
    fn a_detached_head_shows_a_short_oid_where_the_branch_would_be() {
        let out = "\
# branch.oid d192a603f6d083519b8bf785bcd41061c97e0cb8
# branch.head (detached)
";
        assert_eq!(parse(out).unwrap().branch, "d192a60");
    }

    #[test]
    fn a_branch_with_no_upstream_has_no_ahead_or_behind() {
        let out = "\
# branch.oid abc1234def5678
# branch.head feat/strip
";
        let r = parse(out).unwrap();
        assert_eq!((r.ahead, r.behind), (0, 0));
    }

    #[test]
    fn ahead_and_behind_come_off_the_ab_header() {
        let out = "\
# branch.oid abc1234def5678
# branch.head main
# branch.upstream origin/main
# branch.ab +2 -3
";
        let r = parse(out).unwrap();
        assert_eq!((r.ahead, r.behind), (2, 3));
    }

    #[test]
    fn output_without_a_branch_header_is_not_a_repository() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("fatal: not a git repository"), None);
    }
}
