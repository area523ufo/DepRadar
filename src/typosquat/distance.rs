//! Edit-distance primitives for typosquat detection.
//!
//! A typosquatted package name is, almost by definition, a small number of
//! keystroke edits away from a popular one. [`levenshtein_distance`] measures
//! that gap, [`nearest_match`] finds the closest legitimate name in a
//! reference set, and [`edit_script`] reconstructs the exact character-level
//! operations that turn the reference name into the candidate. No scoring or
//! thresholding lives here — callers decide what counts as "suspiciously
//! close".

use std::fmt;

/// A single character-level edit operation transforming a reference `target`
/// name into a `candidate` name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    /// Inserted `ch` at 0-based `pos` in the candidate string.
    Insert { pos: usize, ch: char },
    /// Deleted `ch` from 0-based `pos` in the target string.
    Delete { pos: usize, ch: char },
    /// Substituted `from` (in target) with `to` (in candidate) at 0-based `pos`.
    Substitute { pos: usize, from: char, to: char },
}

impl fmt::Display for EditOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Insert { pos, ch } => write!(f, "inserted `{ch}` at {pos}"),
            Self::Delete { pos, ch } => write!(f, "deleted `{ch}` at {pos}"),
            Self::Substitute { pos, from, to } => {
                write!(f, "substituted `{from}` -> `{to}` at {pos}")
            }
        }
    }
}

/// Computes a minimal sequence of [`EditOp`]s that transforms `target` (the
/// reference package name) into `candidate`.
///
/// Uses a suffix dynamic-programming table so that forward reconstruction
/// greedily preserves the longest matching prefixes first (for example,
/// turning `"express"` into `"expresss"` reports `Insert { pos: 7, ch: 's' }`
/// at the trailing character rather than in the middle of the `'s'` run).
pub fn edit_script(target: &str, candidate: &str) -> Vec<EditOp> {
    let t: Vec<char> = target.chars().collect();
    let c: Vec<char> = candidate.chars().collect();
    let m = t.len();
    let n = c.len();

    // `dp[i][j]` holds the minimum edit distance between `t[i..]` and `c[j..]`.
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[n] = m - i;
    }
    for (j, cell) in dp[m].iter_mut().enumerate() {
        *cell = n - j;
    }

    for i in (0..m).rev() {
        for j in (0..n).rev() {
            let sub_cost = usize::from(t[i] != c[j]);
            dp[i][j] = (dp[i + 1][j] + 1)
                .min(dp[i][j + 1] + 1)
                .min(dp[i + 1][j + 1] + sub_cost);
        }
    }

    let mut ops = Vec::with_capacity(dp[0][0]);
    let mut i = 0;
    let mut j = 0;

    while i < m || j < n {
        if i < m && j < n && t[i] == c[j] && dp[i][j] == dp[i + 1][j + 1] {
            i += 1;
            j += 1;
        } else if i < m && j < n && dp[i][j] == dp[i + 1][j + 1] + 1 {
            ops.push(EditOp::Substitute {
                pos: j,
                from: t[i],
                to: c[j],
            });
            i += 1;
            j += 1;
        } else if j < n && dp[i][j] == dp[i][j + 1] + 1 {
            ops.push(EditOp::Insert { pos: j, ch: c[j] });
            j += 1;
        } else if i < m && dp[i][j] == dp[i + 1][j] + 1 {
            ops.push(EditOp::Delete { pos: i, ch: t[i] });
            i += 1;
        }
    }

    ops
}

/// Renders a human-readable equation explaining how `target` became
/// `candidate` via `ops` (e.g. `` `expresss` = `express` + inserted `s` at 7 ``).
pub fn format_edit_script(candidate: &str, target: &str, ops: &[EditOp]) -> String {
    if ops.is_empty() {
        return format!("`{candidate}` = `{target}`");
    }
    let steps = ops
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" + ");
    format!("`{candidate}` = `{target}` + {steps}")
}

/// Standard Levenshtein edit distance between `a` and `b`: the minimum number
/// of single-character insertions, deletions, or substitutions that turns one
/// string into the other.
///
/// Operates on Unicode scalar values (`char`), so multi-byte characters count
/// as one edit. Runs in `O(a.len() * b.len())` time and `O(b.len())` extra
/// space using the usual two-row dynamic-programming table.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    // `prev[j]` holds the distance between the processed prefix of `a` and the
    // first `j` characters of `b`. It starts as the distance from the empty
    // prefix of `a`, which is just `j` deletions.
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr: Vec<usize> = vec![0; b.len() + 1];

    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let substitution_cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j + 1] + 1) // delete a[i]
                .min(curr[j] + 1) // insert b[j]
                .min(prev[j] + substitution_cost); // substitute / match
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b.len()]
}

