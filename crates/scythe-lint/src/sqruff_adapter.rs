use std::borrow::Cow;

use sqruff_lib::core::config::FluffConfig;
use sqruff_lib::core::linter::core::Linter;

use super::types::{SqruffConfig, Violation};

/// Sqruff violation with line/position info for display.
#[derive(Debug, Clone)]
pub struct SqruffViolation {
    pub violation: Violation,
    pub line_no: usize,
    pub line_pos: usize,
    pub fixable: bool,
}

/// Errors constructing or running the sqruff linter from `[lint.sqruff]`
/// configuration.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SqruffConfigError {
    /// `[lint.sqruff.rules]` maps a rule code to a value other than
    /// `"off"`.
    ///
    /// sqruff-lib 0.39 has no notion of per-rule severity: its `rules` key
    /// is a rule *allowlist*, not a severity table, and every sqruff
    /// finding is hardcoded to `Severity::Warn` regardless of any
    /// configured value. Only `"off"` (excluding the rule) can be honoured.
    #[error(
        "[lint.sqruff.rules] \"{rule}\" = \"{value}\" is not supported: sqruff has no per-rule severity, only \
         \"off\" is a valid value. Use \"off\" to disable the rule, or remove the entry to leave it enabled."
    )]
    UnsupportedRuleValue { rule: String, value: String },

    /// sqruff rejected the assembled configuration, e.g. because
    /// `[lint.sqruff.rules]` references an unknown rule code (a typo).
    #[error("sqruff rejected the linter configuration: {0}")]
    Linter(String),
}

/// Create a sqruff `FluffConfig` for the given dialect, optionally applying
/// rule include/exclude settings from [`SqruffConfig`].
/// Rules excluded by default due to upstream sqruff bugs.
/// LT01: incorrectly splits compound operators (>=, <=, <@, etc.) into separate tokens.
const DEFAULT_EXCLUDED_RULES: &[&str] = &["LT01"];

/// Build the sqruff config source for a dialect, applying `[lint.sqruff.rules]`.
///
/// Every entry in `rules` must be `"off"` — sqruff cannot express per-rule
/// severity (see [`SqruffConfigError::UnsupportedRuleValue`]). Any other
/// value is rejected rather than silently misinterpreted as an allowlist
/// entry.
fn make_config(dialect: &str, sqruff_config: Option<&SqruffConfig>) -> Result<FluffConfig, SqruffConfigError> {
    let mut source = format!("[sqruff]\ndialect = {}\n", dialect);

    let mut excluded: Vec<&str> = DEFAULT_EXCLUDED_RULES.to_vec();
    if let Some(cfg) = sqruff_config {
        for (k, v) in &cfg.rules {
            if v != "off" {
                return Err(SqruffConfigError::UnsupportedRuleValue {
                    rule: k.clone(),
                    value: v.clone(),
                });
            }
            if !excluded.contains(&k.as_str()) {
                excluded.push(k.as_str());
            }
        }
    }

    if !excluded.is_empty() {
        source.push_str(&format!("exclude_rules = {}\n", excluded.join(",")));
    }

    Ok(FluffConfig::from_source(&source, None))
}

fn make_linter(dialect: &str, sqruff_config: Option<&SqruffConfig>) -> Result<Linter, SqruffConfigError> {
    let config = make_config(dialect, sqruff_config)?;
    Linter::new(config, None, None, false).map_err(SqruffConfigError::Linter)
}

fn to_sqruff_violations(result: &sqruff_lib::core::linter::linted_file::LintedFile) -> Vec<SqruffViolation> {
    result
        .violations()
        .iter()
        .map(|v| {
            let rule_code = v.rule_code();
            SqruffViolation {
                violation: Violation {
                    rule_id: Cow::Owned(format!("SQ-{}", rule_code)),
                    message: v.description.clone(),
                    fix: None,
                },
                line_no: v.line_no,
                line_pos: v.line_pos,
                fixable: v.fixable,
            }
        })
        .collect()
}

