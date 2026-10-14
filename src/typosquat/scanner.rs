//! Combined typosquat scoring: the single entry point that ties the
//! edit-distance signal ([`super::distance`]) and the homoglyph signal
//! ([`super::homoglyph`]) together into one answer.
//!
//! Each underlying signal stays independent and boolean/threshold-free on its
//! own — this module is where "is this name suspicious, and why" finally gets
//! decided. [`score_package`] runs a candidate against both signals and
//! [`ScanResult`] reports which (if either, or both) fired.

use std::fmt;

use super::dataset::load_popular_packages;
use super::distance::{edit_script, format_edit_script, nearest_match, EditOp};
use super::homoglyph::homoglyph_impersonation;

/// Maximum edit distance from a popular package name that counts as
/// "suspiciously close" rather than coincidental.
///
/// A distance of 0 is an exact match — the real package, not a typosquat —
/// and is never flagged regardless of this threshold. 1-2 covers the classic
/// single-keystroke typo (`expresss`, `requezts`); anything farther is
/// treated as unrelated.
pub const DISTANCE_THRESHOLD: usize = 2;

/// Result of scoring a candidate package name against both typosquat
/// signals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    /// The name that was scored, as given by the caller.
    pub candidate: String,
    /// Set when [`nearest_match`] found a popular package within
    /// [`DISTANCE_THRESHOLD`] edits (and not an exact match): the matched
    /// name and the edit distance to it.
    pub distance_match: Option<(String, usize)>,
    /// Exact character-level edit operations that transform the matched
    /// popular package into `candidate` when [`Self::distance_match`] is set.
    pub edit_ops: Vec<EditOp>,
    /// Set when [`homoglyph_impersonation`] found that the candidate, once
    /// lookalike characters are folded back to Latin, is a popular package:
    /// the matched name.
    pub homoglyph_match: Option<String>,
}

impl ScanResult {
    /// True if either signal fired.
    pub fn is_likely_typosquat(&self) -> bool {
        self.distance_match.is_some() || self.homoglyph_match.is_some()
    }
}

impl fmt::Display for ScanResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.is_likely_typosquat() {
            return write!(f, "{}: no typosquat signal detected", self.candidate);
        }

        write!(f, "{}: likely typosquat", self.candidate)?;
        if let Some((name, distance)) = &self.distance_match {
            let ops = if self.edit_ops.is_empty() {
                edit_script(name, &self.candidate)
            } else {
                self.edit_ops.clone()
            };
            let explanation = format_edit_script(&self.candidate, name, &ops);
            write!(
                f,
                "\n  - edit distance {distance} from popular package \"{name}\" ({explanation})"
            )?;
        }
        if let Some(name) = &self.homoglyph_match {
            write!(
                f,
                "\n  - homoglyph impersonation of popular package \"{name}\""
            )?;
        }
        Ok(())
    }
}