/// Returns the name in `dataset` with the smallest [`levenshtein_distance`] to
/// `candidate`, together with that distance.
///
/// On a tie the earliest entry in `dataset` wins. An empty `dataset` yields
/// `(String::new(), usize::MAX)`.
pub fn nearest_match(candidate: &str, dataset: &[String]) -> (String, usize) {
    let mut best: Option<(&String, usize)> = None;

    for name in dataset {
        let distance = levenshtein_distance(candidate, name);
        if best.is_none_or(|(_, best_distance)| distance < best_distance) {
            best = Some((name, distance));
            if distance == 0 {
                break; // nothing can beat an exact match
            }
        }
    }

    match best {
        Some((name, distance)) => (name.clone(), distance),
        None => (String::new(), usize::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typosquat::dataset::load_popular_packages;

    #[test]
    fn distance_between_identical_strings_is_zero() {
        assert_eq!(levenshtein_distance("express", "express"), 0);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn distance_handles_empty_operand() {
        assert_eq!(levenshtein_distance("", "react"), 5);
        assert_eq!(levenshtein_distance("react", ""), 5);
    }

    #[test]
    fn one_character_off_typosquat_has_distance_one() {
        // dropped 's' — the classic install-time typo
        assert_eq!(levenshtein_distance("expres", "express"), 1);
        // extra 's'
        assert_eq!(levenshtein_distance("expresss", "express"), 1);
        // single substitution
        assert_eq!(levenshtein_distance("requezts", "requests"), 1);
    }

    #[test]
    fn distance_counts_transposition_as_two_edits() {
        // plain Levenshtein has no transposition op: swapping two chars is one
        // deletion + one insertion (or two substitutions).
        assert_eq!(levenshtein_distance("recat", "react"), 2);
    }

    #[test]
    fn edit_script_captures_insert_delete_substitute_and_transposition() {
        // Insert: "express" -> "expresss" (extra 's' at index 7)
        let insert_ops = edit_script("express", "expresss");
        assert_eq!(insert_ops, vec![EditOp::Insert { pos: 7, ch: 's' }]);
        assert_eq!(
            format_edit_script("expresss", "express", &insert_ops),
            "`expresss` = `express` + inserted `s` at 7"
        );

        // Delete: "express" -> "expres" (dropped 's' at index 6)
        let delete_ops = edit_script("express", "expres");
        assert_eq!(delete_ops, vec![EditOp::Delete { pos: 6, ch: 's' }]);
        assert_eq!(
            format_edit_script("expres", "express", &delete_ops),
            "`expres` = `express` + deleted `s` at 6"
        );

        // Substitute: "requests" -> "requezts" ('s' -> 'z' at index 5)
        let sub_ops = edit_script("requests", "requezts");
        assert_eq!(
            sub_ops,
            vec![EditOp::Substitute {
                pos: 5,
                from: 's',
                to: 'z'
            }]
        );
        assert_eq!(
            format_edit_script("requezts", "requests", &sub_ops),
            "`requezts` = `requests` + substituted `s` -> `z` at 5"
        );

        // Transposition: "react" -> "recat" (two edits at positions 2 and 3)
        let trans_ops = edit_script("react", "recat");
        assert_eq!(trans_ops.len(), 2);
        assert_eq!(
            trans_ops,
            vec![
                EditOp::Substitute {
                    pos: 2,
                    from: 'a',
                    to: 'c'
                },
                EditOp::Substitute {
                    pos: 3,
                    from: 'c',
                    to: 'a'
                },
            ]
        );
        assert_eq!(
            format_edit_script("recat", "react", &trans_ops),
            "`recat` = `react` + substituted `a` -> `c` at 2 + substituted `c` -> `a` at 3"
        );
    }

    #[test]
    fn nearest_match_identifies_misspelled_popular_package() {
        let dataset = load_popular_packages();

        // deliberately misspelled 'express' (npm) — one inserted character
        let (name, distance) = nearest_match("expresss", &dataset);
        assert_eq!(name, "express");
        assert_eq!(distance, 1);

        // deliberately misspelled 'requests' (PyPI) — one substituted character
        let (name, distance) = nearest_match("requezts", &dataset);
        assert_eq!(name, "requests");
        assert_eq!(distance, 1);
    }

    #[test]
    fn nearest_match_returns_exact_entry_with_distance_zero() {
        let dataset = load_popular_packages();
        let (name, distance) = nearest_match("react", &dataset);
        assert_eq!(name, "react");
        assert_eq!(distance, 0);
    }

    #[test]
    fn nearest_match_on_empty_dataset_is_sentinel() {
        let (name, distance) = nearest_match("anything", &[]);
        assert_eq!(name, "");
        assert_eq!(distance, usize::MAX);
    }
}
