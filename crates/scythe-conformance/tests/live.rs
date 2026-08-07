//! Live-database conformance tests: one `#[tokio::test]` per engine, each
//! gated by that engine's own Cargo feature so a build without a driver
//! never even sees the test function. The whole file is gated by
//! `live-tests` -- without it, this compiles to an empty test binary
//! rather than running against a real database.
//!
//! Every test selects *only its own engine* for the run, mirroring how the
//! CI matrix invokes this: each matrix job enables exactly one engine's
//! feature. Every other engine a fixture lists (this batch: SQLite,
//! PostgreSQL, MySQL, MariaDB are implemented; MSSQL and Oracle are not)
//! shows up as an explicit, printed skip in `report.summary()` -- see
//! `scythe_conformance::runner`'s module docs for why that's the correct
//! outcome for an engine that is simply out of scope for *this* run,
//! versus a hard error for one that was selected but cannot run.
#![cfg(feature = "live-tests")]

use std::path::Path;

use scythe_conformance::{Engine, RunnerConfig, fixture, run};

fn testing_data_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing_data/nullability_live")
}

fn load_fixtures_and_divergences() -> (Vec<fixture::LiveFixture>, Vec<scythe_conformance::DivergenceEntry>) {
    let root = testing_data_root();
    let schemas_root = root.join("_schemas");
    let fixtures = fixture::load_fixtures(&root, &schemas_root).expect("committed fixtures must load cleanly");
    let divergences_path = root.join("DIVERGENCES.toml");
    let divergences =
        scythe_conformance::divergence::load(&divergences_path).expect("committed DIVERGENCES.toml must load cleanly");
    (fixtures, divergences)
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_leg_is_sound() {
    let (fixtures, divergences) = load_fixtures_and_divergences();
    let config = RunnerConfig::from_env(vec![Engine::Sqlite], testing_data_root().join("_schemas"));
    let report = run(&fixtures, &divergences, &config)
        .await
        .expect("run must not hard-error");
    println!("{}", report.summary());
    assert!(report.is_pass(), "{}", report.summary());
    assert!(
        !report.verdicts.is_empty(),
        "expected at least one SQLite leg to actually run"
    );
}

#[cfg(feature = "pg")]
#[tokio::test]
async fn postgres_leg_is_sound() {
    let url = std::env::var("SCYTHE_CONFORMANCE_POSTGRES_URL")
        .expect("SCYTHE_CONFORMANCE_POSTGRES_URL must be set to run this leg");
    let (fixtures, divergences) = load_fixtures_and_divergences();
    let mut config = RunnerConfig::from_env(vec![Engine::Postgresql], testing_data_root().join("_schemas"));
    config.postgres_url = Some(url);
    let report = run(&fixtures, &divergences, &config)
        .await
        .expect("run must not hard-error");
    println!("{}", report.summary());
    assert!(report.is_pass(), "{}", report.summary());
    assert!(
        !report.verdicts.is_empty(),
        "expected at least one PostgreSQL leg to actually run"
    );
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn mysql_leg_is_sound() {
    let url =
        std::env::var("SCYTHE_CONFORMANCE_MYSQL_ADMIN_URL").expect("SCYTHE_CONFORMANCE_MYSQL_ADMIN_URL must be set");
    let (fixtures, divergences) = load_fixtures_and_divergences();
    let mut config = RunnerConfig::from_env(vec![Engine::Mysql], testing_data_root().join("_schemas"));
    config.mysql_admin_url = Some(url);
    let report = run(&fixtures, &divergences, &config)
        .await
        .expect("run must not hard-error");
    println!("{}", report.summary());
    assert!(report.is_pass(), "{}", report.summary());
    assert!(
        !report.verdicts.is_empty(),
        "expected at least one MySQL leg to actually run"
    );
}

#[cfg(feature = "mariadb")]
#[tokio::test]
async fn mariadb_leg_is_sound() {
    let url = std::env::var("SCYTHE_CONFORMANCE_MARIADB_ADMIN_URL")
        .expect("SCYTHE_CONFORMANCE_MARIADB_ADMIN_URL must be set");
    let (fixtures, divergences) = load_fixtures_and_divergences();
    let mut config = RunnerConfig::from_env(vec![Engine::Mariadb], testing_data_root().join("_schemas"));
    config.mariadb_admin_url = Some(url);
    let report = run(&fixtures, &divergences, &config)
        .await
        .expect("run must not hard-error");
    println!("{}", report.summary());
    assert!(report.is_pass(), "{}", report.summary());
    assert!(
        !report.verdicts.is_empty(),
        "expected at least one MariaDB leg to actually run"
    );
}

/// A fixture-listed engine that is not selected must appear as an explicit
/// skip in the report -- never silently absent. Runs whenever at least one
/// driver feature is on, independent of which.
#[cfg(any(feature = "pg", feature = "sqlite", feature = "mysql", feature = "mariadb"))]
#[tokio::test]
async fn unselected_engines_are_reported_as_explicit_skips_not_silently_dropped() {
    let (fixtures, divergences) = load_fixtures_and_divergences();
    // Select an engine that is compiled in (whichever is available) but
    // deliberately don't select the others the committed fixture lists
    // (mssql, oracle -- always out of scope this batch -- plus whichever
    // of pg/sqlite/mysql/mariadb isn't compiled into this test binary).
    let selected = if cfg!(feature = "sqlite") {
        vec![Engine::Sqlite]
    } else if cfg!(feature = "pg") {
        vec![Engine::Postgresql]
    } else if cfg!(feature = "mysql") {
        vec![Engine::Mysql]
    } else {
        vec![Engine::Mariadb]
    };
    let mut config = RunnerConfig::from_env(selected, testing_data_root().join("_schemas"));
    // Give every driver a shot at connecting, if configured, so the only
    // reason an engine doesn't run is that it wasn't selected -- not that
    // it lacked configuration.
    config.postgres_url = std::env::var("SCYTHE_CONFORMANCE_POSTGRES_URL").ok();
    config.mysql_admin_url = std::env::var("SCYTHE_CONFORMANCE_MYSQL_ADMIN_URL").ok();
    config.mariadb_admin_url = std::env::var("SCYTHE_CONFORMANCE_MARIADB_ADMIN_URL").ok();

    let report = run(&fixtures, &divergences, &config)
        .await
        .expect("run must not hard-error");
    println!("{}", report.summary());

    assert!(
        report.skipped.iter().any(|s| s.engine == Engine::Mssql),
        "an mssql leg listed by a fixture but not selected must be recorded as an explicit skip: {:?}",
        report.skipped
    );
    assert!(
        report.skipped.iter().any(|s| s.engine == Engine::Oracle),
        "an oracle leg listed by a fixture but not selected must be recorded as an explicit skip: {:?}",
        report.skipped
    );
}
