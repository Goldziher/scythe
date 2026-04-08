/// A type override that replaces the inferred neutral type for a column or SQL type.
///
/// Overrides are evaluated in order: the first match wins. A `column` match
/// (e.g. `"users.metadata"`) takes priority over a `db_type` match when both
/// fields are set on the same override entry.
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
        if let Some(ref col) = self.column {
            return col == column_match;
        }
        if let Some(ref dt) = self.db_type {
            return dt.eq_ignore_ascii_case(col_neutral_type);
        }
        false
    }
}

/// Find the first override that matches a column and return its neutral type.
///
/// Returns `None` when no override matches — the caller should fall through to
/// the default type-resolution path.
pub fn find_override<'a>(
    overrides: &'a [TypeOverride],
    column_match: &str,
    col_neutral_type: &str,
) -> Option<&'a str> {
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
        // column match succeeds regardless of db_type
        assert!(o.matches("users.name", "int32"));
        // column mismatch means no match (db_type not checked when column is set)
        assert!(!o.matches("other.name", "text"));
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
        // column match wins over db_type match
        assert_eq!(
            find_override(&overrides, "users.metadata", "jsonb"),
            Some("json")
        );
        // db_type fallback for non-column-matched columns
        assert_eq!(
            find_override(&overrides, "posts.data", "jsonb"),
            Some("string")
        );
        // no match
        assert_eq!(find_override(&overrides, "posts.data", "text"), None);
    }

    #[test]
    fn test_find_override_empty_list() {
        assert_eq!(find_override(&[], "users.id", "int32"), None);
    }
}
