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
//! PostgreSQL only. The comparison needs `pg_catalog`, and callers are
//! expected to skip non-PostgreSQL engines rather than pass them here.

pub mod catalog;
pub mod diff;
pub mod live;
pub mod model;

use scythe_lint::reporters::Finding;
use tokio_postgres::Client;

use crate::error::InspectError;

pub use catalog::describe_catalog;
pub use diff::{
    DriftSeverities, SC_DRF01, SC_DRF02, SC_DRF03, SC_DRF04, SC_DRF05, SC_DRF06, SC_DRF07, diff as diff_schemas,
};
pub use live::fetch_live_schema;
pub use model::{ColumnDescription, EnumDescription, SchemaDescription, TableDescription, object_key};

/// Fetch the live schema once and diff every supplied DDL description against
/// it.
///
/// Callers pass `(label, description)` pairs — one per `[[sql]]` block — so a
/// multi-block config attributes each finding to the block whose DDL drifted.
/// The live schema is read once rather than per block: it is the same database
/// for all of them, and re-reading `pg_catalog` per block would multiply the
/// cost of the check by the number of blocks for no extra information.
///
/// Returns an error only when the catalog cannot be read at all. Everything
/// else — including a schema scythe cannot fully interpret — comes back as
/// findings.
pub async fn drift_findings(
    client: &Client,
    schemas: &[(&str, &SchemaDescription)],
    severities: &DriftSeverities,
) -> Result<Vec<Finding>, InspectError> {
    if schemas.is_empty() {
        return Ok(Vec::new());
    }

    let live = fetch_live_schema(client).await?;

    Ok(schemas
        .iter()
        .flat_map(|(label, ddl)| diff_schemas(ddl, &live, severities, label))
        .collect())
}
