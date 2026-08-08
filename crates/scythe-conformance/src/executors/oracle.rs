//! Oracle live driver, behind the `oracle` feature.
//!
//! ## Isolation: a fresh user, because in Oracle a user *is* a schema
//!
//! Oracle has no cheap `CREATE SCHEMA`: its `CREATE SCHEMA` statement does
//! not create a namespace at all, it only bundles DDL into one transaction
//! against an *existing* schema. In Oracle a schema is the set of objects
//! owned by a user, and the only way to get a new one is `CREATE USER`.
//! So [`OracleExecutor::connect`] creates a user named by
//! [`super::unique_namespace`] and then connects *as that user*, which
//! makes the fixtures' unqualified `users`/`orders` resolve to that user's
//! own schema. See [`super::unique_namespace`] for why that name must be
//! unique per connection and not merely per process.
//!
//! `ALTER SESSION SET CURRENT_SCHEMA` was the cheaper-looking alternative
//! and is the wrong tool twice over: it needs the target schema to already
//! exist (so a user still has to be created), and it redirects *name
//! resolution* only -- the session keeps its original privileges, so
//! `CREATE TABLE` would still fail or land somewhere unintended.
//!
//! ## Identifier case
//!
//! Oracle folds unquoted identifiers to upper case, so a query selecting
//! `total_sum` gets a result column named `TOTAL_SUM` back. scythe's
//! analyzer normalizes every identifier to lower case, so the runner
//! compares against lower-case names. [`OracleExecutor::query_nullness`]
//! therefore folds driver-reported names to lower case rather than letting
//! every Oracle leg fail with a `MissingColumn` -- and hard-errors if two
//! distinct reported names fold together, because
//! [`ObservedRow::new`] would otherwise silently keep only one of them and
//! answer for the wrong column.

use std::sync::OnceLock;

use sibyl::{Environment, Session};

use crate::executor::{Executor, ObservedRow};
use crate::fixture::Engine;

/// Password for every isolated user this executor creates. A constant --
/// not a generated secret -- because these users exist only inside a
/// throwaway conformance container and are useless outside it; generating
/// one would imply a confidentiality property this does not have.
/// Double-quoted at the point of use so Oracle takes it verbatim.
const NAMESPACE_PASSWORD: &str = "Scythe_Conformance1";

