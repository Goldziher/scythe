use std::borrow::Cow;

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Off,
    Warn,
    Error,
}

// ---------------------------------------------------------------------------
// RuleCategory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleCategory {
    Naming,
    Safety,
    Style,
    Performance,
    Antipattern,
    Codegen,
}

impl std::fmt::Display for RuleCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleCategory::Naming => write!(f, "naming"),
            RuleCategory::Safety => write!(f, "safety"),
            RuleCategory::Style => write!(f, "style"),
            RuleCategory::Performance => write!(f, "performance"),
            RuleCategory::Antipattern => write!(f, "antipattern"),
            RuleCategory::Codegen => write!(f, "codegen"),
        }
    }
}

// ---------------------------------------------------------------------------
// Violation & LintFix
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LintFix {
    pub description: String,
    pub replacement: String,
}

#[derive(Debug, Clone)]
pub struct Violation {
    pub rule_id: Cow<'static, str>,
    pub message: String,
    pub fix: Option<LintFix>,
}

// ---------------------------------------------------------------------------
// LintContext
// ---------------------------------------------------------------------------

pub struct LintContext<'a> {
    pub sql: &'a str,
    pub stmt: &'a sqlparser::ast::Statement,
    pub analyzed: &'a scythe_core::analyzer::AnalyzedQuery,
    pub catalog: &'a scythe_core::catalog::Catalog,
    pub annotations: &'a scythe_core::parser::Annotations,
}

// ---------------------------------------------------------------------------
// LintConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct LintConfig {
    #[serde(default)]
    pub categories: ahash::AHashMap<RuleCategory, Severity>,
    #[serde(default)]
    pub rules: ahash::AHashMap<String, Severity>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_serde_round_trip() {
        let json = r#""warn""#;
        let sev: Severity = serde_json::from_str(json).unwrap();
        assert_eq!(sev, Severity::Warn);

        let json = r#""error""#;
        let sev: Severity = serde_json::from_str(json).unwrap();
        assert_eq!(sev, Severity::Error);

        let json = r#""off""#;
        let sev: Severity = serde_json::from_str(json).unwrap();
        assert_eq!(sev, Severity::Off);
    }

    #[test]
    fn rule_category_serde_round_trip() {
        let json = r#""naming""#;
        let cat: RuleCategory = serde_json::from_str(json).unwrap();
        assert_eq!(cat, RuleCategory::Naming);

        let json = r#""safety""#;
        let cat: RuleCategory = serde_json::from_str(json).unwrap();
        assert_eq!(cat, RuleCategory::Safety);

        let json = r#""performance""#;
        let cat: RuleCategory = serde_json::from_str(json).unwrap();
        assert_eq!(cat, RuleCategory::Performance);
    }

    #[test]
    fn rule_category_variants_exist() {
        // Ensure all six categories are distinct
        let cats = [
            RuleCategory::Naming,
            RuleCategory::Safety,
            RuleCategory::Style,
            RuleCategory::Performance,
            RuleCategory::Antipattern,
            RuleCategory::Codegen,
        ];
        for (i, a) in cats.iter().enumerate() {
            for (j, b) in cats.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[test]
    fn rule_category_display() {
        assert_eq!(RuleCategory::Naming.to_string(), "naming");
        assert_eq!(RuleCategory::Safety.to_string(), "safety");
        assert_eq!(RuleCategory::Style.to_string(), "style");
        assert_eq!(RuleCategory::Performance.to_string(), "performance");
        assert_eq!(RuleCategory::Antipattern.to_string(), "antipattern");
        assert_eq!(RuleCategory::Codegen.to_string(), "codegen");
    }

    #[test]
    fn violation_creation() {
        let v = Violation {
            rule_id: Cow::Borrowed("SC-T01"),
            message: "test violation".to_string(),
            fix: None,
        };
        assert_eq!(v.rule_id, "SC-T01");
        assert_eq!(v.message, "test violation");
        assert!(v.fix.is_none());
    }

    #[test]
    fn violation_with_fix() {
        let v = Violation {
            rule_id: Cow::Owned("SC-T02".to_string()),
            message: "needs fix".to_string(),
            fix: Some(LintFix {
                description: "apply fix".to_string(),
                replacement: "fixed sql".to_string(),
            }),
        };
        assert!(v.fix.is_some());
        let fix = v.fix.unwrap();
        assert_eq!(fix.description, "apply fix");
        assert_eq!(fix.replacement, "fixed sql");
    }

    #[test]
    fn lint_config_default_is_empty() {
        let config = LintConfig::default();
        assert!(config.categories.is_empty());
        assert!(config.rules.is_empty());
    }

    #[test]
    fn lint_config_deserialize() {
        let json = r#"{"categories": {"safety": "error"}, "rules": {"SC-S01": "off"}}"#;
        let config: LintConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.categories.len(), 1);
        assert_eq!(
            config.categories.get(&RuleCategory::Safety),
            Some(&Severity::Error)
        );
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules.get("SC-S01"), Some(&Severity::Off));
    }
}
