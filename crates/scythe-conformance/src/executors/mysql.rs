//! MySQL and MariaDB live driver, behind the `mysql` and `mariadb`
//! features respectively.
//!
//! Both engines are wire-protocol compatible and driven by the same
//! `mysql_async` client; [`MySqlExecutor<E>`] is generic over an
//! [`EngineMarker`] so `Executor::ENGINE` still reports the right
//! [`Engine`] for each without duplicating the driver logic.

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, Row as MySqlRow, Value};

use crate::executor::{Executor, ObservedRow};
use crate::fixture::Engine;

#[derive(Debug, thiserror::Error)]
pub enum MySqlError {
    #[error("connecting: {0}")]
    Connect(#[source] mysql_async::Error),
    #[error("preparing an isolated database: {0}")]
    Isolate(#[source] mysql_async::Error),
    #[error("query failed: {0}")]
    Query(#[source] mysql_async::Error),
}

/// Distinguishes MySQL from MariaDB at the type level so
/// `MySqlExecutor<Mysql>` and `MySqlExecutor<Mariadb>` report different
/// [`Executor::ENGINE`] consts while sharing one driver implementation --
/// the wire protocol and client library are the same for both.
pub trait EngineMarker {
    const ENGINE: Engine;
}

pub struct Mysql;
impl EngineMarker for Mysql {
    const ENGINE: Engine = Engine::Mysql;
}

pub struct Mariadb;
impl EngineMarker for Mariadb {
    const ENGINE: Engine = Engine::Mariadb;
}

pub struct MySqlExecutor<E: EngineMarker> {
    conn: Conn,
    _marker: std::marker::PhantomData<fn() -> E>,
}

impl<E: EngineMarker> MySqlExecutor<E> {
    /// Connect using `admin_url` and isolate all subsequent operations
    /// inside a fresh database of its own, named by
    /// [`super::unique_namespace`] -- see there for why that name must not
    /// be a fixed one.
    ///
    /// `admin_url` must have `CREATE DATABASE` privileges -- the per-app
    /// `scythe` user the containers provision is scoped to only its own
    /// `scythe_test` database, so this needs the root/admin credential.
    /// Isolation matters because other workstreams (e.g.
    /// `integration_tests`) may already populate `scythe_test` with tables
    /// of the same names this crate's fixtures use (`users`, `orders`,
    /// `tags`, `user_tags`); running against it directly would corrupt or
    /// be corrupted by unrelated concurrent test data.
    pub async fn connect(admin_url: &str) -> Result<Self, MySqlError> {
        let opts = Opts::from_url(admin_url).map_err(|source| MySqlError::Connect(source.into()))?;
        let mut conn = Conn::new(opts).await.map_err(MySqlError::Connect)?;

        let database = super::unique_namespace();
        conn.query_drop(format!("DROP DATABASE IF EXISTS {database}"))
            .await
            .map_err(MySqlError::Isolate)?;
        conn.query_drop(format!("CREATE DATABASE {database}"))
            .await
            .map_err(MySqlError::Isolate)?;
        conn.query_drop(format!("USE {database}"))
            .await
            .map_err(MySqlError::Isolate)?;

        Ok(Self {
            conn,
            _marker: std::marker::PhantomData,
        })
    }
}

impl<E: EngineMarker + Send> Executor for MySqlExecutor<E> {
    const ENGINE: Engine = E::ENGINE;
    type Error = MySqlError;

    async fn seed_schema(&mut self, schema_sql: &str) -> Result<(), Self::Error> {
        // Unlike Postgres's simple-query protocol or SQLite's
        // `execute_batch`, `mysql_async` does not execute multiple
        // statements in one round trip by default, so each `CREATE TABLE`
        // must be sent individually.
        for statement in super::split_statements(schema_sql) {
            self.conn.query_drop(statement).await.map_err(MySqlError::Query)?;
        }
        Ok(())
    }

    async fn seed_run(&mut self, statements: &[String]) -> Result<(), Self::Error> {
        for statement in statements {
            self.conn
                .query_drop(statement.as_str())
                .await
                .map_err(MySqlError::Query)?;
        }
        Ok(())
    }

    async fn query_nullness(&mut self, query_sql: &str) -> Result<Vec<ObservedRow>, Self::Error> {
        let rows: Vec<MySqlRow> = self.conn.query(query_sql).await.map_err(MySqlError::Query)?;

        let mut observed = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut columns = Vec::with_capacity(row.len());
            for (idx, column) in row.columns_ref().iter().enumerate() {
                let is_null = matches!(row.as_ref(idx), Some(Value::NULL));
                columns.push((column.name_str().into_owned(), is_null));
            }
            observed.push(ObservedRow::new(columns));
        }
        Ok(observed)
    }
}
