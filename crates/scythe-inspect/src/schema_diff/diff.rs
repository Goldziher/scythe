//! The drift comparison itself: pure, synchronous, database-free.
//!
//! Everything that talks to a server lives in [`live`](super::live); this
//! module only sees two [`SchemaDescription`]s that someone else already
//! fetched. That split is deliberate — the rules below are where the bugs
//! would be, and keeping them free of I/O is what lets every rule, every skip
//! path and every empty-input case be covered by an ordinary unit test.

use std::collections::HashMap;

use scythe_lint::registry::RuleRegistry;
use scythe_lint::reporters::Finding;
use scythe_lint::rules::drift::DRIFT_RULE_IDS;
use scythe_lint::types::Severity;

use super::model::{ColumnDescription, SchemaDescription, TableDescription};

/// A table declared in the DDL that the live database does not have.
pub const SC_DRF01: &str = "SC-DRF01";
/// A table in the live database that the DDL does not declare.
pub const SC_DRF02: &str = "SC-DRF02";
/// A column declared in the DDL that the live table does not have.
pub const SC_DRF03: &str = "SC-DRF03";
/// A column on the live table that the DDL does not declare.
pub const SC_DRF04: &str = "SC-DRF04";
/// A column whose DDL type disagrees with the live type.
pub const SC_DRF05: &str = "SC-DRF05";
/// A column whose DDL nullability disagrees with the live nullability.
pub const SC_DRF06: &str = "SC-DRF06";
/// An enum whose DDL value set disagrees with the live value set.
pub const SC_DRF07: &str = "SC-DRF07";

/// Tag applied to `Finding::source` so reporters and CI filters can tell
/// drift findings apart from lint, audit, inspect and verify findings.
const SOURCE: &str = "drift";

/// Effective severity for each drift rule, resolved once from a
/// [`RuleRegistry`].
///
/// Resolved up front rather than looked up per finding because the registry is
/// consumed by `LintEngine` in the CLI, and because a drift run can produce
/// thousands of findings against a large schema.
///
/// A rule missing from the map is `Off`: [`RuleRegistry::active_rules`] omits
/// rules a user switched off, so absence and "off" are the same statement.
#[derive(Debug, Clone)]
pub struct DriftSeverities {
    by_rule: HashMap<&'static str, Severity>,
}

impl DriftSeverities {
    /// Resolve severities from a registry that has already had the user's
    /// `[lint]` configuration applied.
    ///
    /// Only `SC-DRF*` rules are read, so passing a registry that also holds
    /// lint or audit rules is harmless.
    pub fn from_registry(registry: &RuleRegistry) -> Self {
        let mut by_rule = HashMap::new();
        for (rule, severity) in registry.active_rules() {
            if let Some(id) = DRIFT_RULE_IDS.iter().find(|known| **known == rule.id()) {
                by_rule.insert(*id, severity);
            }
        }
        Self { by_rule }
    }

    /// Severity for a rule ID, or [`Severity::Off`] when the user disabled it.
    pub fn severity_for(&self, rule_id: &str) -> Severity {
        self.by_rule.get(rule_id).copied().unwrap_or(Severity::Off)
    }
}

impl Default for DriftSeverities {
    /// The shipped defaults, with no user configuration applied.
    fn default() -> Self {
        Self::from_registry(&scythe_lint::drift_registry())
    }
}

/// Accumulates findings while dropping the ones whose rule is switched off.
///
/// Filtering at push time rather than afterwards keeps the message
/// construction — which formats type names and value lists — off the hot path
/// for rules the user disabled.
struct FindingSink<'a> {
    findings: Vec<Finding>,
    severities: &'a DriftSeverities,
    label: &'a str,
}

impl<'a> FindingSink<'a> {
    fn new(severities: &'a DriftSeverities, label: &'a str) -> Self {
        Self {
            findings: Vec::new(),
            severities,
            label,
        }
    }

