//! Live-PG tests for schema drift — only run when the `live-tests` feature is
//! enabled AND `$SCYTHE_TEST_DATABASE_URL` is set, matching `pg_live.rs` and
//! `verify_live.rs`.
//!
//! The rules themselves are covered exhaustively by unit tests in
//! `schema_diff::diff`, which need no database. What cannot be tested without
//! a server is the fetch layer: that `pg_catalog` is read correctly, that a
//! view is not mistaken for a missing table, and — the case the whole feature
//! exists for — that nullability drift is visible here when
//! [`verify_queries`] is structurally incapable of seeing it.
//!
//! Each test owns a private schema and points `search_path` at it, so the
//! fetch layer sees that schema and nothing else. That makes these tests
//! independent even though the CI workflow runs them serially.

#![cfg(feature = "live-tests")]

use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;
use scythe_inspect::schema_diff::{DriftSeverities, describe_catalog, diff_schemas, fetch_live_schema};
use scythe_lint::reporters::Finding;
use tokio_postgres::{Client, NoTls};

fn url() -> String {
    std::env::var("SCYTHE_TEST_DATABASE_URL").expect(
        "SCYTHE_TEST_DATABASE_URL must be set for live-tests \
         (e.g. postgres://scythe:scythe@localhost:5432/scythe_inspect_test)",
    )
}

/// Connect, then create `schema` fresh and make it the only schema on the
/// connection's `search_path`.
///
/// Isolating by `search_path` rather than by name filtering is what keeps
/// these tests independent: `fetch_live_schema` scopes itself to
/// `current_schemas(false)`, so another test's fixture is invisible here.
async fn client_with_schema(schema: &str, ddl: &str) -> Client {
    let (client, connection) = tokio_postgres::connect(&url(), NoTls)
        .await
        .expect("test setup: connect");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE; \
             CREATE SCHEMA {schema}; \
             SET search_path TO {schema}; \
             {ddl}"
        ))
        .await
        .expect("test setup: schema fixture");

    client
}

/// Compare `ddl` against whatever the connection's `search_path` schema holds.
async fn drift_for(client: &Client, ddl: &str) -> Vec<Finding> {
    let catalog = Catalog::from_ddl_with_dialect(&[ddl], &SqlDialect::PostgreSQL).expect("catalog from ddl");
    let live = fetch_live_schema(client).await.expect("fetch live schema");

    let ddl_schema = describe_catalog(&catalog).expect("describe catalog");

    diff_schemas(&ddl_schema, &live, &DriftSeverities::default(), "test")
}

fn rule_ids(findings: &[Finding]) -> Vec<&str> {
    findings.iter().map(|f| f.rule_id.as_str()).collect()
}

#[tokio::test]
async fn silent_when_the_database_matches_the_ddl() {
    let ddl = "CREATE TABLE users (
                   id      integer PRIMARY KEY,
                   email   text NOT NULL,
                   bio     text,
                   created timestamptz NOT NULL
               );";
    let client = client_with_schema("drift_match", ddl).await;

    let findings = drift_for(&client, ddl).await;
    assert!(
        findings.is_empty(),
        "a schema in sync with itself must produce no findings: {:?}",
        findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}

