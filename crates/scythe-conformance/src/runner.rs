//! Wires fixtures, the analyzer, codegen, a live [`Executor`], and the four
//! assertions in [`crate::verdict`] into one real [`Verdict`] per (fixture,
//! engine, column) -- the part of this crate that makes every guarantee in
//! [`crate::verdict`] and [`crate::divergence`] real rather than aspirational.
//!
//! ## The dyn-incompatibility problem
//!
//! [`Executor`] is not object-safe: it has an associated `const ENGINE` and
//! its methods return `impl Future` (RPITIT), neither of which is
//! dyn-compatible. So this runner cannot hold a `Vec<Box<dyn Executor>>`
//! and iterate a fixture's `live.engines` generically -- it has to `match`
//! on [`Engine`] and monomorphize a concrete executor type per arm (see
//! [`run_one_leg`]).
//!
//! ## The silent-skip trap, and the two outcomes this runner allows instead
//!
//! The natural way to write that match is `_ => continue` for any engine
//! whose driver isn't compiled in -- which is exactly the failure mode this
//! whole crate exists to prevent: a green result produced by examining
//! nothing, with no way to tell "skipped" from "passed" in the output. This
//! runner draws a deliberate line between two engine states, and only one
//! of them is a skip:
//!
//! - **Selected but not runnable** (its feature is off, or it has no
//!   connection configuration): a **hard error**. [`run`] returns `Err`
//!   before evaluating a single fixture. Asking to run an engine that
//!   cannot run is a misconfiguration, not a scoping decision, and must
//!   stop the whole batch rather than silently produce a partial result.
//! - **Listed by a fixture but not selected for this invocation** (e.g. a
//!   fixture lists all six engines while this job selected only Oracle):
//!   an **explicit, recorded skip**. It is pushed onto
//!   [`RunReport::skipped`] with the fixture name, engine, and a reason, so
//!   it is always visible in the report -- never indistinguishable from a
//!   pass, and never simply absent.
//!
//! The second case has to be a skip rather than a hard error because the CI
//! matrix runs one engine per job: each job compiles exactly one driver
//! feature, so every other engine a fixture lists is out of scope for that
//! invocation and must not fail it.
//!
//! All six engines now have a driver. The `EngineNotCompiled` arm is what
//! remains of the bring-up: it fires when an engine is *selected* in a
//! build that did not compile its feature, which is still a
//! misconfiguration rather than a scoping decision.

use std::path::PathBuf;

use ahash::AHashMap;

use crate::divergence::{DivergenceEntry, DivergenceError};
use crate::executor::{Executor, MissingColumn};
use crate::fixture::{Engine, LiveFixture, RowCountMismatch as FixtureRowCountMismatch};
use crate::verdict::{
    ColumnFacts, ColumnFactsError, FixtureEngineFacts, RowCountMismatch as FactsRowCountMismatch, Verdict, evaluate,
};

/// Connection configuration and engine selection for one invocation of
/// [`run`]. Built once per process (typically from environment variables
/// via [`RunnerConfig::from_env`]) and shared read-only across every leg.
#[derive(Debug, Clone, Default)]
pub struct RunnerConfig {
    /// The engines this invocation actually runs. Every fixture-declared
    /// engine outside this set is recorded as an explicit skip, never run.
    pub selected_engines: Vec<Engine>,
    /// Root directory holding `<schema_profile>/<engine>.sql` live schema
    /// files (`testing_data/nullability_live/_schemas` in this repo).
    pub schemas_root: PathBuf,
    /// `postgres://` connection URL. Required iff `Engine::Postgresql` is selected.
    pub postgres_url: Option<String>,
    /// Admin (database-creating) connection URL for MySQL. Required iff
    /// `Engine::Mysql` is selected -- the containers' per-app `scythe` user
    /// is scoped to its own database, so isolating this crate's tables
    /// into a dedicated database needs elevated privileges.
    pub mysql_admin_url: Option<String>,
    /// Same as `mysql_admin_url`, for MariaDB. Required iff `Engine::Mariadb` is selected.
    pub mariadb_admin_url: Option<String>,
    /// `sqlserver://user:pass@host:port?database=db` connection URL for SQL
    /// Server. Required iff `Engine::Mssql` is selected. Admin, for the same
    /// reason as MySQL's: the runner creates a database per connection to
    /// isolate itself, which the per-app login cannot do.
    pub mssql_admin_url: Option<String>,
    /// `oracle://user:pass@host:port/service` connection URL. Required iff
    /// `Engine::Oracle` is selected. Admin, and more strictly so than the
    /// others: Oracle's unit of isolation is a *user*, so this credential
    /// must be able to `CREATE USER` and grant privileges.
    pub oracle_admin_url: Option<String>,
}

