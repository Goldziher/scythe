//! `[inspect]` section of `scythe.toml`.
//!
//! Provides [`InspectConfig`] (deserialized from TOML) and
//! [`parse_inspect_section`] which reads `scythe.toml`, extracts and validates
//! the `[inspect]` block, and returns it — or `None` when no such block exists.
//!
//! The shape mirrors the `[audit]` section; see `crates/scythe-lint/src/audit/`
//! for the precedent.
//!
//! ```toml
//! [inspect]
//! database_url = "postgres://localhost/dev"
//! api_schemas  = ["public", "api"]
//! extra_rules  = ["./inspect-rules.toml"]
//!
//! [inspect.severity_overrides]
//! "SC-INS10" = "error"
//! "SC-INS13" = "off"
//!
//! [[inspect.suppression]]
//! rule   = "SC-INS09"
//! schema = "public"
//! object = "pgtap"
//!
//! [[inspect.check]]
//! id          = "USER-INS-001"
//! name        = "no-comments-on-tables"
//! category    = "schema"
//! severity    = "warn"
//! engines     = ["postgres"]
//! description = "tables must have COMMENT ON TABLE"
//! message     = "table `{schema_name}.{table_name}` has no COMMENT"
//! sql         = """
//! SELECT n.nspname AS schema_name, c.relname AS table_name
//! FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
//! WHERE c.relkind = 'r' AND n.nspname = 'app'
//!   AND obj_description(c.oid, 'pg_class') IS NULL
//! """
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use scythe_lint::types::Severity;

use crate::spec::{CheckSpec, ConfigError, load_checks_from_file, validate_message_bindings};

// ---------------------------------------------------------------------------
// SuppressionRule
// ---------------------------------------------------------------------------

/// A single suppression rule from `[[inspect.suppression]]`.
///
/// A finding is suppressed when ALL fields that are `Some` match:
/// - `rule`   — the finding's `rule_id` must equal this.
/// - `schema` — the row binding key whose name contains `"schema"` (e.g.
///   `schema_name`) must equal this value.
/// - `object` — any binding key whose name contains `"name"` (e.g.
///   `table_name`, `extension_name`) must equal this value.
#[derive(Debug, Clone, Deserialize)]
pub struct SuppressionRule {
    /// Required: the rule ID this suppression silences (e.g. `"SC-INS09"`).
    pub rule: String,
    /// Optional: only suppress for this schema name.
    #[serde(default)]
    pub schema: Option<String>,
    /// Optional: only suppress for this object name (table, extension, etc.).
    #[serde(default)]
    pub object: Option<String>,
}

// ---------------------------------------------------------------------------
// InspectConfig
// ---------------------------------------------------------------------------