/// SC-DRF05 compares neutral types for exact equality, which only works
/// because both sides normalise the same way — `serial` and `int4` both reach
/// `int32`, `varchar(n)`/`char(n)`/`text` all reach `string`, a domain
/// resolves to its base type, and an enum reaches `enum::name` from the DDL
/// and from `pg_type` alike.
///
/// This is the guard on that claim. Every type below is declared identically
/// on both sides, so a single finding here means exact matching has turned an
/// ordinary schema into noise — the failure mode that would make users
/// disable the whole check.
#[tokio::test]
async fn exact_type_matching_stays_silent_across_the_common_type_surface() {
    let ddl = "CREATE TYPE mood AS ENUM ('happy', 'sad');
               CREATE DOMAIN us_zip AS text;
               CREATE TABLE wide (
                   c_serial      serial,
                   c_bigserial   bigserial,
                   c_smallint    smallint,
                   c_integer     integer,
                   c_bigint      bigint,
                   c_real        real,
                   c_double      double precision,
                   c_numeric     numeric(10,2),
                   c_bool        boolean,
                   c_text        text,
                   c_varchar     varchar(50),
                   c_char        char(10),
                   c_uuid        uuid,
                   c_bytea       bytea,
                   c_date        date,
                   c_time        time,
                   c_timestamp   timestamp,
                   c_timestamptz timestamptz,
                   c_interval    interval,
                   c_json        json,
                   c_jsonb       jsonb,
                   c_inet        inet,
                   c_text_array  text[],
                   c_int_array   integer[],
                   c_enum        mood,
                   c_domain      us_zip
               );";
    let client = client_with_schema("drift_type_surface", ddl).await;

    let findings = drift_for(&client, ddl).await;
    assert!(
        findings.is_empty(),
        "exact type matching must not fire on a schema that is in sync: {:?}",
        findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}

/// The migration `types_are_compatible` waves through and drift must not.
/// That predicate lets an *inferred* `string` widen to `uuid`, because static
/// query inference genuinely cannot always pin a type down. A DDL declaration
/// is not an inference, so a column still declared `text` against a live
/// `uuid` is real drift — and this is the live proof, not just a unit test
/// over hand-built descriptions.
#[tokio::test]
async fn reports_a_text_column_the_database_migrated_to_uuid() {
    let client = client_with_schema("drift_text_to_uuid", "CREATE TABLE users (id integer, token uuid);").await;

    let findings = drift_for(&client, "CREATE TABLE users (id integer, token text);").await;

    assert_eq!(rule_ids(&findings), vec!["SC-DRF05"], "{findings:?}");
    assert!(findings[0].message.contains("token"), "{}", findings[0].message);
}

/// The same for a column converted to an enum: the DDL still says `text`.
#[tokio::test]
async fn reports_a_text_column_the_database_migrated_to_an_enum() {
    let client = client_with_schema(
        "drift_text_to_enum",
        "CREATE TYPE status AS ENUM ('active'); CREATE TABLE users (id integer, state status);",
    )
    .await;

    let findings = drift_for(&client, "CREATE TABLE users (id integer, state text);").await;

    assert_eq!(rule_ids(&findings), vec!["SC-DRF05"], "{findings:?}");
}

/// Widening a column to `bigint` leaves generated code holding an `i32` for a
/// value that no longer fits in one.
#[tokio::test]
async fn reports_an_integer_column_the_database_widened_to_bigint() {
    let client = client_with_schema("drift_int_widen", "CREATE TABLE counters (id integer, hits bigint);").await;

    let findings = drift_for(&client, "CREATE TABLE counters (id integer, hits integer);").await;

    assert_eq!(rule_ids(&findings), vec!["SC-DRF05"], "{findings:?}");
}

/// The case that justifies drift checking. `verify_queries` prepares the
/// statement and the server reports type OIDs but nothing about NULL-ness, so
/// it cannot see that `email` is nullable in production. Reading
/// `pg_attribute.attnotnull` can.
#[tokio::test]
async fn reports_nullability_drift_that_query_verification_cannot_see() {
    let client = client_with_schema(
        "drift_nullability",
        "CREATE TABLE users (id integer PRIMARY KEY, email text);",
    )
    .await;

    let ddl = "CREATE TABLE users (id integer PRIMARY KEY, email text NOT NULL);";
    let findings = drift_for(&client, ddl).await;

    assert_eq!(rule_ids(&findings), vec!["SC-DRF06"], "{findings:?}");
    assert!(findings[0].message.contains("email"), "{}", findings[0].message);

    // The other half of the claim: the existing verifier is silent on exactly
    // this schema, so drift is the only thing that catches it.
    let catalog = Catalog::from_ddl_with_dialect(&[ddl], &SqlDialect::PostgreSQL).expect("catalog");
    let parsed = parse_query_with_dialect(
        "-- @name GetEmail\n-- @returns :one\nSELECT email FROM users WHERE id = $1;",
        &SqlDialect::PostgreSQL,
    )
    .expect("parse query");
    let analyzed = analyze(&catalog, &parsed).expect("analyze query");

    let verify_findings = scythe_inspect::verify_queries(&client, "test", &[analyzed]).await;
    assert!(
        verify_findings.is_empty(),
        "verify_queries is expected to be blind to nullability: {verify_findings:?}"
    );
}

#[tokio::test]
async fn reports_a_table_the_ddl_declares_and_the_database_lacks() {
    let client = client_with_schema("drift_missing_table", "CREATE TABLE users (id integer);").await;

    let findings = drift_for(
        &client,
        "CREATE TABLE users (id integer); CREATE TABLE orders (id integer);",
    )
    .await;

    assert_eq!(rule_ids(&findings), vec!["SC-DRF01"], "{findings:?}");
    assert!(findings[0].message.contains("orders"), "{}", findings[0].message);
}

/// A migrations ledger is the canonical instance of this, and it must warn
/// rather than fail the run.
#[tokio::test]
async fn warns_about_a_table_only_the_database_has() {
    let client = client_with_schema(
        "drift_extra_table",
        "CREATE TABLE users (id integer); CREATE TABLE schema_migrations (version text);",
    )
    .await;

    let findings = drift_for(&client, "CREATE TABLE users (id integer);").await;

    assert_eq!(rule_ids(&findings), vec!["SC-DRF02"], "{findings:?}");
    assert_eq!(findings[0].severity, scythe_lint::types::Severity::Warn);
}

/// Enum drift is the reason the fetch layer reads `pg_catalog` rather than
/// `information_schema`, which reports `USER-DEFINED` for every enum column
/// and never names the type.
#[tokio::test]
async fn reports_enum_value_drift() {
    let client = client_with_schema(
        "drift_enum",
        "CREATE TYPE status AS ENUM ('active', 'banned', 'pending'); \
         CREATE TABLE users (id integer, state status NOT NULL);",
    )
    .await;

    let findings = drift_for(
        &client,
        "CREATE TYPE status AS ENUM ('active', 'banned'); \
         CREATE TABLE users (id integer, state status NOT NULL);",
    )
    .await;

    assert_eq!(rule_ids(&findings), vec!["SC-DRF07"], "{findings:?}");
    assert!(findings[0].message.contains("pending"), "{}", findings[0].message);
}

/// Scythe's catalog stores views alongside tables, so a live query restricted
/// to `relkind = 'r'` would report every view as a missing table.
#[tokio::test]
async fn a_view_is_not_reported_as_a_missing_table() {
    let ddl = "CREATE TABLE users (id integer, active boolean NOT NULL); \
               CREATE VIEW active_users AS SELECT id, active FROM users;";
    let client = client_with_schema("drift_view", ddl).await;

    let findings = drift_for(&client, ddl).await;

    assert!(
        findings.is_empty(),
        "a view present on both sides must produce no findings, \
         including no nullability drift from PostgreSQL reporting every view column as nullable: {:?}",
        findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}

/// A type outside scythe's neutral vocabulary must skip the type comparison
/// rather than report drift on a schema that is perfectly in sync.
#[tokio::test]
async fn an_unmappable_column_type_is_skipped_rather_than_reported() {
    let ddl = "CREATE TABLE docs (id integer, body xml);";
    let client = client_with_schema("drift_unmappable", ddl).await;

    let findings = drift_for(&client, ddl).await;

    assert!(
        findings.is_empty(),
        "an `xml` column scythe cannot map is not evidence of drift: {:?}",
        findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}
