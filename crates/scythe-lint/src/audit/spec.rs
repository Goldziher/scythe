//! `RuleSpec` — TOML schema for a matcher-based lint rule.
//!
//! Canonical built-in rules ship in `rules/security.toml` (compiled in with
//! `include_str!`). User-supplied rules must carry IDs that start with `USER-`;
//! canonical built-in IDs use the `SC-` prefix and are reserved.

use std::path::Path;

use scythe_core::dialect::SqlDialect;
use serde::Deserialize;

use crate::types::{RuleCategory, Severity};

/// Schema version used in TOML rule files.  Reject files with a higher
/// version so we can evolve the format without silently misreading fields.
pub const SCHEMA_VERSION: u32 = 1;

/// The canonical built-in rule IDs that users cannot override or reuse.
pub const CANONICAL_RULE_IDS: &[&str] = &[
    // Security
    "SC-SEC01", "SC-SEC02", "SC-SEC03", "SC-SEC04", "SC-SEC05", "SC-SEC06", "SC-SEC07", "SC-SEC08",
    "SC-SEC09", "SC-SEC10", "SC-SEC11", "SC-SEC12", // Migration
    "SC-MIG01", "SC-MIG02", "SC-MIG03", "SC-MIG04", "SC-MIG05", "SC-MIG06", "SC-MIG07", "SC-MIG08",
    "SC-MIG09", "SC-MIG10", "SC-MIG11", "SC-MIG12", "SC-MIG13", "SC-MIG14", "SC-MIG15", "SC-MIG16",
    "SC-MIG17", "SC-MIG18", "SC-MIG19",
];

// ---------------------------------------------------------------------------
// Serde helpers
// ---------------------------------------------------------------------------

fn default_category() -> RuleCategory {
    RuleCategory::Security
}

fn deserialize_dialects<'de, D>(deserializer: D) -> Result<Vec<SqlDialect>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let strs: Vec<String> = Vec::deserialize(deserializer)?;
    strs.iter()
        .map(|s| {
            SqlDialect::from_str(s)
                .ok_or_else(|| serde::de::Error::custom(format!("unknown SQL dialect: {s:?}")))
        })
        .collect()
}

fn default_dialects() -> Vec<SqlDialect> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Top-level TOML container
// ---------------------------------------------------------------------------

/// Top-level structure of a rule TOML file.
#[derive(Debug, Deserialize)]
pub struct RuleFile {
    /// Must equal [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The rules defined in this file.
    #[serde(rename = "rule")]
    pub rules: Vec<RuleSpec>,
}

// ---------------------------------------------------------------------------
// RuleSpec
// ---------------------------------------------------------------------------

/// Metadata for a single matcher-based lint rule, as stored in TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct RuleSpec {
    /// Unique identifier, e.g. `SC-SEC01` or `USER-001`.
    pub id: String,
    /// Short human-readable name, e.g. `dangerous-function`.
    pub name: String,
    /// Category for severity-override grouping.
    #[serde(default = "default_category")]
    pub category: RuleCategory,
    /// Default severity.
    pub severity: Severity,
    /// Dialects this rule applies to.  Empty means all dialects.
    #[serde(
        default = "default_dialects",
        deserialize_with = "deserialize_dialects"
    )]
    pub dialects: Vec<SqlDialect>,
    /// CWE identifiers (e.g. `["CWE-78"]`).
    #[serde(default)]
    pub cwe: Vec<String>,
    /// One-line description used in docs and by `extract_cwe`.
    pub description: String,
    /// Message template with `{var}` placeholders filled from matcher bindings.
    pub message: String,
    /// Name into `MatcherRegistry` — must match a registered matcher.
    pub matcher: String,
    /// Opaque per-rule configuration passed to the matcher function.
    #[serde(default)]
    pub matcher_args: toml::Table,
}

/// Validation error for a user-supplied `RuleSpec`.
#[derive(Debug, thiserror::Error)]
pub enum SpecValidationError {
    #[error("rule id {0:?} must start with 'USER-'")]
    MissingUserPrefix(String),
    #[error("rule id {0:?} collides with a built-in canonical rule")]
    CanonicalIdCollision(String),
}

