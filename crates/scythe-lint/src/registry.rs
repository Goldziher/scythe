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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LintConfig;

    /// A trivial rule used only in tests.
    struct TestRule {
        id: &'static str,
        category: RuleCategory,
        severity: Severity,
    }

    impl TestRule {
        fn new(id: &'static str, category: RuleCategory, severity: Severity) -> Self {
            Self {
                id,
                category,
                severity,
            }
        }
    }

    impl LintRule for TestRule {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            "test-rule"
        }
        fn category(&self) -> RuleCategory {
            self.category
        }
        fn default_severity(&self) -> Severity {
            self.severity
        }
        fn description(&self) -> &'static str {
            "a test rule"
        }
    }

    #[test]
    fn new_registry_is_empty() {
        let reg = RuleRegistry::new();
        assert!(reg.rules.is_empty());
        assert!(reg.severity_overrides.is_empty());
        assert!(reg.category_overrides.is_empty());
        assert!(reg.active_rules().is_empty());
    }

    #[test]
    fn register_adds_rule() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new(
            "TR-01",
            RuleCategory::Safety,
            Severity::Warn,
        )));
        assert_eq!(reg.rules.len(), 1);
        assert_eq!(reg.active_rules().len(), 1);
    }

    #[test]
    fn default_registry_has_22_rules() {
        let reg = default_registry();
        assert_eq!(reg.rules.len(), 22);
    }

    #[test]
    fn apply_config_rule_level_override() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new(
            "TR-01",
            RuleCategory::Safety,
            Severity::Warn,
        )));

        let mut config = LintConfig::default();
        config.rules.insert("TR-01".to_string(), Severity::Error);
        reg.apply_config(&config);

        let active = reg.active_rules();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].1, Severity::Error);
    }

    #[test]
    fn apply_config_category_level_override() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new(
            "TR-01",
            RuleCategory::Safety,
            Severity::Warn,
        )));

        let mut config = LintConfig::default();
        config
            .categories
            .insert(RuleCategory::Safety, Severity::Error);
        reg.apply_config(&config);

        let active = reg.active_rules();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].1, Severity::Error);
    }

    #[test]
    fn effective_severity_rule_override_beats_category_override() {
        let mut reg = RuleRegistry::new();
        let rule = TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn);
        reg.register(Box::new(TestRule::new(
            "TR-01",
            RuleCategory::Safety,
            Severity::Warn,
        )));

        let mut config = LintConfig::default();
        config
            .categories
            .insert(RuleCategory::Safety, Severity::Off);
        config.rules.insert("TR-01".to_string(), Severity::Error);
        reg.apply_config(&config);

        // Rule-level override wins over category override
        assert_eq!(reg.effective_severity(&rule), Severity::Error);
    }

    #[test]
    fn effective_severity_category_override_beats_default() {
        let mut reg = RuleRegistry::new();
        let rule = TestRule::new("TR-01", RuleCategory::Naming, Severity::Warn);

        let mut config = LintConfig::default();
        config
            .categories
            .insert(RuleCategory::Naming, Severity::Error);
        reg.apply_config(&config);

        assert_eq!(reg.effective_severity(&rule), Severity::Error);
    }

    #[test]
    fn effective_severity_falls_back_to_default() {
        let reg = RuleRegistry::new();
        let rule = TestRule::new("TR-01", RuleCategory::Safety, Severity::Warn);
        assert_eq!(reg.effective_severity(&rule), Severity::Warn);
    }

    #[test]
    fn active_rules_filters_out_off() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new(
            "TR-01",
            RuleCategory::Safety,
            Severity::Warn,
        )));
        reg.register(Box::new(TestRule::new(
            "TR-02",
            RuleCategory::Safety,
            Severity::Off,
        )));
        reg.register(Box::new(TestRule::new(
            "TR-03",
            RuleCategory::Style,
            Severity::Error,
        )));

        let active = reg.active_rules();
        assert_eq!(active.len(), 2);
        let ids: Vec<&str> = active.iter().map(|(r, _)| r.id()).collect();
        assert!(ids.contains(&"TR-01"));
        assert!(ids.contains(&"TR-03"));
        assert!(!ids.contains(&"TR-02"));
    }

    #[test]
    fn active_rules_returns_overridden_severity() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new(
            "TR-01",
            RuleCategory::Safety,
            Severity::Warn,
        )));

        let mut config = LintConfig::default();
        config.rules.insert("TR-01".to_string(), Severity::Error);
        reg.apply_config(&config);

        let active = reg.active_rules();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].0.id(), "TR-01");
        assert_eq!(active[0].1, Severity::Error);
    }

    #[test]
    fn active_rules_config_can_turn_off_rule() {
        let mut reg = RuleRegistry::new();
        reg.register(Box::new(TestRule::new(
            "TR-01",
            RuleCategory::Safety,
            Severity::Warn,
        )));

        let mut config = LintConfig::default();
        config.rules.insert("TR-01".to_string(), Severity::Off);
        reg.apply_config(&config);

        assert!(reg.active_rules().is_empty());
    }
}
