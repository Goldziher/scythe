//! `SchemaCatalogDriver` — the engine-agnostic interface for reading a live
//! database's schema into the neutral [`SchemaDescription`] catalog shape.
//!
//! This is a different abstraction from [`DbDriver`](crate::driver::DbDriver):
//! `DbDriver` runs the `SC-INS` health-check registry — hand-written,
//! engine-specific SQL living in `checks.toml` — and returns
//! [`Finding`](scythe_lint::reporters::Finding)s. Porting that registry to a
//! second engine means writing and validating a new query per check against
//! that engine's own catalog views, which is real per-engine work this trait
//! does not attempt.
//!
//! What *is* engine-independent is the shape schema drift already compares:
//! tables, their columns, each column's neutral type and nullability, and
//! (see [`ColumnDescription::primary_key`](super::model::ColumnDescription))
//! which columns identify a row. [`fetch_live_schema`](super::live::fetch_live_schema)
//! builds exactly that shape for PostgreSQL by reading `pg_catalog` directly
//! through a `tokio_postgres::Client` — there was no trait to implement
//! because there was only ever the one engine. This trait is that missing
//! seam: an engine gains catalog introspection by implementing `connect` and
//! `fetch_schema` against its own driver, with no change required to
//! [`SchemaDescription`], [`diff`](super::diff::diff), or anything that
//! consumes them.
//!
//! PostgreSQL is not (yet) rewired through this trait. `fetch_live_schema`'s
//! free-function signature — `(&Client, &[String])` — is a public API that
//! `scythe-cli`'s `scythe check` already calls directly; wrapping it here
//! would either change that signature or leave two ways to do the same
//! fetch. Bound `Self: Sized`-free by construction (no method here returns
//! `Self`), so a `PostgresCatalogDriver` adapter over the existing free
//! functions can be added later without touching this trait.

use async_trait::async_trait;

use crate::error::InspectError;

use super::model::SchemaDescription;

/// Read a live database's schema into a [`SchemaDescription`].
///
/// Construction is expected to be infallible and connectionless (mirroring
/// [`DbDriver`](crate::driver::DbDriver)); [`connect`](Self::connect) opens
/// the connection and [`fetch_schema`](Self::fetch_schema) reads the catalog.
/// Splitting them keeps a driver usable in contexts that only need identity
/// (e.g. a future `--list-engines`) without touching the network or disk.
///
/// `fetch_schema` takes `&mut self` rather than `&self`: `mysql_async`'s
/// `Queryable` trait requires an exclusive reference for every query, and a
/// trait meant to be implemented by more than one driver has to accept the
/// stricter of the two rather than force each implementation to hide
/// interior mutability behind a lock it does not otherwise need.
#[async_trait]
pub trait SchemaCatalogDriver: Send {
    /// Stable engine identifier — e.g. `"sqlite"`, `"mysql"`. Matches the
    /// engine strings [`DbDriver::engine`](crate::driver::DbDriver::engine)
    /// and `scythe_core::dialect::SqlDialect::from_str` use, so a caller that
    /// already resolved an engine name for one can reuse it for the other.
    fn engine(&self) -> &'static str;

    /// Open a connection (or, for an embedded engine like SQLite, a file
    /// handle) using `url`. Implementations should be idempotent: calling
    /// `connect` twice replaces any connection already held.
    async fn connect(&mut self, url: &str) -> Result<(), InspectError>;

    /// Read every table this connection can see into a [`SchemaDescription`].
    ///
    /// `declared_schemas` is the schema-qualifier scope the committed DDL
    /// declares — see [`fetch_live_schema`](super::live::fetch_live_schema)'s
    /// doc comment for why PostgreSQL's version unions this with the
    /// connection's search path. Engines with no comparable multi-schema
    /// scoping (SQLite has exactly one implicit schema per file) are free to
    /// ignore the parameter; it is part of the trait so a caller does not
    /// need to special-case engines that do have the concept.
    async fn fetch_schema(&mut self, declared_schemas: &[String]) -> Result<SchemaDescription, InspectError>;
}