impl RuleSpec {
    /// Validate that a user-supplied spec carries the required `USER-` prefix
    /// and does not collide with any built-in ID.
    pub fn validate_user_rule(&self) -> Result<(), SpecValidationError> {
        if !self.id.starts_with("USER-") {
            return Err(SpecValidationError::MissingUserPrefix(self.id.clone()));
        }
        if CANONICAL_RULE_IDS.contains(&self.id.as_str()) {
            return Err(SpecValidationError::CanonicalIdCollision(self.id.clone()));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AuditConfigError
// ---------------------------------------------------------------------------

/// Errors that can arise while loading or validating user-supplied audit rules.
#[derive(Debug, thiserror::Error)]
pub enum AuditConfigError {
    #[error("failed to read rule file '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse rule file '{path}': {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("rule file '{path}' has schema_version {found}, expected {expected}")]
    SchemaVersionMismatch {
        path: String,
        found: u32,
        expected: u32,
    },
    #[error("invalid rule '{rule_id}' in '{path}': {reason}")]
    InvalidRule {
        path: String,
        rule_id: String,
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// File-loading helpers
// ---------------------------------------------------------------------------

/// Parse a TOML rule file from an in-memory string.
///
/// `source` is used solely for error-message attribution.
pub fn parse_rule_file(content: &str, source: &str) -> Result<Vec<RuleSpec>, AuditConfigError> {
    let file: RuleFile = toml::from_str(content).map_err(|e| AuditConfigError::Toml {
        path: source.to_string(),
        source: e,
    })?;

    if file.schema_version != SCHEMA_VERSION {
        return Err(AuditConfigError::SchemaVersionMismatch {
            path: source.to_string(),
            found: file.schema_version,
            expected: SCHEMA_VERSION,
        });
    }

    Ok(file.rules)
}

/// Read a TOML rule file from disk and return the parsed `RuleSpec`s.
pub fn load_rules_from_file(path: &Path) -> Result<Vec<RuleSpec>, AuditConfigError> {
    let path_str = path.display().to_string();
    let content = std::fs::read_to_string(path).map_err(|e| AuditConfigError::Io {
        path: path_str.clone(),
        source: e,
    })?;
    parse_rule_file(&content, &path_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_TOML: &str = r#"
schema_version = 1

[[rule]]
id = "SC-SEC01"
name = "dangerous-function"
severity = "error"
description = "call to dangerous fn (CWE-78)"
message = "call to {func}"
matcher = "function_name_in_set"

[rule.matcher_args]
functions = ["pg_read_file"]
"#;

    #[test]
    fn toml_round_trip_minimal() {
        let file: RuleFile =
            toml::from_str(MINIMAL_TOML).expect("should parse minimal TOML fixture");
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert_eq!(file.rules.len(), 1);
        let rule = &file.rules[0];
        assert_eq!(rule.id, "SC-SEC01");
        assert_eq!(rule.name, "dangerous-function");
        assert_eq!(rule.severity, Severity::Error);
        assert_eq!(rule.category, RuleCategory::Security);
        assert_eq!(rule.matcher, "function_name_in_set");
        assert_eq!(
            rule.matcher_args
                .get("functions")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1)
        );
    }

    #[test]
    fn user_rule_validates_prefix_ok() {
        let mut file: RuleFile = toml::from_str(MINIMAL_TOML).unwrap();
        file.rules[0].id = "USER-001".to_string();
        assert!(file.rules[0].validate_user_rule().is_ok());
    }

    #[test]
    fn user_rule_rejects_missing_prefix() {
        let file: RuleFile = toml::from_str(MINIMAL_TOML).unwrap();
        let err = file.rules[0].validate_user_rule().unwrap_err();
        assert!(matches!(err, SpecValidationError::MissingUserPrefix(_)));
    }

    #[test]
    fn canonical_rule_ids_count() {
        assert_eq!(CANONICAL_RULE_IDS.len(), 31);
        assert!(CANONICAL_RULE_IDS.contains(&"SC-SEC01"));
        assert!(CANONICAL_RULE_IDS.contains(&"SC-SEC12"));
        assert!(CANONICAL_RULE_IDS.contains(&"SC-MIG01"));
        assert!(CANONICAL_RULE_IDS.contains(&"SC-MIG09"));
        assert!(CANONICAL_RULE_IDS.contains(&"SC-MIG13"));
        assert!(CANONICAL_RULE_IDS.contains(&"SC-MIG18"));
        assert!(CANONICAL_RULE_IDS.contains(&"SC-MIG19"));
    }

    #[test]
    fn rule_with_dialects_parses() {
        let toml_str = r#"
schema_version = 1

[[rule]]
id = "SC-SEC01"
name = "test"
severity = "warn"
description = "desc"
message = "msg"
matcher = "function_name_in_set"
dialects = ["postgres"]
"#;
        let file: RuleFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.rules[0].dialects.len(), 1);
        assert_eq!(file.rules[0].dialects[0], SqlDialect::PostgreSQL);
    }
}