/// Deserialized representation of the `[inspect]` section in `scythe.toml`.
///
/// All fields default to `None` / empty collections so that a minimal
/// `[inspect]` block (`[inspect]` with no keys) is accepted without error.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InspectConfig {
    /// Optional database URL — lower precedence than CLI positional arg and
    /// the `DATABASE_URL` / `SCYTHE_DATABASE_URL` environment variables.
    #[serde(default)]
    pub database_url: Option<String>,

    /// Schemas to treat as the "API surface" for SC-INS10.
    ///
    /// SC-INS10 reports tables without RLS across ALL user schemas; the CLI
    /// post-filters findings to only those in `api_schemas` (or `["public"]`
    /// when this list is empty). Expand this list to add more schemas to the
    /// check scope.
    #[serde(default)]
    pub api_schemas: Vec<String>,

    /// Paths to additional check TOML files, resolved relative to `scythe.toml`.
    #[serde(default)]
    pub extra_rules: Vec<String>,

    /// Per-rule severity overrides.  Key is the check ID; value is the
    /// desired severity (`"warn"`, `"error"`, or `"off"`).  `"off"` removes
    /// the check entirely from the active set.
    #[serde(default)]
    pub severity_overrides: HashMap<String, Severity>,

    /// Suppression rules.  Each entry silences a specific finding.
    #[serde(default)]
    pub suppression: Vec<SuppressionRule>,

    /// Inline user-defined checks.  Each entry must have an ID prefixed with
    /// `USER-INS-` and must pass both `validate_user_check` and
    /// `validate_message_bindings`.
    #[serde(default, rename = "check")]
    pub check: Vec<CheckSpec>,
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Read `scythe.toml` at `config_path`, extract the `[inspect]` block (if
/// any), validate every user check in `inspect.check` and in `extra_rules`
/// files, and resolve `extra_rules` paths relative to `config_path`'s parent
/// directory.
///
/// Returns:
/// - `Ok(None)` if `config_path` doesn't exist or has no `[inspect]` block.
/// - `Ok(Some(config))` on success.
/// - `Err(ConfigError)` if the TOML is malformed or validation fails.
///
/// User-defined `[[inspect.check]]` blocks must:
/// 1. Pass `CheckSpec::validate_user_check` (ID must start with `USER-INS-`).
/// 2. Pass `validate_message_bindings` (all `{var}` placeholders present in SQL).
///
/// Checks loaded from `extra_rules` files go through the same validation.
pub fn parse_inspect_section(config_path: &Path) -> Result<Option<InspectConfig>, ConfigError> {
    if !config_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(config_path).map_err(|e| ConfigError::Io {
        path: config_path.display().to_string(),
        source: e,
    })?;

    let parsed: toml::Value = toml::from_str(&content).map_err(|e| ConfigError::Toml {
        path: config_path.display().to_string(),
        source: e,
    })?;

    let inspect_value = match parsed.get("inspect") {
        Some(v) => v,
        None => return Ok(None),
    };

    let mut config: InspectConfig =
        inspect_value
            .clone()
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError::Toml {
                path: config_path.display().to_string(),
                source: e,
            })?;

    let config_dir = config_path.parent().unwrap_or(Path::new("."));

    // ------------------------------------------------------------------
    // Validate inline [[inspect.check]] specs.
    // ------------------------------------------------------------------
    for spec in &config.check {
        spec.validate_user_check()
            .map_err(|e| ConfigError::InvalidCheck {
                path: config_path.display().to_string(),
                check_id: spec.id.clone(),
                reason: e.to_string(),
            })?;
        validate_message_bindings(spec).map_err(|e| ConfigError::InvalidCheck {
            path: config_path.display().to_string(),
            check_id: spec.id.clone(),
            reason: e.to_string(),
        })?;
    }

    // ------------------------------------------------------------------
    // Load and validate extra_rules files.
    // ------------------------------------------------------------------
    // Resolve each path relative to config_path's directory and replace
    // the string list with absolute-ish paths so callers don't need to
    // repeat the resolution.
    let mut resolved_extra_rules: Vec<String> = Vec::new();
    for rel_path in &config.extra_rules {
        let abs_path = config_dir.join(rel_path);
        let abs_str = abs_path.display().to_string();

        let specs = load_checks_from_file(&abs_path)?;
        for spec in &specs {
            spec.validate_user_check()
                .map_err(|e| ConfigError::InvalidCheck {
                    path: abs_str.clone(),
                    check_id: spec.id.clone(),
                    reason: e.to_string(),
                })?;
            validate_message_bindings(spec).map_err(|e| ConfigError::InvalidCheck {
                path: abs_str.clone(),
                check_id: spec.id.clone(),
                reason: e.to_string(),
            })?;
        }

        resolved_extra_rules.push(abs_str);
    }
    config.extra_rules = resolved_extra_rules;

    Ok(Some(config))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write `content` to a uniquely-named temp file and return its path.
    ///
    /// The caller must keep the returned `std::path::PathBuf` alive for the
    /// duration of the test; the file is deleted when the `PathBuf` drops
    /// because we use a predictable path based on the thread ID and a counter.
    /// Since tests each write a different path this is safe.
    fn write_toml(content: &str) -> (std::path::PathBuf, std::fs::File) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("scythe_inspect_cfg_test_{}.toml", n));
        let mut f = std::fs::File::create(&path).expect("create temp file");
        f.write_all(content.as_bytes()).expect("write");
        (path, f)
    }

    // -----------------------------------------------------------------------
    // parses_inspect_section_full
    // -----------------------------------------------------------------------

    #[test]
    fn parses_inspect_section_full() {
        let toml = r#"
[inspect]
database_url = "postgres://localhost/dev"
api_schemas  = ["public", "api"]
extra_rules  = []

[inspect.severity_overrides]
"SC-INS10" = "error"
"SC-INS13" = "off"

[[inspect.suppression]]
rule   = "SC-INS09"
schema = "public"
object = "pgtap"

[[inspect.check]]
id          = "USER-INS-001"
name        = "test-check"
category    = "schema"
severity    = "warn"
engines     = ["postgres"]
description = "test"
message     = "schema {schema_name} table {table_name}"
sql         = "SELECT n.nspname AS schema_name, c.relname AS table_name FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind = 'r'"
"#;
        let (path, _f) = write_toml(toml);
        let cfg = parse_inspect_section(&path)
            .expect("parses")
            .expect("has inspect block");

        assert_eq!(
            cfg.database_url.as_deref(),
            Some("postgres://localhost/dev")
        );
        assert_eq!(cfg.api_schemas, vec!["public", "api"]);
        assert_eq!(
            cfg.severity_overrides.get("SC-INS10"),
            Some(&Severity::Error)
        );
        assert_eq!(cfg.severity_overrides.get("SC-INS13"), Some(&Severity::Off));
        assert_eq!(cfg.suppression.len(), 1);
        assert_eq!(cfg.suppression[0].rule, "SC-INS09");
        assert_eq!(cfg.suppression[0].schema.as_deref(), Some("public"));
        assert_eq!(cfg.suppression[0].object.as_deref(), Some("pgtap"));
        assert_eq!(cfg.check.len(), 1);
        assert_eq!(cfg.check[0].id, "USER-INS-001");
    }

    // -----------------------------------------------------------------------
    // parses_inspect_section_minimal
    // -----------------------------------------------------------------------

    #[test]
    fn parses_inspect_section_minimal() {
        let toml = "[inspect]\n";
        let (path, _f) = write_toml(toml);
        let cfg = parse_inspect_section(&path)
            .expect("parses")
            .expect("has inspect block");

        assert!(cfg.database_url.is_none());
        assert!(cfg.api_schemas.is_empty());
        assert!(cfg.extra_rules.is_empty());
        assert!(cfg.severity_overrides.is_empty());
        assert!(cfg.suppression.is_empty());
        assert!(cfg.check.is_empty());
    }

    // -----------------------------------------------------------------------
    // returns_none_when_no_inspect_block
    // -----------------------------------------------------------------------

    #[test]
    fn returns_none_when_no_inspect_block() {
        let toml = "[audit]\nextra_rules = []\n";
        let (path, _f) = write_toml(toml);
        let result = parse_inspect_section(&path).expect("parses");
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // returns_none_when_file_missing
    // -----------------------------------------------------------------------

    #[test]
    fn returns_none_when_file_missing() {
        let result =
            parse_inspect_section(Path::new("/tmp/nonexistent-scythe-abc123.toml")).expect("ok");
        assert!(result.is_none());
    }

    // -----------------------------------------------------------------------
    // rejects_canonical_id_in_user_check
    // -----------------------------------------------------------------------

    #[test]
    fn rejects_canonical_id_in_user_check() {
        // SC-INS01 doesn't have the USER-INS- prefix, so the error is
        // MissingUserPrefix (wrapped in ConfigError::InvalidCheck).
        let toml = r#"
[inspect]
[[inspect.check]]
id          = "SC-INS01"
name        = "collision"
category    = "schema"
severity    = "warn"
engines     = ["postgres"]
description = "test"
message     = "msg"
sql         = "SELECT 1 AS x"
"#;
        let (path, _f) = write_toml(toml);
        let err = parse_inspect_section(&path).expect_err("should fail validation");
        match err {
            ConfigError::InvalidCheck { check_id, .. } => {
                assert_eq!(check_id, "SC-INS01");
            }
            other => panic!("expected InvalidCheck, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // rejects_missing_user_prefix
    // -----------------------------------------------------------------------

    #[test]
    fn rejects_missing_user_prefix() {
        let toml = r#"
[inspect]
[[inspect.check]]
id          = "BAD-001"
name        = "bad"
category    = "schema"
severity    = "warn"
engines     = ["postgres"]
description = "test"
message     = "msg"
sql         = "SELECT 1 AS x"
"#;
        let (path, _f) = write_toml(toml);
        let err = parse_inspect_section(&path).expect_err("should fail");
        match err {
            ConfigError::InvalidCheck {
                check_id, reason, ..
            } => {
                assert_eq!(check_id, "BAD-001");
                assert!(reason.contains("USER-INS-"), "reason: {reason}");
            }
            other => panic!("expected InvalidCheck, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // rejects_user_check_with_invalid_binding
    // -----------------------------------------------------------------------

    #[test]
    fn rejects_user_check_with_invalid_binding() {
        // message has {foo} but SQL only projects `bar`
        let toml = r#"
[inspect]
[[inspect.check]]
id          = "USER-INS-002"
name        = "bad-binding"
category    = "schema"
severity    = "warn"
engines     = ["postgres"]
description = "test"
message     = "problem with {foo}"
sql         = "SELECT 1 AS bar"
"#;
        let (path, _f) = write_toml(toml);
        let err = parse_inspect_section(&path).expect_err("should fail");
        match err {
            ConfigError::InvalidCheck {
                check_id, reason, ..
            } => {
                assert_eq!(check_id, "USER-INS-002");
                assert!(reason.contains("foo"), "reason: {reason}");
            }
            other => panic!("expected InvalidCheck, got {other:?}"),
        }
    }
}
