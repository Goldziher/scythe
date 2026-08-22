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
//! [`PostgresCatalogSource`] implements the same trait while preserving
//! `fetch_live_schema`'s established free-function API for callers that
//! already own a `tokio_postgres::Client`.

use async_trait::async_trait;
use scythe_core::catalog::Catalog;
use tokio_postgres::{Client, NoTls};

use crate::error::InspectError;

use super::live::{fetch_live_catalog, fetch_live_schema};
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

/// PostgreSQL-backed live catalog source.
pub struct PostgresCatalogSource {
    client: Option<Client>,
}

impl PostgresCatalogSource {
    /// Build an unconnected source.
    pub fn new() -> Self {
        Self { client: None }
    }

    /// Build a source around an established PostgreSQL client.
    pub fn from_client(client: Client) -> Self {
        Self { client: Some(client) }
    }

    /// Fetch a codegen catalog from the connected database.
    ///
    /// # Errors
    ///
    /// Returns [`InspectError::NotConnected`] before [`Self::connect`] or
    /// when a PostgreSQL catalog query or catalog validation fails.
    pub async fn fetch_catalog(&mut self, declared_schemas: &[String]) -> Result<Catalog, InspectError> {
        let client = self
            .client
            .as_ref()
            .ok_or(InspectError::NotConnected { engine: "postgres" })?;
        fetch_live_catalog(client, declared_schemas).await
    }
}

impl Default for PostgresCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SchemaCatalogDriver for PostgresCatalogSource {
    fn engine(&self) -> &'static str {
        "postgres"
    }

    async fn connect(&mut self, url: &str) -> Result<(), InspectError> {
        let (client, connection) =
            tokio_postgres::connect(url, NoTls)
                .await
                .map_err(|source| InspectError::Connect {
                    engine: "postgres",
                    source: Box::new(source),
                })?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(error = %error, "PostgreSQL catalog connection terminated");
            }
        });
        self.client = Some(client);
        Ok(())
    }

    async fn fetch_schema(&mut self, declared_schemas: &[String]) -> Result<SchemaDescription, InspectError> {
        let client = self
            .client
            .as_ref()
            .ok_or(InspectError::NotConnected { engine: "postgres" })?;
        fetch_live_schema(client, declared_schemas).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_name_is_postgres() {
        assert_eq!(PostgresCatalogSource::new().engine(), "postgres");
    }

    #[tokio::test]
    async fn fetch_schema_without_connect_errors() {
        let error = PostgresCatalogSource::new()
            .fetch_schema(&[])
            .await
            .expect_err("unconnected source must fail");
        assert!(matches!(error, InspectError::NotConnected { engine: "postgres" }));
    }

    #[tokio::test]
    async fn fetch_catalog_without_connect_errors() {
        let error = PostgresCatalogSource::new()
            .fetch_catalog(&[])
            .await
            .expect_err("unconnected source must fail");
        assert!(matches!(error, InspectError::NotConnected { engine: "postgres" }));
    }
}
