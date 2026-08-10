//! Validation of `[rule.matcher_args]` against what each matcher can act on.
//!
//! Most matchers open by reading a list out of their args and returning
//! nothing when it is empty:
//!
//! ```text
//! let functions = read_string_list(args, "functions");
//! if functions.is_empty() {
//!     return Vec::new();
//! }
//! ```
//!
//! So a rule that misspells the key (`function = [...]`), gives it the wrong
//! TOML type (`functions = "pg_read_file"`), leaves it empty, or fills it
//! with values the matcher does not recognise (`kinds = ["tables"]`) is
//! accepted at load time, listed by `scythe audit --list-rules`, counted
//! among the active rules — and can never produce a finding. The audit
//! reports "No findings" and the user reads it as a clean bill of health
//! (#165).
//!
//! [`validate_matcher_args`] turns each of those into a loud failure at
//! registration time, before any SQL is examined. It only rejects
//! configurations under which the matcher is *provably* inert; a matcher with
//! no entry here has no arguments it can be starved of.

use ahash::AHashSet;

/// A rule's `matcher_args` cannot produce a finding under any input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("matcher '{matcher}' can never produce a finding with these matcher_args: {reason}")]
pub struct MatcherArgsError {
    /// The matcher named by the rule.
    pub matcher: String,
    /// What is missing, empty, or unusable, naming the offending key.
    pub reason: String,
}

/// The TOML shape a matcher reads a key as. Reading it as anything else
/// yields nothing: `read_string_list` ignores a bare string, and an
/// `as_str()` read ignores an array.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// An array of strings.
    List,
    /// A single string.
    Scalar,
}

impl Shape {
    fn describe(self) -> &'static str {
        match self {
            Shape::List => "an array of strings",
            Shape::Scalar => "a string",
        }
    }
}

/// What a matcher needs before it can fire.
enum Requirement {
    /// Every named key must hold a non-empty array of strings. The matcher
    /// bails out when any one of them reads as empty.
    AllNonEmptyLists(&'static [&'static str]),
    /// At least one of `keys`, read in its declared shape, must include at
    /// least one entry from `allowed`. Anything else is a value the matcher
    /// has no branch for.
    AnyKeyIntersects {
        keys: &'static [(&'static str, Shape)],
        allowed: &'static [&'static str],
        /// Whether the matcher lowercases the value before comparing. When
        /// it does not, `"PUBLIC"` is as inert as `"pubic"` and must be
        /// rejected just the same.
        case_insensitive: bool,
    },
}

/// Every matcher whose `matcher_args` can render it inert, and what it needs.
///
/// Kept in step with the matchers by two tests: one checks that every
/// canonical shipped rule satisfies its matcher's entry (so an entry that
/// demands the wrong key fails immediately), and one checks that every
/// matcher named here exists in the canonical [`super::MatcherRegistry`] (so
/// a renamed matcher cannot leave a stale entry silently validating nothing).
const REQUIREMENTS: &[(&str, Requirement)] = &[
    ("function_name_in_set", Requirement::AllNonEmptyLists(&["functions"])),
    (
        "weak_hash_over_sensitive_column",
        Requirement::AllNonEmptyLists(&["functions", "column_patterns"]),
    ),
    (
        "select_star_over_pii_columns",
        Requirement::AllNonEmptyLists(&["column_patterns"]),
    ),
    ("column_type_disallowed", Requirement::AllNonEmptyLists(&["disallowed"])),
    (
        "grantee_includes",
        Requirement::AnyKeyIntersects {
            keys: &[("grantee", Shape::Scalar)],
            allowed: &["public"],
            case_insensitive: false,
        },
    ),
    (
        "grant_kind",
        Requirement::AnyKeyIntersects {
            keys: &[("kind", Shape::Scalar)],
            allowed: &["all"],
            case_insensitive: false,
        },
    ),
    (
        "drop_statement",
        Requirement::AnyKeyIntersects {
            keys: &[("kinds", Shape::List)],
            allowed: &["table", "database", "schema", "column"],
            case_insensitive: true,
        },
    ),
    (
        "session_mutation",
        Requirement::AnyKeyIntersects {
            keys: &[("kinds", Shape::List)],
            allowed: &["set_role", "set_session_authorization", "reset_role"],
            case_insensitive: true,
        },
    ),
    (
        "add_constraint_without_using_index",
        Requirement::AnyKeyIntersects {
            keys: &[("kinds", Shape::List)],
            allowed: &["unique", "primary_key"],
            case_insensitive: true,
        },
    ),
    (
        "role_with_attribute",
        Requirement::AnyKeyIntersects {
            keys: &[("attributes", Shape::List), ("attribute", Shape::Scalar)],
            allowed: super::matchers::role_with_attribute::RECOGNIZED_ATTRIBUTES,
            case_insensitive: true,
        },
    ),
];

