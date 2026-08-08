//! SQLite live driver, behind the `sqlite` feature.
//!
//! Every [`SqliteExecutor`] opens a private, in-process, in-memory
//! database. Unlike the network engines, SQLite needs no dedicated
//! schema/database isolation scheme to avoid colliding with other
//! workstreams sharing a container: an in-memory connection is never
//! visible to any other process by construction.

use rusqlite::Connection;
use rusqlite::types::ValueRef;

use crate::executor::{Executor, ObservedRow};
use crate::fixture::Engine;

#[derive(Debug, thiserror::Error)]
#[error("sqlite error: {0}")]
pub struct SqliteError(#[from] rusqlite::Error);

pub struct SqliteExecutor {
    connection: Connection,
}

impl SqliteExecutor {
    /// Open a fresh, private in-memory database.
    pub fn open_in_memory() -> Result<Self, SqliteError> {
        let connection = Connection::open_in_memory()?;
        Ok(Self { connection })
    }
}

impl Executor for SqliteExecutor {
    const ENGINE: Engine = Engine::Sqlite;
    type Error = SqliteError;

    async fn seed_schema(&mut self, schema_sql: &str) -> Result<(), Self::Error> {
        // `execute_batch` accepts multiple `;`-separated statements in one
        // call -- no manual statement-splitting needed for SQLite.
        self.connection.execute_batch(schema_sql)?;
        Ok(())
    }

    async fn seed_run(&mut self, statements: &[String]) -> Result<(), Self::Error> {
        for statement in statements {
            self.connection.execute(statement, [])?;
        }
        Ok(())
    }

    async fn query_nullness(&mut self, query_sql: &str) -> Result<Vec<ObservedRow>, Self::Error> {
        let mut statement = self.connection.prepare(query_sql)?;
        // Column names must be read before `.query()` borrows `statement`
        // mutably for the lifetime of the row iterator.
        let column_names: Vec<String> = statement.column_names().into_iter().map(str::to_string).collect();

        let mut rows = statement.query([])?;
        let mut observed = Vec::new();
        while let Some(row) = rows.next()? {
            let mut columns = Vec::with_capacity(column_names.len());
            for (idx, name) in column_names.iter().enumerate() {
                let is_null = matches!(row.get_ref(idx)?, ValueRef::Null);
                columns.push((name.clone(), is_null));
            }
            observed.push(ObservedRow::new(columns));
        }
        Ok(observed)
    }
}