impl RunnerConfig {
    /// Build a config from `SCYTHE_CONFORMANCE_*` environment variables.
    /// Connection URLs that are unset become `None`; [`ensure_available`]
    /// (called from [`run`]) is what turns "selected but unset" into a
    /// hard error, not this constructor.
    pub fn from_env(selected_engines: Vec<Engine>, schemas_root: PathBuf) -> Self {
        Self {
            selected_engines,
            schemas_root,
            postgres_url: std::env::var("SCYTHE_CONFORMANCE_POSTGRES_URL").ok(),
            mysql_admin_url: std::env::var("SCYTHE_CONFORMANCE_MYSQL_ADMIN_URL").ok(),
            mariadb_admin_url: std::env::var("SCYTHE_CONFORMANCE_MARIADB_ADMIN_URL").ok(),
            mssql_admin_url: std::env::var("SCYTHE_CONFORMANCE_MSSQL_ADMIN_URL").ok(),
            oracle_admin_url: std::env::var("SCYTHE_CONFORMANCE_ORACLE_ADMIN_URL").ok(),
        }
    }
}

/// One (fixture, engine) leg that was **not** run, and why. Always
/// constructed explicitly and pushed onto [`RunReport::skipped`] -- never
/// simply omitted -- so a skip is always visible in the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedLeg {
    pub fixture: String,
    pub engine: Engine,
    pub reason: String,
}

/// The outcome of one [`run`] invocation: every leg that actually ran (as a
/// real [`Verdict`], post-[`crate::divergence::reconcile`]) plus every leg
/// that was explicitly skipped.
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    pub verdicts: Vec<Verdict>,
    pub skipped: Vec<SkippedLeg>,
}

impl RunReport {
    /// Whether every leg that ran passed. Deliberately independent of
    /// `skipped`: a report with only skips (e.g. no engine selected) is
    /// vacuously "passing" by this method alone -- callers that care about
    /// that distinction should also check `skipped`/`verdicts` directly,
    /// which is why both are public fields rather than folded into one
    /// opaque pass/fail bit.
    pub fn is_pass(&self) -> bool {
        self.verdicts.iter().all(Verdict::is_pass)
    }

