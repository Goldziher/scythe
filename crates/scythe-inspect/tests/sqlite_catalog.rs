//! In-process SQLite catalog-introspection tests, exercised through the
//! crate's public API — no server, no live-test gating.
//!
//! Unlike the PostgreSQL and MySQL live-test files in this directory,
//! `SqliteCatalogSource::connect` opens a private database file directly, so
//! these run as part of the ordinary `cargo test`. `src/sqlite/mod.rs` has
//! its own unit tests covering the same fetch logic against `:memory:`
//! databases it can seed directly (having access to the private
//! `rusqlite::Connection` field); this file instead seeds a temp-file
//! database through a *separate* `rusqlite` connection and reads it back
//! purely through `SqliteCatalogSource`'s public surface, proving the two
//! connections agree on what is in the file.

use scythe_inspect::schema_diff::SchemaCatalogDriver;
use scythe_inspect::sqlite::SqliteCatalogSource;

/// Seed a temp-file SQLite database with `ddl`, then introspect it through
/// `SqliteCatalogSource`'s public `connect`/`fetch_schema`.
async fn introspect(ddl: &str) -> scythe_inspect::schema_diff::SchemaDescription {
    let file = tempfile::NamedTempFile::new().expect("create temp sqlite file");
    let path = file.path().to_str().expect("temp path is valid utf-8").to_string();

    {
        let seed_connection = rusqlite::Connection::open(&path).expect("open temp db for seeding");
        seed_connection.execute_batch(ddl).expect("seed ddl");
    }

    let mut source = SqliteCatalogSource::new();
    source.connect(&path).await.expect("connect to sqlite file");
    source.fetch_schema(&[]).await.expect("fetch schema")
}

#[test]
fn engine_name_is_sqlite() {
    assert_eq!(SqliteCatalogSource::new().engine(), "sqlite");
}

#[tokio::test]
async fn an_empty_database_introspects_as_an_empty_catalog() {
    let description = introspect("").await;
    assert!(description.tables.is_empty());
    assert!(description.enums.is_empty());
}

#[tokio::test]
async fn describes_a_table_s_columns_types_nullability_and_primary_key() {
    let description = introspect(
        "CREATE TABLE users (
             id    INTEGER PRIMARY KEY,
             email TEXT NOT NULL,
             age   INTEGER,
             score REAL
         );",
    )
    .await;

    let users = &description.tables["users"];

    let id = &users.columns["id"];
    assert_eq!(id.neutral_type.as_deref(), Some("int64"));
    assert!(id.primary_key);
    assert!(
        !id.nullable,
        "an INTEGER PRIMARY KEY column is the rowid alias and is NOT NULL"
    );

    let email = &users.columns["email"];
    assert_eq!(email.neutral_type.as_deref(), Some("string"));
    assert!(!email.nullable);
    assert!(!email.primary_key);

    let age = &users.columns["age"];
    assert!(age.nullable);

    let score = &users.columns["score"];
    assert_eq!(score.neutral_type.as_deref(), Some("float64"));
}

#[tokio::test]
async fn a_view_s_nullability_is_not_authoritative() {
    let description = introspect(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, active INTEGER NOT NULL);
         CREATE VIEW active_users AS SELECT id, active FROM users;",
    )
    .await;

    assert!(description.tables["users"].nullability_is_authoritative);
    assert!(!description.tables["active_users"].nullability_is_authoritative);
}