    fn is_enabled(&self, rule_id: &str) -> bool {
        self.severities.severity_for(rule_id) != Severity::Off
    }

    fn push(&mut self, rule_id: &'static str, rule_name: &str, message: String) {
        let severity = self.severities.severity_for(rule_id);
        if severity == Severity::Off {
            return;
        }
        self.findings.push(Finding {
            file: self.label.to_string(),
            query_name: None,
            rule_id: rule_id.to_string(),
            rule_name: Some(rule_name.to_string()),
            rule_description: Some("committed DDL does not match the live database schema".to_string()),
            severity,
            message,
            line: None,
            column: None,
            cwe: Vec::new(),
            source: Some(SOURCE.to_string()),
        });
    }
}

/// Compare the schema scythe built from committed DDL against the schema a
/// live database reports, and return one finding per disagreement.
///
/// `label` is only used to fill `Finding::file` — typically the `[[sql]]`
/// block name, so a mixed-schema config says which block drifted.
///
/// Pure and synchronous by construction: both descriptions are already
/// fetched, so nothing here can block, fail, or depend on a connection.
pub fn diff(
    ddl: &SchemaDescription,
    live: &SchemaDescription,
    severities: &DriftSeverities,
    label: &str,
) -> Vec<Finding> {
    let mut sink = FindingSink::new(severities, label);

    diff_tables(&mut sink, ddl, live);
    diff_enums(&mut sink, ddl, live);

    sink.findings
}

fn diff_tables(sink: &mut FindingSink<'_>, ddl: &SchemaDescription, live: &SchemaDescription) {
    for (key, ddl_table) in &ddl.tables {
        match live.tables.get(key) {
            Some(live_table) => diff_columns(sink, ddl_table, live_table),
            None => sink.push(
                SC_DRF01,
                "table-missing-from-database",
                format!(
                    "table `{}` is declared in the schema but does not exist in the database",
                    ddl_table.display_name
                ),
            ),
        }
    }

    if !sink.is_enabled(SC_DRF02) {
        return;
    }

    for (key, live_table) in &live.tables {
        if ddl.tables.contains_key(key) {
            continue;
        }
        sink.push(
            SC_DRF02,
            "table-missing-from-ddl",
            format!(
                "table `{}` exists in the database but is not declared in the schema",
                live_table.display_name
            ),
        );
    }
}

fn diff_columns(sink: &mut FindingSink<'_>, ddl_table: &TableDescription, live_table: &TableDescription) {
    for (key, ddl_column) in &ddl_table.columns {
        let Some(live_column) = live_table.columns.get(key) else {
            sink.push(
                SC_DRF03,
                "column-missing-from-database",
                format!(
                    "column `{}.{}` is declared in the schema but does not exist in the database",
                    ddl_table.display_name, ddl_column.name
                ),
            );
            continue;
        };

        diff_column_type(sink, ddl_table, ddl_column, live_column);
        diff_column_nullability(sink, ddl_table, live_table, ddl_column, live_column);
    }

    for (key, live_column) in &live_table.columns {
        if ddl_table.columns.contains_key(key) {
            continue;
        }
        sink.push(
            SC_DRF04,
            "column-missing-from-ddl",
            format!(
                "column `{}.{}` exists in the database but is not declared in the schema",
                live_table.display_name, live_column.name
            ),
        );
    }
}

