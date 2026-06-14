//! Live-PG integration tests — only run when the `live-tests` feature is
//! enabled AND `$SCYTHE_TEST_DATABASE_URL` is set.
//!
//! Each test creates its own schema, runs the relevant check, and drops the
//! schema on the way out, so they're safe to run against a shared PG.

#![cfg(feature = "live-tests")]

use scythe_inspect::{DbDriver, PostgresDriver};
use scythe_lint::types::Severity;
use tokio_postgres::NoTls;

fn url() -> String {
    std::env::var("SCYTHE_TEST_DATABASE_URL").expect(
        "SCYTHE_TEST_DATABASE_URL must be set for live-tests (e.g. \
         postgres://postgres:postgres@localhost/postgres)",
    )
}

/// Connect a raw tokio-postgres client for setup/teardown SQL — we don't
/// route fixture DDL through the driver to keep the test isolated from the
/// rules under test.
async fn raw_client() -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(&url(), NoTls)
        .await
        .expect("test setup: connect");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

/// Run the entire scythe-inspect Postgres pipeline against the live DB.
async fn run_all_findings() -> Vec<scythe_lint::reporters::Finding> {
    let mut driver = PostgresDriver::new();
    driver.connect(&url()).await.expect("driver connect");
    driver.run_all().await.expect("driver run_all")
}

#[tokio::test]
async fn sc_ins01_fires_on_fk_without_covering_index() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins01_fixture CASCADE;
        CREATE SCHEMA sc_ins01_fixture;
        CREATE TABLE sc_ins01_fixture.users (id bigint PRIMARY KEY);
        CREATE TABLE sc_ins01_fixture.orders (
            id bigint PRIMARY KEY,
            user_id bigint REFERENCES sc_ins01_fixture.users(id)
        );
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS01" && f.message.contains("sc_ins01_fixture.orders"));
    assert!(
        hit.is_some(),
        "expected SC-INS01 finding on orders.user_id, got {:?}",
        findings
    );
    assert_eq!(hit.unwrap().severity, Severity::Warn);

    client
        .batch_execute("DROP SCHEMA sc_ins01_fixture CASCADE")
        .await
        .ok();
}

#[tokio::test]
async fn sc_ins02_fires_on_policy_with_rls_disabled() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins02_fixture CASCADE;
        CREATE SCHEMA sc_ins02_fixture;
        CREATE TABLE sc_ins02_fixture.tenants (id bigint PRIMARY KEY, owner_id bigint);
        ALTER TABLE sc_ins02_fixture.tenants ENABLE ROW LEVEL SECURITY;
        CREATE POLICY tenant_iso ON sc_ins02_fixture.tenants USING (owner_id > 0);
        ALTER TABLE sc_ins02_fixture.tenants DISABLE ROW LEVEL SECURITY;
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS02" && f.message.contains("sc_ins02_fixture.tenants"));
    assert!(
        hit.is_some(),
        "expected SC-INS02 finding on tenants, got {:?}",
        findings
    );
    assert_eq!(hit.unwrap().severity, Severity::Error);

    client
        .batch_execute("DROP SCHEMA sc_ins02_fixture CASCADE")
        .await
        .ok();
}

#[tokio::test]
async fn sc_ins03_fires_on_duplicate_index() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins03_fixture CASCADE;
        CREATE SCHEMA sc_ins03_fixture;
        CREATE TABLE sc_ins03_fixture.events (id bigint PRIMARY KEY, kind text);
        CREATE INDEX events_kind_a ON sc_ins03_fixture.events (kind);
        CREATE INDEX events_kind_b ON sc_ins03_fixture.events (kind);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS03" && f.message.contains("sc_ins03_fixture.events"));
    assert!(
        hit.is_some(),
        "expected SC-INS03 finding on events, got {:?}",
        findings
    );
    assert_eq!(hit.unwrap().severity, Severity::Warn);

    client
        .batch_execute("DROP SCHEMA sc_ins03_fixture CASCADE")
        .await
        .ok();
}
