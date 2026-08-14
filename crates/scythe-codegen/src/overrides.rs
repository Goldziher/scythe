use ahash::AHashSet;

use scythe_core::analyzer::AnalyzedColumn;

/// A type override that replaces the inferred neutral type for a column or SQL type.
///
/// Overrides are evaluated in order: the first match wins. A `column` match
/// (e.g. `"users.metadata"`) takes priority over a `db_type` match when both
/// fields are set on the same override entry, but a `column` that fails to
/// match falls through to `db_type` on the same entry rather than making the
/// whole entry a no-op (#189) -- the two predicates degrade independently,
/// they don't gate each other.
#[derive(Debug, Clone)]
pub struct TypeOverride {
    /// Fully-qualified column reference in `"table.column"` format.
    pub column: Option<String>,
    /// SQL type name (matched case-insensitively against the column's neutral type).
    pub db_type: Option<String>,
    /// Target neutral type to substitute (e.g. `"string"`, `"json"`).
    pub neutral_type: Option<String>,
}

impl TypeOverride {
    /// Check if this override matches a column.
    ///
    /// `column_match` is `"table_name.column_name"` (empty string if unknown).
    /// `col_neutral_type` is the neutral type inferred by the analyzer.
    pub fn matches(&self, column_match: &str, col_neutral_type: &str) -> bool {
        if let Some(ref col) = self.column
            && col == column_match
        {
            return true;
        }
        if let Some(ref dt) = self.db_type {
            return dt.eq_ignore_ascii_case(col_neutral_type);
        }
        false
    }
}

/// Every `"table.column"` reference actually present across a set of resolved columns,
/// keyed on [`AnalyzedColumn::source_relation`] -- the analyzer's per-column owning
/// relation (#189) -- rather than a query-level `source_table`. A column with no single
/// owning relation (`source_relation: None`: a computed expression, a literal, a function
/// result) contributes nothing, which is exactly what a qualified `column` override can
/// never legitimately target either.
pub fn column_references<'a>(columns: impl Iterator<Item = &'a AnalyzedColumn>) -> AHashSet<String> {
    columns
        .filter_map(|col| {
            col.source_relation
                .as_deref()
                .map(|relation| format!("{relation}.{}", col.name))
        })
        .collect()
}

/// `column` overrides whose target names no `"table.column"` pair in `known_references`.
///
/// This is the other half of #189: fixing `column_match` to bind per-column stops the
/// override from silently doing nothing, but a `column` that names a table or column that
/// plain doesn't exist in this schema (a typo, a renamed table) would otherwise go quiet
/// again the same way -- it just never gets a chance to fire. `db_type`-only entries are
/// never reported here: they have no single column to miss.
pub fn unmatched_column_overrides<'a>(
    overrides: &'a [TypeOverride],
    known_references: &AHashSet<String>,
) -> Vec<&'a TypeOverride> {
    overrides
        .iter()
        .filter(|o| matches!(&o.column, Some(c) if !known_references.contains(c)))
        .collect()
}

/// Find the first override that matches a column and return its neutral type.
///
/// Returns `None` when no override matches — the caller should fall through to
/// the default type-resolution path.
pub fn find_override<'a>(overrides: &'a [TypeOverride], column_match: &str, col_neutral_type: &str) -> Option<&'a str> {
    overrides.iter().find_map(|o| {
        if o.matches(column_match, col_neutral_type) {
            o.neutral_type.as_deref()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_override_matches() {
        let o = TypeOverride {
            column: Some("users.metadata".to_string()),
            db_type: None,
            neutral_type: Some("json".to_string()),
        };
        assert!(o.matches("users.metadata", "jsonb"));
        assert!(!o.matches("posts.metadata", "jsonb"));
    }

    #[test]
    fn test_db_type_override_matches() {
        let o = TypeOverride {
            column: None,
            db_type: Some("ltree".to_string()),
            neutral_type: Some("string".to_string()),
        };
        assert!(o.matches("", "ltree"));
        assert!(o.matches("any.col", "LTREE"));
        assert!(!o.matches("any.col", "text"));
    }

    #[test]
    fn test_column_takes_priority_over_db_type() {
        let o = TypeOverride {
            column: Some("users.name".to_string()),
            db_type: Some("text".to_string()),
            neutral_type: Some("custom".to_string()),
        };
        assert!(o.matches("users.name", "int32"));
        assert!(!o.matches("other.name", "int32"));
    }

    /// Must fail before the fix: `matches` used to return `false` as soon as `column` was
    /// set and didn't match, without ever checking `db_type` -- so a combined entry went
    /// completely silent for every column except the one literally named in `column`,
    /// instead of degrading to the type-level rule (#189).
    #[test]
    fn test_column_miss_falls_through_to_db_type() {
        let o = TypeOverride {
            column: Some("users.name".to_string()),
            db_type: Some("text".to_string()),
            neutral_type: Some("custom".to_string()),
        };
        assert!(o.matches("other.name", "text"));
        assert!(o.matches("other.name", "TEXT"));
    }

    #[test]
    fn test_find_override_first_match_wins() {
        let overrides = vec![
            TypeOverride {
                column: Some("users.metadata".to_string()),
                db_type: None,
                neutral_type: Some("json".to_string()),
            },
            TypeOverride {
                column: None,
                db_type: Some("jsonb".to_string()),
                neutral_type: Some("string".to_string()),
            },
        ];
        assert_eq!(find_override(&overrides, "users.metadata", "jsonb"), Some("json"));
        assert_eq!(find_override(&overrides, "posts.data", "jsonb"), Some("string"));
        assert_eq!(find_override(&overrides, "posts.data", "text"), None);
    }

    #[test]
    fn test_find_override_empty_list() {
        assert_eq!(find_override(&[], "users.id", "int32"), None);
    }

    fn column(name: &str, relation: Option<&str>) -> AnalyzedColumn {
        AnalyzedColumn {
            name: name.to_string(),
            neutral_type: "string".to_string(),
            source_relation: relation.map(str::to_string),
            ..Default::default()
        }
    }

    /// A computed expression, literal, or function result -- `source_relation: None` --
    /// contributes no `"table.column"` reference: it is exactly what a qualified `column`
    /// override can never legitimately target either (#189).
    #[test]
    fn test_column_references_skips_columns_with_no_source_relation() {
        let columns = [column("id", Some("users")), column("total", None)];
        let refs = column_references(columns.iter());
        assert_eq!(refs.len(), 1);
        assert!(refs.contains("users.id"));
    }

    /// Must fail before the fix existed: a `column` override naming a table or column
    /// present nowhere in `known_references` (a typo) must be reported; a matching one and
    /// a `db_type`-only one (no single column to miss) must not be.
    #[test]
    fn test_unmatched_column_overrides_reports_only_the_typo() {
        let known = column_references([column("id", Some("users"))].iter());
        let overrides = vec![
            TypeOverride {
                column: Some("users.id".to_string()),
                db_type: None,
                neutral_type: Some("string".to_string()),
            },
            TypeOverride {
                column: Some("users.emial".to_string()),
                db_type: None,
                neutral_type: Some("string".to_string()),
            },
            TypeOverride {
                column: None,
                db_type: Some("jsonb".to_string()),
                neutral_type: Some("string".to_string()),
            },
        ];
        let unmatched = unmatched_column_overrides(&overrides, &known);
        assert_eq!(unmatched.len(), 1);
        assert_eq!(unmatched[0].column.as_deref(), Some("users.emial"));
    }
}
