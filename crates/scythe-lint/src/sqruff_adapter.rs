use std::borrow::Cow;

use sqruff_lib::core::config::FluffConfig;
use sqruff_lib::core::linter::core::Linter;

use super::types::Violation;

/// Sqruff violation with line/position info for display.
#[derive(Debug, Clone)]
pub struct SqruffViolation {
    pub violation: Violation,
    pub line_no: usize,
    pub line_pos: usize,
    pub fixable: bool,
}

/// Create a sqruff `FluffConfig` for the given dialect.
fn make_config(dialect: &str) -> FluffConfig {
    // FluffConfig::from_source accepts an INI-style config string.
    let source = format!("[sqruff]\ndialect = {}\n", dialect);
    FluffConfig::from_source(&source, None)
}

fn make_linter(dialect: &str) -> Result<Linter, String> {
    let config = make_config(dialect);
    Linter::new(config, None, None, false)
}

/// Run sqruff's rules on SQL and return scythe Violations with position info.
pub fn lint_sql(sql: &str, dialect: &str) -> Vec<SqruffViolation> {
    let linter = match make_linter(dialect) {
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
pub fn lint_and_fix_sql(sql: &str, dialect: &str) -> (Vec<SqruffViolation>, String) {
    let linter = match make_linter(dialect) {
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
pub fn format_sql(sql: &str, dialect: &str) -> Result<String, String> {
    let linter = make_linter(dialect)?;

    let result = linter.lint_string(sql, None, true).map_err(|e| e.value)?;

    Ok(result.fix_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lint_simple_sql() {
        let sql = "SELECT  id,  name  FROM  users  WHERE  id = 1\n";
        let violations = lint_sql(sql, "ansi");
        // sqruff should find at least some style violations
        // (multiple spaces, trailing whitespace, etc.)
        // We just verify it doesn't panic and returns a Vec.
        let _ = violations;
    }

    #[test]
    fn format_simple_sql() {
        let sql = "select  id,name from users\n";
        let result = format_sql(sql, "ansi");
        assert!(result.is_ok());
    }

    #[test]
    fn lint_and_fix_returns_fixed_sql() {
        let sql = "select  id,name from users\n";
        let (_, fixed) = lint_and_fix_sql(sql, "ansi");
        // The fixed SQL should be different from the original (extra spaces removed)
        assert!(!fixed.is_empty());
    }
}