    /// A human-readable summary listing every failure and every skip by
    /// name -- suitable for CI logs. Never silently drops a skip: that is
    /// the entire point of this type.
    pub fn summary(&self) -> String {
        use std::fmt::Write as _;

        let failed: Vec<&Verdict> = self.verdicts.iter().filter(|v| !v.is_pass()).collect();
        let mut out = String::new();
        let _ = writeln!(
            out,
            "{} leg(s) ran ({} passed, {} failed), {} leg(s) skipped",
            self.verdicts.len(),
            self.verdicts.len() - failed.len(),
            failed.len(),
            self.skipped.len()
        );
        for verdict in &failed {
            let _ = writeln!(out, "  FAIL {} / {}", verdict.fixture, verdict.engine);
            for failure in &verdict.failures {
                let _ = writeln!(out, "    - {failure:?}");
            }
        }
        for skip in &self.skipped {
            let _ = writeln!(out, "  SKIP {} / {}: {}", skip.fixture, skip.engine, skip.reason);
        }
        out
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("engine {engine} was selected to run, but this crate was not compiled with its {feature:?} feature")]
    EngineNotCompiled { engine: Engine, feature: &'static str },
    #[error("engine {engine} was selected to run, but no connection configuration was provided for it")]
    MissingConnectionConfig { engine: Engine },
    #[error("fixture {fixture:?} on engine {engine}: connecting: {reason}")]
    Connect {
        fixture: String,
        engine: Engine,
        reason: String,
    },
    #[error("fixture {fixture:?} on engine {engine}: analyzing schema_sql/query_sql: {source}")]
    Analysis {
        fixture: String,
        engine: Engine,
        #[source]
        source: scythe_core::errors::ScytheError,
    },
    #[error("fixture {fixture:?} on engine {engine}: resolving generated columns: {source}")]
    Resolve {
        fixture: String,
        engine: Engine,
        #[source]
        source: scythe_core::errors::ScytheError,
    },
    #[error("fixture {fixture:?} on engine {engine}: reading live schema {path:?}: {source}")]
    ReadSchema {
        fixture: String,
        engine: Engine,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("fixture {fixture:?} on engine {engine}: driver operation failed: {source}")]
    Driver {
        fixture: String,
        engine: Engine,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("fixture {fixture:?} on engine {engine}: {source}")]
    RowCount {
        fixture: String,
        engine: Engine,
        #[source]
        source: FixtureRowCountMismatch,
    },
    #[error(
        "fixture {fixture:?} on engine {engine}: run {run:?} row {row} column {column:?}: the fixture declares this value {declared} but the engine returned it {observed}"
    )]
    RowNullnessMismatch {
        fixture: String,
        engine: Engine,
        run: String,
        row: usize,
        column: String,
        declared: &'static str,
        observed: &'static str,
    },
    #[error(
        "fixture {fixture:?} on engine {engine}: the live engine's row does not carry an expected column: {source}"
    )]
    MissingObservedColumn {
        fixture: String,
        engine: Engine,
        #[source]
        source: MissingColumn,
    },
    #[error("fixture {fixture:?} on engine {engine}: {source}")]
    Facts {
        fixture: String,
        engine: Engine,
        #[source]
        source: FactsRowCountMismatch,
    },
    #[error("fixture {fixture:?} on engine {engine}: {source}")]
    ColumnFacts {
        fixture: String,
        engine: Engine,
        #[source]
        source: ColumnFactsError,
    },
    #[error(transparent)]
    Divergence(#[from] DivergenceError),
}

/// Whether a driver for `engine` is compiled into this build, purely from
/// active Cargo features -- `cfg!()` evaluates to a compile-time-constant
/// bool, so this function is exhaustive and correct regardless of which
/// features are active, without itself needing `#[cfg(feature = ...)]`.
fn is_compiled(engine: Engine) -> bool {
    match engine {
        Engine::Postgresql => cfg!(feature = "pg"),
        Engine::Sqlite => cfg!(feature = "sqlite"),
        Engine::Mysql => cfg!(feature = "mysql"),
        Engine::Mariadb => cfg!(feature = "mariadb"),
        Engine::Mssql => cfg!(feature = "mssql"),
        Engine::Oracle => cfg!(feature = "oracle"),
    }
}

/// The Cargo feature that compiles `engine`'s driver in, named in the
/// `EngineNotCompiled` error so the fix is stated rather than guessed at.
fn feature_for(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgresql => "pg",
        Engine::Sqlite => "sqlite",
        Engine::Mysql => "mysql",
        Engine::Mariadb => "mariadb",
        Engine::Mssql => "mssql",
        Engine::Oracle => "oracle",
    }
}

