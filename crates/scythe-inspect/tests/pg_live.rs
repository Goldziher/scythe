//! Live-PG integration tests — only run when the `live-tests` feature is
//! enabled AND `$SCYTHE_TEST_DATABASE_URL` is set.
//!
//! Each test creates its own schema, runs the relevant check, and drops the
//! schema on the way out, so they're safe to run against a shared PG.
//!
//! ## PG-version compatibility
//!
//! The full matrix is PG 12, 14, 16, 17.  Most checks use `pg_catalog` tables
//! available since PG 9.1 (PG 12 is the floor).  Exceptions:
//!
//! - **SC-INS07** (`security-definer-view`, `min_pg_version = 15`): skipped by
//!   the driver on PG 12/14.  The positive test creates a plain view (valid on
//!   all versions); the negative test uses `WITH (security_invoker=true)` which
//!   is PG-15+ DDL — but the version guard fires before that DDL is issued, so
//!   both tests return early on PG < 15 without error.
//!
//! - **SC-INS13** (`sequence-overflow-risk`): uses `pg_sequences` (PG 10+).
//!   PG 12 floor covers this.
//!
//! - **SC-INS12** (`partition-without-default`): declarative partitioning DDL
//!   (`PARTITION BY RANGE`, `PARTITION OF … DEFAULT`) is available since PG 10.
//!   PG 12 floor covers this.

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

/// Return the connected server's PG major version (e.g. `12`, `16`, `17`).
///
/// Reads `server_version_num` (a decimal integer like `160004`) and divides by
/// 10 000.  Used by version-gated tests that need to skip before issuing DDL
/// that is invalid on older clusters.
async fn pg_major_version(client: &tokio_postgres::Client) -> u32 {
    let row = client
        .query_one("SELECT current_setting('server_version_num')::int AS v", &[])
        .await
        .expect("version query");
    let num: i32 = row.get("v");
    (num / 10_000) as u32
}