/// Check that `args` can drive `matcher` to a finding.
///
/// Returns `Ok(())` for any matcher with no declared requirement — those
/// read no arguments they can be starved of.
pub fn validate_matcher_args(matcher: &str, args: &toml::Table) -> Result<(), MatcherArgsError> {
    let Some((_, requirement)) = REQUIREMENTS.iter().find(|(name, _)| *name == matcher) else {
        return Ok(());
    };

    let fail = |reason: String| {
        Err(MatcherArgsError {
            matcher: matcher.to_string(),
            reason,
        })
    };

    match requirement {
        Requirement::AllNonEmptyLists(keys) => {
            for key in *keys {
                match string_values(args, key, Shape::List) {
                    None => {
                        return fail(format!(
                            "'{key}' is required and must be {} (it is {})",
                            Shape::List.describe(),
                            describe(args, key)
                        ));
                    }
                    Some(values) if values.is_empty() => {
                        return fail(format!("'{key}' is empty, so the matcher has nothing to look for"));
                    }
                    Some(_) => {}
                }
            }
            Ok(())
        }
        Requirement::AnyKeyIntersects {
            keys,
            allowed,
            case_insensitive,
        } => {
            let allowed_set: AHashSet<&str> = allowed.iter().copied().collect();
            let matches = |value: &str| {
                if *case_insensitive {
                    allowed_set.contains(value.to_ascii_lowercase().as_str())
                } else {
                    allowed_set.contains(value)
                }
            };

            let any_usable = keys
                .iter()
                .filter_map(|(key, shape)| string_values(args, key, *shape))
                .any(|values| values.iter().any(|v| matches(v)));

            if any_usable {
                return Ok(());
            }

            let key_list = keys
                .iter()
                .map(|(key, shape)| format!("'{key}' ({})", shape.describe()))
                .collect::<Vec<_>>()
                .join(" or ");
            let allowed_list = allowed
                .iter()
                .map(|v| format!("\"{v}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let found = keys
                .iter()
                .map(|(key, _)| format!("{key} = {}", describe(args, key)))
                .collect::<Vec<_>>()
                .join(", ");
            fail(format!(
                "{key_list} must name at least one of [{allowed_list}] (found {found})"
            ))
        }
    }
}

/// The string values under `key`, read in the `shape` its matcher reads it
/// as. `None` means the key is absent or holds something of another shape —
/// which reads as nothing in the matcher, so it must read as nothing here.
///
/// Non-string entries inside an array are dropped, matching the matchers'
/// own `filter_map(|v| v.as_str())` readers: `functions = [1, 2]` reads as an
/// empty list there and must read as an empty list here.
fn string_values(args: &toml::Table, key: &str, shape: Shape) -> Option<Vec<String>> {
    match (shape, args.get(key)?) {
        (Shape::Scalar, toml::Value::String(s)) => Some(vec![s.clone()]),
        (Shape::List, toml::Value::Array(arr)) => {
            Some(arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        }
        _ => None,
    }
}

/// A short description of what `key` actually holds, for error messages.
fn describe(args: &toml::Table, key: &str) -> String {
    match args.get(key) {
        None => "absent".to_string(),
        Some(toml::Value::Array(arr)) if arr.is_empty() => "an empty array".to_string(),
        Some(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{MatcherRegistry, canonical_specs};

    fn table(toml_src: &str) -> toml::Table {
        toml_src.parse::<toml::Table>().expect("test TOML must parse")
    }

    /// The shipped canonical rules are the reference implementation of "args
    /// a matcher can act on". If a requirement here demanded the wrong key
    /// or the wrong values, this fails on the very rules the matcher was
    /// written for.
    #[test]
    fn every_canonical_rule_satisfies_its_matchers_requirement() {
        for spec in canonical_specs() {
            assert_eq!(
                validate_matcher_args(&spec.matcher, &spec.matcher_args),
                Ok(()),
                "canonical rule {} does not satisfy its own matcher's argument requirement",
                spec.id
            );
        }
    }

    /// A requirement keyed on a matcher that no longer exists validates
    /// nothing and would never be noticed. Renaming a matcher must break
    /// this rather than silently drop its argument checking.
    #[test]
    fn every_requirement_names_a_registered_matcher() {
        let registry = MatcherRegistry::canonical();
        for (matcher, _) in REQUIREMENTS {
            assert!(
                registry.get(matcher).is_some(),
                "REQUIREMENTS names '{matcher}', which is not a registered matcher"
            );
        }
    }

    /// Pinned, both directions: a new matcher that reads args must either
    /// declare a requirement or be a deliberate omission, and a requirement
    /// cannot be dropped without the deletion being visible.
    #[test]
    fn matchers_with_declared_requirements_are_exactly_these_ten() {
        let mut declared: Vec<&str> = REQUIREMENTS.iter().map(|(name, _)| *name).collect();
        declared.sort_unstable();
        assert_eq!(
            declared,
            vec![
                "add_constraint_without_using_index",
                "column_type_disallowed",
                "drop_statement",
                "function_name_in_set",
                "grant_kind",
                "grantee_includes",
                "role_with_attribute",
                "select_star_over_pii_columns",
                "session_mutation",
                "weak_hash_over_sensitive_column",
            ]
        );
    }

    #[test]
    fn matcher_without_a_declared_requirement_accepts_empty_args() {
        assert_eq!(validate_matcher_args("cartesian_join", &toml::Table::new()), Ok(()));
    }

    #[test]
    fn missing_required_list_is_rejected_naming_the_key() {
        let err = validate_matcher_args("function_name_in_set", &toml::Table::new())
            .expect_err("a matcher with nothing to look for must be rejected");
        assert_eq!(err.matcher, "function_name_in_set");
        assert!(err.reason.contains("'functions'"), "reason: {}", err.reason);
        assert!(err.reason.contains("absent"), "reason: {}", err.reason);
    }

    #[test]
    fn empty_required_list_is_rejected() {
        let err = validate_matcher_args("function_name_in_set", &table("functions = []"))
            .expect_err("an empty list must be rejected");
        assert!(err.reason.contains("is empty"), "reason: {}", err.reason);
    }

    /// A string where the matcher reads an array is the misconfiguration
    /// most likely to look correct in a diff.
    #[test]
    fn required_list_given_as_a_bare_string_is_rejected() {
        let err = validate_matcher_args("function_name_in_set", &table(r#"functions = "pg_read_file""#))
            .expect_err("a string is not an array of strings");
        assert!(err.reason.contains("array of strings"), "reason: {}", err.reason);
    }

    #[test]
    fn a_second_required_list_is_checked_too() {
        assert!(
            validate_matcher_args("weak_hash_over_sensitive_column", &table(r#"functions = ["md5"]"#)).is_err(),
            "column_patterns is required as well as functions"
        );
        assert_eq!(
            validate_matcher_args(
                "weak_hash_over_sensitive_column",
                &table(
                    r#"
functions = ["md5"]
column_patterns = ["password"]
"#
                )
            ),
            Ok(())
        );
    }

    #[test]
    fn unrecognised_kind_is_rejected_and_lists_what_is_allowed() {
        let err = validate_matcher_args("drop_statement", &table(r#"kinds = ["tables"]"#))
            .expect_err("'tables' is not a kind this matcher tests for");
        assert!(err.reason.contains("\"table\""), "reason: {}", err.reason);
        assert!(err.reason.contains("\"schema\""), "reason: {}", err.reason);
    }

    #[test]
    fn a_recognised_kind_alongside_an_unrecognised_one_is_accepted() {
        assert_eq!(
            validate_matcher_args("drop_statement", &table(r#"kinds = ["table", "sequence"]"#)),
            Ok(()),
            "the matcher still fires on 'table'"
        );
    }

    /// `grantee_includes` compares its arg without lowercasing, so
    /// `"PUBLIC"` is inert and must be rejected — the case-insensitive
    /// treatment given to `kinds` would let it through.
    #[test]
    fn grantee_is_matched_case_sensitively() {
        assert_eq!(
            validate_matcher_args("grantee_includes", &table(r#"grantee = "public""#)),
            Ok(())
        );
        assert!(
            validate_matcher_args("grantee_includes", &table(r#"grantee = "PUBLIC""#)).is_err(),
            "the matcher compares without lowercasing, so PUBLIC never fires"
        );
    }

    /// `kinds` *is* lowercased by its matchers, so rejecting an uppercase
    /// spelling here would be a false rejection.
    #[test]
    fn kinds_are_matched_case_insensitively() {
        assert_eq!(
            validate_matcher_args("drop_statement", &table(r#"kinds = ["TABLE"]"#)),
            Ok(())
        );
    }

    /// The shape cuts both ways: `grantee_includes` reads its key with
    /// `as_str()`, so an array there is as inert as a missing key.
    #[test]
    fn scalar_key_given_as_an_array_is_rejected() {
        let err = validate_matcher_args("grantee_includes", &table(r#"grantee = ["public"]"#))
            .expect_err("the matcher reads 'grantee' as a string, not an array");
        assert!(err.reason.contains("a string"), "reason: {}", err.reason);
    }

    /// And a list key given as a scalar: `drop_statement` reads `kinds` with
    /// an array reader, so `kinds = "table"` never fires.
    #[test]
    fn list_key_given_as_a_scalar_is_rejected() {
        assert!(
            validate_matcher_args("drop_statement", &table(r#"kinds = "table""#)).is_err(),
            "the matcher reads 'kinds' as an array, not a string"
        );
    }

    #[test]
    fn role_attribute_accepts_either_key() {
        assert_eq!(
            validate_matcher_args("role_with_attribute", &table(r#"attributes = ["superuser"]"#)),
            Ok(())
        );
        assert_eq!(
            validate_matcher_args("role_with_attribute", &table(r#"attribute = "bypassrls""#)),
            Ok(())
        );
        assert!(
            validate_matcher_args("role_with_attribute", &table(r#"attribute = "loginy""#)).is_err(),
            "an attribute the matcher has no branch for must be rejected"
        );
    }

    /// The singular key must be enough even when an empty plural key is also
    /// present — validation and the matcher agree on this only because the
    /// matcher falls through instead of letting `attributes` shadow it.
    #[test]
    fn role_attribute_falls_through_from_an_empty_plural_key() {
        let args = table(
            r#"
attributes = []
attribute = "superuser"
"#,
        );
        assert_eq!(validate_matcher_args("role_with_attribute", &args), Ok(()));
    }

    #[test]
    fn constraint_kinds_are_restricted_to_the_two_the_matcher_tests() {
        assert_eq!(
            validate_matcher_args("add_constraint_without_using_index", &table(r#"kinds = ["unique"]"#)),
            Ok(())
        );
        assert!(
            validate_matcher_args("add_constraint_without_using_index", &table(r#"kinds = ["check"]"#)).is_err(),
            "the matcher only inspects UNIQUE and PRIMARY KEY constraints"
        );
    }
}
