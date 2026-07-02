pub mod audit;
pub mod engine;
pub mod registry;
pub mod reporters;
pub mod rule;
pub mod rules;
pub mod sqruff_adapter;
pub mod types;

pub use audit::{
    AuditConfigError, MatcherFn, MatcherHit, MatcherRegistry, MatcherRule, RuleSpec, SuppressionSet, canonical_specs,
    load_rules_from_file, parse_rule_file, register_user_rules,
};
pub use engine::{LintEngine, LintReport, QueryViolation};
pub use registry::{RuleRegistry, default_registry};
pub use reporters::{Finding, Format, emit as emit_findings, extract_cwe};
pub use rule::LintRule;
pub use sqruff_adapter::SqruffViolation;
pub use types::{LintConfig, LintContext, LintFix, RuleCategory, Severity, SqruffConfig, Violation};