/// The codegen backend whose rendered output stands in for "what scythe
/// actually generates" when computing G for `engine`.
///
/// Every arm is a *Rust* backend, and every one of their manifests renders
/// an optional column as `Option<{T}>`. That is what makes G comparable
/// across engines: A1 fidelity and A2 soundness both ask whether the
/// generated code wraps a column, so if two engines were checked against
/// backends in different target languages, a difference in G could just be
/// a difference in how those languages spell "optional" rather than a
/// difference in scythe's inference.
///
/// `rust-sqlx` cannot be used uniformly: sqlx has no SQL Server or Oracle
/// driver, and `get_backend` rejects those engines outright rather than
/// quietly substituting a manifest -- which is how the first MSSQL leg
/// surfaced as a hard `Resolve` error instead of a wrong verdict.
fn representative_backend(engine: Engine) -> &'static str {
    match engine {
        Engine::Postgresql | Engine::Mysql | Engine::Mariadb | Engine::Sqlite => "rust-sqlx",
        Engine::Mssql => "rust-tiberius",
        Engine::Oracle => "rust-sibyl",
    }
}

/// Validates that `engine` can actually run under `config`: compiled in,
/// and (for network engines) has connection configuration. Called for
/// every selected engine *before* [`run`] evaluates a single fixture, so a
/// misconfigured batch fails fast instead of partway through.
fn ensure_available(engine: Engine, config: &RunnerConfig) -> Result<(), RunnerError> {
    if !is_compiled(engine) {
        return Err(RunnerError::EngineNotCompiled {
            engine,
            feature: feature_for(engine),
        });
    }
    let has_connection_config = match engine {
        Engine::Postgresql => config.postgres_url.is_some(),
        Engine::Mysql => config.mysql_admin_url.is_some(),
        Engine::Mariadb => config.mariadb_admin_url.is_some(),
        Engine::Mssql => config.mssql_admin_url.is_some(),
        Engine::Oracle => config.oracle_admin_url.is_some(),
        Engine::Sqlite => true, // in-memory, no external configuration needed
    };
    if !has_connection_config {
        return Err(RunnerError::MissingConnectionConfig { engine });
    }
    Ok(())
}

/// Run every fixture against every engine `config` selects, apply the
/// divergence registry, and return the combined report.
///
/// `entries` is passed straight to [`crate::divergence::reconcile`] -- the
/// *only* place this runner touches the divergence registry, so
/// staleness-then-suppression ordering (see that function's docs) cannot
/// be gotten wrong by a caller reaching for `apply`/`check_staleness`
/// directly and getting the order backwards.
pub async fn run(
    fixtures: &[LiveFixture],
    entries: &[DivergenceEntry],
    config: &RunnerConfig,
) -> Result<RunReport, RunnerError> {
    for &engine in &config.selected_engines {
        ensure_available(engine, config)?;
    }

    let mut report = RunReport::default();

    for fixture in fixtures {
        for &engine in &fixture.live.engines {
            if !config.selected_engines.contains(&engine) {
                report.skipped.push(SkippedLeg {
                    fixture: fixture.name.clone(),
                    engine,
                    reason: "not selected for this run".to_string(),
                });
                continue;
            }

            let verdict = run_one_leg(fixture, engine, config).await?;
            report.verdicts.push(verdict);
        }
    }

    report.verdicts = crate::divergence::reconcile(entries, report.verdicts, &config.selected_engines)?;
    Ok(report)
}