/// Whether a DDL-declared neutral type and a live neutral type describe the
/// same column.
///
/// Deliberately *not*
/// [`types_are_compatible`](crate::verify::pg_types::types_are_compatible),
/// which the query verifier uses. The two answer different questions:
///
/// - `types_are_compatible` asks "could static *inference* have produced this,
///   given it sometimes cannot pin a type down?". It therefore lets the
///   inferred side widen — `string` is accepted against `uuid`, `json` and
///   `inet`, enums are accepted against `string` in both directions, and
///   integer and float widths are interchangeable — because an untyped
///   parameter or an unspecialised expression legitimately infers as the
///   coarser type and the server's choice is authoritative.
/// - Drift asks "does the committed DDL still describe this column?". Nothing
///   here is an inference: [`describe_catalog`](super::describe_catalog) reads
///   the literal declared column type out of the DDL. There is no coarseness
///   to forgive, so every one of those tolerances would instead hide a real
///   migration: `ALTER COLUMN x TYPE uuid` against a column still declared
///   `text` compares as compatible under the verifier's predicate and would
///   report nothing — exactly the drift this rule exists to catch.
///
/// So the answer is exact equality. Both sides are already normalised into
/// the same neutral vocabulary by the same code paths, which absorbs the
/// spellings that genuinely mean one type: `serial`/`int4` both reach
/// `int32`, `varchar(n)`/`char(n)`/`text` all reach `string`, `numeric(p,s)`
/// reaches `decimal`, a domain resolves to its base type on both sides, and an
/// enum reaches `enum::name` from the DDL and from `pg_type` alike. A
/// difference that survives that normalisation is a real difference — a
/// declared `int32` against a live `int64` means generated code holds an `i32`
/// for a column that no longer fits in one.
fn types_match_for_drift(ddl_type: &str, live_type: &str) -> bool {
    ddl_type == live_type
}

fn diff_column_type(
    sink: &mut FindingSink<'_>,
    ddl_table: &TableDescription,
    ddl_column: &ColumnDescription,
    live_column: &ColumnDescription,
) {
    // ~keep A type either side cannot express in scythe's neutral vocabulary is not
    // evidence of drift — it is evidence that scythe has no opinion about the
    // type. Comparing anyway would report every `xml`, `point` or extension
    // column as a mismatch on a schema that is perfectly in sync.
    let (Some(ddl_type), Some(live_type)) = (&ddl_column.neutral_type, &live_column.neutral_type) else {
        return;
    };

    if types_match_for_drift(ddl_type, live_type) {
        return;
    }

    sink.push(
        SC_DRF05,
        "column-type-mismatch",
        format!(
            "column `{}.{}` is declared as `{}` in the schema but the database reports `{}`",
            ddl_table.display_name, ddl_column.name, ddl_type, live_type
        ),
    );
}

fn diff_column_nullability(
    sink: &mut FindingSink<'_>,
    ddl_table: &TableDescription,
    live_table: &TableDescription,
    ddl_column: &ColumnDescription,
    live_column: &ColumnDescription,
) {
    if !ddl_table.nullability_is_authoritative || !live_table.nullability_is_authoritative {
        return;
    }

    if ddl_column.nullable == live_column.nullable {
        return;
    }

    // The two directions fail differently and the message has to say which.
    // A column the schema calls NOT NULL but the database lets be NULL is the
    // dangerous one: scythe generated a non-optional field for it, so the
    // first NULL row in production fails to decode.
    let message = if live_column.nullable {
        format!(
            "column `{}.{}` is declared NOT NULL in the schema but is nullable in the database — \
             generated code treats it as always present and will fail to decode a NULL row",
            ddl_table.display_name, ddl_column.name
        )
    } else {
        format!(
            "column `{}.{}` is declared nullable in the schema but is NOT NULL in the database — \
             generated code wraps it in an optional it can never be",
            ddl_table.display_name, ddl_column.name
        )
    };

    sink.push(SC_DRF06, "column-nullability-mismatch", message);
}

