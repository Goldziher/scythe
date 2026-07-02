//! Inline suppression comments — `-- scythe-audit: ignore[ID1,ID2,...]`.
//!
//! # Annotation syntax
//!
//! ```text
//! -- scythe-audit: ignore[SC-SEC01,SC-SEC02] reason="vetted by security team"
//! ```
//!
//! - Leading whitespace is allowed.
//! - The comment prefix must be `--` (two dashes); block comments are not supported.
//! - IDs are `[A-Z][A-Z0-9-]*`, separated by commas with no internal whitespace.
//! - The optional `reason="..."` clause is parsed and discarded.
//! - Malformed annotations are silently ignored (no panic, no suppression created).
//!
//! # Attachment rules
//!
//! An annotation attaches to the next non-blank, non-comment line. From that line
//! onward, every line belonging to the same statement is suppressed (until either
//! a blank line or a line containing `;` terminates the statement).  Multiple
//! consecutive annotation lines stack their ID sets.

use ahash::{AHashMap, AHashSet};

/// The literal prefix that introduces a suppression annotation.
const ANNOTATION_PREFIX: &str = "-- scythe-audit: ignore[";

// ---------------------------------------------------------------------------
// SuppressionSet
// ---------------------------------------------------------------------------

/// Set of per-line rule-ID suppressions parsed from inline annotations.
#[derive(Debug, Default)]
pub struct SuppressionSet {
    /// Map: 1-based line number → set of suppressed rule IDs.
    by_line: AHashMap<usize, AHashSet<String>>,
}

impl SuppressionSet {
    /// Parse a complete SQL string and build the suppression set from every
    /// `-- scythe-audit: ignore[…]` annotation found in it.
    pub fn parse(sql: &str) -> Self {
        let mut set = Self::default();

        // Per-line classification.
        let lines: Vec<&str> = sql.split('\n').collect();
        let n = lines.len();

        // Pending IDs collected from consecutive annotation lines; attached to
        // the first following non-blank, non-comment line.
        let mut pending: AHashSet<String> = AHashSet::new();

        let mut i = 0;
        while i < n {
            let trimmed = lines[i].trim();
            if trimmed.is_empty() {
                // Blank line — discard any pending suppressions that never
                // found a target (annotation immediately followed by blank).
                pending.clear();
                i += 1;
                continue;
            }

            if let Some(ids) = try_parse_annotation(trimmed) {
                // This line is a scythe-audit annotation.
                pending.extend(ids);
                i += 1;
                continue;
            }

            if trimmed.starts_with("--") {
                // Ordinary SQL comment — does NOT consume the pending set; the
                // suppression should still attach to the next statement line.
                i += 1;
                continue;
            }

            // Non-blank, non-comment line: attach pending suppressions.
            if !pending.is_empty() {
                let ids_to_attach: AHashSet<String> = pending.drain().collect();

                // Suppress this line and every following line until a blank
                // line or a line containing `;` (statement terminator).
                let mut j = i;
                loop {
                    let entry = set.by_line.entry(j + 1).or_default();
                    entry.extend(ids_to_attach.iter().cloned());

                    let has_semicolon = lines[j].contains(';');
                    j += 1;
                    if has_semicolon || j >= n || lines[j].trim().is_empty() {
                        break;
                    }
                }
            }

            i += 1;
        }

        set
    }

    /// Return `true` if `rule_id` is suppressed on `line` (1-based).
    pub fn is_suppressed(&self, rule_id: &str, line: usize) -> bool {
        self.by_line.get(&line).is_some_and(|ids| ids.contains(rule_id))
    }

