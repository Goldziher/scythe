//! `CheckRegistry` — loads and validates [`CheckSpec`]s from TOML.
//!
//! The canonical built-in registry is bootstrapped from the embedded
//! `postgres/checks.toml` via `include_str!` at compile time. User-supplied
//! checks layer on top via [`CheckRegistry::with_user_checks`].

use std::path::Path;

use crate::spec::{CheckSpec, ConfigError, validate_message_bindings};

/// Registry of all check specs available for a given run.
///
/// Build the canonical built-in registry with [`CheckRegistry::canonical`],
/// then optionally extend it with user checks via
/// [`CheckRegistry::with_user_checks`].
pub struct CheckRegistry {
    checks: Vec<CheckSpec>,
}

impl CheckRegistry {
    /// Build the canonical registry from the embedded `postgres/checks.toml`.
    ///
    /// Panics on parse or binding-validation failure — a broken canonical TOML
    /// is a programming error that must be fixed before shipping, so a panic
    /// at startup is the correct signal.
    pub fn canonical() -> Self {
        const CANONICAL_SRC: &str = include_str!("postgres/checks.toml");
        const CANONICAL_LABEL: &str = "<built-in postgres/checks.toml>";

        let checks = crate::spec::parse_check_file(CANONICAL_SRC, CANONICAL_LABEL)
            .expect("canonical checks.toml must parse correctly");

        for spec in &checks {
            validate_message_bindings(spec).unwrap_or_else(|e| {
                panic!(
                    "canonical check {id} has invalid message bindings: {e}",
                    id = spec.id
                )
            });
        }

        Self { checks }
    }

    /// Extend the registry with user-defined checks loaded from `path`.
    ///
    /// Each user check must:
    /// - Have an ID prefixed with `USER-INS-`.
    /// - Not collide with any canonical ID.
    /// - Have all `{var}` placeholders in `message` present in the SQL
    ///   projection.
    ///
    /// Returns `Err` if the file cannot be read, parsed, or validated.
    pub fn with_user_checks(mut self, path: &Path) -> Result<Self, ConfigError> {
        let specs = crate::spec::load_checks_from_file(path)?;
        let path_str = path.display().to_string();

        for spec in specs {
            spec.validate_user_check()
                .map_err(|e| ConfigError::InvalidCheck {
                    path: path_str.clone(),
                    check_id: spec.id.clone(),
                    reason: format!("{e}"),
                })?;

            validate_message_bindings(&spec).map_err(|e| ConfigError::InvalidCheck {
                path: path_str.clone(),
                check_id: spec.id.clone(),
                reason: format!("{e}"),
            })?;

            self.checks.push(spec);
        }

        Ok(self)
    }

    /// Look up a check by its ID. Returns `None` if not found.
    pub fn get(&self, id: &str) -> Option<&CheckSpec> {
        self.checks.iter().find(|c| c.id == id)
    }

    /// Iterate checks that apply to `engine`.
    ///
    /// `engine` is matched case-sensitively against the `engines` list of each
    /// spec. Use `"postgres"` or `"mysql"`.
    pub fn for_engine<'a>(&'a self, engine: &'a str) -> impl Iterator<Item = &'a CheckSpec> {
        self.checks
            .iter()
            .filter(move |c| c.engines.iter().any(|e| e == engine))
    }

    /// Return all checks in the registry, in load order.
    pub fn all(&self) -> &[CheckSpec] {
        &self.checks
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_registry_has_canonical_postgres_checks() {
        use crate::spec::CANONICAL_CHECK_IDS;
        let reg = CheckRegistry::canonical();
        assert_eq!(
            reg.for_engine("postgres").count(),
            CANONICAL_CHECK_IDS.len()
        );
    }

    #[test]
    fn for_engine_filters_by_engine() {
        use crate::spec::CANONICAL_CHECK_IDS;
        // Build a registry with a synthetic MySQL-only spec appended.
        let mut reg = CheckRegistry::canonical();
        reg.checks.push(CheckSpec {
            id: "USER-INS-MYSQL-01".to_string(),
            name: "mysql-only".to_string(),
            category: crate::spec::CheckCategory::Schema,
            severity: scythe_lint::types::Severity::Warn,
            engines: vec!["mysql".to_string()],
            description: "test".to_string(),
            message: "test".to_string(),
            sql: "SELECT 1 AS x".to_string(),
            cwe: vec![],
            explanation: None,
            remediation: None,
            min_pg_version: None,
        });

        // postgres engine should still only see the canonical checks
        assert_eq!(
            reg.for_engine("postgres").count(),
            CANONICAL_CHECK_IDS.len()
        );
        // mysql engine should see only the synthetic one
        assert_eq!(reg.for_engine("mysql").count(), 1);
    }

    #[test]
    fn canonical_check_ids_are_present() {
        use crate::spec::CANONICAL_CHECK_IDS;
        let reg = CheckRegistry::canonical();
        for id in CANONICAL_CHECK_IDS {
            assert!(
                reg.get(id).is_some(),
                "canonical registry missing check {id}"
            );
        }
    }

    #[test]
    fn canonical_registry_schema_version_matches() {
        let content = include_str!("postgres/checks.toml");
        let file: crate::spec::CheckFile = toml::from_str(content).expect("parses");
        assert_eq!(file.schema_version, crate::spec::SCHEMA_VERSION);
    }
}
