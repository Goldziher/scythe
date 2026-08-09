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
//! An annotation attaches only to the statement whose first non-blank,
//! non-comment line **immediately** follows it — a blank line between the
//! annotation and the statement discards the pending annotation entirely (it
//! does not "reach through" the blank line). Multiple consecutive annotation
//! lines stack their ID sets onto whatever statement follows them.
//!
//! # Keying: statement index, not source line
//!
//! Suppressions are keyed by **statement index** (0-based, in file order),
//! not by source line. Keying by line previously over-suppressed: two
//! statements written on the same line (`DROP TABLE a; DROP TABLE b;`) share
//! one line number, so a suppression intended only for the first spilled
//! onto the second. A single statement spanning multiple lines still gets
//! exactly one index — the annotation attaches at the line where the
//! statement begins, and every later line of that same statement carries no
//! `pending` set of its own to (re-)attach.
//!
//! The statement index is computed the same way a caller iterating
//! [`sqlparser::parser::Parser::parse_sql`]'s output would number statements:
//! it advances by one for every top-level `;` encountered. A caller must
//! look a suppression up by that same 0-based enumeration index (`idx` in
//! `for (idx, stmt) in statements.iter().enumerate()`), not by a computed
//! source line.

use ahash::{AHashMap, AHashSet};

/// The literal prefix that introduces a suppression annotation.
const ANNOTATION_PREFIX: &str = "-- scythe-audit: ignore[";

/// Set of per-statement rule-ID suppressions parsed from inline annotations.
#[derive(Debug, Default)]
pub struct SuppressionSet {
    /// Map: 0-based statement index (in file order) → set of suppressed rule IDs.
    by_statement: AHashMap<usize, AHashSet<String>>,
}

impl SuppressionSet {
    /// Parse a complete SQL string and build the suppression set from every
    /// `-- scythe-audit: ignore[…]` annotation found in it.
    pub fn parse(sql: &str) -> Self {
        let mut set = Self::default();

        let lines: Vec<&str> = sql.split('\n').collect();
        let n = lines.len();

        let mut pending: AHashSet<String> = AHashSet::new();
        let mut stmt_index: usize = 0;

        let mut i = 0;
        while i < n {
            let trimmed = lines[i].trim();
            if trimmed.is_empty() {
                pending.clear();
                i += 1;
                continue;
            }

            if let Some(ids) = try_parse_annotation(trimmed) {
                pending.extend(ids);
                i += 1;
                continue;
            }

            if trimmed.starts_with("--") {
                i += 1;
                continue;
            }

            // A content line: it belongs to the statement at `stmt_index`.
            // Attach any pending annotation to that statement exactly once
            // -- later lines of the same (possibly multi-line) statement
            // have no pending set left to attach.
            if !pending.is_empty() {
                let ids_to_attach: AHashSet<String> = pending.drain().collect();
                set.by_statement.entry(stmt_index).or_default().extend(ids_to_attach);
            }

            // Advance the statement counter once per top-level `;` crossed
            // on this line. A statement that shares a line with an earlier,
            // annotated statement (`DROP TABLE a; DROP TABLE b;`) must land
            // on its own, later index rather than inheriting the first
            // statement's suppression.
            stmt_index += lines[i].matches(';').count();

            i += 1;
        }

        set
    }

    /// Return `true` if `rule_id` is suppressed on the statement at
    /// `stmt_index` (0-based, in file order — see the module docs).
    pub fn is_suppressed(&self, rule_id: &str, stmt_index: usize) -> bool {
        self.by_statement
            .get(&stmt_index)
            .is_some_and(|ids| ids.contains(rule_id))
    }

    /// Return `true` if no suppressions are recorded.
    pub fn is_empty(&self) -> bool {
        self.by_statement.is_empty()
    }
}

