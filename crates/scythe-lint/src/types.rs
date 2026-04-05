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
