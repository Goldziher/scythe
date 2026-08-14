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
use sqlparser::dialect::{Dialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

/// Schema version used in TOML check files. Reject files with a higher
/// version so we can evolve the format without silently misreading fields.
pub const SCHEMA_VERSION: u32 = 1;

/// The canonical built-in check IDs that users cannot override or reuse.
pub const CANONICAL_CHECK_IDS: &[&str] = &[
    "SC-INS01", "SC-INS02", "SC-INS03", "SC-INS04", "SC-INS05", "SC-INS06", "SC-INS07", "SC-INS08", "SC-INS09",
    "SC-INS10", "SC-INS11", "SC-INS12", "SC-INS13",
];

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

/// Top-level structure of a check TOML file.
#[derive(Debug, Deserialize)]
pub struct CheckFile {
    /// Must equal [`SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The checks defined in this file.
    #[serde(rename = "check")]
    pub checks: Vec<CheckSpec>,
}

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

    /// The projected column whose value names the object a finding is about —
    /// what `[[inspect.suppression]] object = "…"` is compared against.
    ///
    /// Declared rather than guessed. The suppression engine used to pick the
    /// binding by scanning the result row for a key containing `"name"`, over a
    /// `HashMap` whose iteration order is randomised per process: SC-INS01 has
    /// two qualifying keys (`table_name`, `constraint_name`) and SC-INS06 has
    /// three, so the same config suppressed on some runs and not on others.
    /// SC-INS12 could never be suppressed at all, because it aliases its object
    /// column `parent_table` and no substring search for `"name"` will ever
    /// find it.
    #[serde(default)]
    pub object_binding: Option<String>,

    /// The projected column whose value names the schema a finding is in —
    /// what `[[inspect.suppression]] schema = "…"` is compared against.
    ///
    /// Declared for the same reason as [`CheckSpec::object_binding`]: a
    /// substring search cannot tell that a check projects no schema at all, and
    /// silently declines to suppress instead of saying so.
    #[serde(default)]
    pub schema_binding: Option<String>,
}

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
    /// `object_binding` or `schema_binding` names a column the SQL does not
    /// project, so `[[inspect.suppression]]` could never match on it.
    #[error(
        "check {check_id:?}: {field} = {binding:?} is not a column this check's SQL projects \
         (available: {available:?})"
    )]
    SuppressionBindingMissing {
        check_id: String,
        field: &'static str,
        binding: String,
        available: Vec<String>,
    },
    /// A canonical built-in check does not declare `object_binding`.
    #[error(
        "check {check_id:?}: canonical checks must declare object_binding so \
         `[[inspect.suppression]] object = \"…\"` has a column to compare against"
    )]
    MissingObjectBinding { check_id: String },
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
    let Some(projection) = sql_projection(spec)? else {
        return Ok(());
    };

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

/// Validate that `object_binding` and `schema_binding` name columns the check's
/// SQL actually projects.
///
/// Checked statically, at registry-load time, because the failure it prevents
/// is invisible at runtime: a suppression rule pointed at a column that does
/// not exist does not error, it simply never matches, and the user sees a
/// finding they believed they had silenced with no indication why.
///
/// `require_object_binding` is set for canonical built-in checks, where the
/// binding is not optional — every one of them projects an object column, and a
/// missing declaration would send suppression back to guessing.
pub fn validate_suppression_bindings(
    spec: &CheckSpec,
    require_object_binding: bool,
) -> Result<(), SpecValidationError> {
    if require_object_binding && spec.object_binding.is_none() {
        return Err(SpecValidationError::MissingObjectBinding {
            check_id: spec.id.clone(),
        });
    }

    if spec.object_binding.is_none() && spec.schema_binding.is_none() {
        return Ok(());
    }

    let Some(projection) = sql_projection(spec)? else {
        return Ok(());
    };

    for (field, binding) in [
        ("object_binding", spec.object_binding.as_deref()),
        ("schema_binding", spec.schema_binding.as_deref()),
    ] {
        let Some(binding) = binding else { continue };
        if !projection.contains(&binding.to_ascii_lowercase()) {
            return Err(SpecValidationError::SuppressionBindingMissing {
                check_id: spec.id.clone(),
                field,
                binding: binding.to_string(),
                available: projection,
            });
        }
    }

    Ok(())
}