/// Attempt to parse a trimmed line as a `-- scythe-audit: ignore[ID,...]`
/// annotation.  Returns `Some(ids)` on success; `None` if the line is not a
/// matching annotation or the annotation is malformed.
fn try_parse_annotation(trimmed: &str) -> Option<Vec<String>> {
    let rest = trimmed.strip_prefix(ANNOTATION_PREFIX)?;

    let close = rest.find(']')?;
    let id_part = &rest[..close];

    if id_part.is_empty() {
        return None;
    }

    let ids: Vec<String> = id_part.split(',').map(|s| s.trim().to_string()).collect();

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_rule_ignore_suppresses_next_statement() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON users TO bob;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC02", 0));
        assert!(!set.is_suppressed("SC-SEC01", 0));
    }

    #[test]
    fn multi_rule_ignore_suppresses_both_ids() {
        let sql = "-- scythe-audit: ignore[SC-SEC01,SC-SEC02]\nSELECT 1;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC01", 0));
        assert!(set.is_suppressed("SC-SEC02", 0));
        assert!(!set.is_suppressed("SC-SEC03", 0));
    }

    #[test]
    fn reason_clause_is_parsed_and_discarded() {
        let sql = r#"-- scythe-audit: ignore[SC-SEC01] reason="vetted"
SELECT pg_read_file('foo');"#;
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC01", 0));
    }

    #[test]
    fn stacked_annotations_union_ids() {
        let sql = "-- scythe-audit: ignore[SC-SEC01]\n-- scythe-audit: ignore[SC-SEC02]\nSELECT 1;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC01", 0));
        assert!(set.is_suppressed("SC-SEC02", 0));
    }

    #[test]
    fn multi_line_statement_suppressed_at_single_statement_index() {
        let sql = "-- scythe-audit: ignore[SC-SEC08]\nSELECT *\nFROM a, b\nWHERE a.id = b.id;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC08", 0));
        // There is only ever one statement here, so index 1 is meaningless,
        // but it must not spuriously appear suppressed either.
        assert!(!set.is_suppressed("SC-SEC08", 1));
    }

    #[test]
    fn blank_line_terminates_suppression_scope() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON a TO x;\n\nGRANT ALL ON b TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC02", 0));
        assert!(!set.is_suppressed("SC-SEC02", 1));
    }

    #[test]
    fn semicolon_terminates_statement() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON a TO x;\nGRANT ALL ON b TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC02", 0));
        assert!(!set.is_suppressed("SC-SEC02", 1));
    }

    /// Regression for #140: two statements sharing one physical line must
    /// resolve to two different statement indices, so an annotation meant
    /// for the first does not silently cover the second.
    #[test]
    fn two_statements_on_one_line_only_first_is_suppressed() {
        let sql = "-- scythe-audit: ignore[SC-MIG01]\nDROP TABLE a; DROP TABLE b;";
        let set = SuppressionSet::parse(sql);
        assert!(
            set.is_suppressed("SC-MIG01", 0),
            "the first statement on the annotated line must be suppressed"
        );
        assert!(
            !set.is_suppressed("SC-MIG01", 1),
            "the second statement sharing the line must NOT inherit the first's suppression"
        );
    }

    /// Regression for #140's documentation/behavior mismatch: the module
    /// doc used to claim an annotation "attaches to the next non-blank,
    /// non-comment line", but the code cleared `pending` on any blank line
    /// first -- so a blank line between the annotation and the statement
    /// silently dropped the suppression. That is now the documented,
    /// intentional contract: verify it holds.
    #[test]
    fn blank_line_between_annotation_and_statement_drops_the_suppression() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\n\nGRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(
            !set.is_suppressed("SC-SEC02", 0),
            "a blank line between the annotation and the statement must drop the suppression"
        );
    }

    #[test]
    fn annotation_at_eof_with_no_following_statement_is_harmless() {
        let sql = "SELECT 1;\n-- scythe-audit: ignore[SC-SEC02]";
        let set = SuppressionSet::parse(sql);
        assert!(!set.is_suppressed("SC-SEC02", 0));
        assert!(!set.is_suppressed("SC-SEC02", 1));
    }

    #[test]
    fn ordinary_comment_does_not_consume_pending_suppression() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\n-- just a comment\nGRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC02", 0));
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
        assert!(!set.is_suppressed("SC-SEC99", 0));
    }

    #[test]
    fn is_suppressed_returns_true_for_known_id_on_suppressed_statement() {
        let sql = "-- scythe-audit: ignore[SC-SEC02]\nGRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_suppressed("SC-SEC02", 0));
    }

    #[test]
    fn unsuppressed_statement_returns_false() {
        let sql = "GRANT ALL ON x TO y;";
        let set = SuppressionSet::parse(sql);
        assert!(!set.is_suppressed("SC-SEC02", 0));
    }

    #[test]
    fn is_empty_when_no_annotations() {
        let sql = "SELECT 1; SELECT 2;";
        let set = SuppressionSet::parse(sql);
        assert!(set.is_empty());
    }
}
