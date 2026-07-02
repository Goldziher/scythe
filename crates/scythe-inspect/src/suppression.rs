//! Suppression engine for live-DB inspection findings.
//!
//! A [`SuppressionEngine`] takes the `[[inspect.suppression]]` rules from
//! `scythe.toml` and can filter a set of `(Finding, bindings)` pairs,
//! dropping entries that match any suppression rule.
//!
//! ## Matching semantics
//!
//! A finding is suppressed when **all** of the following hold:
//! - `rule.rule == finding.rule_id` (always required).
//! - `rule.schema` is `None`, or the finding has a binding key containing
//!   `"schema"` (e.g. `schema_name`) whose value equals `rule.schema`.
//! - `rule.object` is `None`, or the finding has a binding key containing
//!   `"name"` (e.g. `table_name`, `extension_name`, `policy_name`) whose
//!   value equals `rule.object`.
//!
//! All comparisons are **case-sensitive string equality** (no glob / regex).
//! Glob support is deferred to Phase 2.

use std::collections::HashMap;

use scythe_lint::reporters::Finding;

use crate::config::SuppressionRule;

// ---------------------------------------------------------------------------
// SuppressionEngine
// ---------------------------------------------------------------------------

/// Applies `[[inspect.suppression]]` rules to post-execution findings.
pub struct SuppressionEngine {
    rules: Vec<SuppressionRule>,
}

impl SuppressionEngine {
    /// Build a new engine from the suppression rules configured in `[inspect]`.
    pub fn new(rules: Vec<SuppressionRule>) -> Self {
        Self { rules }
    }

    /// Return `true` if `finding` should be suppressed given `bindings` (the
    /// raw SQL result columns that produced the finding).
    ///
    /// `bindings` is the `HashMap<String, String>` produced by the runner for
    /// each result row — the same map that was used to render the finding's
    /// message template.
    pub fn is_suppressed(&self, finding: &Finding, bindings: &HashMap<String, String>) -> bool {
        for rule in &self.rules {
            if rule.rule != finding.rule_id {
                continue;
            }

            // Schema filter: look for a binding key that contains "schema"
            // (e.g. `schema_name`). If the rule specifies a schema, that key's
            // value must equal it; if no such key exists in bindings, skip.
            if let Some(expected_schema) = &rule.schema {
                let schema_value = bindings
                    .iter()
                    .find(|(k, _)| k.contains("schema"))
                    .map(|(_, v)| v.as_str());
                match schema_value {
                    Some(v) if v == expected_schema.as_str() => {}
                    _ => continue,
                }
            }

            // Object filter: look for any binding key that contains "name"
            // but NOT "schema" (to avoid re-matching schema_name).  Examples:
            // table_name, extension_name, function_name, policy_name.
            if let Some(expected_object) = &rule.object {
                let object_value = bindings
                    .iter()
                    .find(|(k, _)| k.contains("name") && !k.contains("schema"))
                    .map(|(_, v)| v.as_str());
                match object_value {
                    Some(v) if v == expected_object.as_str() => {}
                    _ => continue,
                }
            }

            // All conditions matched — this finding is suppressed.
            return true;
        }

        false
    }

    /// Filter a list of `(Finding, bindings)` pairs, returning only those that
    /// are NOT suppressed.
    ///
    /// The `bindings` are consumed here; callers should not use them after
    /// filtering.  The returned `Vec<Finding>` is ready for emission via the
    /// standard reporters.
    pub fn filter(&self, pairs: Vec<(Finding, HashMap<String, String>)>) -> Vec<Finding> {
        pairs
            .into_iter()
            .filter(|(finding, bindings)| !self.is_suppressed(finding, bindings))
            .map(|(finding, _)| finding)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use scythe_lint::types::Severity;

    fn make_finding(rule_id: &str, message: &str) -> Finding {
        Finding {
            file: String::new(),
            query_name: None,
            rule_id: rule_id.to_string(),
            rule_name: None,
            rule_description: None,
            severity: Severity::Warn,
            message: message.to_string(),
            line: None,
            column: None,
            cwe: vec![],
            source: Some("inspect".to_string()),
        }
    }

    fn make_bindings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    // -----------------------------------------------------------------------
    // suppresses_matching_rule_only
    // -----------------------------------------------------------------------

    #[test]
    fn suppresses_matching_rule_only() {
        let engine = SuppressionEngine::new(vec![SuppressionRule {
            rule: "SC-INS09".to_string(),
            schema: None,
            object: None,
        }]);

        let finding = make_finding("SC-INS09", "extension in public");
        let bindings = make_bindings(&[("extension_name", "pgtap"), ("schema_name", "public")]);
        assert!(engine.is_suppressed(&finding, &bindings));

        // Different rule_id — must NOT be suppressed.
        let other = make_finding("SC-INS01", "fk without index");
        assert!(!engine.is_suppressed(&other, &bindings));
    }

    // -----------------------------------------------------------------------
    // suppresses_matching_rule_and_schema
    // -----------------------------------------------------------------------

    #[test]
    fn suppresses_matching_rule_and_schema() {
        let engine = SuppressionEngine::new(vec![SuppressionRule {
            rule: "SC-INS09".to_string(),
            schema: Some("public".to_string()),
            object: None,
        }]);

        // Matching schema → suppressed.
        let f1 = make_finding("SC-INS09", "");
        let b1 = make_bindings(&[("schema_name", "public"), ("extension_name", "pgtap")]);
        assert!(engine.is_suppressed(&f1, &b1));

        // Different schema → NOT suppressed.
        let f2 = make_finding("SC-INS09", "");
        let b2 = make_bindings(&[("schema_name", "other"), ("extension_name", "pgtap")]);
        assert!(!engine.is_suppressed(&f2, &b2));
    }

    // -----------------------------------------------------------------------
    // does_not_suppress_when_object_mismatches
    // -----------------------------------------------------------------------

    #[test]
    fn does_not_suppress_when_object_mismatches() {
        let engine = SuppressionEngine::new(vec![SuppressionRule {
            rule: "SC-INS09".to_string(),
            schema: Some("public".to_string()),
            object: Some("pgtap".to_string()),
        }]);

        // Schema matches but object doesn't → NOT suppressed.
        let f = make_finding("SC-INS09", "");
        let b = make_bindings(&[("schema_name", "public"), ("extension_name", "uuid-ossp")]);
        assert!(!engine.is_suppressed(&f, &b));

        // Both match → suppressed.
        let f2 = make_finding("SC-INS09", "");
        let b2 = make_bindings(&[("schema_name", "public"), ("extension_name", "pgtap")]);
        assert!(engine.is_suppressed(&f2, &b2));
    }

    // -----------------------------------------------------------------------
    // filter_returns_only_non_suppressed
    // -----------------------------------------------------------------------

    #[test]
    fn filter_returns_only_non_suppressed() {
        let engine = SuppressionEngine::new(vec![SuppressionRule {
            rule: "SC-INS09".to_string(),
            schema: None,
            object: None,
        }]);

        let pairs = vec![
            (
                make_finding("SC-INS09", "suppressed"),
                make_bindings(&[("schema_name", "public")]),
            ),
            (
                make_finding("SC-INS01", "kept"),
                make_bindings(&[("schema_name", "public")]),
            ),
        ];

        let kept = engine.filter(pairs);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].rule_id, "SC-INS01");
    }
}
