//! SQL Server live driver, behind the `mssql` feature.
//!
//! ## Isolation: a fresh database, not a fresh schema
//!
//! Every [`MssqlExecutor::connect`] creates a database of its own, named by
//! [`super::unique_namespace`] -- see there for why that name must be
//! unique per *connection* and not merely per process.
//!
//! T-SQL does have `CREATE SCHEMA`, and it is the cheaper object, but it is
//! the wrong one here: the fixtures name their tables unqualified
//! (`users`, `orders`), so which schema they land in is decided by the
//! connection's *default schema*. That is a property of the database
//! principal (`ALTER USER ... WITH DEFAULT_SCHEMA`), not of the session, so
//! two concurrent legs sharing the `sa` login could not point at different
//! schemas without writing to shared, server-wide state -- and would then
//! race on it exactly the way the fixed-namespace bug described in
//! [`super::unique_namespace`] did. A database, by contrast, is selected
//! per connection, so two connections physically cannot see each other's
//! tables.
//!
//! ## Why this reconnects instead of issuing `USE`
//!
//! `tiberius`'s `execute`/`query` send every statement through
//! `sp_executesql` (`RpcProcId::ExecuteSQL`). A `USE` inside `sp_executesql`
//! changes the database context of that inner batch only and reverts when
//! it returns, so the schema would silently be created back in the admin
//! database instead. Connecting a second client with
//! `Config::database(namespace)` sets the context for the whole session and
//! has no such failure mode.

use tiberius::{AuthMethod, Client, Config, Row};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::executor::{Executor, ObservedRow};
use crate::fixture::Engine;