/// Scores `name` against the bundled popular-package dataset using both
/// typosquat signals and returns why (if at all) it looks suspicious.
///
/// Loads [`load_popular_packages`] internally, so callers only need a name.
pub fn score_package(name: &str) -> ScanResult {
    let dataset = load_popular_packages();

    let (nearest_name, distance) = nearest_match(name, &dataset);
    let (distance_match, edit_ops) = if distance > 0 && distance <= DISTANCE_THRESHOLD {
        let ops = edit_script(&nearest_name, name);
        (Some((nearest_name, distance)), ops)
    } else {
        (None, Vec::new())
    };

    let homoglyph_match = homoglyph_impersonation(name, &dataset);

    ScanResult {
        candidate: name.to_string(),
        distance_match,
        edit_ops,
        homoglyph_match,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_popular_package_is_not_flagged() {
        let result = score_package("react");
        assert_eq!(result.distance_match, None);
        assert!(result.edit_ops.is_empty());
        assert_eq!(result.homoglyph_match, None);
        assert!(!result.is_likely_typosquat());
    }

    #[test]
    fn unrelated_name_is_not_flagged() {
        // Long and distinctive enough that its nearest dataset neighbour is
        // far past the threshold (verified: distance 26 to the closest npm
        // entry).
        let result = score_package("definitely-not-a-real-package-zzz123");
        assert_eq!(result.distance_match, None);
        assert!(result.edit_ops.is_empty());
        assert_eq!(result.homoglyph_match, None);
        assert!(!result.is_likely_typosquat());
    }

    #[test]
    fn distance_only_match_is_flagged_with_reason() {
        // One inserted 's' — no homoglyphs involved.
        let result = score_package("expresss");
        assert_eq!(result.distance_match, Some(("express".to_string(), 1)));
        assert_eq!(result.edit_ops, vec![EditOp::Insert { pos: 7, ch: 's' }]);
        assert_eq!(result.homoglyph_match, None);
        assert!(result.is_likely_typosquat());
    }

    #[test]
    fn homoglyph_only_match_is_flagged_with_reason() {
        // "requests" (PyPI) written in full-width Latin: identical edit
        // distance behaviour to any other unrelated string, but flagged by
        // the homoglyph signal.
        let fullwidth = "\u{FF52}\u{FF45}\u{FF51}\u{FF55}\u{FF45}\u{FF53}\u{FF54}\u{FF53}";
        let result = score_package(fullwidth);
        assert_eq!(result.distance_match, None);
        assert!(result.edit_ops.is_empty());
        assert_eq!(result.homoglyph_match.as_deref(), Some("requests"));
        assert!(result.is_likely_typosquat());
    }

    #[test]
    fn both_signals_can_fire_on_the_same_candidate() {
        // "react" with a Cyrillic 'а' (U+0430): a single-character
        // substitution, so it is both one edit away from "react" *and* a
        // homoglyph impersonation of it.
        let result = score_package("re\u{0430}ct");
        assert_eq!(result.distance_match, Some(("react".to_string(), 1)));
        assert_eq!(
            result.edit_ops,
            vec![EditOp::Substitute {
                pos: 2,
                from: 'a',
                to: '\u{0430}'
            }]
        );
        assert_eq!(result.homoglyph_match.as_deref(), Some("react"));
        assert!(result.is_likely_typosquat());
    }

    #[test]
    fn display_reports_both_reasons_when_both_fire() {
        let result = score_package("re\u{0430}ct");
        let rendered = result.to_string();
        assert!(rendered.contains("likely typosquat"));
        assert!(rendered.contains("edit distance 1 from popular package \"react\""));
        assert!(rendered.contains("`re\u{0430}ct` = `react` + substituted `a` -> `\u{0430}` at 2"));
        assert!(rendered.contains("homoglyph impersonation of popular package \"react\""));
    }

    #[test]
    fn display_renders_insert_delete_substitute_and_transposition_scripts() {
        // Insert case
        let insert_rendered = score_package("expresss").to_string();
        assert!(insert_rendered.contains("`expresss` = `express` + inserted `s` at 7"));

        // Delete case
        let delete_rendered = score_package("expres").to_string();
        assert!(delete_rendered.contains("`expres` = `express` + deleted `s` at 6"));

        // Substitute case
        let sub_rendered = score_package("requezts").to_string();
        assert!(sub_rendered.contains("`requezts` = `requests` + substituted `s` -> `z` at 5"));

        // Transposition case (2 edits in Levenshtein)
        let trans_rendered = score_package("recat").to_string();
        assert!(trans_rendered.contains(
            "`recat` = `react` + substituted `a` -> `c` at 2 + substituted `c` -> `a` at 3"
        ));
    }

    #[test]
    fn display_reports_no_signal_for_clean_name() {
        let result = score_package("react");
        assert_eq!(result.to_string(), "react: no typosquat signal detected");
    }
}
