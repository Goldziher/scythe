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

/// Check that `[lint.sqruff]` assembles into a usable sqruff configuration,
/// without linting any of the caller's SQL.
///
/// `scythe lint` calls this once per `[[sql]]` block *before* it reads any
/// query file. Without it, an invalid `[lint.sqruff.rules]` entry surfaces
/// as an error against whichever query file happened to be read first, and
/// that error aborts the whole run — discarding every scythe-native finding
/// the run would otherwise have reported, including the security rules.
/// Validating up front keeps a configuration mistake reported as one.
///
/// Returns `Ok(())` without checking anything when `enabled = false`, since
/// [`lint_sql`] and [`lint_and_fix_sql`] never reach sqruff in that case.
///
/// This is [`SqruffLinter::for_linting`] with the linter thrown away, and is
/// deliberately nothing more than that: "is this configuration usable?" and
/// "build the thing that uses it" must not be two separate derivations that
/// can disagree. A caller that intends to lint should keep the linter
/// instead of calling this and then building another one.
pub fn validate_config(dialect: &str, sqruff_config: Option<&SqruffConfig>) -> Result<(), SqruffConfigError> {
    SqruffLinter::for_linting(dialect, sqruff_config).map(|_| ())
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
///
/// Builds a [`SqruffLinter`] per call, which is the expensive part (#130).
/// Callers linting more than one file should build one [`SqruffLinter`] and
/// call [`SqruffLinter::lint`] on it instead.
pub fn lint_sql(
    sql: &str,
    dialect: &str,
    sqruff_config: Option<&SqruffConfig>,
) -> Result<Vec<SqruffViolation>, SqruffConfigError> {
    match SqruffLinter::for_linting(dialect, sqruff_config)? {
        Some(linter) => linter.lint(sql),
        None => Ok(Vec::new()),
    }
}

/// Run sqruff lint with auto-fix enabled, returning violations and the fixed SQL.
///
/// Returns the input SQL unchanged with no violations, without running
/// sqruff, when `sqruff_config` is present and `enabled = false`. Returns
/// [`SqruffConfigError`] if the configuration is invalid or sqruff itself
/// rejects it, rather than silently reporting no violations.
///
/// Builds a [`SqruffLinter`] per call — see [`lint_sql`] for why a
/// multi-file caller should not.
pub fn lint_and_fix_sql(
    sql: &str,
    dialect: &str,
    sqruff_config: Option<&SqruffConfig>,
) -> Result<(Vec<SqruffViolation>, String), SqruffConfigError> {
    match SqruffLinter::for_linting(dialect, sqruff_config)? {
        Some(linter) => linter.lint_and_fix(sql),
        None => Ok((Vec::new(), sql.to_string())),
    }
}

/// Format SQL using sqruff (lint with fix, return the fixed string).
///
/// Note: `[lint.sqruff].enabled` is intentionally NOT consulted here —
/// `scythe fmt` always formats regardless of whether sqruff-based linting
/// is disabled. `[lint.sqruff.rules]` validation still applies when a config
/// is passed, since [`SqruffLinter::new`] is shared with [`lint_sql`]'s
/// construction path.
///
/// Builds a [`SqruffLinter`] per call — see [`lint_sql`] for why a
/// multi-file caller should not.
pub fn format_sql(sql: &str, dialect: &str, sqruff_config: Option<&SqruffConfig>) -> Result<String, String> {
    let linter = SqruffLinter::new(dialect, sqruff_config).map_err(|e| e.to_string())?;
    linter.format(sql)
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

    #[test]
    fn validate_config_accepts_a_usable_configuration() {
        let mut rules = ahash::AHashMap::new();
        rules.insert("LT02".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: true, rules };

        assert!(validate_config("ansi", Some(&cfg)).is_ok());
        assert!(validate_config("ansi", None).is_ok());
    }

    /// `validate_config` must reject exactly what `lint_sql` rejects, so
    /// hoisting the check ahead of the per-file loop cannot let a bad
    /// configuration through.
    #[test]
    fn validate_config_rejects_what_lint_sql_rejects() {
        let sql = "SELECT 1\n";

        let mut bad_value = ahash::AHashMap::new();
        bad_value.insert("LT02".to_string(), "warn".to_string());
        let cfg = SqruffConfig {
            enabled: true,
            rules: bad_value,
        };
        assert!(lint_sql(sql, "ansi", Some(&cfg)).is_err());
        assert!(matches!(
            validate_config("ansi", Some(&cfg)),
            Err(SqruffConfigError::UnsupportedRuleValue { .. })
        ));

        let mut unknown_rule = ahash::AHashMap::new();
        unknown_rule.insert("ZZ99-NOT-A-REAL-RULE".to_string(), "off".to_string());
        let cfg = SqruffConfig {
            enabled: true,
            rules: unknown_rule,
        };
        assert!(lint_sql(sql, "ansi", Some(&cfg)).is_err());
        assert!(matches!(
            validate_config("ansi", Some(&cfg)),
            Err(SqruffConfigError::Linter(_))
        ));
    }

    /// `enabled = false` short-circuits sqruff entirely in `lint_sql`, so
    /// validation must not reject a config those paths never look at.
    #[test]
    fn validate_config_skips_validation_when_disabled() {
        let mut rules = ahash::AHashMap::new();
        rules.insert("ZZ99-NOT-A-REAL-RULE".to_string(), "off".to_string());
        let cfg = SqruffConfig { enabled: false, rules };

        assert!(lint_sql("SELECT 1\n", "ansi", Some(&cfg)).is_ok());
        assert!(validate_config("ansi", Some(&cfg)).is_ok());
    }
    /// #130: one linter must serve many files. A caller hoisting
    /// construction out of its per-file loop has to get exactly what the
    /// per-call helper would have produced for each of those files — same
    /// rule ids, same line, same column — or the perf fix would be a
    /// behaviour change.
    #[test]
    fn reused_linter_produces_identical_findings_to_per_call_lint_sql() {
        let files = [
            "select id FROM users where id = 1\n",
            "SELECT  a,  b  FROM  t\n",
            "SELECT 1\n",
            "select x from y join z on y.id=z.id\n",
        ];

        // Deliberately not `mut`: `lint` taking `&self` is the property that
        // makes hoisting possible at all.
        let linter = SqruffLinter::new("ansi", None).expect("linter should build");

        for sql in files {
            let hoisted = linter.lint(sql).expect("hoisted lint should succeed");
            let per_call = lint_sql(sql, "ansi", None).expect("per-call lint should succeed");

            let hoisted_keys: Vec<(String, usize, usize)> = hoisted
                .iter()
                .map(|v| (v.violation.rule_id.to_string(), v.line_no, v.line_pos))
                .collect();
            let per_call_keys: Vec<(String, usize, usize)> = per_call
                .iter()
                .map(|v| (v.violation.rule_id.to_string(), v.line_no, v.line_pos))
                .collect();

            assert_eq!(hoisted_keys, per_call_keys, "findings diverged for {sql:?}");
        }
    }

    /// #130: the same identity requirement for the fix path.
    #[test]
    fn reused_linter_fixes_identically_to_per_call_lint_and_fix_sql() {
        let sql = "select  id,name from users\n";
        let linter = SqruffLinter::new("ansi", None).expect("linter should build");

        let (_, hoisted_fixed) = linter.lint_and_fix(sql).expect("hoisted fix should succeed");
        let (_, per_call_fixed) = lint_and_fix_sql(sql, "ansi", None).expect("per-call fix should succeed");
        assert_eq!(hoisted_fixed, per_call_fixed);
    }

    /// #130: and for the format path, across several inputs on one linter.
    #[test]
    fn reused_linter_formats_identically_to_per_call_format_sql() {
        let linter = SqruffLinter::new("ansi", None).expect("linter should build");
        for sql in ["select  id,name from users\n", "SELECT a FROM b WHERE a >= 1\n"] {
            let hoisted = linter.format(sql).expect("hoisted format should succeed");
            let per_call = format_sql(sql, "ansi", None).expect("per-call format should succeed");
            assert_eq!(hoisted, per_call, "formatting diverged for {sql:?}");
        }
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
    /// builds nothing — and therefore validates nothing, exactly as the old
    /// per-call `lint_sql` did.
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
        let sql = "select  id,name from users\n";
        assert_eq!(
            format_sql(sql, "ansi", Some(&cfg)).expect("fmt must format with linting disabled"),
            format_sql(sql, "ansi", Some(&enabled)).expect("fmt must format with linting enabled"),
            "enabled = false must not change what fmt produces"
        );
    }

    /// `validate_config` and the linter the caller goes on to build must be
    /// one derivation, not two that can drift apart: whatever construction
    /// rejects, validation rejects, with the same error.
    #[test]
    fn validate_config_and_linter_construction_agree() {
        let cases: [(&str, &str); 2] = [("LT02", "warn"), ("ZZ99-NOT-A-REAL-RULE", "off")];

        for (rule, value) in cases {
            let mut rules = ahash::AHashMap::new();
            rules.insert(rule.to_string(), value.to_string());
            let cfg = SqruffConfig { enabled: true, rules };

            let validated = validate_config("ansi", Some(&cfg));
            let constructed = SqruffLinter::for_linting("ansi", Some(&cfg));
            assert_eq!(
                validated.is_err(),
                constructed.is_err(),
                "validate_config and construction disagreed on {rule} = {value}"
            );
            assert_eq!(
                validated.unwrap_err().to_string(),
                constructed.err().expect("construction must fail too").to_string(),
                "validate_config and construction gave different messages for {rule} = {value}"
            );
        }
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