/// The lowercased column names `spec.sql` projects, or `None` when they cannot
/// be determined statically (a wildcard projection, a set operation, a
/// CTE-only body).
///
/// `None` means "unknown", never "empty": treating an undeterminable projection
/// as an empty one would reject every binding on a check whose SQL is merely
/// shaped in a way this parser does not model.
fn sql_projection(spec: &CheckSpec) -> Result<Option<Vec<String>>, SpecValidationError> {
    let dialect: Box<dyn Dialect> = if spec.engines.iter().any(|e| e == "mysql") {
        Box::new(MySqlDialect {})
    } else {
        Box::new(PostgreSqlDialect {})
    };

    let stmts = Parser::parse_sql(&*dialect, &spec.sql).map_err(|e| SpecValidationError::SqlParseError {
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
        _ => return Ok(None),
    };

    let has_wildcard = select.projection.iter().any(|item| {
        matches!(
            item,
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _) | SelectItem::ExprWithAliases { .. }
        )
    });
    if has_wildcard {
        return Ok(None);
    }

    Ok(Some(
        select
            .projection
            .iter()
            .map(|item| match item {
                SelectItem::ExprWithAlias { alias, .. } => alias.value.to_ascii_lowercase(),
                SelectItem::UnnamedExpr(expr) => expr_to_name(expr),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect(),
    ))
}

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
            object_binding: None,
            schema_binding: None,
        }
    }

    #[test]
    fn canonical_checks_toml_parses() {
        let content = include_str!("postgres/checks.toml");
        let file: CheckFile = toml::from_str(content).expect("canonical TOML parses");
        assert_eq!(file.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn canonical_mysql_checks_toml_parses() {
        let content = include_str!("mysql/checks.toml");
        let file: CheckFile = toml::from_str(content).expect("canonical mysql TOML parses");
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert_eq!(file.checks.len(), 4);
        assert!(file.checks.iter().all(|c| c.engines == vec!["mysql".to_string()]));
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

    /// A check declaring `engines = ["mysql"]` must have its SQL parsed under
    /// the MySQL dialect for binding validation, not PostgreSQL's default.
    /// Backtick-quoted identifiers are valid MySQL syntax and invalid
    /// PostgreSQL syntax, so this would fail to validate under the wrong
    /// dialect even though the check is well-formed.
    #[test]
    fn validate_message_bindings_uses_mysql_dialect_for_a_mysql_check() {
        let mut spec = make_spec(
            "SC-INS-MY01",
            "table {table_name}",
            "SELECT `c`.`TABLE_NAME` AS table_name FROM `information_schema`.`TABLES` `c`",
        );
        spec.engines = vec!["mysql".to_string()];
        validate_message_bindings(&spec).expect("backtick-quoted identifiers parse under the MySQL dialect");
    }

    /// A binding pointed at a column the SQL does not project cannot ever
    /// match, and at runtime that looks exactly like "the suppression rule is
    /// wrong" rather than "the check is wrong". Catching it at load time is
    /// what turns a silent non-match into a message.
    #[test]
    fn should_reject_an_object_binding_the_sql_does_not_project() {
        let mut spec = make_spec(
            "USER-INS-001",
            "table {table_name}",
            "SELECT c.relname AS table_name FROM pg_class c",
        );
        spec.object_binding = Some("parent_table".to_string());

        let err = validate_suppression_bindings(&spec, false).unwrap_err();
        let SpecValidationError::SuppressionBindingMissing { field, binding, .. } = err else {
            panic!("expected SuppressionBindingMissing, got {err:?}");
        };
        assert_eq!(field, "object_binding");
        assert_eq!(binding, "parent_table");
    }

    #[test]
    fn should_reject_a_schema_binding_the_sql_does_not_project() {
        let mut spec = make_spec(
            "USER-INS-001",
            "table {table_name}",
            "SELECT c.relname AS table_name FROM pg_class c",
        );
        spec.schema_binding = Some("schema_name".to_string());

        let err = validate_suppression_bindings(&spec, false).unwrap_err();
        assert!(matches!(
            err,
            SpecValidationError::SuppressionBindingMissing {
                field: "schema_binding",
                ..
            }
        ));
    }

    #[test]
    fn should_accept_bindings_the_sql_projects() {
        let mut spec = make_spec(
            "USER-INS-001",
            "table {schema_name}.{table_name}",
            "SELECT n.nspname AS schema_name, c.relname AS table_name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace",
        );
        spec.object_binding = Some("table_name".to_string());
        spec.schema_binding = Some("schema_name".to_string());

        validate_suppression_bindings(&spec, true).expect("both bindings are projected");
    }

    /// A canonical check without an object binding sends suppression back to
    /// guessing, which is the defect the field exists to remove — so the
    /// registry must refuse to load one.
    #[test]
    fn should_reject_a_canonical_check_that_declares_no_object_binding() {
        let spec = make_spec(
            "SC-INS99",
            "table {table_name}",
            "SELECT c.relname AS table_name FROM pg_class c",
        );

        let err = validate_suppression_bindings(&spec, true).unwrap_err();
        assert!(
            matches!(err, SpecValidationError::MissingObjectBinding { .. }),
            "{err:?}"
        );
    }

    /// A user check may leave both undeclared; only canonical checks are held
    /// to the stricter rule.
    #[test]
    fn should_accept_a_user_check_that_declares_no_bindings() {
        let spec = make_spec(
            "USER-INS-001",
            "table {table_name}",
            "SELECT c.relname AS table_name FROM pg_class c",
        );
        validate_suppression_bindings(&spec, false).expect("bindings are optional for user checks");
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
        let result = spec.validate_user_check();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SpecValidationError::MissingUserPrefix(_)));
    }

    #[test]
    fn validate_user_check_accepts_valid_user_id() {
        let spec = make_spec("USER-INS-001", "msg {x}", "SELECT 1 AS x");
        assert!(spec.validate_user_check().is_ok());
    }
}
