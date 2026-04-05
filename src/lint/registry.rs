use ahash::AHashMap;

use super::rule::LintRule;
use super::rules;
use super::types::{LintConfig, RuleCategory, Severity};

// ---------------------------------------------------------------------------
// RuleRegistry
// ---------------------------------------------------------------------------

pub struct RuleRegistry {
    rules: Vec<Box<dyn LintRule>>,
    severity_overrides: AHashMap<String, Severity>,
    category_overrides: AHashMap<RuleCategory, Severity>,
}

impl Default for RuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RuleRegistry {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            severity_overrides: AHashMap::new(),
            category_overrides: AHashMap::new(),
        }
    }

    /// Register a new rule.
    pub fn register(&mut self, rule: Box<dyn LintRule>) {
        self.rules.push(rule);
    }

    /// Apply a lint configuration (category and per-rule overrides).
    pub fn apply_config(&mut self, config: &LintConfig) {
        for (&cat, &sev) in &config.categories {
            self.category_overrides.insert(cat, sev);
        }
        for (id, &sev) in &config.rules {
            self.severity_overrides.insert(id.clone(), sev);
        }
    }

    /// Return the effective severity for a given rule.
    pub fn effective_severity(&self, rule: &dyn LintRule) -> Severity {
        // Per-rule override takes priority
        if let Some(&sev) = self.severity_overrides.get(rule.id()) {
            return sev;
        }
        // Category override
        if let Some(&sev) = self.category_overrides.get(&rule.category()) {
            return sev;
        }
        rule.default_severity()
    }

    /// Return references to all rules whose effective severity is not Off.
    pub fn active_rules(&self) -> Vec<(&dyn LintRule, Severity)> {
        self.rules
            .iter()
            .filter_map(|r| {
                let sev = self.effective_severity(r.as_ref());
                if sev == Severity::Off {
                    None
                } else {
                    Some((r.as_ref(), sev))
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Default registry with all 22 built-in rules
// ---------------------------------------------------------------------------

pub fn default_registry() -> RuleRegistry {
    let mut reg = RuleRegistry::new();

    // Safety rules
    reg.register(Box::new(rules::safety::UpdateWithoutWhere));
    reg.register(Box::new(rules::safety::DeleteWithoutWhere));
    reg.register(Box::new(rules::safety::NoSelectStar));
    reg.register(Box::new(rules::safety::UnusedParams));
    reg.register(Box::new(rules::safety::MissingReturning));
    reg.register(Box::new(rules::safety::AmbiguousColumnInJoin));

    // Codegen rules
    reg.register(Box::new(rules::codegen::MissingReturnsAnnotation));
    reg.register(Box::new(rules::codegen::ExecWithReturning));
    reg.register(Box::new(rules::codegen::DuplicateQueryNames));

    // Naming rules
    reg.register(Box::new(rules::naming::PreferSnakeCaseColumns));
    reg.register(Box::new(rules::naming::PreferSnakeCaseTables));
    reg.register(Box::new(rules::naming::QueryNameConvention));
    reg.register(Box::new(rules::naming::ConsistentAliasCasing));

    // Antipattern rules
    reg.register(Box::new(rules::antipattern::NotEqualNull));
    reg.register(Box::new(rules::antipattern::ImplicitTypeCoercion));
    reg.register(Box::new(rules::antipattern::OrInJoinCondition));

    // Performance rules
    reg.register(Box::new(rules::performance::OrderWithoutLimit));
    reg.register(Box::new(rules::performance::LikeStartsWithWildcard));
    reg.register(Box::new(rules::performance::NotInSubquery));

    // Style rules
    reg.register(Box::new(rules::style::PreferExplicitJoin));
    reg.register(Box::new(rules::style::PreferCoalesceOverCase));
    reg.register(Box::new(rules::style::PreferCountStar));

    reg
}