fn diff_enums(sink: &mut FindingSink<'_>, ddl: &SchemaDescription, live: &SchemaDescription) {
    if !sink.is_enabled(SC_DRF07) {
        return;
    }

    for (key, ddl_enum) in &ddl.enums {
        // An enum present on only one side is not reported. It is only
        // reachable through a column that uses it, and such a column is
        // already covered: the table is either missing (SC-DRF01/02) or the
        // column's type differs (SC-DRF05). Reporting the type itself as well
        // would double up on one underlying change.
        let Some(live_enum) = live.enums.get(key) else {
            continue;
        };

        // Compared as sets, not as ordered sequences. Reordering an enum's
        // labels changes PostgreSQL's sort order but nothing about the values
        // generated code must handle, so flagging a pure reorder would be
        // noise in the one report that has to stay worth reading.
        let missing_from_database = difference(&ddl_enum.values, &live_enum.values);
        let missing_from_ddl = difference(&live_enum.values, &ddl_enum.values);

        if missing_from_database.is_empty() && missing_from_ddl.is_empty() {
            continue;
        }

        let mut detail = Vec::new();
        if !missing_from_database.is_empty() {
            detail.push(format!(
                "missing from the database: {}",
                quoted_list(&missing_from_database)
            ));
        }
        if !missing_from_ddl.is_empty() {
            detail.push(format!("missing from the schema: {}", quoted_list(&missing_from_ddl)));
        }

        sink.push(
            SC_DRF07,
            "enum-values-mismatch",
            format!(
                "enum `{}` has different values in the schema and the database ({})",
                ddl_enum.display_name,
                detail.join("; ")
            ),
        );
    }
}

/// Values present in `left` but not in `right`, in `left`'s order.
fn difference(left: &[String], right: &[String]) -> Vec<String> {
    left.iter().filter(|value| !right.contains(value)).cloned().collect()
}

