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

/// The SQL a freshly built linter is asked to lint, purely to force sqruff to
/// validate the rule codes in `[lint.sqruff.rules]`.
///
/// `Linter::new` accepts an `exclude_rules` list without looking at it;
/// sqruff only resolves rule codes when a string is actually linted. Probing
/// with a trivial statement is therefore what turns a typo'd rule code into a
/// construction error instead of a run that silently reports nothing.
const RULE_CODE_PROBE_SQL: &str = "SELECT 1\n";

/// A constructed sqruff linter, reusable across every file in a run.
///
/// Building one is expensive and building one is *all* that is expensive:
/// `FluffConfig::from_source` constructs the dialect, which compiles the
/// dialect's `fancy_regex` lexer. On a 2000-query project that construction
/// measured at 61% of the whole `scythe lint` process — more than twice what
/// the actual linting cost (#130). Every method here takes `&self`, so one
/// linter serves an entire `[[sql]]` block (or, for `scythe fmt`, an entire
/// run).
///
/// Construction also *validates*: it probes the assembled configuration with
/// [`RULE_CODE_PROBE_SQL`], so an unusable `[lint.sqruff]` fails when the
/// linter is built rather than against whichever query file happened to be
/// read first. That is what lets a caller hoist construction out of its
/// per-file loop without turning a configuration mistake into a per-file
/// error: by the time [`lint`](Self::lint) runs, the configuration is known
/// good.
pub struct SqruffLinter {
    linter: Linter,
}

impl SqruffLinter {
    /// Build and validate a linter for `dialect`.
    ///
    /// `[lint.sqruff].enabled` is deliberately **not** consulted: this is the
    /// constructor `scythe fmt` uses, and `fmt` formats regardless of whether
    /// sqruff-based *linting* is switched off. Callers that must honour
    /// `enabled` want [`for_linting`](Self::for_linting) instead.
    pub fn new(dialect: &str, sqruff_config: Option<&SqruffConfig>) -> Result<Self, SqruffConfigError> {
        let config = make_config(dialect, sqruff_config)?;
        let linter = Linter::new(config, None, None, false).map_err(SqruffConfigError::Linter)?;

        linter
            .lint_string(RULE_CODE_PROBE_SQL, None, false)
            .map_err(|e| SqruffConfigError::Linter(e.value))?;

        Ok(Self { linter })
    }

    /// Build a linter for the *lint* path, honouring `[lint.sqruff].enabled`.
    ///
    /// Returns `Ok(None)` when `enabled = false`. Nothing is constructed and
    /// nothing is validated in that case, matching what the lint path does
    /// with a disabled config: it never reaches sqruff at all, so a
    /// configuration it will never read must not fail the run.
    pub fn for_linting(dialect: &str, sqruff_config: Option<&SqruffConfig>) -> Result<Option<Self>, SqruffConfigError> {
        if sqruff_config.is_some_and(|cfg| !cfg.enabled) {
            return Ok(None);
        }
        Self::new(dialect, sqruff_config).map(Some)
    }

    /// Run sqruff's rules over `sql`.
    pub fn lint(&self, sql: &str) -> Result<Vec<SqruffViolation>, SqruffConfigError> {
        let result = self
            .linter
            .lint_string(sql, None, false)
            .map_err(|e| SqruffConfigError::Linter(e.value))?;
        Ok(to_sqruff_violations(&result))
    }

    /// Run sqruff's rules over `sql` with auto-fix enabled, returning the
    /// violations found and the fixed SQL.
    pub fn lint_and_fix(&self, sql: &str) -> Result<(Vec<SqruffViolation>, String), SqruffConfigError> {
        let result = self
            .linter
            .lint_string(sql, None, true)
            .map_err(|e| SqruffConfigError::Linter(e.value))?;
        let violations = to_sqruff_violations(&result);
        Ok((violations, result.fix_string()))
    }

    /// Format `sql` (lint with fix, return the fixed string).
    ///
    /// The error is sqruff's own message, unwrapped: by the time a linter
    /// exists its configuration has already been validated, so a failure here
    /// is about *this* SQL and must not be reported as a configuration
    /// problem.
    pub fn format(&self, sql: &str) -> Result<String, String> {
        let result = self.linter.lint_string(sql, None, true).map_err(|e| e.value)?;
        Ok(rejoin_split_operators(&result.fix_string()))
    }
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

    /// Build the linter the `scythe lint` path builds, asserting the config
    /// is one it accepts. `for_linting` returns `None` for `enabled = false`;
    /// every caller of this helper is testing the enabled case.
    fn enabled_linter(dialect: &str, sqruff_config: Option<&SqruffConfig>) -> SqruffLinter {
        SqruffLinter::for_linting(dialect, sqruff_config)
            .expect("configuration should be accepted")
            .expect("an enabled configuration must build a linter")
    }