/// Run sqruff's rules on SQL and return scythe Violations with position info.
///
/// Returns an empty result without running sqruff when `sqruff_config` is
/// present and `enabled = false`. Returns [`SqruffConfigError`] if the
/// configuration is invalid or sqruff itself rejects it (e.g. an unknown
/// rule code in `[lint.sqruff.rules]`) rather than silently reporting no
/// violations.
pub fn lint_sql(
    sql: &str,
    dialect: &str,
    sqruff_config: Option<&SqruffConfig>,
) -> Result<Vec<SqruffViolation>, SqruffConfigError> {
    if sqruff_config.is_some_and(|cfg| !cfg.enabled) {
        return Ok(Vec::new());
    }

    let linter = make_linter(dialect, sqruff_config)?;

    let result = linter
        .lint_string(sql, None, false)
        .map_err(|e| SqruffConfigError::Linter(e.value))?;

    Ok(to_sqruff_violations(&result))
}

/// Run sqruff lint with auto-fix enabled, returning violations and the fixed SQL.
///
/// Returns the input SQL unchanged with no violations, without running
/// sqruff, when `sqruff_config` is present and `enabled = false`. Returns
/// [`SqruffConfigError`] if the configuration is invalid or sqruff itself
/// rejects it, rather than silently reporting no violations.
pub fn lint_and_fix_sql(
    sql: &str,
    dialect: &str,
    sqruff_config: Option<&SqruffConfig>,
) -> Result<(Vec<SqruffViolation>, String), SqruffConfigError> {
    if sqruff_config.is_some_and(|cfg| !cfg.enabled) {
        return Ok((Vec::new(), sql.to_string()));
    }

    let linter = make_linter(dialect, sqruff_config)?;

    let result = linter
        .lint_string(sql, None, true)
        .map_err(|e| SqruffConfigError::Linter(e.value))?;

    let violations = to_sqruff_violations(&result);
    let fixed = result.fix_string();
    Ok((violations, fixed))
}

/// Format SQL using sqruff (lint with fix, return the fixed string).
///
/// Note: `[lint.sqruff].enabled` is intentionally NOT consulted here —
/// `scythe fmt` always formats regardless of whether sqruff-based linting
/// is disabled. `[lint.sqruff.rules]` validation still applies, since
/// `make_config`/`make_linter` are shared with [`lint_sql`].
pub fn format_sql(sql: &str, dialect: &str, sqruff_config: Option<&SqruffConfig>) -> Result<String, String> {
    let linter = make_linter(dialect, sqruff_config).map_err(|e| e.to_string())?;

    let result = linter.lint_string(sql, None, true).map_err(|e| e.value)?;

    let fixed = result.fix_string();

    Ok(rejoin_split_operators(&fixed))
}

