//! PostgreSQL live driver, behind the `pg` feature.
//!
//! All operations run inside a dedicated `scythe_conformance` schema,
//! dropped and recreated fresh on every [`PgExecutor::connect`] -- never
//! `public`, which other workstreams sharing the same database (e.g.
//! `integration_tests`) may already populate with tables of the same names
//! this crate's fixtures use (`users`, `orders`, `tags`, `user_tags`).
//! Running against `public` would corrupt or be corrupted by unrelated
//! concurrent test data.

use tokio_postgres::types::{FromSql, Type};
use tokio_postgres::{Client, NoTls};

use crate::executor::{Executor, ObservedRow};
use crate::fixture::Engine;

/// Catch-all `FromSql` adapter: decodes any Postgres value into just its
/// nullness, without needing to know -- or decode -- its concrete type.
/// This is what lets [`PgExecutor::query_nullness`] stay generic across
/// arbitrary result-column types instead of hand-rolling a per-neutral-type
/// dispatch.
struct AnyValue(bool);

impl<'a> FromSql<'a> for AnyValue {
    fn from_sql(_ty: &Type, _raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(AnyValue(false))
    }

    fn from_sql_null(_ty: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(AnyValue(true))
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error("connecting to postgres: {0}")]
    Connect(#[source] tokio_postgres::Error),
    #[error("preparing an isolated schema: {0}")]
    Isolate(#[source] tokio_postgres::Error),
    #[error("postgres query failed: {0}")]
    Query(#[source] tokio_postgres::Error),
}

pub struct PgExecutor {
    client: Client,
}

impl PgExecutor {
    /// Connect to `conn_str` (a standard `postgres://` URL) and isolate all
    /// subsequent operations inside a fresh `scythe_conformance` schema.
    pub async fn connect(conn_str: &str) -> Result<Self, PgError> {
        let (client, connection) = tokio_postgres::connect(conn_str, NoTls)
            .await
            .map_err(PgError::Connect)?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                eprintln!("scythe-conformance: postgres connection error: {error}");
            }
        });

        client
            .batch_execute(
                "DROP SCHEMA IF EXISTS scythe_conformance CASCADE;\n\
                 CREATE SCHEMA scythe_conformance;\n\
                 SET search_path TO scythe_conformance;",
            )
            .await
            .map_err(PgError::Isolate)?;

        Ok(Self { client })
    }
}

impl Executor for PgExecutor {
    const ENGINE: Engine = Engine::Postgresql;
    type Error = PgError;

    async fn seed_schema(&mut self, schema_sql: &str) -> Result<(), Self::Error> {
        // The simple query protocol (batch_execute) accepts multiple
        // `;`-separated statements in one round trip -- no manual
        // statement-splitting needed for Postgres.
        self.client.batch_execute(schema_sql).await.map_err(PgError::Query)
    }

    async fn seed_run(&mut self, statements: &[String]) -> Result<(), Self::Error> {
        for statement in statements {
            self.client
                .execute(statement.as_str(), &[])
                .await
                .map_err(PgError::Query)?;
        }
        Ok(())
    }

    async fn query_nullness(&mut self, query_sql: &str) -> Result<Vec<ObservedRow>, Self::Error> {
        let rows = self.client.query(query_sql, &[]).await.map_err(PgError::Query)?;

        let mut observed = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut columns = Vec::with_capacity(row.columns().len());
            for (idx, column) in row.columns().iter().enumerate() {
                let value: AnyValue = row.get(idx);
                columns.push((column.name().to_string(), value.0));
            }
            observed.push(ObservedRow::new(columns));
        }
        Ok(observed)
    }
}
