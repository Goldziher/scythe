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
//! - `rule.schema` is `None`, or the check's declared schema column has that
//!   value.
//! - `rule.object` is `None`, or the check's declared object column has that
//!   value.
//!
//! All comparisons are **case-sensitive string equality** (no glob / regex).
//!
//! ## Why the columns are declared and not guessed
//!
//! The columns come from [`CheckSpec::object_binding`] and
//! [`CheckSpec::schema_binding`], resolved once from the
//! [`CheckRegistry`](crate::registry::CheckRegistry) in
//! [`SuppressionEngine::bind_to_registry`].
//!
//! They used to be found by scanning the result row for a key containing
//! `"name"`. The row is a `HashMap` whose iteration order is randomised per
//! process, and several checks project more than one qualifying key — SC-INS01
//! has `table_name` and `constraint_name`, SC-INS06 has `table_name`,
//! `role_name` and `policy_names` — so the same config against the same
//! database suppressed on some runs and not on others (12 of 20 consecutive
//! runs, measured). A non-deterministic suppression is worse than a broken one:
//! CI goes green locally and red on re-run with nothing changed.
//!
//! The same scan could never match SC-INS12 at all, which aliases its object
//! column `parent_table` — no substring search for `"name"` will find it, so
//! `object = "…"` silently disabled the rule's suppression entirely.

use std::collections::HashMap;

use scythe_lint::reporters::Finding;

use crate::config::SuppressionRule;
use crate::registry::CheckRegistry;

/// The columns of one check's result row that identify what a finding is about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RuleBindings {
    schema_column: Option<String>,
    object_column: Option<String>,
}

/// Column names a check is likely to have used for its object, in the order
/// they are tried when the check declares no `object_binding`.
///
/// Only user-defined checks can reach this: every canonical check is required
/// to declare its binding, and [`crate::spec::validate_suppression_bindings`]
/// enforces that at registry-load time. The order is fixed and the search
/// after it is over sorted keys, so the answer is the same on every run — which
/// is the property that was actually missing before, independent of which
/// column is the "right" one.
const OBJECT_COLUMN_FALLBACK_ORDER: [&str; 12] = [
    "object_name",
    "table_name",
    "view_name",
    "sequence_name",
    "function_name",
    "extension_name",
    "index_name",
    "constraint_name",
    "policy_name",
    "column_name",
    "parent_table",
    "name",
];

/// Applies `[[inspect.suppression]]` rules to post-execution findings.
pub struct SuppressionEngine {
    rules: Vec<SuppressionRule>,
    /// Declared match columns per check ID, populated by
    /// [`SuppressionEngine::bind_to_registry`].
    bindings: HashMap<String, RuleBindings>,
}

impl SuppressionEngine {
    /// Build a new engine from the suppression rules configured in `[inspect]`.
    ///
    /// Call [`bind_to_registry`](Self::bind_to_registry) before filtering so
    /// each check's declared match columns are used; without it every check
    /// falls back to the deterministic column search.
    pub fn new(rules: Vec<SuppressionRule>) -> Self {
        Self {
            rules,
            bindings: HashMap::new(),
        }
    }

    /// Read each check's `object_binding` / `schema_binding` out of `registry`.
    ///
    /// Separate from `new` because the rules come from `scythe.toml` while the
    /// registry is assembled later from the canonical TOML plus the user's
    /// inline and extra checks; the driver wires the two together once it holds
    /// both.
    pub fn bind_to_registry(&mut self, registry: &CheckRegistry) {
        self.bindings = registry
            .all()
            .iter()
            .map(|spec| {
                (
                    spec.id.clone(),
                    RuleBindings {
                        schema_column: spec.schema_binding.clone(),
                        object_column: spec.object_binding.clone(),
                    },
                )
            })
            .collect();
    }

    /// Return `true` if `finding` should be suppressed given `bindings` (the
    /// raw SQL result columns that produced the finding).
    ///
    /// `bindings` is the `HashMap<String, String>` produced by the runner for
    /// each result row — the same map that was used to render the finding's
    /// message template.
    pub fn is_suppressed(&self, finding: &Finding, bindings: &HashMap<String, String>) -> bool {
        let declared = self.bindings.get(&finding.rule_id);

        for rule in &self.rules {
            if rule.rule != finding.rule_id {
                continue;
            }

            if let Some(expected_schema) = &rule.schema {
                let column = declared.and_then(|d| d.schema_column.as_deref());
                match schema_value(column, bindings) {
                    Some(value) if value == expected_schema.as_str() => {}
                    _ => continue,
                }
            }

            if let Some(expected_object) = &rule.object {
                let column = declared.and_then(|d| d.object_column.as_deref());
                match object_value(column, bindings) {
                    Some(value) if value == expected_object.as_str() => {}
                    _ => continue,
                }
            }

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

/// The value `[[inspect.suppression]] schema = "…"` is compared against.
fn schema_value<'a>(declared_column: Option<&str>, bindings: &'a HashMap<String, String>) -> Option<&'a str> {
    if let Some(column) = declared_column {
        return bindings.get(column).map(String::as_str);
    }
    first_by_sorted_key(bindings, |key| key.contains("schema"))
}

/// The value `[[inspect.suppression]] object = "…"` is compared against.
///
/// Returns `None` — leaving the finding in place — when nothing plausible is
/// found. Suppression fails open on purpose: a finding that survives a
/// suppression rule is visible and can be investigated, whereas a finding
/// dropped because some arbitrary column happened to match is gone with no
/// trace.
fn object_value<'a>(declared_column: Option<&str>, bindings: &'a HashMap<String, String>) -> Option<&'a str> {
    if let Some(column) = declared_column {
        return bindings.get(column).map(String::as_str);
    }

    for candidate in OBJECT_COLUMN_FALLBACK_ORDER {
        if let Some(value) = bindings.get(candidate) {
            return Some(value.as_str());
        }
    }

    first_by_sorted_key(bindings, |key| key.ends_with("_name") && !key.contains("schema"))
}