    #[test]
    fn lint_simple_sql() {
        let sql = "SELECT  id,  name  FROM  users  WHERE  id = 1\n";
        let linter = SqruffLinter::new("ansi", None).expect("linter should build");
        assert!(linter.lint(sql).is_ok());
    }

    /// SQL the fix path actually rewrites, and the result it must produce.
    ///
    /// Picked deliberately: most of the obvious "badly written" SQL is left
    /// untouched here, because `LT01` — the layout rule behind nearly every
    /// whitespace fix — is in [`DEFAULT_EXCLUDED_RULES`]. A fix-path test
    /// built on SQL sqruff does not change asserts nothing.
    const FIXABLE_SQL: &str = "SELECT a\nFROM b\nwhere a = 1\n";
    const FIXED_SQL: &str = "SELECT a\nFROM b\nWHERE a = 1\n";

    #[test]
    fn format_simple_sql() {
        let linter = SqruffLinter::new("ansi", None).expect("linter should build");
        let formatted = linter.format(FIXABLE_SQL).expect("format should succeed");
        assert_eq!(formatted, FIXED_SQL);
    }

    #[test]
    fn lint_and_fix_returns_fixed_sql() {
        let linter = SqruffLinter::new("ansi", None).expect("linter should build");
        let (violations, fixed) = linter.lint_and_fix(FIXABLE_SQL).expect("lint_and_fix should succeed");
        assert!(!violations.is_empty(), "expected the fixable SQL to violate a rule");
        assert_eq!(fixed, FIXED_SQL);
    }

    #[test]
    fn lint_with_sqruff_config() {
        let sql = "SELECT  id,  name  FROM  users  WHERE  id = 1\n";
        let cfg = SqruffConfig {
            enabled: true,
            rules: ahash::AHashMap::new(),
        };
        assert!(enabled_linter("ansi", Some(&cfg)).lint(sql).is_ok());
    }

    /// #113: `enabled = false` must fully disable sqruff on the lint path.
    /// `for_linting` builds nothing, and `scythe lint` reports nothing for
    /// that `None` — even for SQL that would otherwise violate rules.
    #[test]
    fn disabled_config_produces_no_lint_findings() {
        let sql = "select id FROM users where id = 1\n";
        let cfg = SqruffConfig {
            enabled: false,
            rules: ahash::AHashMap::new(),
        };

        // Sanity check: this SQL does produce a violation when enabled.
        let baseline = enabled_linter("ansi", None)
            .lint(sql)
            .expect("baseline lint should succeed");
        assert!(!baseline.is_empty(), "expected baseline SQL to have violations");

        let built = SqruffLinter::for_linting("ansi", Some(&cfg)).expect("disabled config should not error");
        assert!(
            built.is_none(),
            "enabled = false must build no linter, leaving the lint path nothing to report"
        );
    }

    /// #113: `enabled = false` must fully disable sqruff on the fix path
    /// too, leaving the SQL unmodified. Same `None`, and `scythe lint --fix`
    /// writes nothing for it.
    #[test]
    fn disabled_config_leaves_sql_unmodified_by_fix() {
        let cfg = SqruffConfig {
            enabled: false,
            rules: ahash::AHashMap::new(),
        };

        // Sanity check: the fix path does rewrite this SQL when enabled.
        let (_, fixed) = enabled_linter("ansi", None)
            .lint_and_fix(FIXABLE_SQL)
            .expect("baseline fix should succeed");
        assert_ne!(
            fixed, FIXABLE_SQL,
            "expected baseline SQL to be rewritten by the fix path"
        );

        let built = SqruffLinter::for_linting("ansi", Some(&cfg)).expect("disabled config should not error");
        assert!(
            built.is_none(),
            "enabled = false must build no linter, so the fix path leaves the SQL untouched"
        );
    }