#[derive(Debug, thiserror::Error)]
pub enum MssqlError {
    #[error("parsing the sqlserver:// URL: {0}")]
    Url(#[source] url::ParseError),
    #[error("connecting: {0}")]
    Connect(#[source] tiberius::error::Error),
    #[error("opening a TCP connection: {0}")]
    Tcp(#[source] std::io::Error),
    #[error("preparing an isolated database: {0}")]
    Isolate(#[source] tiberius::error::Error),
    #[error("query failed: {0}")]
    Query(#[source] tiberius::error::Error),
}

pub struct MssqlExecutor {
    client: Client<Compat<TcpStream>>,
}

impl MssqlExecutor {
    /// Connect using `admin_url` -- a `sqlserver://user:pass@host:port?database=db`
    /// URL, the same shape `integration_tests/rust-tiberius-mssql` reads
    /// from `MSSQL_URL` -- and isolate all subsequent operations inside a
    /// fresh database of its own.
    ///
    /// `admin_url` must have `CREATE DATABASE` privileges (the `sa` login
    /// in the container images CI uses); its `database` query parameter is
    /// only the database the *first*, administrative connection binds to
    /// while creating the isolated one, and nothing this executor runs
    /// afterwards touches it.
    pub async fn connect(admin_url: &str) -> Result<Self, MssqlError> {
        let url = url::Url::parse(admin_url).map_err(MssqlError::Url)?;
        let namespace = super::unique_namespace();

        // ~keep `DROP ... IF EXISTS` before `CREATE`, even though the namespace is
        // unique per connection: a previous run that crashed mid-leg can
        // leave a database behind whose name this process would reuse only
        // if the OS recycled its pid, and failing on that would be an
        // opaque, unreproducible CI flake rather than a real finding.
        let mut admin = Self::client_for(&url, None).await?;
        admin
            .execute(format!("DROP DATABASE IF EXISTS {namespace}"), &[])
            .await
            .map_err(MssqlError::Isolate)?;
        admin
            .execute(format!("CREATE DATABASE {namespace}"), &[])
            .await
            .map_err(MssqlError::Isolate)?;

        let client = Self::client_for(&url, Some(&namespace)).await?;
        Ok(Self { client })
    }

    /// Dial the server described by `url`, binding the session to
    /// `database` when one is given and to the URL's own `database` query
    /// parameter otherwise.
    async fn client_for(url: &url::Url, database: Option<&str>) -> Result<Client<Compat<TcpStream>>, MssqlError> {
        let mut config = Config::new();
        config.host(url.host_str().unwrap_or("localhost"));
        config.port(url.port().unwrap_or(1433));
        config.authentication(AuthMethod::sql_server(url.username(), url.password().unwrap_or("")));
        match database {
            Some(database) => config.database(database),
            None => {
                if let Some((_, database)) = url.query_pairs().find(|(key, _)| key == "database") {
                    config.database(database);
                }
            }
        }
        // The CI container serves a self-signed certificate, exactly as in
        // `integration_tests/rust-tiberius-mssql`.
        config.trust_cert();

        let tcp = TcpStream::connect(config.get_addr()).await.map_err(MssqlError::Tcp)?;
        tcp.set_nodelay(true).map_err(MssqlError::Tcp)?;
        Client::connect(config, tcp.compat_write())
            .await
            .map_err(MssqlError::Connect)
    }
}

impl Executor for MssqlExecutor {
    const ENGINE: Engine = Engine::Mssql;
    type Error = MssqlError;

    async fn seed_schema(&mut self, schema_sql: &str) -> Result<(), Self::Error> {
        // T-SQL requires `CREATE TABLE` to be the first statement in its
        // batch, so the schema file cannot be sent as one blob the way
        // Postgres's `batch_execute` or SQLite's `execute_batch` accept it.
        for statement in super::split_statements(schema_sql) {
            self.client.execute(statement, &[]).await.map_err(MssqlError::Query)?;
        }
        Ok(())
    }

    async fn seed_run(&mut self, statements: &[String]) -> Result<(), Self::Error> {
        for statement in statements {
            self.client
                .execute(statement.as_str(), &[])
                .await
                .map_err(MssqlError::Query)?;
        }
        Ok(())
    }

    async fn query_nullness(&mut self, query_sql: &str) -> Result<Vec<ObservedRow>, Self::Error> {
        let rows: Vec<Row> = self
            .client
            .query(query_sql, &[])
            .await
            .map_err(MssqlError::Query)?
            .into_first_result()
            .await
            .map_err(MssqlError::Query)?;

        let mut observed = Vec::with_capacity(rows.len());
        for row in &rows {
            let columns: Vec<(String, bool)> = row
                .cells()
                .map(|(column, data)| (column.name().to_string(), is_null(data)))
                .collect();
            observed.push(ObservedRow::new(columns));
        }
        Ok(observed)
    }
}

/// Whether `data` carries SQL NULL.
///
/// Deliberately exhaustive, with no `_` arm: `tiberius::ColumnData` encodes
/// nullness inside a per-type `Option`, and a wildcard would answer
/// "non-null" for any variant added by a future `tiberius` release. That is
/// a fail-open answer to the one question A2 soundness is built on -- a
/// column the engine actually returned NULL would be reported as non-null,
/// and the suite would go green on precisely the case it exists to catch.
/// A new variant must instead be a compile error here.
fn is_null(data: &tiberius::ColumnData<'_>) -> bool {
    use tiberius::ColumnData;
    match data {
        ColumnData::U8(value) => value.is_none(),
        ColumnData::I16(value) => value.is_none(),
        ColumnData::I32(value) => value.is_none(),
        ColumnData::I64(value) => value.is_none(),
        ColumnData::F32(value) => value.is_none(),
        ColumnData::F64(value) => value.is_none(),
        ColumnData::Bit(value) => value.is_none(),
        ColumnData::String(value) => value.is_none(),
        ColumnData::Guid(value) => value.is_none(),
        ColumnData::Binary(value) => value.is_none(),
        ColumnData::Numeric(value) => value.is_none(),
        ColumnData::Xml(value) => value.is_none(),
        ColumnData::DateTime(value) => value.is_none(),
        ColumnData::SmallDateTime(value) => value.is_none(),
        ColumnData::Time(value) => value.is_none(),
        ColumnData::Date(value) => value.is_none(),
        ColumnData::DateTime2(value) => value.is_none(),
        ColumnData::DateTimeOffset(value) => value.is_none(),
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    #[test]
    fn is_null_reports_true_for_every_absent_value() {
        assert!(is_null(&tiberius::ColumnData::I32(None)));
        assert!(is_null(&tiberius::ColumnData::String(None)));
        assert!(is_null(&tiberius::ColumnData::Numeric(None)));
    }

    #[test]
    fn is_null_reports_false_for_a_present_value() {
        assert!(!is_null(&tiberius::ColumnData::I32(Some(1))));
        assert!(!is_null(&tiberius::ColumnData::String(Some(Cow::Borrowed(
            "ada@example.com"
        )))));
    }

    #[test]
    fn is_null_reports_false_for_a_present_but_empty_string() {
        // SQL Server keeps `''` distinct from NULL (unlike Oracle). A2
        // soundness would be wrong in the other direction if this ever
        // started reporting `true`.
        assert!(!is_null(&tiberius::ColumnData::String(Some(Cow::Borrowed("")))));
    }
}
