//! Schema drift: compare the schema scythe compiled from committed DDL
//! against the schema a live PostgreSQL database actually has.
//!
//! Scythe generates code from DDL checked into the repository. Nothing in that
//! pipeline asks whether the database the generated code will run against
//! still looks like that DDL, so a migration applied out of band — a column
//! dropped, a `NOT NULL` relaxed, an enum value added — leaves generated code
//! that compiles, passes tests against a fresh database, and fails in
//! production.
//!
//! ## What this catches that nothing else can
//!
//! [`verify_queries`](crate::verify::verify_queries) already prepares every
//! query server-side and compares the reported shape against static inference,
//! but it explicitly *cannot* check nullability: preparing a statement makes
//! PostgreSQL report type OIDs and nothing about NULL-ness. `SC-DRF06` closes
//! that gap by reading `pg_attribute.attnotnull` directly, which is the only
//! way scythe can tell a user their `NOT NULL` assumption is false in
//! production.
//!
//! ## Shape
//!
//! - [`describe_catalog`] reduces a [`Catalog`](scythe_core::catalog::Catalog)
//!   to a [`SchemaDescription`]. No I/O.
//! - [`fetch_live_schema`] reads the same description out of `pg_catalog`.
//!   All of the I/O, none of the logic.
//! - [`diff`] compares two already-fetched descriptions and returns findings.
//!   Pure and synchronous, so every rule is unit-testable with no database.
//!
//! Severities come from a [`RuleRegistry`](scythe_lint::RuleRegistry) built by
//! [`drift_registry`](scythe_lint::drift_registry) and carried in
//! [`DriftSeverities`], so `[lint]` in `scythe.toml` tunes drift rules exactly
//! as it tunes every other `SC-*` rule.
//!
//! [`diff`] and [`SchemaDescription`] were always engine-agnostic — they
//! never touch a connection. [`fetch_live_schema`] and [`drift_findings`]
//! (the free functions `scythe-cli`'s `scythe check` calls directly) remain
//! PostgreSQL-only, reading `pg_catalog` through a `tokio_postgres::Client`
//! parameter. Catalog-reading for other engines is not similarly restricted:
//! [`source::SchemaCatalogDriver`] is the engine-agnostic trait for it, and
//! [`crate::sqlite::SqliteCatalogSource`] / [`crate::mysql::MySqlCatalogSource`]
//! implement it. Nothing currently wires their output into [`diff`] the way
//! `fetch_live_schema`'s is — that plumbing (and, for PostgreSQL, populating
//! [`ColumnDescription::primary_key`]) is a follow-up, not this change.

pub mod catalog;
pub mod diff;
pub mod live;
pub mod model;
pub mod source;

use scythe_lint::reporters::Finding;
use tokio_postgres::Client;

use crate::error::InspectError;

pub use catalog::describe_catalog;
pub use diff::{
    DriftSeverities, SC_DRF01, SC_DRF02, SC_DRF03, SC_DRF04, SC_DRF05, SC_DRF06, SC_DRF07, diff as diff_schemas,
};
pub use live::fetch_live_schema;
pub use model::{ColumnDescription, EnumDescription, SchemaDescription, TableDescription, object_key};
pub use source::SchemaCatalogDriver;

/// Fetch the live schema once and diff every supplied DDL description against
/// it.
///
/// Callers pass `(label, description)` pairs — one per `[[sql]]` block — so a
/// multi-block config attributes each finding to the block whose DDL drifted.
/// The live schema is read once rather than per block: it is the same database
/// for all of them, and re-reading `pg_catalog` per block would multiply the
/// cost of the check by the number of blocks for no extra information.
///
/// The live read is scoped to the connection's `search_path` **plus** the
/// schemas the supplied descriptions qualify their objects with, so a DDL that
/// declares `app.accounts` is compared against `app` even when the connection's
/// search path never mentions it.
///
/// # Errors
///
/// - [`InspectError::NoSchemasToCompare`] when `schemas` is empty, or when
///   every description in it is empty. Both mean the check has nothing to
///   compare — a `[[sql]]` block whose schema glob matched no file, say — and
///   returning "no findings" for that reports a clean schema on the strength of
///   work that was never done. A drift gate that passes vacuously is worse than
///   no gate, because CI records it as evidence.
/// - Whatever [`fetch_live_schema`] returns when the catalog cannot be read,
///   including [`InspectError::EmptySchemaScope`].
///
/// Everything else — including a schema scythe cannot fully interpret — comes
/// back as findings.
pub async fn drift_findings(
    client: &Client,
    schemas: &[(&str, &SchemaDescription)],
    severities: &DriftSeverities,
) -> Result<Vec<Finding>, InspectError> {
    if schemas
        .iter()
        .all(|(_, ddl)| ddl.tables.is_empty() && ddl.enums.is_empty())
    {
        return Err(InspectError::NoSchemasToCompare);
    }

    let declared = declared_schemas(schemas);
    let live = fetch_live_schema(client, &declared).await?;

    Ok(schemas
        .iter()
        .flat_map(|(label, ddl)| diff_schemas(ddl, &live, severities, label))
        .collect())
}

