//! Schema-drift rules (`SC-DRF01`–`SC-DRF13`).
//!
//! These rules never fire from `scythe lint`: they describe disagreements
//! between the committed DDL and a *live* database, which only
//! `scythe check --database-url` can observe.  They are modelled as
//! [`LintRule`]s anyway, and live in a [`RuleRegistry`](crate::RuleRegistry)
//! built by [`drift_registry`](crate::registry::drift_registry), so that their
//! severities come from the same `[lint]` section of `scythe.toml` as every
//! other `SC-*` rule.  Hard-coding a `Severity` where the drift finding is
//! constructed would make the check un-tunable — and an untunable drift check
//! that errors on a `schema_migrations` table nobody committed is a check
//! users switch off entirely.
//!
//! The default severities encode one asymmetry.  DDL that promises something
//! the database does not deliver breaks generated code, so it is an error.
//! The database holding an object the DDL never mentioned is usually a
//! migration ledger, an extension's bookkeeping table, or a colleague's
//! scratch table — noise, so it warns.

use crate::rule::LintRule;
use crate::types::{RuleCategory, Severity};

/// Declare a drift rule.
///
/// Every drift rule is pure metadata — id, name, default severity,
/// description — because the comparison itself needs a live connection and so
/// cannot run through [`LintRule::check_query`] or
/// [`LintRule::check_catalog`].  Writing seven near-identical impls by hand
/// would invite exactly the copy-paste slip (a duplicated `id()`) that makes a
/// rule silently unconfigurable.
macro_rules! drift_rule {
    ($struct_name:ident, $id:literal, $name:literal, $severity:expr, $description:literal) => {
        #[doc = $description]
        pub struct $struct_name;

        impl LintRule for $struct_name {
            fn id(&self) -> &'static str {
                $id
            }
            fn name(&self) -> &'static str {
                $name
            }
            fn category(&self) -> RuleCategory {
                RuleCategory::Drift
            }
            fn default_severity(&self) -> Severity {
                $severity
            }
            fn description(&self) -> &'static str {
                $description
            }
        }
    };
}

drift_rule!(
    TableMissingFromDatabase,
    "SC-DRF01",
    "table-missing-from-database",
    Severity::Error,
    "Table declared in the DDL does not exist in the live database"
);

drift_rule!(
    TableMissingFromDdl,
    "SC-DRF02",
    "table-missing-from-ddl",
    // Warns, not errors: every real database carries tables the committed DDL
    // never declares -- `schema_migrations`, `_sqlx_migrations`, extension
    // bookkeeping. Defaulting to Error would fail the very first run against a
    // production database and teach users to pass `--database-url` never.
    Severity::Warn,
    "Table exists in the live database but is not declared in the DDL"
);

drift_rule!(
    ColumnMissingFromDatabase,
    "SC-DRF03",
    "column-missing-from-database",
    Severity::Error,
    "Column declared in the DDL does not exist on the live table"
);

drift_rule!(
    ColumnMissingFromDdl,
    "SC-DRF04",
    "column-missing-from-ddl",
    // Errors rather than warns, unlike its table-level sibling SC-DRF02:
    // scythe expands `SELECT *` from the DDL catalog, so a column the database
    // has and the DDL does not makes every generated `SELECT *` decoder read a
    // row wider than the struct it decodes into.
    Severity::Error,
    "Column exists on the live table but is not declared in the DDL"
);

drift_rule!(
    ColumnTypeMismatch,
    "SC-DRF05",
    "column-type-mismatch",
    Severity::Error,
    "Column's DDL type does not match the type the live database reports"
);

drift_rule!(
    ColumnNullabilityMismatch,
    "SC-DRF06",
    "column-nullability-mismatch",
    // The rule that justifies drift checking existing at all. `verify_queries`
    // cannot check nullability -- preparing a statement makes the server
    // report type OIDs and nothing about NULL-ness -- so reading the catalog
    // is the only way scythe can tell a user their `NOT NULL` assumption is
    // false in production, where the generated non-optional field will fail to
    // decode the first NULL row it meets.
    Severity::Error,
    "Column's DDL nullability does not match the live database"
);

