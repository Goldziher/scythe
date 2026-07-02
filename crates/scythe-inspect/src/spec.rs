//! `CheckSpec` — TOML schema for a live-DB inspection check.
//!
//! Canonical built-in checks ship in `postgres/checks.toml` (compiled in with
//! `include_str!`). User-supplied checks must carry IDs that start with
//! `USER-INS-`; canonical built-in IDs use the `SC-INS` prefix and are
//! reserved.

use std::path::Path;

use regex::Regex;
use scythe_lint::types::Severity;
use serde::Deserialize;
use sqlparser::ast::Statement;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

/// Schema version used in TOML check files. Reject files with a higher
/// version so we can evolve the format without silently misreading fields.
pub const SCHEMA_VERSION: u32 = 1;

/// The canonical built-in check IDs that users cannot override or reuse.
pub const CANONICAL_CHECK_IDS: &[&str] = &[
    "SC-INS01", "SC-INS02", "SC-INS03", "SC-INS04", "SC-INS05", "SC-INS06", "SC-INS07", "SC-INS08", "SC-INS09",
    "SC-INS10", "SC-INS11", "SC-INS12", "SC-INS13",
];

// ---------------------------------------------------------------------------
// CheckCategory
// ---------------------------------------------------------------------------

/// Broad category for a live-DB check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckCategory {
    /// Security-relevant catalog state (e.g. RLS disabled, SECURITY DEFINER).
    Security,
    /// Query performance (e.g. missing FK index, duplicate index).
    Performance,
    /// Operational reliability (e.g. sequence overflow, partition gaps).
    Reliability,
    /// Schema shape checks (e.g. missing primary key).
    Schema,
}

impl std::fmt::Display for CheckCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckCategory::Security => write!(f, "security"),
            CheckCategory::Performance => write!(f, "performance"),
            CheckCategory::Reliability => write!(f, "reliability"),
            CheckCategory::Schema => write!(f, "schema"),
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level TOML container
// ---------------------------------------------------------------------------

/// Top-level structure of a check TOML file.
#[derive(Debug, Deserialize)]
pub struct CheckFile {
    /// Must equal [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The checks defined in this file.
    #[serde(rename = "check")]
    pub checks: Vec<CheckSpec>,
}

// ---------------------------------------------------------------------------
// CheckSpec
// ---------------------------------------------------------------------------

/// Metadata for a single live-DB catalog check, as stored in TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct CheckSpec {
    /// Unique identifier, e.g. `"SC-INS01"` or `"USER-INS-001"`.
    pub id: String,
    /// Short kebab-case slug, e.g. `"missing-fk-index"`.
    pub name: String,
    /// Broad category for grouping in output.
    pub category: CheckCategory,
    /// Default severity.
    pub severity: Severity,
    /// Engine names this check applies to, e.g. `["postgres"]`.
    pub engines: Vec<String>,
    /// One-line description used in `--list-checks` output.
    pub description: String,
    /// Message template with `{var}` placeholders bound from SQL result columns.
    ///
    /// Every `{var}` name must correspond to a column alias in `sql`.
    pub message: String,
    /// SQL executed verbatim against the driver client.
    ///
    /// Must be a `SELECT` returning zero or more rows; each row produces one
    /// [`crate::driver::CheckCatalogEntry`]-worth of finding data.
    pub sql: String,
    /// CWE identifiers for SARIF output, e.g. `["CWE-732"]`.
    #[serde(default)]
    pub cwe: Vec<String>,
    /// Long-form rationale surfaced by `--explain`.
    #[serde(default)]
    pub explanation: Option<String>,
    /// Remediation guidance surfaced by `--explain`.
    #[serde(default)]
    pub remediation: Option<String>,
    /// Minimum Postgres major version, e.g. `12`, `14`, `15`, `16`.
    ///
    /// The runner converts this to `server_version_num` form (multiplied by
    /// 10000) and compares to the live cluster's version. Checks declaring
    /// a higher major version than the cluster's are skipped silently.
    #[serde(default)]
    pub min_pg_version: Option<u32>,
}

// ---------------------------------------------------------------------------
// Validation errors
// ---------------------------------------------------------------------------

/// Validation error for a user-supplied or canonical [`CheckSpec`].
#[derive(Debug, thiserror::Error)]
pub enum SpecValidationError {
    /// User check ID is missing the required `USER-INS-` prefix.
    #[error("check id {0:?} must start with 'USER-INS-'")]
    MissingUserPrefix(String),
    /// User check ID collides with a canonical built-in ID.
    #[error("check id {0:?} collides with a built-in canonical check")]
    CanonicalIdCollision(String),
    /// A `{var}` placeholder in `message` is not present in the SQL projection.
    #[error(
        "check {check_id:?}: message placeholder '{{{binding}}}' not found in SQL projection \
         (available: {available:?})"
    )]
    MessageBindingMissing {
        check_id: String,
        binding: String,
        available: Vec<String>,
    },
    /// The SQL body could not be parsed.
    #[error("check {check_id:?}: SQL parse error: {reason}")]
    SqlParseError { check_id: String, reason: String },
    /// The SQL body is not a SELECT statement.
    #[error("check {check_id:?}: SQL must be a SELECT statement, got a different statement type")]
    SqlNotSelect { check_id: String },
}