/// Dispatch to a concrete, monomorphized [`Executor`] for `engine`. This is
/// the match the module-level docs describe: every arm either runs a real
/// executor or returns an explicit [`RunnerError`] -- there is no `_ =>`
/// wildcard and no arm that silently does nothing.
///
/// `fixture` and `config` go genuinely unused when no driver feature is
/// active (every arm degenerates to its `EngineNotCompiled` branch) -- the
/// `allow` below is scoped to exactly that configuration via `cfg_attr`, so
/// a build with any driver feature on keeps the lint live.
#[cfg_attr(
    not(any(
        feature = "pg",
        feature = "sqlite",
        feature = "mysql",
        feature = "mariadb",
        feature = "mssql",
        feature = "oracle"
    )),
    allow(unused_variables)
)]
async fn run_one_leg(fixture: &LiveFixture, engine: Engine, config: &RunnerConfig) -> Result<Verdict, RunnerError> {
    match engine {
        Engine::Postgresql => {
            #[cfg(feature = "pg")]
            {
                let url = config
                    .postgres_url
                    .as_deref()
                    .ok_or(RunnerError::MissingConnectionConfig { engine })?;
                let executor = crate::executors::postgres::PgExecutor::connect(url)
                    .await
                    .map_err(|source| RunnerError::Connect {
                        fixture: fixture.name.clone(),
                        engine,
                        reason: source.to_string(),
                    })?;
                evaluate_fixture(fixture, executor, config).await
            }
            #[cfg(not(feature = "pg"))]
            {
                Err(RunnerError::EngineNotCompiled { engine, feature: "pg" })
            }
        }
        Engine::Sqlite => {
            #[cfg(feature = "sqlite")]
            {
                let executor = crate::executors::sqlite::SqliteExecutor::open_in_memory().map_err(|source| {
                    RunnerError::Connect {
                        fixture: fixture.name.clone(),
                        engine,
                        reason: source.to_string(),
                    }
                })?;
                evaluate_fixture(fixture, executor, config).await
            }
            #[cfg(not(feature = "sqlite"))]
            {
                Err(RunnerError::EngineNotCompiled {
                    engine,
                    feature: "sqlite",
                })
            }
        }
        Engine::Mysql => {
            #[cfg(feature = "mysql")]
            {
                let url = config
                    .mysql_admin_url
                    .as_deref()
                    .ok_or(RunnerError::MissingConnectionConfig { engine })?;
                let executor = crate::executors::mysql::MySqlExecutor::<crate::executors::mysql::Mysql>::connect(url)
                    .await
                    .map_err(|source| RunnerError::Connect {
                        fixture: fixture.name.clone(),
                        engine,
                        reason: source.to_string(),
                    })?;
                evaluate_fixture(fixture, executor, config).await
            }
            #[cfg(not(feature = "mysql"))]
            {
                Err(RunnerError::EngineNotCompiled {
                    engine,
                    feature: "mysql",
                })
            }
        }
        Engine::Mariadb => {
            #[cfg(feature = "mariadb")]
            {
                let url = config
                    .mariadb_admin_url
                    .as_deref()
                    .ok_or(RunnerError::MissingConnectionConfig { engine })?;
                let executor = crate::executors::mysql::MySqlExecutor::<crate::executors::mysql::Mariadb>::connect(url)
                    .await
                    .map_err(|source| RunnerError::Connect {
                        fixture: fixture.name.clone(),
                        engine,
                        reason: source.to_string(),
                    })?;
                evaluate_fixture(fixture, executor, config).await
            }
            #[cfg(not(feature = "mariadb"))]
            {
                Err(RunnerError::EngineNotCompiled {
                    engine,
                    feature: "mariadb",
                })
            }
        }
        Engine::Mssql => {
            #[cfg(feature = "mssql")]
            {
                let url = config
                    .mssql_admin_url
                    .as_deref()
                    .ok_or(RunnerError::MissingConnectionConfig { engine })?;
                let executor = crate::executors::mssql::MssqlExecutor::connect(url)
                    .await
                    .map_err(|source| RunnerError::Connect {
                        fixture: fixture.name.clone(),
                        engine,
                        reason: source.to_string(),
                    })?;
                evaluate_fixture(fixture, executor, config).await
            }
            #[cfg(not(feature = "mssql"))]
            {
                Err(RunnerError::EngineNotCompiled {
                    engine,
                    feature: "mssql",
                })
            }
        }
        Engine::Oracle => {
            #[cfg(feature = "oracle")]
            {
                let url = config
                    .oracle_admin_url
                    .as_deref()
                    .ok_or(RunnerError::MissingConnectionConfig { engine })?;
                let executor = crate::executors::oracle::OracleExecutor::connect(url)
                    .await
                    .map_err(|source| RunnerError::Connect {
                        fixture: fixture.name.clone(),
                        engine,
                        reason: source.to_string(),
                    })?;
                evaluate_fixture(fixture, executor, config).await
            }
            #[cfg(not(feature = "oracle"))]
            {
                Err(RunnerError::EngineNotCompiled {
                    engine,
                    feature: "oracle",
                })
            }
        }
    }
}

