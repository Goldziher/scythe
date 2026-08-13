//! Mapping from a SQLite column's *declared* type to scythe's neutral type
//! vocabulary.
//!
//! SQLite does not enforce the type a `CREATE TABLE` declares: every column
//! (other than an `INTEGER PRIMARY KEY` rowid alias) can hold any storage
//! class regardless of its declaration. What `PRAGMA table_info` reports is
//! the declared type *string* verbatim — `"INT"`, `"VARCHAR(255)"`,
//! `"BLOB"`, even a made-up word — and SQLite's own type affinity rules
//! (<https://www.sqlite.org/datatype3.html#type_affinity>) turn that string
//! into one of five storage affinities by substring match, in this order:
//! contains `"INT"` → INTEGER, contains `"CHAR"`/`"CLOB"`/`"TEXT"` → TEXT,
//! contains `"BLOB"` or empty → BLOB, contains `"REAL"`/`"FLOA"`/`"DOUB"` →
//! REAL, anything else → NUMERIC.
//!
//! [`neutral_type_for_sqlite`] follows that same order, with two additions
//! layered on top purely for scythe's benefit: `"BOOLEAN"`/`"BOOL"` maps to
//! `bool` and `"DATE"`/`"DATETIME"`/`"TIMESTAMP"` map to `date`/`datetime`
//! rather than falling through to the generic NUMERIC catch-all. Raw SQLite
//! affinity would call all of those NUMERIC, which is correct for how SQLite
//! stores them but useless for generating a typed field. Both additions are
//! checked before the INTEGER/TEXT rules so a hypothetical `"BOOLINT"` still
//! prefers the more specific, more useful answer.
//!
//! This is inherently a best-effort mapping — the declared type is a hint,
//! not a guarantee — documented here rather than treated as an oversight to
//! fix: there is no stronger signal SQLite exposes to fix it with.

/// Convert a SQLite column's declared type string into scythe's neutral type
/// name. Always succeeds: SQLite's own affinity rules are total, falling
/// through to `"decimal"` (the NUMERIC affinity) for anything unrecognised,
/// so there is no `None` case the way there is for PostgreSQL OIDs scythe has
/// no opinion about.
pub fn neutral_type_for_sqlite(declared_type: &str) -> String {
    let upper = declared_type.trim().to_ascii_uppercase();

    if upper.is_empty() {
        // BLOB affinity: SQLite assigns this to a column with no declared
        // type at all.
        return "bytes".to_string();
    }

    // scythe-specific refinements, checked ahead of raw SQLite affinity.
    if upper.contains("BOOL") {
        return "bool".to_string();
    }
    if upper.contains("DATETIME") || upper.contains("TIMESTAMP") {
        return "datetime".to_string();
    }
    if upper.contains("DATE") {
        return "date".to_string();
    }

    // Raw SQLite type-affinity rules, in their documented order.
    if upper.contains("INT") {
        // SQLite's INTEGER storage class is always a signed 8-byte value
        // regardless of a narrower declaration like `SMALLINT`, so this maps
        // to the widest neutral integer rather than guessing a width the
        // engine itself does not enforce.
        return "int64".to_string();
    }
    if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        return "string".to_string();
    }
    if upper.contains("BLOB") {
        return "bytes".to_string();
    }
    if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        return "float64".to_string();
    }

    // NUMERIC affinity catch-all: DECIMAL, NUMERIC, and anything else SQLite
    // would also fall through on.
    "decimal".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_integer_affinity_types_to_int64() {
        for declared in ["INTEGER", "INT", "TINYINT", "SMALLINT", "BIGINT", "int"] {
            assert_eq!(neutral_type_for_sqlite(declared), "int64", "{declared}");
        }
    }

    #[test]
    fn maps_text_affinity_types_to_string() {
        for declared in ["TEXT", "VARCHAR(255)", "CHAR(10)", "CLOB", "NVARCHAR"] {
            assert_eq!(neutral_type_for_sqlite(declared), "string", "{declared}");
        }
    }

    #[test]
    fn maps_real_affinity_types_to_float64() {
        for declared in ["REAL", "DOUBLE", "DOUBLE PRECISION", "FLOAT"] {
            assert_eq!(neutral_type_for_sqlite(declared), "float64", "{declared}");
        }
    }

    #[test]
    fn maps_blob_and_empty_declarations_to_bytes() {
        assert_eq!(neutral_type_for_sqlite("BLOB"), "bytes");
        assert_eq!(neutral_type_for_sqlite(""), "bytes");
    }

    #[test]
    fn maps_numeric_catch_all_to_decimal() {
        for declared in ["NUMERIC", "DECIMAL(10,2)", "made_up_type"] {
            assert_eq!(neutral_type_for_sqlite(declared), "decimal", "{declared}");
        }
    }

    /// The scythe-specific refinements: raw SQLite affinity would call all
    /// three of these NUMERIC, which is technically correct and useless.
    #[test]
    fn refines_boolean_and_date_declarations_past_raw_affinity() {
        assert_eq!(neutral_type_for_sqlite("BOOLEAN"), "bool");
        assert_eq!(neutral_type_for_sqlite("BOOL"), "bool");
        assert_eq!(neutral_type_for_sqlite("DATE"), "date");
        assert_eq!(neutral_type_for_sqlite("DATETIME"), "datetime");
        assert_eq!(neutral_type_for_sqlite("TIMESTAMP"), "datetime");
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(neutral_type_for_sqlite("integer"), neutral_type_for_sqlite("INTEGER"));
        assert_eq!(neutral_type_for_sqlite("boolean"), neutral_type_for_sqlite("BOOLEAN"));
    }
}
