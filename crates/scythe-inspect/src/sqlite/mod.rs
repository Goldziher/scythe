//! SQLite catalog introspection — reads a SQLite database's schema into the
//! neutral [`SchemaDescription`](crate::schema_diff::SchemaDescription)
//! catalog shape via [`SchemaCatalogDriver`].
//!
//! SQLite needs no server: [`SqliteCatalogSource::connect`] opens a file (or
//! `:memory:`) directly through `rusqlite`, which is what makes this engine
//! testable in-process with no live-database gating — see
//! `tests/sqlite_catalog.rs`.
//!
//! ## Scope
//!
//! SQLite has exactly one implicit schema per connection (`main`, plus
//! whatever else was `ATTACH`ed, which this reads nothing about); the
//! `declared_schemas` parameter [`SchemaCatalogDriver::fetch_schema`] takes
//! for multi-schema engines has nothing to do here and is ignored.
//!
//! ## What this does not do
//!
//! This crate's `SC-INS` health checks ([`DbDriver`](crate::driver::DbDriver))
//! are not implemented for SQLite — see `crate::unsupported` and the crate
//! root docs. This module only answers "what tables and columns does this
//! database have", not "does this database have any operational problems".

pub mod types;

use async_trait::async_trait;
use rusqlite::Connection;

use crate::error::InspectError;
use crate::schema_diff::model::{ColumnDescription, SchemaDescription, TableDescription, object_key};
use crate::schema_diff::source::SchemaCatalogDriver;

use types::neutral_type_for_sqlite;

/// Relation types [`SqliteCatalogSource`] reads out of `sqlite_master`.
const RELATION_TYPES: [&str; 2] = ["table", "view"];

/// A [`SchemaCatalogDriver`] backed by a `rusqlite::Connection`.
///
/// Construction is infallible and connectionless, matching every other
/// driver in this crate: call [`connect`](SchemaCatalogDriver::connect)
/// before [`fetch_schema`](SchemaCatalogDriver::fetch_schema).
pub struct SqliteCatalogSource {
    connection: Option<Connection>,
}

impl SqliteCatalogSource {
    /// Build an unconnected driver.
    pub fn new() -> Self {
        Self { connection: None }
    }
}

impl Default for SqliteCatalogSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SchemaCatalogDriver for SqliteCatalogSource {
    fn engine(&self) -> &'static str {
        "sqlite"
    }

    /// Open `url` as a SQLite database file. `rusqlite::Connection::open`
    /// treats the literal string `:memory:` as a private in-memory database,
    /// so passing that through covers the in-process test case with no
    /// special-casing here.
    async fn connect(&mut self, url: &str) -> Result<(), InspectError> {
        let connection = Connection::open(url).map_err(|e| InspectError::Connect {
            engine: "sqlite",
            source: Box::new(e),
        })?;
        self.connection = Some(connection);
        Ok(())
    }

    async fn fetch_schema(&mut self, _declared_schemas: &[String]) -> Result<SchemaDescription, InspectError> {
        let connection = self
            .connection
            .as_ref()
            .ok_or(InspectError::NotConnected { engine: "sqlite" })?;

        let mut description = SchemaDescription::new();

        for (name, relation_type) in list_relations(connection)? {
            let mut table = TableDescription::new(name.clone());
            if relation_type != "table" {
                // A SQLite view's columns come from whatever expression
                // defines them; `PRAGMA table_info` reports `notnull = 0`
                // for every one of them regardless of the underlying
                // table, exactly the reason PostgreSQL views get the same
                // treatment in `schema_diff::live`.
                table = table.without_authoritative_nullability();
            }

            for column in fetch_columns(connection, &name)? {
                table = table.with_column(column);
            }

            description.tables.insert(object_key(&name), table);
        }

        Ok(description)
    }
}

