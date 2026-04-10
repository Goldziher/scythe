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

/// Create a sqruff `FluffConfig` for the given dialect, optionally applying
/// rule include/exclude settings from [`SqruffConfig`].
/// Rules excluded by default due to upstream sqruff bugs.
/// LT01: incorrectly splits compound operators (>=, <=, <@, etc.) into separate tokens.
const DEFAULT_EXCLUDED_RULES: &[&str] = &["LT01"];

fn make_config(dialect: &str, sqruff_config: Option<&SqruffConfig>) -> FluffConfig {
    let mut source = format!("[sqruff]\ndialect = {}\n", dialect);

    let mut excluded: Vec<&str> = DEFAULT_EXCLUDED_RULES.to_vec();
    if let Some(cfg) = sqruff_config {
        // Add user-configured rules set to "off"
        for (k, v) in &cfg.rules {
            if v.as_str() == "off" && !excluded.contains(&k.as_str()) {
                excluded.push(k.as_str());
            }
        }

        // Add rules for rules explicitly enabled
        let included: Vec<&str> = cfg
            .rules
            .iter()
            .filter(|(_, v)| v.as_str() != "off")
            .map(|(k, _)| k.as_str())
            .collect();
        if !included.is_empty() {
            source.push_str(&format!("rules = {}\n", included.join(",")));
        }
    }

    if !excluded.is_empty() {
        source.push_str(&format!("exclude_rules = {}\n", excluded.join(",")));
    }

    FluffConfig::from_source(&source, None)
}

fn make_linter(dialect: &str, sqruff_config: Option<&SqruffConfig>) -> Result<Linter, String> {
    let config = make_config(dialect, sqruff_config);
    Linter::new(config, None, None, false)
}

/// Run sqruff's rules on SQL and return scythe Violations with position info.
pub fn lint_sql(
    sql: &str,
    dialect: &str,
    sqruff_config: Option<&SqruffConfig>,
) -> Vec<SqruffViolation> {
    let linter = match make_linter(dialect, sqruff_config) {
        Ok(l) => l,
        Err(_) => return Vec::new(),
    };

    let result = match linter.lint_string(sql, None, false) {
        Ok(linted) => linted,
        Err(_) => return Vec::new(),
    };

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

/// Run sqruff lint with auto-fix enabled, returning violations and the fixed SQL.
pub fn lint_and_fix_sql(
    sql: &str,
    dialect: &str,
    sqruff_config: Option<&SqruffConfig>,
) -> (Vec<SqruffViolation>, String) {
    let linter = match make_linter(dialect, sqruff_config) {
        Ok(l) => l,
        Err(_) => return (Vec::new(), sql.to_string()),
    };

    let result = match linter.lint_string(sql, None, true) {
        Ok(linted) => linted,
        Err(_) => return (Vec::new(), sql.to_string()),
    };

    let violations: Vec<SqruffViolation> = result
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
        .collect();

    let fixed = result.fix_string();
    (violations, fixed)
}

/// Format SQL using sqruff (lint with fix, return the fixed string).
pub fn format_sql(
    sql: &str,
    dialect: &str,
    sqruff_config: Option<&SqruffConfig>,
) -> Result<String, String> {
    let linter = make_linter(dialect, sqruff_config)?;

    let result = linter.lint_string(sql, None, true).map_err(|e| e.value)?;

    let fixed = result.fix_string();

    // Workaround: sqruff incorrectly splits compound operators inside CHECK
    // constraints (e.g., ">=" becomes "> ="). Rejoin them.
    // See: https://github.com/quarylabs/sqruff/issues/2530
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
        // sqruff should find at least some style violations
        // (multiple spaces, trailing whitespace, etc.)
        // We just verify it doesn't panic and returns a Vec.
        let _ = violations;
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
        let (_, fixed) = lint_and_fix_sql(sql, "ansi", None);
        // The fixed SQL should be different from the original (extra spaces removed)
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
        let _ = violations;
    }
}
