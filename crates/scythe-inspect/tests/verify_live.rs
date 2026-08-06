//! Live-PG tests for `verify_queries` — only run when the `live-tests` feature
//! is enabled AND `$SCYTHE_TEST_DATABASE_URL` is set.
//!
//! These assert the two properties that matter for a verification pass:
//! it must stay silent when inference is right (no false positives, or the
//! signal is worthless), and it must fire when inference drifts from what the
//! server actually reports.
//!
//! Each test creates its own schema and drops it on the way out.  Unlike
//! `pg_live.rs` these are genuinely independent — a prepared statement only
//! sees the objects its own query names — so they are safe to run in parallel.

#![cfg(feature = "live-tests")]

use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;
use scythe_inspect::verify_queries;
use tokio_postgres::{Client, NoTls};

fn url() -> String {
    std::env::var("SCYTHE_TEST_DATABASE_URL").expect(
        "SCYTHE_TEST_DATABASE_URL must be set for live-tests \
         (e.g. postgres://scythe:scythe@localhost:5432/scythe_inspect_test)",
    )
}

async fn client() -> Client {
    let (client, connection) = tokio_postgres::connect(&url(), NoTls)
        .await
        .expect("test setup: connect");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

/// Analyze `query` against `ddl` exactly as `scythe check` does, then verify
/// the result against the live database.
async fn findings_for(client: &Client, ddl: &str, query: &str) -> Vec<scythe_lint::reporters::Finding> {
    let dialect = SqlDialect::PostgreSQL;
    let catalog = Catalog::from_ddl_with_dialect(&[ddl], &dialect).expect("catalog from ddl");
    let parsed = parse_query_with_dialect(query, &dialect).expect("parse query");
    let analyzed = analyze(&catalog, &parsed).expect("analyze query");

    verify_queries(client, "test", &[analyzed]).await
}

#[tokio::test]
async fn silent_when_inference_matches_the_database() {
    let client = client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS verify_ok CASCADE;
        CREATE SCHEMA verify_ok;
        CREATE TABLE verify_ok.users (
            id     integer PRIMARY KEY,
            name   text NOT NULL,
            status text NOT NULL
        );
        SET search_path TO verify_ok;
        ",
        )
        .await
        .expect("setup");

    let ddl = "CREATE TABLE users (id integer PRIMARY KEY, name text NOT NULL, status text NOT NULL);";
    let query = "-- @name FindByStatus\n-- @returns :many\nSELECT id, name FROM users WHERE status = $1;";

    let findings = findings_for(&client, ddl, query).await;

    assert!(
        findings.is_empty(),
        "correct inference must produce no findings, got: {findings:?}"
    );

    client
        .batch_execute("DROP SCHEMA IF EXISTS verify_ok CASCADE;")
        .await
        .expect("teardown");
}

/// The core promise of the feature: when the DDL disagrees with the live
/// database about a column's type, the mismatch is reported.
#[tokio::test]
async fn reports_result_column_type_mismatch() {
    let client = client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS verify_coltype CASCADE;
        CREATE SCHEMA verify_coltype;
        CREATE TABLE verify_coltype.items (id integer PRIMARY KEY, total numeric NOT NULL);
        SET search_path TO verify_coltype;
        ",
        )
        .await
        .expect("setup");

    // The DDL claims `total` is boolean; the database says numeric.
    let ddl = "CREATE TABLE items (id integer PRIMARY KEY, total boolean NOT NULL);";
    let query = "-- @name GetItems\n-- @returns :many\nSELECT id, total FROM items;";

    let findings = findings_for(&client, ddl, query).await;

    assert!(
        findings.iter().any(|f| f.rule_id == "SC-VER03"),
        "expected SC-VER03 for the boolean/numeric mismatch, got: {findings:?}"
    );

    client
        .batch_execute("DROP SCHEMA IF EXISTS verify_coltype CASCADE;")
        .await
        .expect("teardown");
}

/// Schema drift: the DDL declares a table that was never migrated. Static
/// analysis is happy because it only reads the DDL; the server is not.
#[tokio::test]
async fn reports_query_the_server_rejects() {
    let client = client().await;

    let ddl = "CREATE TABLE verify_never_migrated (id integer PRIMARY KEY, label text NOT NULL);";
    let query = "-- @name GetPending\n-- @returns :many\nSELECT id, label FROM verify_never_migrated;";

    let findings = findings_for(&client, ddl, query).await;

    let rejected = findings
        .iter()
        .find(|f| f.rule_id == "SC-VER01")
        .expect("expected SC-VER01 when the server rejects the query");

    assert!(
        rejected.message.contains("does not exist"),
        "the message must name the real cause, not a bare 'db error': {}",
        rejected.message
    );
}