    /// Return `true` if no suppressions are recorded.
    pub fn is_empty(&self) -> bool {
        self.by_line.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Annotation parser
// ---------------------------------------------------------------------------

/// Attempt to parse a trimmed line as a `-- scythe-audit: ignore[ID,...]`
/// annotation.  Returns `Some(ids)` on success; `None` if the line is not a
/// matching annotation or the annotation is malformed.
fn try_parse_annotation(trimmed: &str) -> Option<Vec<String>> {
    // Must start with the exact prefix.
    let rest = trimmed.strip_prefix(ANNOTATION_PREFIX)?;

    // Find the closing `]`.
    let close = rest.find(']')?;
    let id_part = &rest[..close];

    // id_part must be non-empty and contain only valid ID characters and commas.
    if id_part.is_empty() {
        return None;
    }

    let ids: Vec<String> = id_part.split(',').map(|s| s.trim().to_string()).collect();

    // Validate each ID: must match `[A-Z][A-Z0-9-]*`.
    for id in &ids {
        if !is_valid_rule_id(id) {
            return None;
        }
    }

    Some(ids)
}

/// Check that `s` matches `[A-Z][A-Z0-9-]*`.
fn is_valid_rule_id(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_rule_ignore_suppresses_next_line() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON users TO bob;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC02", 2));
        assert!(!set.is_suppressed("SC-SEC01", 2));
    }

    #[test]
    fn multi_rule_ignore_suppresses_both_ids() {
        let sql = "-- scythe-audit: ignore[SC-SEC01,SC-SEC02]\nSELECT 1;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC01", 2));
        assert!(set.is_suppressed("SC-SEC02", 2));
        assert!(!set.is_suppressed("SC-SEC03", 2));
    }

    #[test]
    fn reason_clause_is_parsed_and_discarded() {
        let sql = r#"-- scythe-audit: ignore[SC-SEC01] reason="vetted"
SELECT pg_read_file('foo');"#;
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC01", 2));
    }

    #[test]
    fn stacked_annotations_union_ids() {
        let sql = "-- scythe-audit: ignore[SC-SEC01]\n-- scythe-audit: ignore[SC-SEC02]\nSELECT 1;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC01", 3));
        assert!(set.is_suppressed("SC-SEC02", 3));
    }

    #[test]
    fn multi_line_statement_suppresses_all_covered_lines() {
        let sql = "-- scythe-audit: ignore[SC-SEC08]\nSELECT *\nFROM a, b\nWHERE a.id = b.id;";
        let set = SuppressionSet::parse(sql);
        // Lines 2, 3, 4 are all part of the statement.
        assert!(set.is_suppressed("SC-SEC08", 2));
        assert!(set.is_suppressed("SC-SEC08", 3));
        assert!(set.is_suppressed("SC-SEC08", 4));
    }

    #[test]
    fn blank_line_terminates_suppression_scope() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON a TO x;\n\nGRANT ALL ON b TO y;";
        let set = SuppressionSet::parse(sql);
        // Line 2 is suppressed; line 4 (after blank on line 3) is NOT.
        assert!(set.is_suppressed("SC-SEC02", 2));
        assert!(!set.is_suppressed("SC-SEC02", 4));
    }

    #[test]
    fn semicolon_terminates_statement() {
        // Two statements on adjacent lines; only the first should be suppressed.
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON a TO x;\nGRANT ALL ON b TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC02", 2));
        assert!(!set.is_suppressed("SC-SEC02", 3));
    }

    #[test]
    fn annotation_at_eof_with_no_following_statement_is_harmless() {
        let sql = "SELECT 1;\n-- scythe-audit: ignore[SC-SEC02]";
        let set = SuppressionSet::parse(sql);
        // No crash; nothing should be suppressed on line 1.
        assert!(!set.is_suppressed("SC-SEC02", 1));
    }

    #[test]
    fn ordinary_comment_does_not_consume_pending_suppression() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\n-- just a comment\nGRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        // The annotation skips the ordinary comment and attaches to line 3.
        assert!(set.is_suppressed("SC-SEC02", 3));
    }

    #[test]
    fn malformed_annotation_empty_brackets_is_silently_ignored() {
        let sql = "-- scythe-audit: ignore[]\nGRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_empty());
    }

    #[test]
    fn malformed_annotation_space_in_id_is_silently_ignored() {
        let sql = "-- scythe-audit: ignore[SC SEC01]\nSELECT 1;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_empty());
    }

    #[test]
    fn is_suppressed_returns_false_for_unknown_id() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(!set.is_suppressed("SC-SEC99", 2));
    }

    #[test]
    fn is_suppressed_returns_true_for_known_id_on_suppressed_line() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC02", 2));
    }

    #[test]
    fn unsuppressed_line_returns_false() {
        let sql = "GRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(!set.is_suppressed("SC-SEC02", 1));
    }

    #[test]
    fn is_empty_when_no_annotations() {
        let sql = "SELECT 1; SELECT 2;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_empty());
    }
}