    /// #114: a non-`"off"` value in `[lint.sqruff.rules]` must be rejected
    /// with a message naming the offending key and value, not silently
    /// treated as an allowlist entry.
    #[test]
    fn non_off_rule_value_is_rejected() {
        let mut rules = ahash::AHashMap::new();
        rules.insert("LT02".to_string(), "warn".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        let err = SqruffLinter::for_linting("ansi", Some(&cfg))
            .err()
            .expect("non-off rule value must be rejected");
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

    /// #114: `"off"` must still exclude the named rule, same as before.
    #[test]
    fn off_rule_value_still_excludes_rule() {
        let sql = "SELECT  id,  name  FROM  users  WHERE  id = 1\n";
        let mut rules = ahash::AHashMap::new();
        rules.insert("LT01".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        // LT01 is already excluded by default; this must not error and
        // must behave identically to the default-excluded case.
        let violations = enabled_linter("ansi", Some(&cfg))
            .lint(sql)
            .expect("off value should be accepted");
        let baseline = enabled_linter("ansi", None).lint(sql).expect("baseline should succeed");
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
        let baseline = enabled_linter("ansi", None)
            .lint(sql)
            .expect("baseline lint should succeed");
        assert!(!baseline.is_empty(), "expected baseline SQL to have violations");

        let err = SqruffLinter::for_linting("ansi", Some(&cfg))
            .err()
            .expect("an unknown rule code must be diagnosed, not silently swallowed");
        assert!(matches!(err, SqruffConfigError::Linter(_)));
    }

    #[test]
    fn for_linting_accepts_a_usable_configuration() {
        let mut rules = ahash::AHashMap::new();
        rules.insert("LT02".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        assert!(
            SqruffLinter::for_linting("ansi", Some(&cfg))
                .expect("a usable configuration must be accepted")
                .is_some()
        );
        assert!(
            SqruffLinter::for_linting("ansi", None)
                .expect("an absent configuration must be accepted")
                .is_some()
        );
    }

    /// #130: hoisting must not weaken diagnostics. An unusable
    /// `[lint.sqruff.rules]` has to fail when the linter is *built*, so a
    /// caller that builds once per `[[sql]]` block still reports a config
    /// mistake as a config mistake instead of as an error against the first
    /// query file it happened to read.
    #[test]
    fn linter_construction_rejects_unknown_rule_code() {
        let mut rules = ahash::AHashMap::new();
        rules.insert("ZZ99-NOT-A-REAL-RULE".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        let err = SqruffLinter::new("ansi", Some(&cfg)).err().expect("must be rejected");
        assert!(matches!(err, SqruffConfigError::Linter(_)));

        let err = SqruffLinter::for_linting("ansi", Some(&cfg))
            .err()
            .expect("must be rejected");
        assert!(matches!(err, SqruffConfigError::Linter(_)));
    }

    /// #130: same, for the value-shape check.
    #[test]
    fn linter_construction_rejects_non_off_rule_value() {
        let mut rules = ahash::AHashMap::new();
        rules.insert("LT02".to_string(), "warn".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        let err = SqruffLinter::new("ansi", Some(&cfg)).err().expect("must be rejected");
        match err {
            SqruffConfigError::UnsupportedRuleValue { rule, value } => {
                assert_eq!(rule, "LT02");
                assert_eq!(value, "warn");
            }
            other => panic!("expected UnsupportedRuleValue, got {other:?}"),
        }
    }

    /// `enabled = false` short-circuits the lint path, so `for_linting`
    /// builds nothing — and therefore validates nothing. A rules table the
    /// run will never read must not fail the run.
    #[test]
    fn for_linting_returns_none_when_disabled_and_skips_validation() {
        let mut rules = ahash::AHashMap::new();
        rules.insert("ZZ99-NOT-A-REAL-RULE".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: false, rules };

        let built = SqruffLinter::for_linting("ansi", Some(&cfg)).expect("disabled config must not error");
        assert!(built.is_none(), "a disabled config must build no linter");
    }

    /// `scythe fmt` formats even when `[lint.sqruff] enabled = false`, so the
    /// `fmt` constructor must ignore `enabled` where the lint constructor
    /// honours it. These two are the only difference between them.
    #[test]
    fn new_ignores_enabled_where_for_linting_honours_it() {
        let cfg = SqruffConfig {
            enabled: false,
            rules: ahash::AHashMap::new(),
        };

        assert!(
            SqruffLinter::new("ansi", Some(&cfg)).is_ok(),
            "fmt's constructor must build a linter even when linting is disabled"
        );
        assert!(
            SqruffLinter::for_linting("ansi", Some(&cfg))
                .expect("disabled config must not error")
                .is_none()
        );

        let enabled = SqruffConfig {
            enabled: true,
            rules: ahash::AHashMap::new(),
        };
        let disabled_output = SqruffLinter::new("ansi", Some(&cfg))
            .expect("fmt's constructor must build a linter even when linting is disabled")
            .format(FIXABLE_SQL)
            .expect("fmt must format with linting disabled");
        let enabled_output = SqruffLinter::new("ansi", Some(&enabled))
            .expect("linter should build")
            .format(FIXABLE_SQL)
            .expect("fmt must format with linting enabled");

        assert_ne!(
            disabled_output, FIXABLE_SQL,
            "fmt must actually reformat the SQL, or comparing the two outputs proves nothing"
        );
        assert_eq!(
            disabled_output, enabled_output,
            "enabled = false must not change what fmt produces"
        );
    }

    /// A derived `Default` would set `enabled: false`, because
    /// `#[serde(default = "default_true")]` only applies to absent TOML
    /// input. Everywhere else an absent `[lint.sqruff]` means enabled.
    #[test]
    fn default_config_is_enabled() {
        assert!(
            SqruffConfig::default().enabled,
            "SqruffConfig::default() must be enabled"
        );
    }
}
