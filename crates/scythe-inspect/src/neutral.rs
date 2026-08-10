//! The one place a neutral type name is put into comparison form.
//!
//! Two sides of every comparison in this crate derive the same fact by
//! different routes: the DDL side runs
//! [`sql_type_to_neutral`](scythe_core::analyzer::sql_type_to_neutral) over the
//! type name a `CREATE TABLE` wrote, and the live side runs
//! [`neutral_type_for`](crate::verify::pg_types::neutral_type_for) over what
//! the server reports. Where those two routes spell one type two ways, the
//! difference is not drift — it is the two routes disagreeing about spelling —
//! and reporting it fires SC-DRF05 and SC-VER03 on schemas that match exactly.
//!
//! Rather than teach each comparison site its own tolerances (which is how the
//! two routes drifted apart in the first place), both sides are pushed through
//! [`normalize_neutral_type`] before they meet.

use std::borrow::Cow;

/// Neutral-type prefixes whose payload is an object name that one side may
/// spell schema-qualified and the other bare.
///
/// `sql_type_to_neutral` renders an enum column from the type name the column
/// declaration used, so `state public.status` produces `enum::public.status`
/// while `pg_type` — which stores the schema separately — always produces
/// `enum::status`. `pg_dump --schema-only` emits qualified column types by
/// default, so this fired on the output of the most ordinary way there is to
/// obtain a schema file.
const QUALIFIED_PAYLOAD_PREFIXES: [&str; 2] = ["enum::", "composite::"];

/// Put a neutral type name into the form both sides of a comparison agree on.
///
/// Idempotent, and a no-op (borrowing its input) for every type name that is
/// already canonical, which is the overwhelming majority.
pub fn normalize_neutral_type(neutral: &str) -> Cow<'_, str> {
    // `array<T>` and `range<T>` wrap a payload that needs the same treatment:
    // an `mood[]` column declared as `public.mood[]` reaches here as
    // `array<enum::public.mood>`.
    if let Some((wrapper, inner)) = split_wrapper(neutral) {
        // Compared by value, not by `Cow` variant: `name` normalises to a
        // borrowed `"string"`, so a variant check would read "unchanged" for a
        // payload that changed.
        let normalized = normalize_neutral_type(inner);
        return if normalized == inner {
            Cow::Borrowed(neutral)
        } else {
            Cow::Owned(format!("{wrapper}<{normalized}>"))
        };
    }

    for prefix in QUALIFIED_PAYLOAD_PREFIXES {
        if let Some(payload) = neutral.strip_prefix(prefix)
            && let Some((_schema, bare)) = payload.rsplit_once('.')
        {
            return Cow::Owned(format!("{prefix}{bare}"));
        }
    }

    // `sql_type_to_neutral` has no arm for PostgreSQL's `name` type, so it
    // falls through to the raw spelling, while `neutral_type_for` maps
    // `Type::NAME` to `string` like every other text type. Left alone, every
    // `name`-typed column in a schema that matches exactly reports SC-DRF05.
    if neutral == "name" {
        return Cow::Borrowed("string");
    }

    Cow::Borrowed(neutral)
}

/// Split `wrapper<inner>` into its parts, or `None` when `neutral` is not a
/// wrapper form.
fn split_wrapper(neutral: &str) -> Option<(&str, &str)> {
    let inner = neutral.strip_suffix('>')?;
    let (wrapper, inner) = inner.split_once('<')?;
    if wrapper.is_empty() {
        None
    } else {
        Some((wrapper, inner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `pg_dump` case: the column declaration names the enum with its
    /// schema, `pg_type` never does, and drift compared the two for equality.
    #[test]
    fn should_strip_the_schema_from_an_enum_payload_when_the_ddl_qualified_it() {
        assert_eq!(normalize_neutral_type("enum::public.status"), "enum::status");
    }

    #[test]
    fn should_strip_the_schema_from_a_composite_payload_when_the_ddl_qualified_it() {
        assert_eq!(normalize_neutral_type("composite::app.address"), "composite::address");
    }

    /// Both spellings must land on one string, or the comparison is still
    /// comparing two different derivations of the same fact.
    #[test]
    fn should_agree_between_the_qualified_and_bare_enum_spellings() {
        assert_eq!(
            normalize_neutral_type("enum::public.status"),
            normalize_neutral_type("enum::status")
        );
    }

    #[test]
    fn should_normalize_inside_an_array_when_the_element_is_a_qualified_enum() {
        assert_eq!(normalize_neutral_type("array<enum::public.mood>"), "array<enum::mood>");
    }

    #[test]
    fn should_normalize_inside_nested_wrappers() {
        assert_eq!(
            normalize_neutral_type("array<array<enum::public.mood>>"),
            "array<array<enum::mood>>"
        );
    }

    /// `sql_type_to_neutral` has no `name` arm and returns the raw spelling;
    /// `neutral_type_for` maps `Type::NAME` to `string`. This was the only
    /// false positive across a 44-column type sweep.
    #[test]
    fn should_map_the_name_type_to_string() {
        assert_eq!(normalize_neutral_type("name"), "string");
    }

    #[test]
    fn should_map_an_array_of_name_to_an_array_of_string() {
        assert_eq!(normalize_neutral_type("array<name>"), "array<string>");
    }

    /// A bare enum name that merely contains no dot must come through
    /// untouched — and borrowed, so the common path allocates nothing.
    #[test]
    fn should_leave_an_already_canonical_type_untouched() {
        for neutral in [
            "int32",
            "string",
            "decimal",
            "enum::status",
            "composite::address",
            "array<string>",
            "range<datetime_tz>",
            "json_nested<array<Foo>>",
        ] {
            let normalized = normalize_neutral_type(neutral);
            assert_eq!(normalized, neutral);
            assert!(
                matches!(normalized, Cow::Borrowed(_)),
                "`{neutral}` is already canonical and must not allocate"
            );
        }
    }

    #[test]
    fn should_be_idempotent() {
        for neutral in ["enum::public.status", "array<enum::public.mood>", "name", "int32"] {
            let once = normalize_neutral_type(neutral).into_owned();
            let twice = normalize_neutral_type(&once).into_owned();
            assert_eq!(once, twice, "normalizing `{neutral}` twice changed the answer");
        }
    }

    /// A stray `<` or `>` must not be mistaken for a wrapper and reassembled
    /// into something else.
    #[test]
    fn should_not_treat_a_malformed_wrapper_as_a_wrapper() {
        assert_eq!(normalize_neutral_type("<int32>"), "<int32>");
        assert_eq!(normalize_neutral_type("array<int32"), "array<int32");
        assert_eq!(normalize_neutral_type("int32>"), "int32>");
    }
}
