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

    // A domain is a constrained alias over a base type (e.g. `CREATE DOMAIN
    // us_zip AS text CHECK (...)`) and carries no representation of its own
    // on the wire — resolve it to whatever its base type maps to, so a
    // domain over `uuid` compares the same way a plain `uuid` column would.
    if let Kind::Domain(base) = pg_type.kind() {
        return neutral_type_for(base);
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
        // ~keep Mirrors `scythe_core::analyzer::type_conversion::sql_type_to_neutral`,
        // which collapses `macaddr` into `inet` too. Kept consistent
        // deliberately rather than fixed here in isolation — see that
        // module for the same wart. `types_are_compatible` below does not
        // treat `inet` as interchangeable with anything else, so a real
        // macaddr/inet confusion at the SQL layer still surfaces as a
        // mismatch against whatever neutral type static inference produced.
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
/// This is deliberately more permissive than string equality, but only in
/// the specific places where static inference genuinely cannot recover what
/// the server will report while still agreeing on meaning:
///
/// - **Integer widths** (`int16`/`int32`/`int64`) against each other, and
///   **float widths** (`float32`/`float64`) against each other: the server's
///   choice is authoritative and a narrower static guess (an integer
///   literal, a `count(*)`, an untyped parameter) is not a defect.
/// - **Enum vs `string`**: several drivers carry enum values as strings on
///   the wire, so the two are treated as agreeing in either direction.
/// - **Any type widening to `string`**: `string` is scythe's fallback when
///   inference cannot pin down a more specific type (an untyped parameter,
///   an expression it doesn't specialize). If the server then reports a more
///   precise wire type — `uuid`, `json`, `inet` — that is inference being
///   coarse, not a defect, so it is accepted. This direction only: if
///   inference *committed* to `uuid`/`json`/`inet` (typically because DDL
///   declared the column as such) but the server reports plain `string`,
///   that is exactly the "wrongly mapped catalog type" case this check
///   exists to catch (SC-VER03/SC-VER05), so it is flagged as a mismatch.
///
/// `uuid`, `json`, and `inet` are otherwise held to exact equality against
/// each other — unlike integer/float widths, these are not narrower or wider
/// views of the same value, they are different types on the wire, and
/// conflating them would hide real catalog mis-mappings. `decimal` is
/// likewise held to exact equality against `float32`/`float64`: `NUMERIC` is
/// exact/arbitrary-precision while `float4`/`float8` are binary
/// floating-point, a precision difference that matters and that static
/// inference does have enough information to get right (the DDL says which
/// one it is), so treating them as interchangeable would bury a real
/// mismatch rather than a width guess.
pub fn types_are_compatible(inferred: &str, reported: &str) -> bool {
    // Both sides first go through the one normalisation point, so a difference
    // that is only a difference in spelling — `enum::public.status` from a
    // `pg_dump` column declaration against the `enum::status` `pg_type` always
    // reports, or a `name` column the DDL side has no arm for — is settled
    // before any tolerance below is consulted. Without this, SC-VER03 fired on
    // every schema-qualified enum column in an otherwise exact match.
    let inferred = crate::neutral::normalize_neutral_type(inferred);
    let reported = crate::neutral::normalize_neutral_type(reported);
    let (inferred, reported) = (inferred.as_ref(), reported.as_ref());

    if inferred == reported {
        return true;
    }

    const INTEGERS: [&str; 3] = ["int16", "int32", "int64"];
    const FLOATS: [&str; 2] = ["float32", "float64"];
    const STRING_WIDENABLE: [&str; 3] = ["uuid", "json", "inet"];

    if INTEGERS.contains(&inferred) && INTEGERS.contains(&reported) {
        return true;
    }
    if FLOATS.contains(&inferred) && FLOATS.contains(&reported) {
        return true;
    }

    // `string` is the coarse fallback; a more precise reported type is not a
    // defect. The reverse (inferred is the specific type, reported is
    // `string`) is intentionally NOT accepted here — see the doc comment.
    if inferred == "string" && STRING_WIDENABLE.contains(&reported) {
        return true;
    }

    // An enum is carried as a string on the wire by several drivers.
    if inferred.starts_with("enum::") && reported == "string" {
        return true;
    }
    if reported.starts_with("enum::") && inferred == "string" {
        return true;
    }

    // A structurally typed JSON column is still a JSON column on the wire.
    // `json_nested<...>` is what nested-aggregate inference produces for
    // `json_agg`/`row_to_json`, `json_typed<...>` what a user's `@json`
    // annotation produces; PostgreSQL reports both as plain `json`
    // (`row_to_json`/`json_agg` return `json`, and `RowDescription` cannot
    // describe the row shape inside). Holding these to string equality
    // would report a verification failure on *every* such column while the
    // inferred type is not merely compatible but strictly more precise.
    if reported == "json" && (inferred.starts_with("json_nested<") || inferred.starts_with("json_typed<")) {
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
    }

    /// SC-VER03's false positive: static inference renders the enum from the
    /// column's own type spelling, so a `pg_dump` schema declaring
    /// `state public.status` infers `enum::public.status` while the server
    /// reports `enum::status` for the very same column.
    #[test]
    fn should_accept_a_qualified_enum_against_the_bare_enum_the_server_reports() {
        assert!(types_are_compatible("enum::public.status", "enum::status"));
        assert!(types_are_compatible("enum::status", "enum::public.status"));
    }

    /// Stripping the qualifier must not make every enum interchangeable —
    /// two genuinely different enum types are still a mismatch.
    #[test]
    fn should_still_reject_two_different_enums_after_stripping_the_qualifier() {
        assert!(!types_are_compatible("enum::public.status", "enum::mood"));
    }

    /// The array form reaches the comparison whole, so the qualifier has to be
    /// stripped inside the wrapper too.
    #[test]
    fn should_accept_a_qualified_enum_array_against_the_bare_array_the_server_reports() {
        assert!(types_are_compatible("array<enum::public.mood>", "array<enum::mood>"));
    }

    /// A `name` column infers as the raw spelling `name` because
    /// `sql_type_to_neutral` has no arm for it, while the server reports
    /// `Type::NAME`, which this module maps to `string`.
    #[test]
    fn should_accept_a_name_typed_column_against_the_reported_string() {
        assert!(types_are_compatible("name", "string"));
    }

    #[test]
    fn enums_are_compatible_with_strings() {
        assert!(types_are_compatible("enum::status", "string"));
        assert!(types_are_compatible("string", "enum::status"));
    }

    /// `decimal` (`NUMERIC`, exact/arbitrary-precision) and the float widths
    /// (binary floating-point) are genuinely different representations, and
    /// the DDL gives static inference enough information to pick the right
    /// one — unlike an integer literal's width, this is not a case where
    /// inference is legitimately unable to recover the server's choice.
    /// Treating them as interchangeable would bury a real catalog mismatch,
    /// so they are held to exact equality in both directions.
    #[test]
    fn decimal_and_float_are_incompatible_because_precision_is_not_a_width_guess() {
        assert!(!types_are_compatible("decimal", "float32"));
        assert!(!types_are_compatible("float32", "decimal"));
        assert!(!types_are_compatible("decimal", "float64"));
    }

    /// `uuid`, `json`, and `inet` are distinct wire types, not width variants
    /// of one another. Treating them as interchangeable would hide exactly
    /// the "wrongly mapped catalog type" bug SC-VER03 exists to catch — e.g.
    /// a column inferred as `uuid` from DDL but the server actually reports
    /// `json`, which is real schema drift, not an inference gap.
    #[test]
    fn uuid_json_and_inet_are_mutually_incompatible() {
        assert!(!types_are_compatible("uuid", "json"));
        assert!(!types_are_compatible("json", "uuid"));
        assert!(!types_are_compatible("inet", "uuid"));
        assert!(!types_are_compatible("uuid", "inet"));
        assert!(!types_are_compatible("inet", "json"));
    }

    /// Static inference falls back to the generic `string` neutral type when
    /// it cannot pin down anything more specific (an untyped parameter, an
    /// expression it doesn't specialize). If the server then reports a more
    /// precise wire type, that is inference being coarse, not a defect — so
    /// this direction of widening is accepted.
    #[test]
    fn inferred_string_widens_to_uuid_json_or_inet_reported_by_the_server() {
        assert!(types_are_compatible("string", "uuid"));
        assert!(types_are_compatible("string", "json"));
        assert!(types_are_compatible("string", "inet"));
    }

    /// The reverse of the widening above is NOT accepted: if inference
    /// committed to a specific type (typically because DDL declared the
    /// column that way) but the server reports plain `string`, that is a
    /// real catalog mis-mapping, not a gap in inference.
    #[test]
    fn specific_inferred_type_does_not_widen_from_reported_string() {
        assert!(!types_are_compatible("uuid", "string"));
        assert!(!types_are_compatible("json", "string"));
        assert!(!types_are_compatible("inet", "string"));
    }

    /// `scythe check --verify` compares inferred types against what the
    /// server reports for the same column. A `json_agg(o.*)` column is
    /// inferred as `json_nested<...>` but reported as plain `json`, and a
    /// user `@json` mapping as `json_typed<...>` for the same reason —
    /// without this, every such column produces a spurious verification
    /// failure.
    #[test]
    fn structurally_typed_json_is_compatible_with_reported_json() {
        assert!(types_are_compatible("json_nested<array<GetUserPostsRowPosts>>", "json"));
        assert!(types_are_compatible(
            "json_nested<array<nullable<GetUserPostsRowPosts>>>",
            "json"
        ));
        assert!(types_are_compatible("json_nested<GetPostAsJsonRowPost>", "json"));
        assert!(types_are_compatible("json_typed<EventData>", "json"));
    }

    /// Only against a reported `json`. A structural JSON type reported as
    /// anything else is a real mismatch, not inference being coarse.
    #[test]
    fn structurally_typed_json_is_not_compatible_with_other_reported_types() {
        assert!(!types_are_compatible("json_nested<array<Foo>>", "string"));
        assert!(!types_are_compatible("json_nested<array<Foo>>", "int32"));
        assert!(!types_are_compatible("json_typed<EventData>", "string"));
    }

    #[test]
    fn genuinely_different_types_are_incompatible() {
        assert!(!types_are_compatible("int32", "string"));
        assert!(!types_are_compatible("bool", "int32"));
        assert!(!types_are_compatible("date", "datetime"));
        assert!(!types_are_compatible("string", "array<string>"));
    }

    #[test]
    fn domain_over_text_resolves_to_the_base_type() {
        // ~keep `pg_type` domains are constructed from catalog metadata that isn't
        // reachable without a live connection, so this exercises
        // `neutral_type_for`'s handling of `Kind::Domain` indirectly via the
        // documented contract: it must recurse into the base type rather
        // than falling through to `None`. `Type::new` lets us build a
        // domain-kinded `Type` without a database.
        let domain = Type::new(
            "us_zip".to_string(),
            0,
            Kind::Domain(Type::TEXT),
            "pg_catalog".to_string(),
        );
        assert_eq!(neutral_type_for(&domain).as_deref(), Some("string"));
    }
}
