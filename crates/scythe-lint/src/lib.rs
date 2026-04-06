pub mod engine;
pub mod registry;
pub mod rule;
pub mod rules;
pub mod sqruff_adapter;
pub mod types;

pub use engine::{LintEngine, LintReport, QueryViolation};
pub use registry::{RuleRegistry, default_registry};
pub use rule::LintRule;
pub use sqruff_adapter::SqruffViolation;
pub use types::{
    LintConfig, LintContext, LintFix, RuleCategory, Severity, SqruffConfig, Violation,
};