#[derive(Debug, thiserror::Error)]
pub enum OracleError {
    #[error("parsing the oracle:// URL: {0}")]
    Url(#[source] url::ParseError),
    #[error("creating the OCI environment: {0}")]
    Environment(#[source] sibyl::Error),
    #[error("connecting: {0}")]
    Connect(#[source] sibyl::Error),
    #[error("preparing an isolated user: {0}")]
    Isolate(#[source] sibyl::Error),
    #[error("query failed: {0}")]
    Query(#[source] sibyl::Error),
    #[error("reading result column metadata: {0}")]
    ColumnMetadata(#[source] sibyl::Error),
    #[error(
        "the driver reported result columns {first:?} and {second:?}, which are the same column name once folded to lower case -- reporting only one of them would answer for the wrong column"
    )]
    AmbiguousColumnCase { first: String, second: String },
}

/// The process-wide OCI environment.
///
/// OCI's environment handle owns the client-side connection machinery and
/// every [`Session`] borrows from it, which makes a per-executor
/// `Environment` a self-referential struct. One `'static` environment shared
/// by every executor is the shape `sibyl` itself documents, and is safe here
/// because the handle is created with `OCI_THREADED` (see `sibyl::env`) and
/// is `Send + Sync`.
static ORACLE: OnceLock<Environment> = OnceLock::new();

/// The process-wide OCI environment, created on first use.
///
/// Written out rather than using `OnceLock::get_or_init` because creating
/// the environment is fallible and `get_or_init` takes an infallible
/// closure: forcing it into that shape would mean panicking inside a
/// library on a missing or misconfigured Oracle client, instead of
/// returning an error the runner can attribute to a fixture and engine.
/// A race here just means one redundant environment is built and dropped.
fn environment() -> Result<&'static Environment, OracleError> {
    if let Some(environment) = ORACLE.get() {
        return Ok(environment);
    }
    let environment = Environment::new().map_err(OracleError::Environment)?;
    let _ = ORACLE.set(environment);
    ORACLE
        .get()
        .ok_or_else(|| OracleError::Environment(sibyl::Error::Interface("OCI environment was not initialized".into())))
}

pub struct OracleExecutor {
    session: Session<'static>,
}

impl OracleExecutor {
    /// Connect using `admin_url` -- an `oracle://user:pass@host:port/service`
    /// URL, the same shape `integration_tests/rust-sibyl-oracle` reads from
    /// `ORACLE_URL` -- and isolate all subsequent operations inside a fresh
    /// user of its own.
    ///
    /// `admin_url` must be able to `CREATE USER` and grant privileges (the
    /// `system` account in the container images CI uses). The per-app
    /// `scythe` user those images provision cannot: it owns exactly one
    /// schema, which other workstreams (e.g. `integration_tests`) already
    /// populate with tables of the same names this crate's fixtures use
    /// (`users`, `orders`, `tags`, `user_tags`).
    pub async fn connect(admin_url: &str) -> Result<Self, OracleError> {
        let url = url::Url::parse(admin_url).map_err(OracleError::Url)?;
        let address = format!(
            "{}:{}/{}",
            url.host_str().unwrap_or("localhost"),
            url.port().unwrap_or(1521),
            url.path().trim_start_matches('/')
        );

        let environment = environment()?;
        let admin = environment
            .connect(&address, url.username(), url.password().unwrap_or(""))
            .await
            .map_err(OracleError::Connect)?;

        let namespace = super::unique_namespace();
        // Granted individually rather than via the `RESOURCE` role, which
        // also carries object-type privileges this suite never needs.
        // `CREATE SEQUENCE` is required despite no fixture declaring one:
        // the live schema's `GENERATED BY DEFAULT AS IDENTITY` columns are
        // backed by system-generated sequences.
        for statement in [
            format!("CREATE USER {namespace} IDENTIFIED BY \"{NAMESPACE_PASSWORD}\""),
            format!("GRANT CREATE SESSION, CREATE TABLE, CREATE SEQUENCE, UNLIMITED TABLESPACE TO {namespace}"),
        ] {
            let prepared = admin.prepare(&statement).await.map_err(OracleError::Isolate)?;
            prepared.execute(()).await.map_err(OracleError::Isolate)?;
        }

        let session = environment
            .connect(&address, &namespace, NAMESPACE_PASSWORD)
            .await
            .map_err(OracleError::Connect)?;

        Ok(Self { session })
    }
}

impl Executor for OracleExecutor {
    const ENGINE: Engine = Engine::Oracle;
    type Error = OracleError;

    async fn seed_schema(&mut self, schema_sql: &str) -> Result<(), Self::Error> {
        // OCI prepares exactly one statement per call and rejects a
        // trailing `;`, so the schema file is split and each terminator
        // dropped -- unlike Postgres's `batch_execute` or SQLite's
        // `execute_batch`, which take the file verbatim.
        for statement in super::split_statements(schema_sql) {
            let prepared = self.session.prepare(&statement).await.map_err(OracleError::Query)?;
            prepared.execute(()).await.map_err(OracleError::Query)?;
        }
        Ok(())
    }

    async fn seed_run(&mut self, statements: &[String]) -> Result<(), Self::Error> {
        for statement in statements {
            let prepared = self
                .session
                .prepare(statement.as_str())
                .await
                .map_err(OracleError::Query)?;
            prepared.execute(()).await.map_err(OracleError::Query)?;
        }
        // Seeding and querying share one session, so the rows would be
        // visible uncommitted -- but an uncommitted seed makes a failing
        // leg impossible to inspect in the container afterwards, and Oracle
        // rolls the whole transaction back when the session closes.
        self.session.commit().await.map_err(OracleError::Query)?;
        Ok(())
    }

    async fn query_nullness(&mut self, query_sql: &str) -> Result<Vec<ObservedRow>, Self::Error> {
        let statement = self.session.prepare(query_sql).await.map_err(OracleError::Query)?;
        let rows = statement.query(()).await.map_err(OracleError::Query)?;

        // Column metadata is only described once the statement has been
        // executed, so this must come after `query`.
        let column_count = statement.column_count().map_err(OracleError::ColumnMetadata)?;
        // Kept as (reported, folded) pairs so a collision can name the two
        // original identifiers -- reporting the folded name twice would say
        // nothing about which columns actually clashed.
        let mut column_names: Vec<(String, String)> = Vec::with_capacity(column_count);
        for position in 0..column_count {
            let info = statement.column(position).ok_or_else(|| {
                OracleError::ColumnMetadata(sibyl::Error::Interface(format!(
                    "the driver reported {column_count} result column(s) but no metadata for column {position}"
                )))
            })?;
            let reported = info.name().map_err(OracleError::ColumnMetadata)?.to_string();
            let folded = reported.to_lowercase();
            if let Some((first, _)) = column_names.iter().find(|(_, existing)| *existing == folded) {
                return Err(OracleError::AmbiguousColumnCase {
                    first: first.clone(),
                    second: reported,
                });
            }
            column_names.push((reported, folded));
        }

        let mut observed = Vec::new();
        while let Some(row) = rows.next().await.map_err(OracleError::Query)? {
            let columns: Vec<(String, bool)> = column_names
                .iter()
                .map(|(_, folded)| folded)
                .enumerate()
                // `Row::is_null` answers `true` for an out-of-range index,
                // which would be fail-open; `position` here is always a
                // real index because it came from `column_count`.
                .map(|(position, name)| (name.clone(), row.is_null(position)))
                .collect();
            observed.push(ObservedRow::new(columns));
        }
        Ok(observed)
    }
}