impl CheckSpec {
    /// Validate that a user-supplied check carries the required `USER-INS-`
    /// prefix and does not collide with any canonical ID.
    pub fn validate_user_check(&self) -> Result<(), SpecValidationError> {
        if !self.id.starts_with("USER-INS-") {
            return Err(SpecValidationError::MissingUserPrefix(self.id.clone()));
        }
        if CANONICAL_CHECK_IDS.contains(&self.id.as_str()) {
            return Err(SpecValidationError::CanonicalIdCollision(self.id.clone()));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Message-binding validation
// ---------------------------------------------------------------------------

/// Extract all `{var}` placeholder names from a message template string.
fn extract_message_placeholders(message: &str) -> Vec<String> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\{(\w+)\}").expect("placeholder regex is valid"));
    re.captures_iter(message).map(|cap| cap[1].to_string()).collect()
}

/// Best-effort column name extraction from an expression.
fn expr_to_name(expr: &sqlparser::ast::Expr) -> String {
    use sqlparser::ast::{Expr, Ident, ObjectNamePart};
    match expr {
        Expr::Identifier(Ident { value, .. }) => value.to_ascii_lowercase(),
        Expr::CompoundIdentifier(parts) => parts.last().map(|i| i.value.to_ascii_lowercase()).unwrap_or_default(),
        Expr::Function(f) => f
            .name
            .0
            .last()
            .and_then(|p| match p {
                ObjectNamePart::Identifier(ident) => Some(ident.value.to_ascii_lowercase()),
                ObjectNamePart::Function(_) => None,
            })
            .unwrap_or_default(),
        Expr::Cast { expr, .. } => expr_to_name(expr),
        _ => String::new(),
    }
}

/// Validate that every `{var}` placeholder in `spec.message` corresponds to a
/// column alias that the `spec.sql` SELECT actually returns.
///
/// This runs at registry-load time for canonical checks and at
/// `with_user_checks` time for user-defined checks, so binding mismatches are
/// caught before any database is queried.
///
/// If the SQL uses a `SELECT *` or we cannot statically determine the
/// projection (e.g. CTE-only bodies), validation is skipped (returns `Ok(())`).
pub fn validate_message_bindings(spec: &CheckSpec) -> Result<(), SpecValidationError> {
    let stmts =
        Parser::parse_sql(&PostgreSqlDialect {}, &spec.sql).map_err(|e| SpecValidationError::SqlParseError {
            check_id: spec.id.clone(),
            reason: format!("{e}"),
        })?;

    let stmt = match stmts.into_iter().next() {
        Some(s) => s,
        None => {
            return Err(SpecValidationError::SqlParseError {
                check_id: spec.id.clone(),
                reason: "empty SQL body".to_string(),
            });
        }
    };

    let query = match stmt {
        Statement::Query(q) => q,
        _ => {
            return Err(SpecValidationError::SqlNotSelect {
                check_id: spec.id.clone(),
            });
        }
    };

    use sqlparser::ast::{SelectItem, SetExpr};
    let select = match *query.body {
        SetExpr::Select(s) => s,
        _ => {
            // Can't inspect a UNION/VALUES body statically — skip validation.
            return Ok(());
        }
    };

    // If any projection item is a wildcard or multi-alias we can't enumerate
    // columns statically — skip validation.
    let has_wildcard = select.projection.iter().any(|item| {
        matches!(
            item,
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) | SelectItem::ExprWithAliases { .. }
        )
    });
    if has_wildcard {
        return Ok(());
    }

    let projection: Vec<String> = select
        .projection
        .iter()
        .map(|item| match item {
            SelectItem::ExprWithAlias { alias, .. } => alias.value.to_ascii_lowercase(),
            SelectItem::UnnamedExpr(expr) => expr_to_name(expr),
            // wildcard / multi-alias handled above
            _ => String::new(),
        })
        .filter(|s| !s.is_empty())
        .collect();

    let placeholders = extract_message_placeholders(&spec.message);

    for ph in &placeholders {
        if !projection.contains(&ph.to_ascii_lowercase()) {
            return Err(SpecValidationError::MessageBindingMissing {
                check_id: spec.id.clone(),
                binding: ph.clone(),
                available: projection,
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// ConfigError — mirrors AuditConfigError shape
// ---------------------------------------------------------------------------

/// Errors that can arise while loading or validating a user-supplied check
/// TOML file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read check file '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse check file '{path}': {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("check file '{path}' has schema_version {found}, expected {expected}")]
    SchemaVersionMismatch { path: String, found: u32, expected: u32 },
    #[error("invalid check '{check_id}' in '{path}': {reason}")]
    InvalidCheck {
        path: String,
        check_id: String,
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// File-loading helpers
// ---------------------------------------------------------------------------

/// Parse a TOML check file from an in-memory string.
///
/// `source` is used solely for error-message attribution.
pub fn parse_check_file(content: &str, source: &str) -> Result<Vec<CheckSpec>, ConfigError> {
    let file: CheckFile = toml::from_str(content).map_err(|e| ConfigError::Toml {
        path: source.to_string(),
        source: e,
    })?;

    if file.schema_version != SCHEMA_VERSION {
        return Err(ConfigError::SchemaVersionMismatch {
            path: source.to_string(),
            found: file.schema_version,
            expected: SCHEMA_VERSION,
        });
    }

    Ok(file.checks)
}

/// Read a TOML check file from disk and return the parsed [`CheckSpec`]s.
pub fn load_checks_from_file(path: &Path) -> Result<Vec<CheckSpec>, ConfigError> {
    let path_str = path.display().to_string();
    let content = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
        path: path_str.clone(),
        source: e,
    })?;
    parse_check_file(&content, &path_str)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_spec(id: &str, message: &str, sql: &str) -> CheckSpec {
        CheckSpec {
            id: id.to_string(),
            name: "test-check".to_string(),
            category: CheckCategory::Performance,
            severity: Severity::Warn,
            engines: vec!["postgres".to_string()],
            description: "test description".to_string(),
            message: message.to_string(),
            sql: sql.to_string(),
            cwe: vec![],
            explanation: None,
            remediation: None,
            min_pg_version: None,
        }
    }

    #[test]
    fn canonical_checks_toml_parses() {
        let content = include_str!("postgres/checks.toml");
        let file: CheckFile = toml::from_str(content).expect("canonical TOML parses");
        assert_eq!(file.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn canonical_checks_count_matches_canonical_ids() {
        use crate::spec::CANONICAL_CHECK_IDS;
        let content = include_str!("postgres/checks.toml");
        let file: CheckFile = toml::from_str(content).expect("canonical TOML parses");
        let sc_ins: Vec<_> = file.checks.iter().filter(|c| c.id.starts_with("SC-INS")).collect();
        assert_eq!(
            sc_ins.len(),
            CANONICAL_CHECK_IDS.len(),
            "TOML SC-INS* count ({}) must match CANONICAL_CHECK_IDS length ({})",
            sc_ins.len(),
            CANONICAL_CHECK_IDS.len(),
        );
    }

    #[test]
    fn validate_message_bindings_catches_missing_binding() {
        let spec = make_spec("SC-INS01", "table {foo} is broken", "SELECT bar AS bar FROM pg_class");
        let err = validate_message_bindings(&spec).unwrap_err();
        match err {
            SpecValidationError::MessageBindingMissing { binding, .. } => {
                assert_eq!(binding, "foo");
            }
            other => panic!("expected MessageBindingMissing, got {other:?}"),
        }
    }

    #[test]
    fn validate_message_bindings_passes_when_all_bound() {
        let spec = make_spec(
            "SC-INS01",
            "table {schema_name}.{table_name}",
            "SELECT n.nspname AS schema_name, c.relname AS table_name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace",
        );
        validate_message_bindings(&spec).expect("all bindings present");
    }

    #[test]
    fn validate_user_check_requires_prefix() {
        let spec = make_spec("BAD-001", "msg", "SELECT 1 AS x");
        let err = spec.validate_user_check().unwrap_err();
        assert!(matches!(err, SpecValidationError::MissingUserPrefix(_)));
    }

    #[test]
    fn validate_user_check_rejects_canonical_collision() {
        let spec = make_spec("SC-INS01", "msg", "SELECT 1 AS x");
        // SC-INS01 doesn't start with USER-INS- so it hits MissingUserPrefix first.
        // Test the canonical collision path with a USER-INS- prefixed but canonical ID:
        // (canonical IDs are all SC-INS*, so a USER-INS- can't actually collide — but
        // the method guards against it defensively)
        let result = spec.validate_user_check();
        assert!(result.is_err());
        // The error should be MissingUserPrefix since "SC-INS01" doesn't start with "USER-INS-"
        assert!(matches!(result.unwrap_err(), SpecValidationError::MissingUserPrefix(_)));
    }

    #[test]
    fn validate_user_check_accepts_valid_user_id() {
        let spec = make_spec("USER-INS-001", "msg {x}", "SELECT 1 AS x");
        assert!(spec.validate_user_check().is_ok());
    }
}
