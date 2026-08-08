use std::collections::HashMap;

use scythe_core::errors::{ErrorCode, ScytheError};

/// Reject any key in `options` that is not in `known`.
///
/// Originally only the TypeScript backends called this -- each one's
/// `apply_options` did it as the first line, before touching any individual
/// option -- while every other backend inherited the
/// [`crate::backend_trait::CodegenBackend::apply_options`] default of
/// `Ok(())`, so a typo like `field_casing = "camelCase"` (meant to be
/// `field_case`) was silently discarded on, say, `java-jdbc` while it was a
/// hard error on `typescript-pg`. The same typo behaving differently
/// depending on target language was itself a trap (#103), so the trait
/// default now calls this too, against a backend's declared option list --
/// see `CodegenBackend::apply_options`. Lives outside `backends::` (a
/// per-language module tree) because both TypeScript and non-TypeScript
/// backends call it directly.
///
/// When a rejected key is within edit distance 2 of a known one, the error
/// suggests it -- close enough to catch `row_typ` -> `row_type` or
/// `outer_join_union` -> `outer_join_unions` without false-positiving on
/// genuinely unrelated keys.
pub fn reject_unknown_options(known: &[&str], options: &HashMap<String, String>) -> Result<(), ScytheError> {
    let mut keys: Vec<&String> = options.keys().collect();
    keys.sort();

    for key in keys {
        if known.contains(&key.as_str()) {
            continue;
        }

        let suggestion = known
            .iter()
            .map(|&candidate| (candidate, levenshtein_distance(key, candidate)))
            .filter(|&(_, distance)| distance <= 2)
            .min_by_key(|&(_, distance)| distance)
            .map(|(candidate, _)| candidate);

        let message = match suggestion {
            Some(suggestion) => format!(
                "unknown option '{key}' (did you mean '{suggestion}'?): valid options are {}",
                known.join(", ")
            ),
            None => format!("unknown option '{key}': valid options are {}", known.join(", ")),
        };
        return Err(ScytheError::new(ErrorCode::InvalidConfig, message));
    }

    Ok(())
}

/// Levenshtein edit distance between two strings, used by
/// [`reject_unknown_options`] to suggest a likely intended option name.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    let mut prev_row: Vec<usize> = (0..=b.len()).collect();
    let mut curr_row = vec![0usize; b.len() + 1];

    for (i, &char_a) in a.iter().enumerate() {
        curr_row[0] = i + 1;
        for (j, &char_b) in b.iter().enumerate() {
            let substitution_cost = usize::from(char_a != char_b);
            curr_row[j + 1] = (prev_row[j + 1] + 1)
                .min(curr_row[j] + 1)
                .min(prev_row[j] + substitution_cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known_options() -> &'static [&'static str] {
        &["row_type", "outer_join_unions", "structs_only", "field_case"]
    }

    #[test]
    fn test_reject_unknown_options_accepts_known_keys() {
        let mut options = HashMap::new();
        options.insert("row_type".to_string(), "zod".to_string());
        options.insert("field_case".to_string(), "camelCase".to_string());
        reject_unknown_options(known_options(), &options).unwrap();
    }

    #[test]
    fn test_reject_unknown_options_accepts_empty_map() {
        reject_unknown_options(known_options(), &HashMap::new()).unwrap();
    }

    /// This must fail before `reject_unknown_options` existed: the default
    /// `apply_options` returned `Ok(())` for any options map, so a typo like
    /// `row_typ = "zod"` silently parsed as valid TOML and had no effect.
    #[test]
    fn test_reject_unknown_options_rejects_unrecognized_key() {
        let mut options = HashMap::new();
        options.insert("row_typ".to_string(), "zod".to_string());
        let err = reject_unknown_options(known_options(), &options).expect_err("row_typ is not a known option");
        let message = err.to_string();
        assert!(message.contains("row_typ"), "{message}");
        assert!(
            message.contains("row_type"),
            "error should list valid options: {message}"
        );
    }

    #[test]
    fn test_reject_unknown_options_suggests_close_typo() {
        let mut options = HashMap::new();
        options.insert("row_typ".to_string(), "zod".to_string());
        let err = reject_unknown_options(known_options(), &options).expect_err("row_typ is not a known option");
        assert!(
            err.to_string().contains("did you mean 'row_type'?"),
            "expected a did-you-mean suggestion: {err}"
        );
    }

    #[test]
    fn test_reject_unknown_options_no_suggestion_when_too_far() {
        let mut options = HashMap::new();
        options.insert("completely_unrelated_option".to_string(), "x".to_string());
        let err = reject_unknown_options(known_options(), &options).expect_err("not a known option");
        assert!(
            !err.to_string().contains("did you mean"),
            "should not suggest anything this far off: {err}"
        );
    }

    #[test]
    fn test_levenshtein_distance_basic_cases() {
        assert_eq!(levenshtein_distance("row_type", "row_type"), 0);
        assert_eq!(levenshtein_distance("row_typ", "row_type"), 1);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }

    /// A user's typo is the user's problem to fix, not evidence that scythe
    /// broke. Reporting it as `INTERNAL_ERROR` reads as "file a bug" for an
    /// error scythe diagnosed perfectly, so the code has to stay
    /// `InvalidConfig` even though the message itself never changes.
    #[test]
    fn should_classify_unknown_option_as_invalid_config_not_internal() {
        let mut options = HashMap::new();
        options.insert("row_typ".to_string(), "zod".to_string());

        let err = reject_unknown_options(&["row_type", "field_case"], &options)
            .expect_err("a misspelled option key must be rejected");

        assert_eq!(err.code, ErrorCode::InvalidConfig);
        assert_eq!(err.code.to_string(), "INVALID_CONFIG");
        assert!(
            err.message.contains("did you mean 'row_type'?"),
            "the suggestion is the useful half of the message: {}",
            err.message
        );
    }

    /// A backend with no options at all (the trait default) must reject any
    /// key, not silently accept it -- the exact protection #103 asks for on
    /// every backend, not just the eleven TypeScript ones.
    #[test]
    fn test_reject_unknown_options_rejects_any_key_against_an_empty_known_set() {
        let mut options = HashMap::new();
        options.insert("row_type".to_string(), "zod".to_string());
        let err = reject_unknown_options(&[], &options).expect_err("a backend with no options accepts no keys");
        assert!(err.to_string().contains("row_type"), "{err}");
        assert_eq!(err.code, ErrorCode::InvalidConfig);
    }
}