/// Quote a SQLite identifier for interpolation into `PRAGMA table_info(...)`,
/// which does not accept a bound parameter for its table-name argument.
/// Doubling an embedded `"` is SQLite's own escape rule for a quoted
/// identifier, so a table legitimately named `"weird"" name"` still resolves
/// to itself rather than breaking out of the identifier.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn query_error(step: &str, error: rusqlite::Error) -> InspectError {
    InspectError::Query {
        engine: "sqlite",
        check_id: format!("sqlite-catalog/{step}"),
        source: Box::new(error),
    }
}

/// List every user table and view, skipping SQLite's own bookkeeping tables
/// (`sqlite_sequence`, `sqlite_stat1`, …), which are never part of a user's
/// schema.
fn list_relations(connection: &Connection) -> Result<Vec<(String, String)>, InspectError> {
    let type_list = RELATION_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT name, type FROM sqlite_master \
         WHERE type IN ({type_list}) AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
         ORDER BY name"
    );

    let mut statement = connection.prepare(&sql).map_err(|e| query_error("sqlite_master", e))?;
    let rows = statement
        .query_map([], |row| {
            let name: String = row.get("name")?;
            let relation_type: String = row.get("type")?;
            Ok((name, relation_type))
        })
        .map_err(|e| query_error("sqlite_master", e))?;

    let mut relations = Vec::new();
    for row in rows {
        relations.push(row.map_err(|e| query_error("sqlite_master", e))?);
    }
    Ok(relations)
}

