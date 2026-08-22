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
use scythe_inspect::schema_diff::{
    DriftSeverities, describe_catalog, diff_schemas, fetch_live_catalog, fetch_live_schema,
};
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
/// these tests independent: `fetch_live_schema`'s scope starts from
/// `current_schemas(false)`, so another test's fixture is invisible here
/// unless the DDL under test qualifies its objects with that schema.
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

/// Compare `ddl` against the schemas in scope: the connection's `search_path`
/// plus whatever schemas the DDL qualifies its own objects with.
async fn drift_for(client: &Client, ddl: &str) -> Vec<Finding> {
    let catalog = Catalog::from_ddl_with_dialect(&[ddl], &SqlDialect::PostgreSQL).expect("catalog from ddl");
    let ddl_schema = describe_catalog(&catalog).expect("describe catalog");
    let declared = declared_schemas(&ddl_schema);

    let live = fetch_live_schema(client, &declared).await.expect("fetch live schema");

    diff_schemas(&ddl_schema, &live, &DriftSeverities::default(), "test")
}

/// The schema qualifiers the DDL wrote, mirroring what `drift_findings` derives
/// for the CLI.
fn declared_schemas(ddl: &scythe_inspect::SchemaDescription) -> Vec<String> {
    let mut declared: Vec<String> = ddl
        .tables
        .values()
        .map(|table| table.display_name.as_str())
        .chain(ddl.enums.values().map(|enum_type| enum_type.display_name.as_str()))
        .chain(ddl.composites.values().map(|composite| composite.display_name.as_str()))
        .filter_map(|name| name.rsplit_once('.').map(|(schema, _)| schema.to_lowercase()))
        .collect();
    declared.sort_unstable();
    declared.dedup();
    declared
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

/// The SC-DRF01 false positive, live. The DDL qualifies its table with a
/// schema that is not on the connection's `search_path`, so the live read used
/// to skip that schema entirely and report the table as missing — in a run
/// where the server would happily have prepared a query against it.
#[tokio::test]
async fn a_ddl_schema_off_the_search_path_is_still_compared() {
    let (client, connection) = tokio_postgres::connect(&url(), NoTls)
        .await
        .expect("test setup: connect");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    client
        .batch_execute(
            "DROP SCHEMA IF EXISTS drift_off_path CASCADE; \
             CREATE SCHEMA drift_off_path; \
             CREATE TABLE drift_off_path.accounts (id integer PRIMARY KEY, name text NOT NULL); \
             SET search_path TO pg_catalog;",
        )
        .await
        .expect("test setup: schema fixture");

    let ddl = "CREATE TABLE drift_off_path.accounts (id integer PRIMARY KEY, name text NOT NULL);";
    let findings = drift_for(&client, ddl).await;

    assert!(
        findings.is_empty(),
        "a table the database demonstrably has must not be reported as missing \
         merely because its schema is off the search path: {:?}",
        findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );

    // The other half of the claim: the server resolves the same table in the
    // same connection, so the drift answer and the server's answer agree.
    let prepared = client
        .prepare("SELECT id, name FROM drift_off_path.accounts")
        .await
        .expect("the server must be able to prepare against the very table drift just checked");
    assert_eq!(prepared.columns().len(), 2);
}

/// `pg_dump --schema-only` qualifies a column's enum type. `pg_type` never
/// does, so this fired SC-DRF05 on every enum column of every dumped schema.
#[tokio::test]
async fn a_schema_qualified_enum_column_is_not_reported_as_drift() {
    let client = client_with_schema(
        "drift_qualified_enum",
        "CREATE TYPE status AS ENUM ('active', 'banned'); \
         CREATE TABLE users (id integer, state status NOT NULL);",
    )
    .await;

    let findings = drift_for(
        &client,
        "CREATE TYPE status AS ENUM ('active', 'banned'); \
         CREATE TABLE users (id integer, state drift_qualified_enum.status NOT NULL);",
    )
    .await;

    assert!(
        findings.is_empty(),
        "a column whose enum type the DDL qualified must compare equal to the bare \
         type `pg_type` reports: {:?}",
        findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}

/// `sql_type_to_neutral` has no `name` arm, so the DDL side produced the raw
/// spelling while `pg_catalog` reports `Type::NAME` → `string`. Across a
/// 44-column sweep this was the only false positive.
#[tokio::test]
async fn a_name_typed_column_is_not_reported_as_drift() {
    let ddl = "CREATE TABLE wide (id integer, c_name name);";
    let client = client_with_schema("drift_name_type", ddl).await;

    let findings = drift_for(&client, ddl).await;

    assert!(
        findings.is_empty(),
        "a `name` column present identically on both sides is not drift: {:?}",
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

#[tokio::test]
async fn reports_every_composite_drift_rule_from_live_catalog_metadata() {
    let client = client_with_schema(
        "drift_composite_rules",
        "CREATE TYPE changed AS (
             type_field uuid,
             nullability_field text,
             live_only_field text
         );
         CREATE TYPE live_only_type AS (value text);",
    )
    .await;

    let catalog = Catalog::from_ddl_with_dialect(
        &["CREATE TYPE changed AS (
                 type_field text,
                 nullability_field text,
                 ddl_only_field text
             );
             CREATE TYPE ddl_only_type AS (value text);"],
        &SqlDialect::PostgreSQL,
    )
    .expect("catalog from committed DDL");
    let mut ddl = describe_catalog(&catalog).expect("describe committed DDL");
    // ~keep PostgreSQL composite DDL has no field-level NOT NULL syntax; mutate the portable model to exercise
    // the rule against the authoritative nullable=true value loaded from pg_attribute.
    ddl.composites
        .get_mut("changed")
        .expect("changed composite")
        .fields
        .get_mut("nullability_field")
        .expect("nullability field")
        .nullable = false;

    let live = fetch_live_schema(&client, &declared_schemas(&ddl))
        .await
        .expect("fetch live composite metadata");
    let findings = diff_schemas(&ddl, &live, &DriftSeverities::default(), "composite-rules");
    let mut actual = rule_ids(&findings);
    actual.sort_unstable();

    assert_eq!(
        actual,
        vec!["SC-DRF08", "SC-DRF09", "SC-DRF10", "SC-DRF11", "SC-DRF12", "SC-DRF13"]
    );
}

#[tokio::test]
async fn live_catalog_preserves_postgres_relations_types_and_primary_keys() {
    let schema = "scythe_drift_live_catalog";
    let client = client_with_schema(
        schema,
        "CREATE TYPE status AS ENUM ('active', 'disabled');
         CREATE DOMAIN postal_code AS text NOT NULL;
         CREATE TYPE geo AS (
             latitude numeric,
             longitude numeric
         );
         CREATE TYPE address AS (
             postal_code postal_code,
             state status,
             history status[],
             coordinates geo
         );
         CREATE TABLE accounts (
             id bigint,
             audit_note text,
             location address,
             state status NOT NULL,
             PRIMARY KEY (id) INCLUDE (audit_note)
         );",
    )
    .await;

    let shadow_schema = "scythe_drift_live_catalog_shadow";
    client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {shadow_schema} CASCADE;
             CREATE SCHEMA {shadow_schema};
             CREATE TYPE {shadow_schema}.status AS ENUM ('shadowed');
             CREATE TYPE {shadow_schema}.address AS (shadow_only integer);
             CREATE TABLE {shadow_schema}.accounts (shadow_only integer);"
        ))
        .await
        .expect("create shadowed qualified objects");

    let catalog = fetch_live_catalog(&client, &[shadow_schema.to_string()])
        .await
        .expect("fetch live catalog");
    let description = fetch_live_schema(&client, &[]).await.expect("fetch live schema");
    let accounts = catalog.get_table("accounts").expect("accounts table");
    assert_eq!(accounts.columns[0].name, "id");
    assert!(accounts.columns[0].primary_key);
    assert!(
        !accounts.columns[1].primary_key,
        "an INCLUDE column is not part of the primary key"
    );
    assert_eq!(accounts.columns[2].sql_type, format!("{schema}.address"));
    assert!(!accounts.columns[3].nullable);
    assert!(catalog.get_table(&format!("{schema}.accounts")).is_some());
    assert!(catalog.get_table(&format!("{shadow_schema}.accounts")).is_some());
    assert_eq!(
        catalog
            .get_table(&format!("{shadow_schema}.accounts"))
            .expect("qualified shadow table")
            .columns[0]
            .name,
        "shadow_only"
    );

    let address = catalog.get_composite("address").expect("address composite");
    assert_eq!(address.fields.len(), 4);
    assert_eq!(address.fields[0].name, "postal_code");
    assert_eq!(address.fields[0].sql_type, format!("{schema}.postal_code"));
    assert_eq!(address.fields[1].sql_type, format!("{schema}.status"));
    assert_eq!(address.fields[2].sql_type, format!("{schema}.status[]"));
    assert_eq!(address.fields[3].sql_type, format!("{schema}.geo"));
    assert!(address.fields.iter().all(|field| field.nullable));
    assert_eq!(
        description.composites["address"].fields["coordinates"]
            .neutral_type
            .as_deref(),
        Some("composite::geo")
    );

    assert_eq!(
        catalog.get_enum("status").expect("status enum").values,
        ["active", "disabled"]
    );
    assert_eq!(
        catalog
            .get_enum(&format!("{shadow_schema}.status"))
            .expect("qualified shadow enum")
            .values,
        ["shadowed"]
    );
    assert_eq!(catalog.get_domain_base_type("postal_code"), Some("text"));
    assert!(
        catalog.get_composite("accounts").is_none(),
        "implicit table row type must be excluded"
    );
}