fn quoted_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("`{value}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_diff::model::{EnumDescription, TableDescription};

    fn severities() -> DriftSeverities {
        DriftSeverities::default()
    }

    fn schema_with(tables: Vec<(&str, TableDescription)>) -> SchemaDescription {
        let mut schema = SchemaDescription::new();
        for (key, table) in tables {
            schema.tables.insert(key.to_string(), table);
        }
        schema
    }

    fn users_table(nullable_email: bool) -> TableDescription {
        TableDescription::new("public.users")
            .with_column(ColumnDescription::new("id", "int32", false))
            .with_column(ColumnDescription::new("email", "string", nullable_email))
    }

    fn rule_ids(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.rule_id.as_str()).collect()
    }

    fn only(findings: &[Finding]) -> &Finding {
        assert_eq!(
            findings.len(),
            1,
            "expected exactly one finding, got {:?}",
            rule_ids(findings)
        );
        &findings[0]
    }

    #[test]
    fn two_empty_schemas_produce_no_findings() {
        let findings = diff(
            &SchemaDescription::new(),
            &SchemaDescription::new(),
            &severities(),
            "block",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn identical_schemas_produce_no_findings() {
        let ddl = schema_with(vec![("users", users_table(true))]);
        let live = schema_with(vec![("users", users_table(true))]);
        assert!(diff(&ddl, &live, &severities(), "block").is_empty());
    }

    /// SC-DRF01: the DDL promises a table the database does not have, so every
    /// generated query against it fails at runtime.
    #[test]
    fn sc_drf01_fires_when_a_ddl_table_is_absent_from_the_database() {
        let ddl = schema_with(vec![("users", users_table(true))]);
        let live = SchemaDescription::new();

        let findings = diff(&ddl, &live, &severities(), "block");
        let finding = only(&findings);

        assert_eq!(finding.rule_id, SC_DRF01);
        assert_eq!(finding.severity, Severity::Error);
        assert_eq!(finding.source.as_deref(), Some("drift"));
        assert_eq!(finding.file, "block");
        assert!(finding.message.contains("public.users"), "{}", finding.message);
    }

    /// SC-DRF02: a table only the database has. Warns, because a migrations
    /// ledger is the single most common thing in this category.
    #[test]
    fn sc_drf02_warns_when_a_database_table_is_absent_from_the_ddl() {
        let ddl = SchemaDescription::new();
        let live = schema_with(vec![(
            "schema_migrations",
            TableDescription::new("public.schema_migrations")
                .with_column(ColumnDescription::new("version", "string", false)),
        )]);

        let findings = diff(&ddl, &live, &severities(), "block");
        let finding = only(&findings);

        assert_eq!(finding.rule_id, SC_DRF02);
        assert_eq!(
            finding.severity,
            Severity::Warn,
            "a migrations table must not fail the check on day one"
        );
    }

    /// SC-DRF03: a column the DDL declares and the database does not have.
    #[test]
    fn sc_drf03_fires_when_a_ddl_column_is_absent_from_the_database() {
        let ddl = schema_with(vec![("users", users_table(true))]);
        let live = schema_with(vec![(
            "users",
            TableDescription::new("public.users").with_column(ColumnDescription::new("id", "int32", false)),
        )]);

        let findings = diff(&ddl, &live, &severities(), "block");
        let finding = only(&findings);

        assert_eq!(finding.rule_id, SC_DRF03);
        assert_eq!(finding.severity, Severity::Error);
        assert!(finding.message.contains("public.users.email"), "{}", finding.message);
    }

    /// SC-DRF04: a column only the database has. An error, not a warning:
    /// scythe expands `SELECT *` from the DDL, so the extra column widens
    /// every generated `SELECT *` row past the struct decoding it.
    #[test]
    fn sc_drf04_fires_when_a_database_column_is_absent_from_the_ddl() {
        let ddl = schema_with(vec![(
            "users",
            TableDescription::new("public.users").with_column(ColumnDescription::new("id", "int32", false)),
        )]);
        let live = schema_with(vec![("users", users_table(true))]);

        let findings = diff(&ddl, &live, &severities(), "block");
        let finding = only(&findings);

        assert_eq!(finding.rule_id, SC_DRF04);
        assert_eq!(finding.severity, Severity::Error);
        assert!(finding.message.contains("public.users.email"), "{}", finding.message);
    }

    /// SC-DRF05: same column name, incompatible neutral types.
    #[test]
    fn sc_drf05_fires_on_an_incompatible_type() {
        let ddl = schema_with(vec![(
            "users",
            TableDescription::new("public.users").with_column(ColumnDescription::new("id", "uuid", false)),
        )]);
        let live = schema_with(vec![(
            "users",
            TableDescription::new("public.users").with_column(ColumnDescription::new("id", "int32", false)),
        )]);

        let findings = diff(&ddl, &live, &severities(), "block");
        let finding = only(&findings);

        assert_eq!(finding.rule_id, SC_DRF05);
        assert_eq!(finding.severity, Severity::Error);
        assert!(finding.message.contains("`uuid`"), "{}", finding.message);
        assert!(finding.message.contains("`int32`"), "{}", finding.message);
    }

    /// Build a one-column schema pair differing only in that column's type.
    fn type_pair(ddl_type: &str, live_type: &str) -> (SchemaDescription, SchemaDescription) {
        (
            schema_with(vec![(
                "t",
                TableDescription::new("t").with_column(ColumnDescription::new("c", ddl_type, false)),
            )]),
            schema_with(vec![(
                "t",
                TableDescription::new("t").with_column(ColumnDescription::new("c", live_type, false)),
            )]),
        )
    }

    fn assert_type_drift_reported(ddl_type: &str, live_type: &str) {
        let (ddl, live) = type_pair(ddl_type, live_type);
        let findings = diff(&ddl, &live, &severities(), "block");
        assert_eq!(
            rule_ids(&findings),
            vec![SC_DRF05],
            "declared `{ddl_type}` against live `{live_type}` must be reported as drift"
        );
    }

    /// The migrations `types_are_compatible` waves through and drift must not.
    /// That predicate forgives an *inferred* `string` widening to a specific
    /// wire type, because static inference genuinely cannot always pin a type
    /// down. A DDL declaration is not an inference, so `ALTER COLUMN ... TYPE
    /// uuid` against a column still declared `text` is real drift.
    #[test]
    fn sc_drf05_reports_a_declared_string_against_a_live_uuid() {
        assert_type_drift_reported("string", "uuid");
    }

    #[test]
    fn sc_drf05_reports_a_declared_string_against_a_live_json() {
        assert_type_drift_reported("string", "json");
    }

    #[test]
    fn sc_drf05_reports_a_declared_string_against_a_live_inet() {
        assert_type_drift_reported("string", "inet");
    }

    /// `types_are_compatible` accepts enum against `string` in both
    /// directions, because several drivers carry enum values as strings on the
    /// wire. For drift both directions are real schema changes: someone either
    /// converted a text column to an enum, or an enum column back to text.
    #[test]
    fn sc_drf05_reports_a_declared_string_against_a_live_enum() {
        assert_type_drift_reported("string", "enum::status");
    }

    #[test]
    fn sc_drf05_reports_a_declared_enum_against_a_live_string() {
        assert_type_drift_reported("enum::status", "string");
    }

    /// Width is not a matter of taste once both sides are declarations:
    /// generated code holds an `i32` for a column that no longer fits in one.
    #[test]
    fn sc_drf05_reports_integer_and_float_width_changes() {
        assert_type_drift_reported("int32", "int64");
        assert_type_drift_reported("float32", "float64");
    }

    /// Both sides normalise through the same neutral vocabulary, so the
    /// spellings that genuinely mean one type must still compare equal —
    /// otherwise exact matching turns every ordinary schema into noise.
    #[test]
    fn sc_drf05_stays_silent_when_the_neutral_types_are_equal() {
        for neutral in [
            "int32",
            "string",
            "decimal",
            "datetime_tz",
            "enum::status",
            "array<string>",
        ] {
            let (ddl, live) = type_pair(neutral, neutral);
            assert!(
                diff(&ddl, &live, &severities(), "block").is_empty(),
                "`{neutral}` must compare equal to itself"
            );
        }
    }

    /// The skip-on-unmappable path, from the live side: a type scythe cannot
    /// map is not evidence of drift.
    #[test]
    fn sc_drf05_skips_the_column_when_the_live_type_is_unmappable() {
        let ddl = schema_with(vec![(
            "shapes",
            TableDescription::new("shapes").with_column(ColumnDescription::new("area", "string", false)),
        )]);
        let live = schema_with(vec![(
            "shapes",
            TableDescription::new("shapes").with_column(ColumnDescription::unmappable("area", false)),
        )]);

        assert!(diff(&ddl, &live, &severities(), "block").is_empty());
    }

    /// The same skip from the DDL side, so neither direction can produce a
    /// false positive on a type scythe has no opinion about.
    #[test]
    fn sc_drf05_skips_the_column_when_the_ddl_type_is_unmappable() {
        let ddl = schema_with(vec![(
            "shapes",
            TableDescription::new("shapes").with_column(ColumnDescription::unmappable("area", false)),
        )]);
        let live = schema_with(vec![(
            "shapes",
            TableDescription::new("shapes").with_column(ColumnDescription::new("area", "string", false)),
        )]);

        assert!(diff(&ddl, &live, &severities(), "block").is_empty());
    }

    /// An unmappable type suppresses only the type comparison. Nullability is
    /// still comparable and is the whole reason drift checking exists.
    #[test]
    fn an_unmappable_type_does_not_suppress_the_nullability_check() {
        let ddl = schema_with(vec![(
            "shapes",
            TableDescription::new("shapes").with_column(ColumnDescription::unmappable("area", false)),
        )]);
        let live = schema_with(vec![(
            "shapes",
            TableDescription::new("shapes").with_column(ColumnDescription::unmappable("area", true)),
        )]);

        let findings = diff(&ddl, &live, &severities(), "block");
        assert_eq!(rule_ids(&findings), vec![SC_DRF06]);
    }

    /// SC-DRF06, the direction that breaks production: the DDL says NOT NULL,
    /// the database says nullable, so generated code has a non-optional field
    /// that a NULL row will fail to decode. `verify_queries` cannot see this —
    /// preparing a statement reports types and nothing about NULL-ness.
    #[test]
    fn sc_drf06_fires_when_the_ddl_claims_not_null_but_the_database_allows_null() {
        let ddl = schema_with(vec![("users", users_table(false))]);
        let live = schema_with(vec![("users", users_table(true))]);

        let findings = diff(&ddl, &live, &severities(), "block");
        let finding = only(&findings);

        assert_eq!(finding.rule_id, SC_DRF06);
        assert_eq!(finding.severity, Severity::Error);
        assert!(
            finding.message.contains("fail to decode a NULL row"),
            "the message must name the failure: {}",
            finding.message
        );
    }

    /// The opposite direction is still drift and still reported, but the
    /// message has to describe the other failure — an optional that can never
    /// be `None`.
    #[test]
    fn sc_drf06_fires_when_the_database_is_stricter_than_the_ddl() {
        let ddl = schema_with(vec![("users", users_table(true))]);
        let live = schema_with(vec![("users", users_table(false))]);

        let findings = diff(&ddl, &live, &severities(), "block");
        let finding = only(&findings);

        assert_eq!(finding.rule_id, SC_DRF06);
        assert!(
            finding.message.contains("optional it can never be"),
            "{}",
            finding.message
        );
    }

    /// PostgreSQL reports every view column as nullable regardless of the
    /// underlying table, while scythe's catalog copies the base column's
    /// NOT NULL through. Comparing them would report drift on every non-null
    /// column of every view.
    #[test]
    fn sc_drf06_is_skipped_when_the_live_relation_is_a_view() {
        let ddl = schema_with(vec![("active_users", users_table(false))]);
        let live = schema_with(vec![(
            "active_users",
            users_table(true).without_authoritative_nullability(),
        )]);

        assert!(diff(&ddl, &live, &severities(), "block").is_empty());
    }

    /// SC-DRF07: the value sets differ, and the message must name which values
    /// are missing on which side.
    #[test]
    fn sc_drf07_fires_when_enum_values_differ() {
        let mut ddl = SchemaDescription::new();
        ddl.enums.insert(
            "status".to_string(),
            EnumDescription::new("status", vec!["active".into(), "banned".into()]),
        );
        let mut live = SchemaDescription::new();
        live.enums.insert(
            "status".to_string(),
            EnumDescription::new("status", vec!["active".into(), "pending".into()]),
        );

        let findings = diff(&ddl, &live, &severities(), "block");
        let finding = only(&findings);

        assert_eq!(finding.rule_id, SC_DRF07);
        assert_eq!(finding.severity, Severity::Error);
        assert!(finding.message.contains("`banned`"), "{}", finding.message);
        assert!(finding.message.contains("`pending`"), "{}", finding.message);
    }

    /// Reordering an enum's labels changes PostgreSQL's sort order but not the
    /// set of values generated code must handle, so it is not drift.
    #[test]
    fn sc_drf07_ignores_pure_reordering() {
        let mut ddl = SchemaDescription::new();
        ddl.enums.insert(
            "status".to_string(),
            EnumDescription::new("status", vec!["active".into(), "banned".into()]),
        );
        let mut live = SchemaDescription::new();
        live.enums.insert(
            "status".to_string(),
            EnumDescription::new("status", vec!["banned".into(), "active".into()]),
        );

        assert!(diff(&ddl, &live, &severities(), "block").is_empty());
    }

    /// An enum only one side declares is reached only through a column that
    /// uses it, and that column is already covered by SC-DRF01/02/05.
    #[test]
    fn sc_drf07_ignores_an_enum_present_on_only_one_side() {
        let mut ddl = SchemaDescription::new();
        ddl.enums.insert(
            "status".to_string(),
            EnumDescription::new("status", vec!["active".into()]),
        );

        assert!(diff(&ddl, &SchemaDescription::new(), &severities(), "block").is_empty());
    }

    /// A missing table must not also produce a column finding for each of its
    /// columns: one change, one finding.
    #[test]
    fn a_missing_table_reports_once_not_once_per_column() {
        let ddl = schema_with(vec![("users", users_table(true))]);
        let findings = diff(&ddl, &SchemaDescription::new(), &severities(), "block");
        assert_eq!(rule_ids(&findings), vec![SC_DRF01]);
    }

    /// Turning a rule off in `[lint]` must actually suppress it, which is the
    /// whole reason severities come from the registry instead of the
    /// construction site.
    #[test]
    fn a_rule_switched_off_in_config_produces_no_finding() {
        use scythe_lint::types::LintConfig;

        let mut registry = scythe_lint::drift_registry();
        let mut config = LintConfig::default();
        config.rules.insert(SC_DRF02.to_string(), Severity::Off);
        registry.apply_config(&config);
        let severities = DriftSeverities::from_registry(&registry);

        let live = schema_with(vec![("schema_migrations", TableDescription::new("schema_migrations"))]);

        assert!(diff(&SchemaDescription::new(), &live, &severities, "block").is_empty());
    }

    /// Raising SC-DRF02 to `Error` must reach the emitted finding — the
    /// severity in the report is the configured one, not the default.
    #[test]
    fn a_rule_raised_in_config_emits_the_configured_severity() {
        use scythe_lint::types::LintConfig;

        let mut registry = scythe_lint::drift_registry();
        let mut config = LintConfig::default();
        config.rules.insert(SC_DRF02.to_string(), Severity::Error);
        registry.apply_config(&config);
        let severities = DriftSeverities::from_registry(&registry);

        let live = schema_with(vec![("schema_migrations", TableDescription::new("schema_migrations"))]);
        let findings = diff(&SchemaDescription::new(), &live, &severities, "block");

        assert_eq!(only(&findings).severity, Severity::Error);
    }

    /// Defaults must come from the shipped registry, so a change to a rule's
    /// `default_severity` cannot silently disagree with what drift emits.
    #[test]
    fn default_severities_match_the_shipped_registry() {
        let severities = DriftSeverities::default();
        assert_eq!(severities.severity_for(SC_DRF01), Severity::Error);
        assert_eq!(severities.severity_for(SC_DRF02), Severity::Warn);
        assert_eq!(severities.severity_for(SC_DRF06), Severity::Error);
    }

    /// An unknown rule ID resolves to `Off` rather than a default, so a typo
    /// in a rule constant cannot smuggle findings into the report.
    #[test]
    fn an_unknown_rule_id_resolves_to_off() {
        assert_eq!(DriftSeverities::default().severity_for("SC-DRF99"), Severity::Off);
    }

    /// Findings must be ordered deterministically, or a CI diff of a drift
    /// report is unreadable between runs.
    #[test]
    fn findings_come_out_in_a_stable_order() {
        let ddl = schema_with(vec![
            ("zebras", TableDescription::new("zebras")),
            ("aardvarks", TableDescription::new("aardvarks")),
        ]);

        let first = diff(&ddl, &SchemaDescription::new(), &severities(), "block");
        let second = diff(&ddl, &SchemaDescription::new(), &severities(), "block");

        let messages: Vec<&str> = first.iter().map(|f| f.message.as_str()).collect();
        let repeat: Vec<&str> = second.iter().map(|f| f.message.as_str()).collect();
        assert_eq!(messages, repeat);
        assert!(messages[0].contains("aardvarks"), "expected sorted order: {messages:?}");
    }

    /// Rule IDs are a user-facing contract (severity overrides, SARIF
    /// taxonomies), so guard the constants against accidental renaming.
    #[test]
    fn rule_ids_are_stable() {
        assert_eq!(SC_DRF01, "SC-DRF01");
        assert_eq!(SC_DRF02, "SC-DRF02");
        assert_eq!(SC_DRF03, "SC-DRF03");
        assert_eq!(SC_DRF04, "SC-DRF04");
        assert_eq!(SC_DRF05, "SC-DRF05");
        assert_eq!(SC_DRF06, "SC-DRF06");
        assert_eq!(SC_DRF07, "SC-DRF07");
    }
}