/// Read `PRAGMA table_info(table)` into [`ColumnDescription`]s, in column
/// declaration order.
fn fetch_columns(connection: &Connection, table: &str) -> Result<Vec<ColumnDescription>, InspectError> {
    let sql = format!("PRAGMA table_info({})", quote_ident(table));
    let mut statement = connection.prepare(&sql).map_err(|e| query_error(table, e))?;

    let rows = statement
        .query_map([], |row| {
            let name: String = row.get("name")?;
            let declared_type: String = row.get("type")?;
            let not_null: i64 = row.get("notnull")?;
            // `pk` is the column's 1-based ordinal position within the
            // primary key (0 when the column is not part of it), which is
            // exactly the "is this column part of the primary key" fact this
            // catalog needs — the ordinal order itself is not carried
            // through, matching how PostgreSQL's `attnotnull` is read as a
            // boolean rather than as a constraint definition.
            let pk: i64 = row.get("pk")?;
            Ok((name, declared_type, not_null != 0, pk != 0))
        })
        .map_err(|e| query_error(table, e))?;

    let mut raw: Vec<(String, String, bool, bool)> = Vec::new();
    for row in rows {
        raw.push(row.map_err(|e| query_error(table, e))?);
    }

    // The rowid alias is NOT NULL even though `PRAGMA table_info` says
    // `notnull = 0`. Verified on SQLite 3.50.6: for `id INTEGER PRIMARY KEY`
    // the pragma reports `notnull = 0`, and `INSERT INTO t (id) VALUES (NULL)`
    // succeeds -- but it succeeds by *auto-assigning the rowid*, so a `SELECT`
    // can never hand back NULL for that column.
    //
    // Reading `notnull` faithfully here would therefore disagree with the DDL
    // side, which already resolves this via
    // `scythe_core::catalog::sqlite_primary_key_forces_not_null` (issue #108,
    // derived against the same SQLite version). Since `schema_diff` compares
    // this live description against that declared one, the disagreement would
    // not be a harmless difference of opinion -- it would report phantom
    // `SC-DRF` drift on every SQLite table with a rowid primary key, i.e. on
    // essentially all of them.
    //
    // The rule matches the DDL side exactly: single-column primary key whose
    // *raw declared type* is exactly `INTEGER`. `INT`, `INTEGER(11)`, `INT4`
    // and `BIGINT` do not qualify; only that spelling makes a rowid alias.
    let primary_key_column_count = raw.iter().filter(|(_, _, _, is_pk)| *is_pk).count();

    let mut columns = Vec::new();
    for (name, declared_type, not_null, is_primary_key) in raw {
        let is_rowid_alias =
            is_primary_key && primary_key_column_count == 1 && declared_type.trim().eq_ignore_ascii_case("integer");
        let neutral_type = neutral_type_for_sqlite(&declared_type);
        let mut column = ColumnDescription::new(name, neutral_type, !(not_null || is_rowid_alias));
        if is_primary_key {
            column = column.as_primary_key();
        }
        columns.push(column);
    }
    Ok(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn source_for(ddl: &str) -> SqliteCatalogSource {
        let mut source = SqliteCatalogSource::new();
        source.connect(":memory:").await.expect("connect");
        {
            let connection = source.connection.as_ref().expect("connected");
            connection.execute_batch(ddl).expect("seed ddl");
        }
        source
    }

    #[test]
    fn engine_name_is_sqlite() {
        assert_eq!(SqliteCatalogSource::new().engine(), "sqlite");
    }

    #[tokio::test]
    async fn fetch_schema_without_connect_errors() {
        let mut source = SqliteCatalogSource::new();
        let error = source.fetch_schema(&[]).await.unwrap_err();
        assert!(matches!(error, InspectError::NotConnected { engine: "sqlite" }));
    }

    #[tokio::test]
    async fn describes_columns_with_neutral_types_and_nullability() {
        let mut source = source_for(
            "CREATE TABLE users (
                 id    INTEGER PRIMARY KEY,
                 email TEXT NOT NULL,
                 bio   TEXT
             );",
        )
        .await;

        let description = source.fetch_schema(&[]).await.expect("fetch schema");
        let users = &description.tables["users"];

        assert_eq!(users.columns["id"].neutral_type.as_deref(), Some("int64"));
        assert!(users.columns["id"].primary_key);
        assert!(!users.columns["email"].nullable);
        assert!(!users.columns["email"].primary_key);
        assert!(users.columns["bio"].nullable);
    }

    #[tokio::test]
    async fn a_composite_primary_key_marks_every_participating_column() {
        let mut source = source_for(
            "CREATE TABLE membership (
                 org_id  INTEGER NOT NULL,
                 user_id INTEGER NOT NULL,
                 role    TEXT NOT NULL,
                 PRIMARY KEY (org_id, user_id)
             );",
        )
        .await;

        let description = source.fetch_schema(&[]).await.expect("fetch schema");
        let membership = &description.tables["membership"];

        assert!(membership.columns["org_id"].primary_key);
        assert!(membership.columns["user_id"].primary_key);
        assert!(!membership.columns["role"].primary_key);
    }

    #[tokio::test]
    async fn views_are_included_without_authoritative_nullability() {
        let mut source = source_for(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, active INTEGER NOT NULL);
             CREATE VIEW active_users AS SELECT id, active FROM users;",
        )
        .await;

        let description = source.fetch_schema(&[]).await.expect("fetch schema");
        assert!(description.tables.contains_key("active_users"));
        assert!(!description.tables["active_users"].nullability_is_authoritative);
        assert!(description.tables["users"].nullability_is_authoritative);
    }

    #[tokio::test]
    async fn sqlite_bookkeeping_tables_are_excluded() {
        // `sqlite_sequence` is created automatically by an
        // `AUTOINCREMENT` column and must never appear as a user table.
        let mut source = source_for("CREATE TABLE counted (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT);").await;

        let description = source.fetch_schema(&[]).await.expect("fetch schema");
        assert!(!description.tables.contains_key("sequence"));
        assert!(
            !description.tables.keys().any(|k| k.starts_with("sqlite_")),
            "{:?}",
            description.tables.keys().collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn an_empty_database_describes_an_empty_schema() {
        let mut source = source_for("").await;
        let description = source.fetch_schema(&[]).await.expect("fetch schema");
        assert!(description.tables.is_empty());
    }

    #[tokio::test]
    async fn a_table_with_no_declared_column_type_maps_to_bytes() {
        let mut source = source_for("CREATE TABLE loose (id, value);").await;
        let description = source.fetch_schema(&[]).await.expect("fetch schema");
        assert_eq!(
            description.tables["loose"].columns["value"].neutral_type.as_deref(),
            Some("bytes")
        );
    }
}
