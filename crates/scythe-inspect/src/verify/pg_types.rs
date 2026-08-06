//! Mapping from PostgreSQL catalog types to scythe's neutral type vocabulary.
//!
//! This is the inverse of [`scythe_core`'s `sql_type_to_neutral`], which maps
//! DDL type *names* to neutral types.  Here the input is what the server
//! reports over the wire for a prepared statement, so the mapping is keyed on
//! `pg_type` rather than on parsed DDL.
//!
//! Only types that scythe's neutral vocabulary can express are mapped.
//! Anything else yields `None`, which the verifier treats as "cannot compare"
//! rather than as a mismatch — a type we have no opinion about must never
//! produce a false positive.

use tokio_postgres::types::{Kind, Type};

/// Convert a PostgreSQL type into scythe's neutral type name.
///
/// Returns `None` when the type has no neutral equivalent, in which case the
/// caller should skip the comparison rather than report a mismatch.
///
/// Arrays render as `array<T>` and enums as `enum::name`, matching the forms
/// produced by the static analyzer.
pub fn neutral_type_for(pg_type: &Type) -> Option<String> {
    if let Kind::Array(inner) = pg_type.kind() {
        let inner_neutral = neutral_type_for(inner)?;
        return Some(format!("array<{inner_neutral}>"));
    }

    if let Kind::Enum(_) = pg_type.kind() {
        return Some(format!("enum::{}", pg_type.name()));
    }

    let neutral = match *pg_type {
        Type::BOOL => "bool",
        Type::INT2 => "int16",
        Type::INT4 => "int32",
        Type::INT8 => "int64",
        Type::FLOAT4 => "float32",
        Type::FLOAT8 => "float64",
        Type::NUMERIC => "decimal",
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::CHAR => "string",
        Type::BYTEA => "bytes",
        Type::UUID => "uuid",
        Type::DATE => "date",
        Type::TIME => "time",
        Type::TIMETZ => "time_tz",
        Type::TIMESTAMP => "datetime",
        Type::TIMESTAMPTZ => "datetime_tz",
        Type::INTERVAL => "interval",
        Type::JSON | Type::JSONB => "json",
        Type::INET | Type::CIDR | Type::MACADDR => "inet",
        Type::INT4_RANGE => "range<int32>",
        Type::INT8_RANGE => "range<int64>",
        Type::NUM_RANGE => "range<decimal>",
        Type::DATE_RANGE => "range<date>",
        Type::TS_RANGE => "range<datetime>",
        Type::TSTZ_RANGE => "range<datetime_tz>",
        _ => return None,
    };

    Some(neutral.to_string())
}

/// Whether a statically-inferred neutral type is compatible with what the
/// server reported.
///
/// This is deliberately more permissive than string equality.  Static
/// inference works from DDL and cannot always recover the exact width the
/// server will choose — an integer literal, a `count(*)`, or an untyped
/// parameter are all cases where scythe and PostgreSQL can legitimately
/// disagree on width while agreeing on meaning.  Flagging those would bury the
/// real mismatches, which are the point of the check.
pub fn types_are_compatible(inferred: &str, reported: &str) -> bool {
    if inferred == reported {
        return true;
    }

    // Integer and float widths: the server's choice is authoritative but a
    // narrower static guess is not a defect worth reporting.
    const INTEGERS: [&str; 3] = ["int16", "int32", "int64"];
    const FLOATS: [&str; 3] = ["float32", "float64", "decimal"];
    const STRINGS: [&str; 4] = ["string", "uuid", "json", "inet"];

    let both_in = |set: &[&str]| set.contains(&inferred) && set.contains(&reported);

    if both_in(&INTEGERS) || both_in(&FLOATS) || both_in(&STRINGS) {
        return true;
    }

    // An enum is carried as a string on the wire by several drivers, and a
    // domain resolves to its base type, so treat those as agreeing.
    if inferred.starts_with("enum::") && reported == "string" {
        return true;
    }
    if reported.starts_with("enum::") && inferred == "string" {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_scalars_to_neutral_names() {
        assert_eq!(neutral_type_for(&Type::BOOL).as_deref(), Some("bool"));
        assert_eq!(neutral_type_for(&Type::INT4).as_deref(), Some("int32"));
        assert_eq!(neutral_type_for(&Type::INT8).as_deref(), Some("int64"));
        assert_eq!(neutral_type_for(&Type::TEXT).as_deref(), Some("string"));
        assert_eq!(neutral_type_for(&Type::NUMERIC).as_deref(), Some("decimal"));
        assert_eq!(neutral_type_for(&Type::TIMESTAMPTZ).as_deref(), Some("datetime_tz"));
        assert_eq!(neutral_type_for(&Type::UUID).as_deref(), Some("uuid"));
    }

    #[test]
    fn maps_arrays_using_the_analyzer_form() {
        assert_eq!(neutral_type_for(&Type::INT4_ARRAY).as_deref(), Some("array<int32>"));
        assert_eq!(neutral_type_for(&Type::TEXT_ARRAY).as_deref(), Some("array<string>"));
    }

    /// A type with no neutral equivalent must not be reported as a mismatch —
    /// returning `None` tells the verifier to skip the comparison.
    #[test]
    fn returns_none_for_types_outside_the_neutral_vocabulary() {
        assert_eq!(neutral_type_for(&Type::POINT), None);
        assert_eq!(neutral_type_for(&Type::XML), None);
    }

    #[test]
    fn identical_types_are_compatible() {
        assert!(types_are_compatible("int32", "int32"));
        assert!(types_are_compatible("array<string>", "array<string>"));
    }

    /// Width differences within a family are legitimate disagreements between
    /// DDL-driven inference and the server's choice, not defects.
    #[test]
    fn width_differences_within_a_family_are_compatible() {
        assert!(types_are_compatible("int32", "int64"));
        assert!(types_are_compatible("float32", "float64"));
        assert!(types_are_compatible("decimal", "float64"));
    }

    #[test]
    fn enums_are_compatible_with_strings() {
        assert!(types_are_compatible("enum::status", "string"));
        assert!(types_are_compatible("string", "enum::status"));
    }

    #[test]
    fn genuinely_different_types_are_incompatible() {
        assert!(!types_are_compatible("int32", "string"));
        assert!(!types_are_compatible("bool", "int32"));
        assert!(!types_are_compatible("date", "datetime"));
        assert!(!types_are_compatible("string", "array<string>"));
    }
}