/// Return `true` (and print a skip message) when the server major version is
/// below `min_major`.  Call at the top of any test that issues DDL or asserts
/// behaviour that requires a minimum PG version.
async fn skip_if_pg_below(client: &tokio_postgres::Client, min_major: u32, test_name: &str) -> bool {
    let v = pg_major_version(client).await;
    if v < min_major {
        eprintln!("skipping {test_name}: requires PG {min_major}+, connected to PG {v}");
        true
    } else {
        false
    }
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

    client.batch_execute("DROP SCHEMA sc_ins01_fixture CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins01_silent_when_fk_has_covering_index() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins01_neg CASCADE;
        CREATE SCHEMA sc_ins01_neg;
        CREATE TABLE sc_ins01_neg.users (id bigint PRIMARY KEY);
        CREATE TABLE sc_ins01_neg.orders (
            id bigint PRIMARY KEY,
            user_id bigint REFERENCES sc_ins01_neg.users(id)
        );
        CREATE INDEX orders_user_id_idx ON sc_ins01_neg.orders (user_id);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS01" && f.message.contains("sc_ins01_neg.orders"));
    assert!(
        hit.is_none(),
        "expected no SC-INS01 finding when FK has a covering index, got {:?}",
        hit
    );

    client.batch_execute("DROP SCHEMA sc_ins01_neg CASCADE").await.ok();
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

    client.batch_execute("DROP SCHEMA sc_ins02_fixture CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins02_silent_when_rls_enabled_with_policy() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins02_neg CASCADE;
        CREATE SCHEMA sc_ins02_neg;
        CREATE TABLE sc_ins02_neg.tenants (id bigint PRIMARY KEY, owner_id bigint);
        ALTER TABLE sc_ins02_neg.tenants ENABLE ROW LEVEL SECURITY;
        CREATE POLICY tenant_iso ON sc_ins02_neg.tenants USING (owner_id > 0);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS02" && f.message.contains("sc_ins02_neg.tenants"));
    assert!(
        hit.is_none(),
        "expected no SC-INS02 when RLS is enabled alongside the policy, got {:?}",
        hit
    );

    client.batch_execute("DROP SCHEMA sc_ins02_neg CASCADE").await.ok();
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
    assert!(hit.is_some(), "expected SC-INS03 finding on events, got {:?}", findings);
    assert_eq!(hit.unwrap().severity, Severity::Warn);

    client.batch_execute("DROP SCHEMA sc_ins03_fixture CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins03_silent_when_indexes_are_distinct() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins03_neg CASCADE;
        CREATE SCHEMA sc_ins03_neg;
        CREATE TABLE sc_ins03_neg.events (id bigint PRIMARY KEY, kind text, category text);
        CREATE INDEX events_kind_idx ON sc_ins03_neg.events (kind);
        CREATE INDEX events_category_idx ON sc_ins03_neg.events (category);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS03" && f.message.contains("sc_ins03_neg.events"));
    assert!(
        hit.is_none(),
        "expected no SC-INS03 when indexes cover different columns, got {:?}",
        hit
    );

    client.batch_execute("DROP SCHEMA sc_ins03_neg CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins04_fires_when_violation_present() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins04_positive CASCADE;
        CREATE SCHEMA sc_ins04_positive;
        CREATE TABLE sc_ins04_positive.nopk (name text, value int);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS04" && f.message.contains("sc_ins04_positive.nopk"))
        .collect();
    assert!(!hits.is_empty(), "expected SC-INS04 to fire on nopk table");
    assert_eq!(hits[0].severity, Severity::Warn);

    client.batch_execute("DROP SCHEMA sc_ins04_positive CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins04_skips_when_violation_absent() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins04_negative CASCADE;
        CREATE SCHEMA sc_ins04_negative;
        CREATE TABLE sc_ins04_negative.haspk (id bigint PRIMARY KEY, name text);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS04" && f.message.contains("sc_ins04_negative.haspk"))
        .collect();
    assert!(hits.is_empty(), "expected no SC-INS04 for table with PK");

    client.batch_execute("DROP SCHEMA sc_ins04_negative CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins05_fires_when_violation_present() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins05_positive CASCADE;
        CREATE SCHEMA sc_ins05_positive;
        CREATE TABLE sc_ins05_positive.guarded (id bigint PRIMARY KEY);
        ALTER TABLE sc_ins05_positive.guarded ENABLE ROW LEVEL SECURITY;
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS05" && f.message.contains("sc_ins05_positive.guarded"))
        .collect();
    assert!(!hits.is_empty(), "expected SC-INS05 to fire: RLS on, no policies");
    assert_eq!(hits[0].severity, Severity::Warn);

    client.batch_execute("DROP SCHEMA sc_ins05_positive CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins05_skips_when_violation_absent() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins05_negative CASCADE;
        CREATE SCHEMA sc_ins05_negative;
        CREATE TABLE sc_ins05_negative.guarded (id bigint PRIMARY KEY);
        ALTER TABLE sc_ins05_negative.guarded ENABLE ROW LEVEL SECURITY;
        CREATE POLICY allow_all ON sc_ins05_negative.guarded USING (true);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS05" && f.message.contains("sc_ins05_negative.guarded"))
        .collect();
    assert!(hits.is_empty(), "expected no SC-INS05: policy is defined");

    client.batch_execute("DROP SCHEMA sc_ins05_negative CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins06_fires_when_violation_present() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins06_positive CASCADE;
        CREATE SCHEMA sc_ins06_positive;
        CREATE TABLE sc_ins06_positive.docs (id bigint PRIMARY KEY, owner text);
        ALTER TABLE sc_ins06_positive.docs ENABLE ROW LEVEL SECURITY;
        CREATE POLICY pol_a ON sc_ins06_positive.docs FOR SELECT USING (owner = current_user);
        CREATE POLICY pol_b ON sc_ins06_positive.docs FOR SELECT USING (true);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS06" && f.message.contains("sc_ins06_positive.docs"))
        .collect();
    assert!(
        !hits.is_empty(),
        "expected SC-INS06 to fire: 2 permissive SELECT policies for public"
    );
    assert_eq!(hits[0].severity, Severity::Warn);

    client.batch_execute("DROP SCHEMA sc_ins06_positive CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins06_skips_when_violation_absent() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins06_negative CASCADE;
        CREATE SCHEMA sc_ins06_negative;
        CREATE TABLE sc_ins06_negative.docs (id bigint PRIMARY KEY, owner text);
        ALTER TABLE sc_ins06_negative.docs ENABLE ROW LEVEL SECURITY;
        CREATE POLICY pol_single ON sc_ins06_negative.docs FOR SELECT USING (owner = current_user);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS06" && f.message.contains("sc_ins06_negative.docs"))
        .collect();
    assert!(
        hits.is_empty(),
        "expected no SC-INS06: only one policy per role/command"
    );

    client.batch_execute("DROP SCHEMA sc_ins06_negative CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins07_fires_when_violation_present() {
    let client = raw_client().await;
    if skip_if_pg_below(&client, 15, "sc_ins07_fires_when_violation_present").await {
        return;
    }
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins07_positive CASCADE;
        CREATE SCHEMA sc_ins07_positive;
        CREATE TABLE sc_ins07_positive.base (id bigint PRIMARY KEY, secret text);
        CREATE VIEW sc_ins07_positive.exposed AS SELECT id FROM sc_ins07_positive.base;
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS07" && f.message.contains("sc_ins07_positive.exposed"))
        .collect();
    assert!(
        !hits.is_empty(),
        "expected SC-INS07 to fire: view lacks security_invoker=true"
    );
    assert_eq!(hits[0].severity, Severity::Error);

    client.batch_execute("DROP SCHEMA sc_ins07_positive CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins07_skips_when_violation_absent() {
    let client = raw_client().await;
    if skip_if_pg_below(&client, 15, "sc_ins07_skips_when_violation_absent").await {
        return;
    }
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins07_negative CASCADE;
        CREATE SCHEMA sc_ins07_negative;
        CREATE TABLE sc_ins07_negative.base (id bigint PRIMARY KEY, secret text);
        CREATE VIEW sc_ins07_negative.safe
            WITH (security_invoker=true)
            AS SELECT id FROM sc_ins07_negative.base;
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS07" && f.message.contains("sc_ins07_negative.safe"))
        .collect();
    assert!(
        hits.is_empty(),
        "expected no SC-INS07 for view with security_invoker=true"
    );

    client.batch_execute("DROP SCHEMA sc_ins07_negative CASCADE").await.ok();
}

// SC-INS08 — SECURITY DEFINER function without fixed search_path

#[tokio::test]
async fn sc_ins08_fires_when_violation_present() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins08_positive CASCADE;
        CREATE SCHEMA sc_ins08_positive;
        CREATE OR REPLACE FUNCTION sc_ins08_positive.risky()
        RETURNS void
        LANGUAGE plpgsql
        SECURITY DEFINER
        AS $$ BEGIN NULL; END; $$;
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS08" && f.message.contains("sc_ins08_positive") && f.message.contains("risky"))
        .collect();
    assert!(
        !hits.is_empty(),
        "expected SC-INS08 to fire: SECURITY DEFINER without search_path"
    );
    assert_eq!(hits[0].severity, Severity::Error);

    client.batch_execute("DROP SCHEMA sc_ins08_positive CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins08_skips_when_violation_absent() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins08_negative CASCADE;
        CREATE SCHEMA sc_ins08_negative;
        CREATE OR REPLACE FUNCTION sc_ins08_negative.safe()
        RETURNS void
        LANGUAGE plpgsql
        SECURITY DEFINER
        SET search_path = pg_catalog, public
        AS $$ BEGIN NULL; END; $$;
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS08" && f.message.contains("sc_ins08_negative") && f.message.contains("safe"))
        .collect();
    assert!(hits.is_empty(), "expected no SC-INS08: function pins search_path");

    client.batch_execute("DROP SCHEMA sc_ins08_negative CASCADE").await.ok();
}

/// Pick the first available test extension and install it in the requested
/// schema. Returns the extension name on success, `None` if no benign
/// extension is available (CI build without contrib modules).
async fn install_test_extension_in_schema(client: &tokio_postgres::Client, schema: &str) -> Option<&'static str> {
    for ext in ["pgcrypto", "btree_gin", "btree_gist"] {
        let stmt = format!("CREATE EXTENSION IF NOT EXISTS {ext} SCHEMA {schema}");
        if client.batch_execute(&stmt).await.is_ok() {
            return Some(ext);
        }
    }
    None
}

#[tokio::test]
async fn sc_ins09_fires_when_violation_present() {
    let client = raw_client().await;

    let Some(ext) = install_test_extension_in_schema(&client, "public").await else {
        println!(
            "skipping sc_ins09_fires_when_violation_present: \
             no benign extension available to install in public"
        );
        return;
    };

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS09" && f.message.contains(ext))
        .collect();
    assert!(
        !hits.is_empty(),
        "expected SC-INS09 to fire for extension `{ext}` in public, got: {:?}",
        findings.iter().filter(|f| f.rule_id == "SC-INS09").collect::<Vec<_>>()
    );
    assert_eq!(hits[0].severity, Severity::Warn);

    client
        .batch_execute(&format!("DROP EXTENSION IF EXISTS {ext}"))
        .await
        .ok();
}

#[tokio::test]
async fn sc_ins09_skips_when_violation_absent() {
    let client = raw_client().await;

    let schema = "sc_ins09_neg_ext_schema";
    client
        .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .await
        .expect("create non-public schema");
    let Some(ext) = install_test_extension_in_schema(&client, schema).await else {
        let findings = run_all_findings().await;
        let leaked: Vec<_> = findings
            .iter()
            .filter(|f| f.rule_id == "SC-INS09" && f.message.contains(schema))
            .collect();
        assert!(
            leaked.is_empty(),
            "schema `{schema}` should not appear in SC-INS09 findings"
        );
        client
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
            .await
            .ok();
        return;
    };

    let findings = run_all_findings().await;
    let leaked: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS09" && f.message.contains(ext) && f.message.contains("public"))
        .collect();
    assert!(
        leaked.is_empty(),
        "extension `{ext}` was installed in `{schema}`, not `public`, but SC-INS09 fired for it: {:?}",
        leaked
    );

    client
        .batch_execute(&format!(
            "DROP EXTENSION IF EXISTS {ext}; DROP SCHEMA IF EXISTS {schema} CASCADE"
        ))
        .await
        .ok();
}

#[tokio::test]
async fn sc_ins10_fires_when_violation_present() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP TABLE IF EXISTS public.sc_ins10_positive_sentinel CASCADE;
        CREATE TABLE public.sc_ins10_positive_sentinel (id bigint PRIMARY KEY);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS10" && f.message.contains("public.sc_ins10_positive_sentinel"))
        .collect();
    assert!(
        !hits.is_empty(),
        "expected SC-INS10 to fire: table in public with RLS off"
    );
    assert_eq!(hits[0].severity, Severity::Warn);

    client
        .batch_execute("DROP TABLE IF EXISTS public.sc_ins10_positive_sentinel CASCADE")
        .await
        .ok();
}

#[tokio::test]
async fn sc_ins10_skips_when_violation_absent() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP TABLE IF EXISTS public.sc_ins10_negative_sentinel CASCADE;
        CREATE TABLE public.sc_ins10_negative_sentinel (id bigint PRIMARY KEY);
        ALTER TABLE public.sc_ins10_negative_sentinel ENABLE ROW LEVEL SECURITY;
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hits: Vec<_> = findings
        .iter()
        .filter(|f| f.rule_id == "SC-INS10" && f.message.contains("public.sc_ins10_negative_sentinel"))
        .collect();
    assert!(hits.is_empty(), "expected no SC-INS10 for table with RLS enabled");

    client
        .batch_execute("DROP TABLE IF EXISTS public.sc_ins10_negative_sentinel CASCADE")
        .await
        .ok();
}

#[tokio::test]
async fn sc_ins11_fires_on_unlogged_table() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins11_pos CASCADE;
        CREATE SCHEMA sc_ins11_pos;
        CREATE UNLOGGED TABLE sc_ins11_pos.t (id int);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings.iter().find(|f| {
        f.rule_id == "SC-INS11" && f.message.contains("sc_ins11_pos") && f.message.contains("sc_ins11_pos.t")
    });
    assert!(
        hit.is_some(),
        "expected SC-INS11 finding for sc_ins11_pos.t, got {:?}",
        findings.iter().filter(|f| f.rule_id == "SC-INS11").collect::<Vec<_>>()
    );
    assert_eq!(hit.unwrap().severity, Severity::Warn);

    client.batch_execute("DROP SCHEMA sc_ins11_pos CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins11_silent_on_logged_table() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins11_neg CASCADE;
        CREATE SCHEMA sc_ins11_neg;
        CREATE TABLE sc_ins11_neg.t (id int);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS11" && f.message.contains("sc_ins11_neg.t"));
    assert!(
        hit.is_none(),
        "expected no SC-INS11 finding for logged sc_ins11_neg.t, got {:?}",
        hit
    );

    client.batch_execute("DROP SCHEMA sc_ins11_neg CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins12_fires_on_partition_without_default() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins12_pos CASCADE;
        CREATE SCHEMA sc_ins12_pos;
        CREATE TABLE sc_ins12_pos.parent (id int) PARTITION BY RANGE (id);
        CREATE TABLE sc_ins12_pos.parent_one
            PARTITION OF sc_ins12_pos.parent FOR VALUES FROM (0) TO (100);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS12" && f.message.contains("sc_ins12_pos.parent"));
    assert!(
        hit.is_some(),
        "expected SC-INS12 finding for sc_ins12_pos.parent (no default partition), got {:?}",
        findings.iter().filter(|f| f.rule_id == "SC-INS12").collect::<Vec<_>>()
    );
    assert_eq!(hit.unwrap().severity, Severity::Warn);

    client.batch_execute("DROP SCHEMA sc_ins12_pos CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins12_silent_when_default_partition_exists() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins12_neg CASCADE;
        CREATE SCHEMA sc_ins12_neg;
        CREATE TABLE sc_ins12_neg.parent (id int) PARTITION BY RANGE (id);
        CREATE TABLE sc_ins12_neg.parent_one
            PARTITION OF sc_ins12_neg.parent FOR VALUES FROM (0) TO (100);
        CREATE TABLE sc_ins12_neg.parent_default
            PARTITION OF sc_ins12_neg.parent DEFAULT;
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS12" && f.message.contains("sc_ins12_neg.parent"));
    assert!(
        hit.is_none(),
        "expected no SC-INS12 finding when DEFAULT partition exists, got {:?}",
        hit
    );

    client.batch_execute("DROP SCHEMA sc_ins12_neg CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins13_fires_on_sequence_over_70_percent() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins13_pos CASCADE;
        CREATE SCHEMA sc_ins13_pos;
        CREATE SEQUENCE sc_ins13_pos.seq MAXVALUE 100;
        SELECT setval('sc_ins13_pos.seq', 80);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS13" && f.message.contains("sc_ins13_pos.seq"));
    assert!(
        hit.is_some(),
        "expected SC-INS13 finding for sc_ins13_pos.seq at 80%, got {:?}",
        findings.iter().filter(|f| f.rule_id == "SC-INS13").collect::<Vec<_>>()
    );
    assert_eq!(hit.unwrap().severity, Severity::Warn);
    assert!(
        hit.unwrap().message.contains("80"),
        "expected percent_used=80 in message, got: {}",
        hit.unwrap().message
    );

    client.batch_execute("DROP SCHEMA sc_ins13_pos CASCADE").await.ok();
}

#[tokio::test]
async fn sc_ins13_silent_on_sequence_under_70_percent() {
    let client = raw_client().await;
    client
        .batch_execute(
            "
        DROP SCHEMA IF EXISTS sc_ins13_neg CASCADE;
        CREATE SCHEMA sc_ins13_neg;
        CREATE SEQUENCE sc_ins13_neg.seq MAXVALUE 100;
        SELECT setval('sc_ins13_neg.seq', 50);
        ",
        )
        .await
        .expect("setup");

    let findings = run_all_findings().await;
    let hit = findings
        .iter()
        .find(|f| f.rule_id == "SC-INS13" && f.message.contains("sc_ins13_neg.seq"));
    assert!(
        hit.is_none(),
        "expected no SC-INS13 finding for sc_ins13_neg.seq at 50%, got {:?}",
        hit
    );

    client.batch_execute("DROP SCHEMA sc_ins13_neg CASCADE").await.ok();
}
