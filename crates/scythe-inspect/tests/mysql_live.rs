//! Live-MySQL catalog-introspection tests — only run when the `live-tests`
//! feature is enabled AND `$SCYTHE_TEST_MYSQL_DATABASE_URL` is set, exactly
//! matching how `pg_live.rs`, `schema_diff_live.rs` and `verify_live.rs` gate
//! their PostgreSQL equivalents in this same directory.
//!
//! Each test creates its own uniquely-named database, points
//! `MySqlCatalogSource` at it via `USE`, and drops the database on the way
//! out. Tests are independent and may run concurrently: a unique database
//! name per test is what keeps one test's fixture invisible to another's
//! `fetch_schema` — the same isolation strategy
//! `scythe-conformance`'s `MySqlExecutor` uses for the same reason (see
//! `crates/scythe-conformance/src/executors/mysql.rs`).

#![cfg(feature = "live-tests")]

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts};
use scythe_inspect::MySqlCatalogSource;
use scythe_inspect::schema_diff::SchemaCatalogDriver;

fn admin_url() -> String {
    std::env::var("SCYTHE_TEST_MYSQL_DATABASE_URL").expect(
        "SCYTHE_TEST_MYSQL_DATABASE_URL must be set for live-tests \
         (e.g. mysql://root:scythe@127.0.0.1:3306/), an account with CREATE DATABASE privileges",
    )
}

/// A short, likely-unique database name for one test's fixture.
///
/// Seeded from the current time rather than a fixed name so concurrently
/// running tests (the default `cargo test` behaviour) never collide on the
/// same database.
fn unique_database() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    format!("scythe_inspect_test_{nanos}")
}

/// Connect with admin privileges, create a fresh database, seed it with
/// `ddl`, then return the database name and a `MySqlCatalogSource` connected
/// to (and scoped to, via `USE`) that database.
async fn source_for(ddl: &str) -> (String, MySqlCatalogSource) {
    let opts = Opts::from_url(&admin_url()).expect("parse SCYTHE_TEST_MYSQL_DATABASE_URL");
    let mut admin = Conn::new(opts).await.expect("connect with admin credentials");

    let database = unique_database();
    admin
        .query_drop(format!("CREATE DATABASE {database}"))
        .await
        .expect("create test database");
    admin
        .query_drop(format!("USE {database}"))
        .await
        .expect("select test database");
    if !ddl.trim().is_empty() {
        for statement in ddl.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            admin.query_drop(statement).await.expect("seed ddl");
        }
    }
    drop(admin);

    let mut url = admin_url();
    if !url.ends_with('/') {
        url.push('/');
    }
    url.push_str(&database);

    let mut source = MySqlCatalogSource::new();
    source.connect(&url).await.expect("connect scoped to test database");

    (database, source)
}

async fn drop_database(database: &str) {
    let opts = Opts::from_url(&admin_url()).expect("parse SCYTHE_TEST_MYSQL_DATABASE_URL");
    let mut admin = Conn::new(opts).await.expect("connect with admin credentials");
    admin
        .query_drop(format!("DROP DATABASE IF EXISTS {database}"))
        .await
        .expect("drop test database");
}

#[tokio::test]
async fn engine_name_is_mysql() {
    assert_eq!(MySqlCatalogSource::new().engine(), "mysql");
}

#[tokio::test]
async fn describes_columns_with_neutral_types_nullability_and_primary_key() {
    let (database, mut source) = source_for(
        "CREATE TABLE users (
             id    INT PRIMARY KEY AUTO_INCREMENT,
             email VARCHAR(255) NOT NULL,
             bio   TEXT,
             active TINYINT(1) NOT NULL DEFAULT 1
         )",
    )
    .await;

    let description = source.fetch_schema(&[]).await.expect("fetch schema");
    let users = &description.tables["users"];

    let id = &users.columns["id"];
    assert_eq!(id.neutral_type.as_deref(), Some("int32"));
    assert!(id.primary_key);
    assert!(!id.nullable);

    let email = &users.columns["email"];
    assert_eq!(email.neutral_type.as_deref(), Some("string"));
    assert!(!email.nullable);
    assert!(!email.primary_key);

    let bio = &users.columns["bio"];
    assert!(bio.nullable);

    let active = &users.columns["active"];
    assert_eq!(
        active.neutral_type.as_deref(),
        Some("bool"),
        "TINYINT(1) must map to bool"
    );

    drop_database(&database).await;
}

#[tokio::test]
async fn a_composite_primary_key_marks_every_participating_column() {
    let (database, mut source) = source_for(
        "CREATE TABLE membership (
             org_id  INT NOT NULL,
             user_id INT NOT NULL,
             role    VARCHAR(32) NOT NULL,
             PRIMARY KEY (org_id, user_id)
         )",
    )
    .await;

    let description = source.fetch_schema(&[]).await.expect("fetch schema");
    let membership = &description.tables["membership"];

    assert!(membership.columns["org_id"].primary_key);
    assert!(membership.columns["user_id"].primary_key);
    assert!(!membership.columns["role"].primary_key);

    drop_database(&database).await;
}

#[tokio::test]
async fn a_view_is_included_without_authoritative_nullability() {
    let (database, mut source) = source_for(
        "CREATE TABLE users (id INT PRIMARY KEY, active TINYINT(1) NOT NULL);
         CREATE VIEW active_users AS SELECT id, active FROM users WHERE active = 1",
    )
    .await;

    let description = source.fetch_schema(&[]).await.expect("fetch schema");

    assert!(description.tables.contains_key("active_users"));
    assert!(!description.tables["active_users"].nullability_is_authoritative);
    assert!(description.tables["users"].nullability_is_authoritative);

    drop_database(&database).await;
}

#[tokio::test]
async fn an_empty_database_describes_an_empty_schema() {
    let (database, mut source) = source_for("").await;
    let description = source.fetch_schema(&[]).await.expect("fetch schema");
    assert!(description.tables.is_empty());
    drop_database(&database).await;
}

#[tokio::test]
async fn fetch_schema_without_connect_errors() {
    let mut source = MySqlCatalogSource::new();
    let error = source.fetch_schema(&[]).await.unwrap_err();
    assert!(matches!(
        error,
        scythe_inspect::InspectError::NotConnected { engine: "mysql" }
    ));
}
