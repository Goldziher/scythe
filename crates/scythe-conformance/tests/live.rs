//! Live-database conformance tests: one `#[tokio::test]` per engine, each
//! gated by that engine's own Cargo feature so a build without a driver
//! never even sees the test function. The whole file is gated by
//! `live-tests` -- without it, this compiles to an empty test binary
//! rather than running against a real database.
//!
//! Every test selects *only its own engine* for the run, mirroring how the
//! CI matrix invokes this: each matrix job enables exactly one engine's
//! feature. Every other engine a fixture lists shows up as an explicit,
//! printed skip in `report.summary()` -- see
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

/// Pins the reason `math_functions/live_power_sqrt_on_nullable_operand_is_null.json`
/// excludes SQLite: rusqlite's bundled amalgamation (`libsqlite3-sys`'s
/// `build.rs`) never passes `-DSQLITE_ENABLE_MATH_FUNCTIONS`, so SQRT and
/// POWER are compiled out of the SQLite this suite actually links against,
/// not merely absent from the system `sqlite3` binary. If `libsqlite3-sys`
/// starts enabling that flag, this test starts failing -- widen the
/// fixture's `engines` list (and add a `_schemas/measurements/sqlite.sql`)
/// instead of re-excluding SQLite for a reason nobody re-checked.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_leg_lacks_math_functions() {
    use scythe_conformance::Executor;
    use scythe_conformance::executors::sqlite::SqliteExecutor;

    let mut executor = SqliteExecutor::open_in_memory().expect("in-memory sqlite must open");
    let error = executor
        .query_nullness("SELECT sqrt(4.0)")
        .await
        .expect_err("SQRT must still be unavailable in the bundled SQLite build");
    assert!(
        error.to_string().contains("no such function: sqrt"),
        "expected a missing-function error, got: {error}"
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

#[cfg(feature = "mssql")]
#[tokio::test]
async fn mssql_leg_is_sound() {
    let url =
        std::env::var("SCYTHE_CONFORMANCE_MSSQL_ADMIN_URL").expect("SCYTHE_CONFORMANCE_MSSQL_ADMIN_URL must be set");
    let (fixtures, divergences) = load_fixtures_and_divergences();
    let mut config = RunnerConfig::from_env(vec![Engine::Mssql], testing_data_root().join("_schemas"));
    config.mssql_admin_url = Some(url);
    let report = run(&fixtures, &divergences, &config)
        .await
        .expect("run must not hard-error");
    println!("{}", report.summary());
    assert!(report.is_pass(), "{}", report.summary());
    assert!(
        !report.verdicts.is_empty(),
        "expected at least one SQL Server leg to actually run"
    );
}

#[cfg(feature = "oracle")]
#[tokio::test]
async fn oracle_leg_is_sound() {
    let url =
        std::env::var("SCYTHE_CONFORMANCE_ORACLE_ADMIN_URL").expect("SCYTHE_CONFORMANCE_ORACLE_ADMIN_URL must be set");
    let (fixtures, divergences) = load_fixtures_and_divergences();
    let mut config = RunnerConfig::from_env(vec![Engine::Oracle], testing_data_root().join("_schemas"));
    config.oracle_admin_url = Some(url);
    let report = run(&fixtures, &divergences, &config)
        .await
        .expect("run must not hard-error");
    println!("{}", report.summary());
    assert!(report.is_pass(), "{}", report.summary());
    assert!(
        !report.verdicts.is_empty(),
        "expected at least one Oracle leg to actually run"
    );
}

/// A fixture-listed engine that is not selected must appear as an explicit
/// skip in the report -- never silently absent. Runs whenever at least one
/// driver feature is on, independent of which.
///
/// The assertion is written against *every* unselected engine rather than
/// naming MSSQL and Oracle, which is what it named while those two were the
/// only engines no job could ever select. Now that all six have drivers,
/// naming a fixed pair would make this test vacuous in exactly the job that
/// selects one of them -- and it is the guard against silent dropping, so a
/// vacuous version of it is worse than none.
#[cfg(any(
    feature = "pg",
    feature = "sqlite",
    feature = "mysql",
    feature = "mariadb",
    feature = "mssql",
    feature = "oracle"
))]
#[tokio::test]
async fn unselected_engines_are_reported_as_explicit_skips_not_silently_dropped() {
    let (fixtures, divergences) = load_fixtures_and_divergences();
    // Select exactly one engine that is compiled into this test binary.
    // The cheap in-process engines come first so this test does not do a
    // second full container round trip in the jobs that have one.
    let selected = if cfg!(feature = "sqlite") {
        Engine::Sqlite
    } else if cfg!(feature = "pg") {
        Engine::Postgresql
    } else if cfg!(feature = "mysql") {
        Engine::Mysql
    } else if cfg!(feature = "mariadb") {
        Engine::Mariadb
    } else if cfg!(feature = "mssql") {
        Engine::Mssql
    } else {
        Engine::Oracle
    };
    let mut config = RunnerConfig::from_env(vec![selected], testing_data_root().join("_schemas"));
    // Give every driver a shot at connecting, if configured, so the only
    // reason an engine doesn't run is that it wasn't selected -- not that
    // it lacked configuration.
    config.postgres_url = std::env::var("SCYTHE_CONFORMANCE_POSTGRES_URL").ok();
    config.mysql_admin_url = std::env::var("SCYTHE_CONFORMANCE_MYSQL_ADMIN_URL").ok();
    config.mariadb_admin_url = std::env::var("SCYTHE_CONFORMANCE_MARIADB_ADMIN_URL").ok();
    config.mssql_admin_url = std::env::var("SCYTHE_CONFORMANCE_MSSQL_ADMIN_URL").ok();
    config.oracle_admin_url = std::env::var("SCYTHE_CONFORMANCE_ORACLE_ADMIN_URL").ok();

    let report = run(&fixtures, &divergences, &config)
        .await
        .expect("run must not hard-error");
    println!("{}", report.summary());

    // Every committed fixture lists all six engines, so each of the five
    // that were not selected must appear as a recorded skip.
    for engine in Engine::ALL {
        if engine == selected {
            continue;
        }
        assert!(
            report.skipped.iter().any(|skip| skip.engine == engine),
            "a {engine} leg listed by a fixture but not selected must be recorded as an explicit skip: {:?}",
            report.skipped
        );
    }
}