/// The schema qualifiers the committed DDL wrote on its own object names.
///
/// Read back off the descriptions rather than threaded down from the config:
/// [`describe_catalog`] stores each object's name exactly as the DDL spelled it
/// (`public.users`, or bare `users` when the DDL did not qualify it), so this
/// is the same list the drift comparison is already making claims about.
///
/// Sorted and deduplicated, because the result decides scope order in
/// [`fetch_live_schema`], which in turn decides which object wins a bare-name
/// collision — an order that varied per run would make drift findings vary
/// per run.
fn declared_schemas(schemas: &[(&str, &SchemaDescription)]) -> Vec<String> {
    let mut declared: Vec<String> = schemas
        .iter()
        .flat_map(|(_, ddl)| {
            let tables = ddl.tables.values().map(|table| table.display_name.as_str());
            let enums = ddl.enums.values().map(|enum_type| enum_type.display_name.as_str());
            tables.chain(enums)
        })
        .filter_map(schema_qualifier)
        .collect();

    declared.sort_unstable();
    declared.dedup();
    declared
}

/// The schema part of a display name, or `None` when the DDL wrote the name
/// bare.
fn schema_qualifier(display_name: &str) -> Option<String> {
    let (schema, _bare) = display_name.rsplit_once('.')?;
    if schema.is_empty() {
        return None;
    }
    Some(schema.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_diff::model::{EnumDescription, TableDescription};

    fn schema_with(tables: &[&str], enums: &[&str]) -> SchemaDescription {
        let mut description = SchemaDescription::new();
        for name in tables {
            description
                .tables
                .insert(object_key(name), TableDescription::new(name.to_string()));
        }
        for name in enums {
            description.enums.insert(
                object_key(name),
                EnumDescription::new(name.to_string(), vec!["a".to_string()]),
            );
        }
        description
    }

    /// The scope fix's input: a DDL that qualifies its tables with `app` has to
    /// put `app` in scope, or SC-DRF01 reports every one of them as missing.
    #[test]
    fn should_collect_the_schema_a_qualified_table_declares() {
        let ddl = schema_with(&["app.accounts"], &[]);
        assert_eq!(declared_schemas(&[("block", &ddl)]), vec!["app".to_string()]);
    }

    /// An enum type can be the only thing a schema holds, and SC-DRF07
    /// compares it, so its qualifier counts too.
    #[test]
    fn should_collect_the_schema_a_qualified_enum_declares() {
        let ddl = schema_with(&[], &["app.status"]);
        assert_eq!(declared_schemas(&[("block", &ddl)]), vec!["app".to_string()]);
    }

    /// The overwhelmingly common case: unqualified DDL adds nothing, so the
    /// live read stays scoped to the search path exactly as before.
    #[test]
    fn should_collect_nothing_when_the_ddl_qualifies_no_object() {
        let ddl = schema_with(&["users", "orders"], &["status"]);
        assert!(declared_schemas(&[("block", &ddl)]).is_empty());
    }

    /// Sorted and deduplicated across every block: this list fixes scope order,
    /// which fixes which object wins a bare-name collision.
    #[test]
    fn should_return_sorted_unique_schemas_across_blocks() {
        let first = schema_with(&["zeta.a", "app.b"], &[]);
        let second = schema_with(&["app.c"], &["zeta.d"]);

        assert_eq!(
            declared_schemas(&[("one", &first), ("two", &second)]),
            vec!["app".to_string(), "zeta".to_string()]
        );
    }

    #[test]
    fn should_lowercase_a_schema_qualifier() {
        assert_eq!(schema_qualifier("App.Accounts").as_deref(), Some("app"));
    }

    #[test]
    fn should_return_no_qualifier_for_a_bare_name() {
        assert_eq!(schema_qualifier("accounts"), None);
    }

    #[test]
    fn should_return_no_qualifier_when_the_schema_part_is_empty() {
        assert_eq!(schema_qualifier(".accounts"), None);
    }
}
