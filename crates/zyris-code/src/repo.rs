//! The context strip on the divider above the input — working directory and git state.

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

#[cfg(test)]
mod tests {
    use super::*;

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