/// Drive one (fixture, engine) leg end to end through an already-connected
/// `executor`: analyze the fixture's portable `schema_sql`/`query_sql` (A),
/// resolve generated columns against a representative backend manifest
/// (G), seed the *live*, per-engine schema (never the analyzer's portable
/// one -- that is the entire point of this crate), run every declared run
/// and observe real nullness (E), and evaluate all four assertions.
///
/// Only called from [`run_one_leg`]'s feature-gated branches, so it is dead
/// code in a build with no driver feature active -- see the `allow` there.
#[cfg_attr(
    not(any(
        feature = "pg",
        feature = "sqlite",
        feature = "mysql",
        feature = "mariadb",
        feature = "mssql",
        feature = "oracle"
    )),
    allow(dead_code)
)]
async fn evaluate_fixture<E: Executor>(
    fixture: &LiveFixture,
    mut executor: E,
    config: &RunnerConfig,
) -> Result<Verdict, RunnerError> {
    let engine = E::ENGINE;
    let err_ctx = || (fixture.name.clone(), engine);

    // The analyzer must run under the dialect of the engine this leg is
    // about to query, not under the default (PostgreSQL). Every
    // dialect-aware inference rule in `scythe_core` keys off
    // `Catalog::dialect()`, so parsing every fixture as PostgreSQL would
    // make all of them dead code from this suite's point of view -- it
    // would compare a PostgreSQL-flavoured inference against Oracle's
    // actual behaviour and call the difference a divergence. `schema_sql`
    // stays the portable DDL: what changes here is the grammar and
    // semantics it is read under, not the schema.
    let dialect = engine.dialect();
    let schema_refs: Vec<&str> = fixture.schema_sql.iter().map(String::as_str).collect();
    let catalog = scythe_core::catalog::Catalog::from_ddl_with_dialect(&schema_refs, &dialect).map_err(|source| {
        let (fixture, engine) = err_ctx();
        RunnerError::Analysis {
            fixture,
            engine,
            source,
        }
    })?;
    let query = scythe_core::parser::parse_query_with_dialect(&fixture.query_sql, &dialect).map_err(|source| {
        let (fixture, engine) = err_ctx();
        RunnerError::Analysis {
            fixture,
            engine,
            source,
        }
    })?;
    let analyzed = scythe_core::analyzer::analyze(&catalog, &query).map_err(|source| {
        let (fixture, engine) = err_ctx();
        RunnerError::Analysis {
            fixture,
            engine,
            source,
        }
    })?;

    // Representative backend for computing G (rendered nullability).
    let backend = scythe_codegen::get_backend(representative_backend(engine), engine.as_str()).map_err(|source| {
        let (fixture, engine) = err_ctx();
        RunnerError::Resolve {
            fixture,
            engine,
            source,
        }
    })?;
    let manifest = backend.manifest();
    let resolved =
        scythe_codegen::resolve::resolve_columns(&analyzed.columns, manifest, &[], "").map_err(|source| {
            let (fixture, engine) = err_ctx();
            RunnerError::Resolve {
                fixture,
                engine,
                source,
            }
        })?;

    let schema_path = config
        .schemas_root
        .join(&fixture.live.schema_profile)
        .join(engine.schema_file_name());
    let live_schema_sql = std::fs::read_to_string(&schema_path).map_err(|source| {
        let (fixture, engine) = err_ctx();
        RunnerError::ReadSchema {
            fixture,
            engine,
            path: schema_path,
            source,
        }
    })?;
    executor.seed_schema(&live_schema_sql).await.map_err(|source| {
        let (fixture, engine) = err_ctx();
        RunnerError::Driver {
            fixture,
            engine,
            source: Box::new(source),
        }
    })?;

    let mut observed_nulls: AHashMap<String, Vec<bool>> = AHashMap::new();
    let mut row_count = 0usize;

    for run in &fixture.live.runs {
        let seed_statements = run.seed.resolve(engine).expect(
            "the fixture loader already validated that every declared engine has resolvable seed SQL for every run",
        );
        executor.seed_run(seed_statements).await.map_err(|source| {
            let (fixture, engine) = err_ctx();
            RunnerError::Driver {
                fixture,
                engine,
                source: Box::new(source),
            }
        })?;

        let observed_rows = executor.query_nullness(&fixture.query_sql).await.map_err(|source| {
            let (fixture, engine) = err_ctx();
            RunnerError::Driver {
                fixture,
                engine,
                source: Box::new(source),
            }
        })?;

        let resolved_rows = crate::expectation::resolve_run_rows(&fixture.live, run, engine);
        run.check_row_count(resolved_rows, observed_rows.len())
            .map_err(|source| {
                let (fixture, engine) = err_ctx();
                RunnerError::RowCount {
                    fixture,
                    engine,
                    source,
                }
            })?;

        for (row_index, observed_row) in observed_rows.iter().enumerate() {
            // Safe to index: check_row_count above already proved the two
            // lengths agree, and rows are matched ordinally against the
            // query's mandatory ORDER BY.
            let declared_row = &resolved_rows[row_index];
            for column in &analyzed.columns {
                let is_null = observed_row.is_null(&column.name).map_err(|source| {
                    let (fixture, engine) = err_ctx();
                    RunnerError::MissingObservedColumn {
                        fixture,
                        engine,
                        source,
                    }
                })?;

                // A fixture's per-row `null`/`non_null` lists read exactly
                // like assertions, so they must be ones. Without this, a
                // fixture could declare a column NULL in a row where the
                // engine returns a value and stay green -- the four
                // assertions below only consume the *observed* nulls, so
                // nothing else ever reads the declaration back. `None` is
                // a column the fixture does not mention for this row (the
                // loader requires every *declared* column to appear, but
                // `analyzed.columns` may be wider), which asserts nothing
                // by design rather than by omission.
                if let Some(declared_null) = declared_row.declared_null(&column.name)
                    && declared_null != is_null
                {
                    let (fixture, engine) = err_ctx();
                    return Err(RunnerError::RowNullnessMismatch {
                        fixture,
                        engine,
                        run: run.name.clone(),
                        row: row_index,
                        column: column.name.clone(),
                        declared: if declared_null { "NULL" } else { "non-NULL" },
                        observed: if is_null { "NULL" } else { "non-NULL" },
                    });
                }

                observed_nulls.entry(column.name.clone()).or_default().push(is_null);
            }
            row_count += 1;
        }
    }

    let mut column_facts = Vec::with_capacity(analyzed.columns.len());
    for (analyzed_column, resolved_column) in analyzed.columns.iter().zip(resolved.iter()) {
        let nulls = observed_nulls.remove(&analyzed_column.name).unwrap_or_default();
        let facts = ColumnFacts::from_analyzed_and_generated(analyzed_column, resolved_column, manifest, nulls)
            .map_err(|source| {
                let (fixture, engine) = err_ctx();
                RunnerError::ColumnFacts {
                    fixture,
                    engine,
                    source,
                }
            })?;
        column_facts.push(facts);
    }

    let facts = FixtureEngineFacts::new(fixture.name.clone(), engine, column_facts, row_count).map_err(|source| {
        let (fixture, engine) = err_ctx();
        RunnerError::Facts {
            fixture,
            engine,
            source,
        }
    })?;

    Ok(evaluate(&facts))
}