drift_rule!(
    EnumValuesMismatch,
    "SC-DRF07",
    "enum-values-mismatch",
    Severity::Error,
    "Enum type's DDL value set does not match the live database"
);

drift_rule!(
    CompositeMissingFromDatabase,
    "SC-DRF08",
    "composite-missing-from-database",
    Severity::Error,
    "Composite type declared in the DDL does not exist in the live database"
);

drift_rule!(
    CompositeMissingFromDdl,
    "SC-DRF09",
    "composite-missing-from-ddl",
    Severity::Warn,
    "Composite type exists in the live database but is not declared in the DDL"
);

drift_rule!(
    CompositeFieldMissingFromDatabase,
    "SC-DRF10",
    "composite-field-missing-from-database",
    Severity::Error,
    "Composite field declared in the DDL does not exist in the live database"
);

drift_rule!(
    CompositeFieldMissingFromDdl,
    "SC-DRF11",
    "composite-field-missing-from-ddl",
    Severity::Error,
    "Composite field exists in the live database but is not declared in the DDL"
);

drift_rule!(
    CompositeFieldTypeMismatch,
    "SC-DRF12",
    "composite-field-type-mismatch",
    Severity::Error,
    "Composite field's DDL type does not match the live database"
);

drift_rule!(
    CompositeFieldNullabilityMismatch,
    "SC-DRF13",
    "composite-field-nullability-mismatch",
    Severity::Error,
    "Composite field's DDL nullability does not match the live database"
);

/// Every drift rule ID, in rule-number order.
///
/// Exposed so consumers can assert the set is complete and so drift findings
/// can be resolved against a registry without hard-coding the list a second
/// time.
pub const DRIFT_RULE_IDS: [&str; 13] = [
    "SC-DRF01", "SC-DRF02", "SC-DRF03", "SC-DRF04", "SC-DRF05", "SC-DRF06", "SC-DRF07", "SC-DRF08", "SC-DRF09",
    "SC-DRF10", "SC-DRF11", "SC-DRF12", "SC-DRF13",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::drift_registry;

    /// Rule IDs are a user-facing contract: they appear in `scythe.toml`
    /// severity overrides, in SARIF taxonomies, and in CI grep lines.
    /// Renaming one silently turns a user's override into a no-op.
    #[test]
    fn registered_rule_ids_match_the_declared_id_list() {
        let registry = drift_registry();
        let mut registered: Vec<&str> = registry.active_rules().iter().map(|(rule, _)| rule.id()).collect();
        registered.sort_unstable();

        let mut expected = DRIFT_RULE_IDS.to_vec();
        expected.sort_unstable();

        assert_eq!(registered, expected);
    }

    /// SC-DRF02 must default to `Warn`. Every database that has ever been
    /// migrated has a migrations table the committed DDL does not declare, so
    /// an `Error` default would make the very first drift run fail for reasons
    /// the user cannot fix.
    #[test]
    fn table_missing_from_ddl_defaults_to_warn() {
        assert_eq!(TableMissingFromDdl.default_severity(), Severity::Warn);
    }

    /// Everything except SC-DRF02 describes the DDL promising something the
    /// database does not deliver, which is what breaks generated code.
    #[test]
    fn every_rule_except_extra_live_objects_defaults_to_error() {
        let registry = drift_registry();
        for (rule, severity) in registry.active_rules() {
            if matches!(rule.id(), "SC-DRF02" | "SC-DRF09") {
                continue;
            }
            assert_eq!(severity, Severity::Error, "rule {} should default to Error", rule.id());
        }
    }

    /// Drift is its own category so that an existing `safety = "off"` cannot
    /// silently disable it.
    #[test]
    fn every_rule_is_in_the_drift_category() {
        let registry = drift_registry();
        for (rule, _) in registry.active_rules() {
            assert_eq!(rule.category(), RuleCategory::Drift, "rule {}", rule.id());
        }
    }
}