/// Rejoin compound operators that sqruff incorrectly splits with whitespace.
fn rejoin_split_operators(sql: &str) -> String {
    sql.replace("> =", ">=")
        .replace("< =", "<=")
        .replace("! =", "!=")
        .replace("< >", "<>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lint_simple_sql() {
        let sql = "SELECT  id,  name  FROM  users  WHERE  id = 1\n";
        let violations = lint_sql(sql, "ansi", None);
        assert!(violations.is_ok());
    }

    #[test]
    fn format_simple_sql() {
        let sql = "select  id,name from users\n";
        let result = format_sql(sql, "ansi", None);
        assert!(result.is_ok());
    }

    #[test]
    fn lint_and_fix_returns_fixed_sql() {
        let sql = "select  id,name from users\n";
        let (_, fixed) = lint_and_fix_sql(sql, "ansi", None).expect("lint_and_fix_sql should succeed");
        assert!(!fixed.is_empty());
    }

    #[test]
    fn lint_with_sqruff_config() {
        let sql = "SELECT  id,  name  FROM  users  WHERE  id = 1\n";
        let cfg = SqruffConfig {
            enabled: true,
            rules: ahash::AHashMap::new(),
        };
        let violations = lint_sql(sql, "ansi", Some(&cfg));
        assert!(violations.is_ok());
    }

    /// #113: `enabled = false` must fully disable sqruff for `lint_sql`,
    /// producing no findings even for SQL that would otherwise violate
    /// rules.
    #[test]
    fn disabled_config_produces_no_lint_findings() {
        let sql = "select id FROM users where id = 1\n";
        let cfg = SqruffConfig {
            enabled: false,
            rules: ahash::AHashMap::new(),
        };

        // Sanity check: this SQL does produce a violation when enabled.
        let baseline = lint_sql(sql, "ansi", None).expect("baseline lint should succeed");
        assert!(!baseline.is_empty(), "expected baseline SQL to have violations");

        let violations = lint_sql(sql, "ansi", Some(&cfg)).expect("disabled config should not error");
        assert!(violations.is_empty(), "enabled = false must suppress all findings");
    }

    /// #113: `enabled = false` must fully disable sqruff for
    /// `lint_and_fix_sql`, returning the SQL unmodified.
    #[test]
    fn disabled_config_leaves_sql_unmodified_by_fix() {
        let sql = "select  id,name from users\n";
        let cfg = SqruffConfig {
            enabled: false,
            rules: ahash::AHashMap::new(),
        };

        let (violations, fixed) = lint_and_fix_sql(sql, "ansi", Some(&cfg)).expect("disabled config should not error");
        assert!(violations.is_empty());
        assert_eq!(fixed, sql, "disabled sqruff must not modify the SQL");
    }

    /// #114: a non-`"off"` value in `[lint.sqruff.rules]` must be rejected
    /// with a message naming the offending key and value, not silently
    /// treated as an allowlist entry.
    #[test]
    fn non_off_rule_value_is_rejected() {
        let sql = "SELECT 1\n";
        let mut rules = ahash::AHashMap::new();
        rules.insert("LT02".to_string(), "warn".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        let err = lint_sql(sql, "ansi", Some(&cfg)).expect_err("non-off rule value must be rejected");
        match &err {
            SqruffConfigError::UnsupportedRuleValue { rule, value } => {
                assert_eq!(rule, "LT02");
                assert_eq!(value, "warn");
            }
            other => panic!("expected UnsupportedRuleValue, got {other:?}"),
        }
        let message = err.to_string();
        assert!(message.contains("LT02"), "message should name the rule: {message}");
        assert!(message.contains("warn"), "message should name the value: {message}");
        assert!(
            message.contains("off"),
            "message should point users at the supported value: {message}"
        );
    }

    /// #114: `format_sql` shares `make_config`/`make_linter` with the lint
    /// path, so it must reject non-`"off"` values too.
    #[test]
    fn format_sql_also_rejects_non_off_rule_value() {
        let sql = "SELECT 1\n";
        let mut rules = ahash::AHashMap::new();
        rules.insert("LT02".to_string(), "error".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        let result = format_sql(sql, "ansi", Some(&cfg));
        assert!(result.is_err());
        let message = result.unwrap_err();
        assert!(message.contains("LT02"));
    }

    /// #114: `"off"` must still exclude the named rule, same as before.
    #[test]
    fn off_rule_value_still_excludes_rule() {
        let sql = "SELECT  id,  name  FROM  users  WHERE  id = 1\n";
        let mut rules = ahash::AHashMap::new();
        rules.insert("LT01".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        // LT01 is already excluded by default; this must not error and
        // must behave identically to the default-excluded case.
        let violations = lint_sql(sql, "ansi", Some(&cfg)).expect("off value should be accepted");
        let baseline = lint_sql(sql, "ansi", None).expect("baseline should succeed");
        assert_eq!(violations.len(), baseline.len());
    }

    /// Silent-swallow regression: an unknown/typo'd rule code in
    /// `[lint.sqruff.rules]` (value `"off"`, so it passes #114's
    /// allowlist-vs-severity check) must surface as an error, not silently
    /// disable all sqruff findings.
    #[test]
    fn unknown_rule_code_is_diagnosed_not_swallowed() {
        let sql = "select id FROM users where id = 1\n";
        let mut rules = ahash::AHashMap::new();
        rules.insert("ZZ99-NOT-A-REAL-RULE".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        // Sanity check: this SQL does produce a violation under default config.
        let baseline = lint_sql(sql, "ansi", None).expect("baseline lint should succeed");
        assert!(!baseline.is_empty(), "expected baseline SQL to have violations");

        let result = lint_sql(sql, "ansi", Some(&cfg));
        assert!(
            result.is_err(),
            "an unknown rule code must be diagnosed, not silently swallowed"
        );
        assert!(matches!(result.unwrap_err(), SqruffConfigError::Linter(_)));
    }

    /// Same silent-swallow regression, but for the fix path.
    #[test]
    fn unknown_rule_code_is_diagnosed_not_swallowed_on_fix() {
        let sql = "select  id,name from users\n";
        let mut rules = ahash::AHashMap::new();
        rules.insert("ZZ99-NOT-A-REAL-RULE".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        let result = lint_and_fix_sql(sql, "ansi", Some(&cfg));
        assert!(result.is_err());
    }
}