/// The value of the lexicographically first key satisfying `predicate`.
///
/// Sorted rather than `HashMap::iter().find`, whose order is seeded randomly
/// per process — the root cause of the non-determinism this module exists to
/// remove.
fn first_by_sorted_key(bindings: &HashMap<String, String>, predicate: impl Fn(&str) -> bool) -> Option<&str> {
    let mut keys: Vec<&String> = bindings.keys().filter(|key| predicate(key)).collect();
    keys.sort_unstable();
    keys.first().map(|key| bindings[*key].as_str())
}

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

    /// An engine wired to the shipped canonical registry, which is how the
    /// driver builds it.
    fn canonical_engine(rules: Vec<SuppressionRule>) -> SuppressionEngine {
        let mut engine = SuppressionEngine::new(rules);
        engine.bind_to_registry(&CheckRegistry::canonical());
        engine
    }

    fn rule(id: &str, schema: Option<&str>, object: Option<&str>) -> SuppressionRule {
        SuppressionRule {
            rule: id.to_string(),
            schema: schema.map(str::to_string),
            object: object.map(str::to_string),
        }
    }

    #[test]
    fn suppresses_matching_rule_only() {
        let engine = canonical_engine(vec![rule("SC-INS09", None, None)]);

        let finding = make_finding("SC-INS09", "extension in public");
        let bindings = make_bindings(&[("extension_name", "pgtap"), ("schema_name", "public")]);
        assert!(engine.is_suppressed(&finding, &bindings));

        let other = make_finding("SC-INS01", "fk without index");
        assert!(!engine.is_suppressed(&other, &bindings));
    }

    #[test]
    fn suppresses_matching_rule_and_schema() {
        let engine = canonical_engine(vec![rule("SC-INS09", Some("public"), None)]);

        let f1 = make_finding("SC-INS09", "");
        let b1 = make_bindings(&[("schema_name", "public"), ("extension_name", "pgtap")]);
        assert!(engine.is_suppressed(&f1, &b1));

        let f2 = make_finding("SC-INS09", "");
        let b2 = make_bindings(&[("schema_name", "other"), ("extension_name", "pgtap")]);
        assert!(!engine.is_suppressed(&f2, &b2));
    }

    #[test]
    fn does_not_suppress_when_object_mismatches() {
        let engine = canonical_engine(vec![rule("SC-INS09", Some("public"), Some("pgtap"))]);

        let f = make_finding("SC-INS09", "");
        let b = make_bindings(&[("schema_name", "public"), ("extension_name", "uuid-ossp")]);
        assert!(!engine.is_suppressed(&f, &b));

        let f2 = make_finding("SC-INS09", "");
        let b2 = make_bindings(&[("schema_name", "public"), ("extension_name", "pgtap")]);
        assert!(engine.is_suppressed(&f2, &b2));
    }

    #[test]
    fn filter_returns_only_non_suppressed() {
        let engine = canonical_engine(vec![rule("SC-INS09", None, None)]);

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

    /// The measured defect: SC-INS01's row has two keys containing `"name"`
    /// (`table_name` and `constraint_name`), and the old lookup took whichever
    /// the randomly-seeded `HashMap` yielded first. `object = "children"` names
    /// the *table*, so every run must suppress.
    #[test]
    fn should_suppress_every_time_when_sc_ins01_projects_two_name_columns() {
        let engine = canonical_engine(vec![rule("SC-INS01", None, Some("children"))]);
        let finding = make_finding("SC-INS01", "");

        for run in 0..64 {
            let bindings = make_bindings(&[
                ("schema_name", "public"),
                ("table_name", "children"),
                ("constraint_name", "children_parent_id_fkey"),
                ("columns", "parent_id"),
            ]);
            assert!(
                engine.is_suppressed(&finding, &bindings),
                "run {run}: suppression must not depend on hash-map iteration order"
            );
        }
    }

    /// The other half of determinism: matching the *constraint* name must fail
    /// every time, not merely most of the time. Before, this suppressed on
    /// whichever runs the map yielded `constraint_name` first.
    #[test]
    fn should_never_suppress_when_the_object_names_a_column_that_is_not_the_object() {
        let engine = canonical_engine(vec![rule("SC-INS01", None, Some("children_parent_id_fkey"))]);
        let finding = make_finding("SC-INS01", "");

        for run in 0..64 {
            let bindings = make_bindings(&[
                ("schema_name", "public"),
                ("table_name", "children"),
                ("constraint_name", "children_parent_id_fkey"),
            ]);
            assert!(
                !engine.is_suppressed(&finding, &bindings),
                "run {run}: only the declared object column may match"
            );
        }
    }

    /// SC-INS06 projects three `name`-ish columns (`table_name`, `role_name`,
    /// `policy_names`) and suppressed in 3 runs out of 10.
    #[test]
    fn should_suppress_every_time_when_sc_ins06_projects_three_name_columns() {
        let engine = canonical_engine(vec![rule("SC-INS06", None, Some("ins06"))]);
        let finding = make_finding("SC-INS06", "");

        for run in 0..64 {
            let bindings = make_bindings(&[
                ("schema_name", "public"),
                ("table_name", "ins06"),
                ("role_name", "app_user"),
                ("command", "SELECT"),
                ("policy_count", "2"),
                ("policy_names", "p_one, p_two"),
            ]);
            assert!(engine.is_suppressed(&finding, &bindings), "run {run}");
        }
    }

    /// The companion defect: SC-INS12 aliases its object column `parent_table`,
    /// which contains no `"name"` substring, so `object = "…"` matched in 0 of
    /// 20 runs — the rule's object filter was dead, not flaky.
    #[test]
    fn should_suppress_sc_ins12_by_its_parent_table_object_column() {
        let engine = canonical_engine(vec![rule("SC-INS12", None, Some("ins12"))]);
        let finding = make_finding("SC-INS12", "");
        let bindings = make_bindings(&[("schema_name", "public"), ("parent_table", "ins12")]);

        assert!(engine.is_suppressed(&finding, &bindings));
    }

    #[test]
    fn should_not_suppress_sc_ins12_when_the_parent_table_differs() {
        let engine = canonical_engine(vec![rule("SC-INS12", None, Some("other"))]);
        let finding = make_finding("SC-INS12", "");
        let bindings = make_bindings(&[("schema_name", "public"), ("parent_table", "ins12")]);

        assert!(!engine.is_suppressed(&finding, &bindings));
    }

    /// The `[inspect]` documentation's own example is
    /// `rule = "SC-INS09", schema = "public", object = "pgtap"`. SC-INS09's SQL
    /// projected no schema column, so the schema clause could never match and
    /// the documented example silenced nothing.
    #[test]
    fn should_suppress_the_documented_sc_ins09_example_including_its_schema_clause() {
        let registry = CheckRegistry::canonical();
        let spec = registry.get("SC-INS09").expect("SC-INS09 is canonical");
        assert_eq!(spec.schema_binding.as_deref(), Some("schema_name"));
        assert!(
            spec.sql.contains("AS schema_name"),
            "the declared schema column must be one the SQL projects: {}",
            spec.sql
        );
    }

    /// A user check that declares no `object_binding` still has to behave the
    /// same way on every run.
    #[test]
    fn should_match_a_user_check_deterministically_without_a_declared_binding() {
        let engine = SuppressionEngine::new(vec![rule("USER-INS-001", None, Some("orders"))]);
        let finding = make_finding("USER-INS-001", "");

        for run in 0..64 {
            let bindings = make_bindings(&[
                ("schema_name", "public"),
                ("table_name", "orders"),
                ("index_name", "orders_pkey"),
            ]);
            assert!(engine.is_suppressed(&finding, &bindings), "run {run}");
        }
    }

    /// Failing open: when nothing identifies the object, the finding stays.
    /// A dropped finding leaves no trace; a surviving one can be investigated.
    #[test]
    fn should_keep_the_finding_when_no_column_identifies_the_object() {
        let engine = SuppressionEngine::new(vec![rule("USER-INS-002", None, Some("anything"))]);
        let finding = make_finding("USER-INS-002", "");
        let bindings = make_bindings(&[("count", "3"), ("ratio", "0.5")]);

        assert!(!engine.is_suppressed(&finding, &bindings));
    }

    /// Every canonical check must declare an object column that its own SQL
    /// projects. This is the registry-level guard the issue asked for: without
    /// it a new check can ship with `object = "…"` quietly dead, exactly as
    /// SC-INS12 did.
    #[test]
    fn should_declare_one_object_column_per_canonical_check() {
        let registry = CheckRegistry::canonical();
        for spec in registry.all() {
            let object = spec
                .object_binding
                .as_deref()
                .unwrap_or_else(|| panic!("canonical check {} declares no object_binding", spec.id));
            assert!(
                spec.sql.contains(&format!("AS {object}")),
                "check {} declares object_binding = {object:?}, which its SQL does not project",
                spec.id
            );
        }
    }
}
