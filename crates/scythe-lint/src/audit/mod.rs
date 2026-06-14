//! Matcher-based rule framework for `scythe audit`.
//!
//! Key types:
//! - [`RuleSpec`] — TOML schema for a single rule (id, message, matcher name, args).
//! - [`MatcherRegistry`] — maps matcher names to `MatcherFn` implementations.
//! - [`MatcherRule`] — a `LintRule` that delegates to a named matcher.
//! - [`canonical_specs`] — loads the six built-in SC-SEC* rules from the
//!   embedded `rules/security.toml`.

pub mod matcher_rule;
pub mod matchers;
pub mod registry;
pub mod spec;
pub mod suppression;

pub use matcher_rule::{MatcherRule, render_template};
pub use registry::{MatcherFn, MatcherHit, MatcherRegistry};
pub use spec::{
    AuditConfigError, CANONICAL_RULE_IDS, RuleFile, RuleSpec, SCHEMA_VERSION, SpecValidationError,
    load_rules_from_file, parse_rule_file,
};
pub use suppression::SuppressionSet;

// ---------------------------------------------------------------------------
// User-rule registration
// ---------------------------------------------------------------------------

/// Register user-supplied rules into an existing `RuleRegistry`.
///
/// Each entry in `user_specs` is a `(RuleSpec, source_path)` pair where
/// `source_path` is included verbatim in error messages for attribution.
///
/// Validation:
/// - ID must start with `USER-`.
/// - ID must not collide with any canonical built-in ID in
///   [`CANONICAL_RULE_IDS`].
/// - The rule's `matcher` field must resolve to a known entry in
///   `matcher_registry`.
pub fn register_user_rules(
    registry: &mut crate::registry::RuleRegistry,
    matcher_registry: &MatcherRegistry,
    user_specs: &[(RuleSpec, String)],
) -> Result<(), AuditConfigError> {
    for (spec, source) in user_specs {
        spec.validate_user_rule()
            .map_err(|e| AuditConfigError::InvalidRule {
                path: source.clone(),
                rule_id: spec.id.clone(),
                reason: e.to_string(),
            })?;

        let matcher_fn =
            matcher_registry
                .get(&spec.matcher)
                .ok_or_else(|| AuditConfigError::InvalidRule {
                    path: source.clone(),
                    rule_id: spec.id.clone(),
                    reason: format!("unknown matcher '{}'", spec.matcher),
                })?;

        registry.register(Box::new(MatcherRule::new(spec.clone(), matcher_fn)));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Canonical spec loader
// ---------------------------------------------------------------------------

const SECURITY_TOML: &str = include_str!("rules/security.toml");
const MIGRATION_TOML: &str = include_str!("rules/migration.toml");
const QUALITY_TOML: &str = include_str!("rules/quality.toml");

/// Load all built-in canonical rule specs (SC-SEC* + SC-RLS* + SC-MIG* +
/// SC-CHK*) from the embedded TOML files.
///
/// # Panics
///
/// Panics if any embedded TOML file fails to parse.  These are build-time
/// assets; a parse failure indicates a developer error, not a runtime
/// condition.
pub fn canonical_specs() -> Vec<RuleSpec> {
    let security: RuleFile = toml::from_str(SECURITY_TOML)
        .expect("canonical security.toml is invalid — fix the source TOML");
    let migration: RuleFile = toml::from_str(MIGRATION_TOML)
        .expect("canonical migration.toml is invalid — fix the source TOML");
    let quality: RuleFile = toml::from_str(QUALITY_TOML)
        .expect("canonical quality.toml is invalid — fix the source TOML");
    let mut all = security.rules;
    all.extend(migration.rules);
    all.extend(quality.rules);
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use spec::CANONICAL_RULE_IDS;

    #[test]
    fn canonical_specs_returns_thirty_five() {
        let specs = canonical_specs();
        assert_eq!(
            specs.len(),
            35,
            "expected exactly 35 canonical specs (12 SC-SEC* + 3 SC-RLS* + 19 SC-MIG* + 1 SC-CHK*)"
        );
    }

    #[test]
    fn canonical_specs_have_expected_ids() {
        let specs = canonical_specs();
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        for expected_id in CANONICAL_RULE_IDS {
            assert!(
                ids.contains(expected_id),
                "canonical specs missing expected id: {expected_id}"
            );
        }
    }

    #[test]
    fn canonical_specs_all_have_matchers() {
        let reg = MatcherRegistry::canonical();
        for spec in canonical_specs() {
            assert!(
                reg.get(&spec.matcher).is_some(),
                "canonical spec {} references unknown matcher: {}",
                spec.id,
                spec.matcher
            );
        }
    }
}
