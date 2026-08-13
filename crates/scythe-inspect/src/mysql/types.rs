//! Mapping from MySQL/MariaDB `information_schema.COLUMNS` type names to
//! scythe's neutral type vocabulary.
//!
//! `DATA_TYPE` (short: `"int"`, `"varchar"`, `"tinyint"`, …) is the primary
//! signal; `COLUMN_TYPE` (full: `"tinyint(1)"`, `"int unsigned"`, `"bit(1)"`,
//! …) is only consulted where `DATA_TYPE` alone is ambiguous — MySQL's
//! `tinyint(1)`-means-boolean and `bit(1)`-means-boolean conventions, neither
//! of which is a distinct `DATA_TYPE` of its own.

/// Convert a MySQL/MariaDB column's `information_schema` type into scythe's
/// neutral type name.
///
/// Returns `None` for a `DATA_TYPE` this mapping has no opinion about — a
/// spatial type (`geometry`, `point`, …), for instance — exactly the same
/// contract [`neutral_type_for`](crate::verify::pg_types::neutral_type_for)
/// uses for PostgreSQL: "no neutral equivalent" is not evidence of anything
/// and must never be reported as a mismatch by a caller that compares it.
///
/// `enum` and `set` map to `string`: unlike PostgreSQL's `CREATE TYPE ... AS
/// ENUM`, MySQL's `ENUM(...)`/`SET(...)` are declared inline on the column
/// with no separate named type to key a comparison on, so there is nothing
/// for an `enum::name` form to name. Several drivers already carry
/// PostgreSQL enum values as plain strings on the wire (see
/// `types_are_compatible`), so this is consistent with an existing tolerance
/// rather than a new one.
pub fn neutral_type_for_mysql(data_type: &str, column_type: &str) -> Option<String> {
    let data_type = data_type.trim().to_ascii_lowercase();
    let column_type = column_type.trim().to_ascii_lowercase();

    let neutral = match data_type.as_str() {
        // MySQL has no dedicated boolean storage type; `BOOLEAN`/`BOOL` are
        // aliases for `TINYINT(1)`, and `information_schema` reports the
        // alias back as `tinyint`. `COLUMN_TYPE` is what still says `(1)`.
        "tinyint" if column_type.starts_with("tinyint(1)") => "bool",
        "tinyint" => "int16",
        "smallint" | "year" => "int16",
        "mediumint" | "int" | "integer" => "int32",
        "bigint" => "int64",
        "decimal" | "numeric" => "decimal",
        "float" => "float32",
        "double" | "double precision" | "real" => "float64",
        "char" | "varchar" | "text" | "tinytext" | "mediumtext" | "longtext" | "enum" | "set" => "string",
        "binary" | "varbinary" | "blob" | "tinyblob" | "mediumblob" | "longblob" => "bytes",
        // `BIT(1)` is MySQL's other common boolean spelling; any wider `BIT(n)`
        // is a genuine bit-string and maps to `bytes` like the binary family.
        "bit" if column_type == "bit(1)" => "bool",
        "bit" => "bytes",
        "date" => "date",
        "time" => "time",
        "datetime" => "datetime",
        // `TIMESTAMP` is stored as UTC and converted to the session time zone
        // on read, which is the behaviour `datetime_tz` exists to describe;
        // `DATETIME` carries no time zone information at all.
        "timestamp" => "datetime_tz",
        "json" => "json",
        "boolean" | "bool" => "bool",
        _ => return None,
    };

    Some(neutral.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tinyint_one_to_bool_and_other_widths_to_int16() {
        assert_eq!(
            neutral_type_for_mysql("tinyint", "tinyint(1)"),
            Some("bool".to_string())
        );
        assert_eq!(
            neutral_type_for_mysql("tinyint", "tinyint(1) unsigned"),
            Some("bool".to_string())
        );
        assert_eq!(
            neutral_type_for_mysql("tinyint", "tinyint(4)"),
            Some("int16".to_string())
        );
    }

    #[test]
    fn maps_integer_family_by_width() {
        assert_eq!(
            neutral_type_for_mysql("smallint", "smallint"),
            Some("int16".to_string())
        );
        assert_eq!(
            neutral_type_for_mysql("mediumint", "mediumint"),
            Some("int32".to_string())
        );
        assert_eq!(neutral_type_for_mysql("int", "int"), Some("int32".to_string()));
        assert_eq!(neutral_type_for_mysql("int", "int unsigned"), Some("int32".to_string()));
        assert_eq!(neutral_type_for_mysql("bigint", "bigint"), Some("int64".to_string()));
        assert_eq!(neutral_type_for_mysql("year", "year(4)"), Some("int16".to_string()));
    }

    #[test]
    fn maps_floating_and_exact_numeric_types() {
        assert_eq!(neutral_type_for_mysql("float", "float"), Some("float32".to_string()));
        assert_eq!(neutral_type_for_mysql("double", "double"), Some("float64".to_string()));
        assert_eq!(
            neutral_type_for_mysql("decimal", "decimal(10,2)"),
            Some("decimal".to_string())
        );
    }

    #[test]
    fn maps_text_family_and_enum_set_to_string() {
        for (data_type, column_type) in [
            ("varchar", "varchar(255)"),
            ("char", "char(10)"),
            ("text", "text"),
            ("enum", "enum('a','b')"),
            ("set", "set('a','b')"),
        ] {
            assert_eq!(
                neutral_type_for_mysql(data_type, column_type),
                Some("string".to_string()),
                "{data_type}"
            );
        }
    }

    #[test]
    fn maps_binary_family_to_bytes() {
        for data_type in ["binary", "varbinary", "blob", "tinyblob", "mediumblob", "longblob"] {
            assert_eq!(
                neutral_type_for_mysql(data_type, data_type),
                Some("bytes".to_string()),
                "{data_type}"
            );
        }
    }

    #[test]
    fn maps_bit_one_to_bool_and_wider_bit_to_bytes() {
        assert_eq!(neutral_type_for_mysql("bit", "bit(1)"), Some("bool".to_string()));
        assert_eq!(neutral_type_for_mysql("bit", "bit(8)"), Some("bytes".to_string()));
    }

    #[test]
    fn maps_temporal_types() {
        assert_eq!(neutral_type_for_mysql("date", "date"), Some("date".to_string()));
        assert_eq!(neutral_type_for_mysql("time", "time"), Some("time".to_string()));
        assert_eq!(
            neutral_type_for_mysql("datetime", "datetime"),
            Some("datetime".to_string())
        );
        assert_eq!(
            neutral_type_for_mysql("timestamp", "timestamp"),
            Some("datetime_tz".to_string())
        );
    }

    #[test]
    fn maps_json_and_boolean_aliases() {
        assert_eq!(neutral_type_for_mysql("json", "json"), Some("json".to_string()));
        assert_eq!(neutral_type_for_mysql("boolean", "boolean"), Some("bool".to_string()));
    }

    #[test]
    fn returns_none_for_a_type_outside_the_neutral_vocabulary() {
        assert_eq!(neutral_type_for_mysql("geometry", "geometry"), None);
        assert_eq!(neutral_type_for_mysql("point", "point"), None);
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(
            neutral_type_for_mysql("INT", "INT"),
            neutral_type_for_mysql("int", "int")
        );
        assert_eq!(
            neutral_type_for_mysql("TINYINT", "TINYINT(1)"),
            neutral_type_for_mysql("tinyint", "tinyint(1)")
        );
    }
}
