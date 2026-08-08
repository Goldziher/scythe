//! The `LiveFixture` model: JSON fixtures under `testing_data/nullability_live/`
//! that describe a live-engine nullability scenario, plus the loader that
//! validates them before any container is touched.
//!
//! Every fixture file on disk also parses as a `tools/test-generator`
//! `Fixture` -- that tool has no `#[serde(deny_unknown_fields)]`, so the
//! `live` block this module reads is purely additive and requires zero
//! changes to that tool.

use std::path::{Path, PathBuf};

use ahash::{AHashMap, AHashSet};
use serde::Deserialize;

use crate::query_shape::{self, QueryShape};

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// One of the six database engines the live suite targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Postgresql,
    Mysql,
    Mariadb,
    Sqlite,
    Mssql,
    Oracle,
}

impl Engine {
    pub const ALL: [Engine; 6] = [
        Engine::Postgresql,
        Engine::Mysql,
        Engine::Mariadb,
        Engine::Sqlite,
        Engine::Mssql,
        Engine::Oracle,
    ];

    /// The key used for this engine in `seed` overrides and
    /// `engine_expectations`.
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Postgresql => "postgresql",
            Engine::Mysql => "mysql",
            Engine::Mariadb => "mariadb",
            Engine::Sqlite => "sqlite",
            Engine::Mssql => "mssql",
            Engine::Oracle => "oracle",
        }
    }

    /// The schema-profile filename for this engine, e.g. `postgresql.sql`.
    /// Every engine -- including mariadb -- resolves to its own literal
    /// file; there is deliberately no fallback to a "similar" engine.
    pub fn schema_file_name(self) -> String {
        format!("{}.sql", self.as_str())
    }

    /// The parser dialect this engine's DDL must be read with.
    ///
    /// Written as an exhaustive `match` rather than routed through
    /// [`scythe_core::dialect::SqlDialect::from_str`] with a fallback: that
    /// function returns `Option`, and any fallback for the `None` arm --
    /// `unwrap_or_default()` in particular -- would parse an unrecognised
    /// engine's DDL as PostgreSQL. In a crate whose whole purpose is to stop
    /// a green result being produced by examining the wrong thing, silently
    /// checking Oracle DDL against PostgreSQL's grammar is precisely the
    /// failure mode. Adding an `Engine` variant is a compile error here.
    pub fn dialect(self) -> scythe_core::dialect::SqlDialect {
        use scythe_core::dialect::SqlDialect;
        match self {
            Engine::Postgresql => SqlDialect::PostgreSQL,
            // MariaDB is wire- and grammar-compatible with MySQL for the DDL
            // subset these fixtures use; scythe has no separate MariaDB dialect.
            Engine::Mysql | Engine::Mariadb => SqlDialect::MySQL,
            Engine::Sqlite => SqlDialect::SQLite,
            Engine::Mssql => SqlDialect::MsSql,
            Engine::Oracle => SqlDialect::Oracle,
        }
    }
}

impl std::fmt::Display for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

/// A JSON fixture under `testing_data/nullability_live/<rule>/`.
#[derive(Debug, Clone, Deserialize)]
pub struct LiveFixture {
    pub name: String,
    pub category: String,
    /// The portable (`tools/test-generator`-flavored) DDL statements the
    /// analyzer sees. Deliberately distinct from `live.schema_profile`'s
    /// per-engine files under `_schemas/` -- the analyzer runs against
    /// this, the live engine runs against those, and reconciling the two
    /// is `crate::runner`'s job, not this module's (see
    /// `validate_schema_reconciliation`, which checks *declared columns*
    /// exist in the live schema, independent of this field).
    pub schema_sql: Vec<String>,
    pub query_sql: String,
    pub expected: ExpectedBlock,
    pub live: LiveBlock,
    /// Populated after loading -- the path on disk this fixture was read from.
    #[serde(skip)]
    pub file_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedBlock {
    pub query: ExpectedQuery,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedQuery {
    pub columns: Vec<ExpectedColumn>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExpectedColumn {
    pub name: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveBlock {
    pub schema_profile: String,
    pub engines: Vec<Engine>,
    pub runs: Vec<Run>,
    #[serde(default)]
    pub null_together: Vec<Vec<String>>,
    /// Legitimate per-engine semantic differences (e.g. Oracle collapsing
    /// `''` into `NULL`) -- not a place to explain away a failure. See
    /// [`crate::divergence`] for that.
    #[serde(default)]
    pub engine_expectations: AHashMap<Engine, EngineExpectation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Run {
    pub name: String,
    pub seed: SeedBlock,
    pub rows: Vec<RowExpectation>,
}

impl Run {
    /// Fails if the live engine returned a different number of rows than
    /// `resolved_rows` (the per-engine-resolved row expectations from
    /// [`crate::expectation::resolve_run_rows`], not necessarily
    /// `self.rows`: an `engine_expectations` override can declare a
    /// different row list for this run/engine). `resolved_rows` is matched
    /// ordinally against `query_sql`'s `ORDER BY`, so a count mismatch means
    /// the fixture (or its seed data) has drifted -- it is always a hard
    /// failure, never a skip.
    pub fn check_row_count(
        &self,
        resolved_rows: &[RowExpectation],
        observed_row_count: usize,
    ) -> Result<(), RowCountMismatch> {
        if resolved_rows.len() != observed_row_count {
            return Err(RowCountMismatch {
                run: self.name.clone(),
                declared: resolved_rows.len(),
                observed: observed_row_count,
            });
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("run {run:?} declares {declared} row(s) but the engine returned {observed}")]
pub struct RowCountMismatch {
    pub run: String,
    pub declared: usize,
    pub observed: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedBlock {
    #[serde(default)]
    pub default: Vec<String>,
    #[serde(flatten)]
    pub per_engine: AHashMap<String, Vec<String>>,
}

impl SeedBlock {
    /// Resolve the seed SQL statements for `engine`: a per-engine override
    /// if one is present, otherwise the shared `default`. `None` means
    /// neither yields any statements -- callers must treat that as a load
    /// error, never as "this engine has no seed, skip it".
    pub fn resolve(&self, engine: Engine) -> Option<&[String]> {
        if let Some(stmts) = self.per_engine.get(engine.as_str()) {
            return if stmts.is_empty() { None } else { Some(stmts) };
        }
        if self.default.is_empty() {
            None
        } else {
            Some(&self.default)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct RowExpectation {
    #[serde(default)]
    pub non_null: Vec<String>,
    #[serde(default)]
    pub null: Vec<String>,
}

impl RowExpectation {
    /// Whether `column` is declared null (`Some(true)`), declared non-null
    /// (`Some(false)`), or not mentioned in this row (`None`).
    pub fn declared_null(&self, column: &str) -> Option<bool> {
        if self.null.iter().any(|c| c == column) {
            Some(true)
        } else if self.non_null.iter().any(|c| c == column) {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EngineExpectation {
    pub reason: String,
    #[serde(default)]
    pub runs: AHashMap<String, Vec<RowExpectation>>,
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("reading fixture {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing fixture {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("glob pattern {pattern:?}: {source}")]
    Glob {
        pattern: String,
        #[source]
        source: glob::PatternError,
    },
    #[error("globbing under {path}: {source}")]
    GlobIter {
        path: PathBuf,
        #[source]
        source: glob::GlobError,
    },
    #[error("fixture {name:?} at {path}: live fixture names must start with 'live_'")]
    MissingLivePrefix { name: String, path: PathBuf },
    #[error("fixture {name:?} at {path}: could not parse query_sql to check for a top-level ORDER BY: {source}")]
    QueryParse {
        name: String,
        path: PathBuf,
        #[source]
        source: sqlparser::parser::ParserError,
    },
    #[error("fixture {name:?} at {path}: query_sql must contain a top-level ORDER BY -- rows are matched ordinally")]
    MissingOrderBy { name: String, path: PathBuf },
    #[error("fixture {name:?} at {path}: live.engines must not be empty -- an empty list asserts nothing")]
    EmptyEngines { name: String, path: PathBuf },
    #[error("fixture {name:?} at {path}: live.runs must not be empty -- an empty list asserts nothing")]
    EmptyRuns { name: String, path: PathBuf },
    #[error(
        "fixture {name:?} at {path}: expected.query.columns must not be empty -- an empty list makes every column-level assertion vacuous"
    )]
    EmptyColumns { name: String, path: PathBuf },
    #[error(
        "fixture {name:?} run {run:?} engine {engine}: resolved rows must not be empty -- an empty list asserts nothing"
    )]
    EmptyRows { name: String, run: String, engine: Engine },
    #[error(
        "fixture {name:?} run {run:?} engine {engine} row {row}: declared column {column:?} is not mentioned as null or non_null -- every declared column must appear in every row"
    )]
    ColumnMissingFromRow {
        name: String,
        run: String,
        engine: Engine,
        row: usize,
        column: String,
    },
    #[error("duplicate fixture name {name:?} in:\n  {first}\n  {second}")]
    DuplicateName {
        name: String,
        first: PathBuf,
        second: PathBuf,
    },
    #[error("fixture {name:?}: engine {engine} is listed but has no schema at {expected}")]
    MissingSchema {
        name: String,
        engine: Engine,
        expected: PathBuf,
    },
    #[error("fixture {name:?} run {run:?}: engine {engine} has no resolvable seed SQL")]
    MissingSeed { name: String, run: String, engine: Engine },
    #[error("fixture {name:?} run {run:?}: seed.{key:?} is not a recognized engine name (expected one of {valid:?})")]
    UnknownSeedEngine {
        name: String,
        run: String,
        key: String,
        valid: Vec<&'static str>,
    },
    #[error("fixture {name:?}: null_together group {group:?} must contain at least 2 columns")]
    NullTogetherGroupTooSmall { name: String, group: Vec<String> },
    #[error(
        "fixture {name:?}: null_together group {group:?} references {column:?}, which is not in expected.query.columns"
    )]
    NullTogetherUnknownColumn {
        name: String,
        group: Vec<String>,
        column: String,
    },
    #[error(
        "fixture {name:?} run {run:?} engine {engine} row {row}: null_together group {group:?} is split -- {column} disagrees with the rest of the group"
    )]
    NullTogetherViolation {
        name: String,
        run: String,
        engine: Engine,
        row: usize,
        group: Vec<String>,
        column: String,
    },
    #[error("fixture {name:?}: {source}")]
    Expectation {
        name: String,
        #[source]
        source: crate::expectation::ExpectationError,
    },
    #[error("fixture {name:?}: parsing live schema for engine {engine} at {path}: {source}")]
    LiveSchemaParse {
        name: String,
        engine: Engine,
        path: PathBuf,
        #[source]
        source: scythe_core::errors::ScytheError,
    },
    #[error(
        "fixture {name:?}: declared column {column:?} resolves to schema column {physical_column:?}, which does not exist in the live schema for engine {engine} at {path} -- the analyzer's schema_sql and the live schema have drifted apart"
    )]
    LiveSchemaColumnMissing {
        name: String,
        engine: Engine,
        column: String,
        physical_column: String,
        path: PathBuf,
    },
}

/// Recursively loads all `.json` fixtures under `fixtures_root`, validating
/// every rule this module enforces before returning. Every engine a fixture
/// lists must have resolvable DDL under `schemas_root` and resolvable seed
/// SQL for every run -- checked here, before any container is touched.
/// Returns fixtures sorted by `(category, name)`.
///
/// Any path component starting with `_` (e.g. `_schemas/`) *below
/// `fixtures_root`* is excluded from the glob, so a directory that holds
/// schema files or other non-fixture data can never be silently picked up
/// as a JSON fixture. The check is deliberately scoped to the path
/// *relative to `fixtures_root`* -- `fixtures_root` itself carries the full
/// filesystem path, including every ancestor directory, and checking those
/// too would silently discard every fixture whenever the checkout happens
/// to sit under a `_`-prefixed directory (e.g. `_work`, which is exactly
/// the prefix GitHub Actions self-hosted runners use).
pub fn load_fixtures(fixtures_root: &Path, schemas_root: &Path) -> Result<Vec<LiveFixture>, FixtureError> {
    let pattern = fixtures_root.join("**/*.json");
    let pattern = pattern.to_string_lossy().to_string();

    let mut fixtures = Vec::new();
    for entry in glob::glob(&pattern).map_err(|source| FixtureError::Glob {
        pattern: pattern.clone(),
        source,
    })? {
        let path = entry.map_err(|source| FixtureError::GlobIter {
            path: fixtures_root.to_path_buf(),
            source,
        })?;

        let relative = path.strip_prefix(fixtures_root).unwrap_or(&path);
        if relative
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('_'))
        {
            continue;
        }

        let contents = std::fs::read_to_string(&path).map_err(|source| FixtureError::Io {
            path: path.clone(),
            source,
        })?;
        let mut fixture: LiveFixture = serde_json::from_str(&contents).map_err(|source| FixtureError::Parse {
            path: path.clone(),
            source,
        })?;
        fixture.file_path = path.clone();
        validate_fixture(&fixture, schemas_root)?;
        fixtures.push(fixture);
    }

    fixtures.sort_by(|a, b| a.category.cmp(&b.category).then_with(|| a.name.cmp(&b.name)));

    let mut seen: AHashMap<String, PathBuf> = AHashMap::new();
    for fixture in &fixtures {
        if let Some(first) = seen.insert(fixture.name.clone(), fixture.file_path.clone()) {
            return Err(FixtureError::DuplicateName {
                name: fixture.name.clone(),
                first,
                second: fixture.file_path.clone(),
            });
        }
    }

    Ok(fixtures)
}

fn validate_fixture(fixture: &LiveFixture, schemas_root: &Path) -> Result<(), FixtureError> {
    if !fixture.name.starts_with("live_") {
        return Err(FixtureError::MissingLivePrefix {
            name: fixture.name.clone(),
            path: fixture.file_path.clone(),
        });
    }

    let shape = query_shape::parse(&fixture.query_sql).map_err(|source| FixtureError::QueryParse {
        name: fixture.name.clone(),
        path: fixture.file_path.clone(),
        source,
    })?;
    if !shape.has_order_by {
        return Err(FixtureError::MissingOrderBy {
            name: fixture.name.clone(),
            path: fixture.file_path.clone(),
        });
    }

    if fixture.live.engines.is_empty() {
        return Err(FixtureError::EmptyEngines {
            name: fixture.name.clone(),
            path: fixture.file_path.clone(),
        });
    }
    if fixture.live.runs.is_empty() {
        return Err(FixtureError::EmptyRuns {
            name: fixture.name.clone(),
            path: fixture.file_path.clone(),
        });
    }
    if fixture.expected.query.columns.is_empty() {
        return Err(FixtureError::EmptyColumns {
            name: fixture.name.clone(),
            path: fixture.file_path.clone(),
        });
    }

    for &engine in &fixture.live.engines {
        let schema_path = schemas_root
            .join(&fixture.live.schema_profile)
            .join(engine.schema_file_name());
        if !schema_path.is_file() {
            return Err(FixtureError::MissingSchema {
                name: fixture.name.clone(),
                engine,
                expected: schema_path,
            });
        }

        for run in &fixture.live.runs {
            if run.seed.resolve(engine).is_none() {
                return Err(FixtureError::MissingSeed {
                    name: fixture.name.clone(),
                    run: run.name.clone(),
                    engine,
                });
            }
        }
    }

    for run in &fixture.live.runs {
        for key in run.seed.per_engine.keys() {
            if !Engine::ALL.iter().any(|e| e.as_str() == key) {
                return Err(FixtureError::UnknownSeedEngine {
                    name: fixture.name.clone(),
                    run: run.name.clone(),
                    key: key.clone(),
                    valid: Engine::ALL.iter().map(|e| e.as_str()).collect(),
                });
            }
        }
    }

    crate::expectation::validate(&fixture.live).map_err(|source| FixtureError::Expectation {
        name: fixture.name.clone(),
        source,
    })?;

    validate_row_coverage(fixture)?;
    validate_null_together(fixture)?;
    validate_schema_reconciliation(fixture, schemas_root, &shape)?;

    Ok(())
}

/// Rejects an empty resolved row list for any (run, engine) pair, and
/// requires every declared column (`expected.query.columns`) to be
/// mentioned -- as null or non-null -- in every resolved row. A row that
/// doesn't mention a declared column asserts nothing about it, which is
/// exactly the fixture-level vacuity this crate exists to prevent (A3
/// already catches this at the column level; this catches it at the
/// fixture level).
fn validate_row_coverage(fixture: &LiveFixture) -> Result<(), FixtureError> {
    let declared: Vec<&str> = fixture.expected.query.columns.iter().map(|c| c.name.as_str()).collect();

    for &engine in &fixture.live.engines {
        for run in &fixture.live.runs {
            let rows = crate::expectation::resolve_run_rows(&fixture.live, run, engine);
            if rows.is_empty() {
                return Err(FixtureError::EmptyRows {
                    name: fixture.name.clone(),
                    run: run.name.clone(),
                    engine,
                });
            }
            for (row_idx, row) in rows.iter().enumerate() {
                for &column in &declared {
                    if row.declared_null(column).is_none() {
                        return Err(FixtureError::ColumnMissingFromRow {
                            name: fixture.name.clone(),
                            run: run.name.clone(),
                            engine,
                            row: row_idx,
                            column: column.to_string(),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Validates `live.null_together`: every group must have at least 2
/// columns (a group of 1 validates trivially and asserts nothing), every
/// column named must be a declared column (an unknown/typo'd name would
/// otherwise validate forever, since [`RowExpectation::declared_null`]
/// returns `None` for it and the old check just skipped `None`s), and the
/// coherence check itself runs against each engine's *resolved* rows
/// (honoring `engine_expectations` overrides) rather than only the base
/// `run.rows` -- an override used to bypass this check entirely.
fn validate_null_together(fixture: &LiveFixture) -> Result<(), FixtureError> {
    let declared: AHashSet<&str> = fixture.expected.query.columns.iter().map(|c| c.name.as_str()).collect();

    for group in &fixture.live.null_together {
        if group.len() < 2 {
            return Err(FixtureError::NullTogetherGroupTooSmall {
                name: fixture.name.clone(),
                group: group.clone(),
            });
        }
        for column in group {
            if !declared.contains(column.as_str()) {
                return Err(FixtureError::NullTogetherUnknownColumn {
                    name: fixture.name.clone(),
                    group: group.clone(),
                    column: column.clone(),
                });
            }
        }

        for &engine in &fixture.live.engines {
            for run in &fixture.live.runs {
                let rows = crate::expectation::resolve_run_rows(&fixture.live, run, engine);
                for (row_idx, row) in rows.iter().enumerate() {
                    let mut declared_for_row: Option<(bool, &str)> = None;
                    for column in group {
                        let Some(is_null) = row.declared_null(column) else {
                            continue;
                        };
                        match declared_for_row {
                            None => declared_for_row = Some((is_null, column)),
                            Some((expected, _)) if expected != is_null => {
                                return Err(FixtureError::NullTogetherViolation {
                                    name: fixture.name.clone(),
                                    run: run.name.clone(),
                                    engine,
                                    row: row_idx,
                                    group: group.clone(),
                                    column: column.clone(),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Reconciles the analyzer-facing `schema_sql` implicitly used by the query
/// with the live `_schemas/<profile>/<engine>.sql` this fixture actually
/// runs against. Without this, nothing stops the two from drifting: the
/// analyzer would be checked against one schema while the live run checks
/// against another, which is the exact failure this crate exists to detect
/// -- inverted.
///
/// For each declared column (`expected.query.columns`, plus anything named
/// in row expectations), `shape` is used to find the physical source
/// column it projects (a direct `alias.column` or bare `column` reference).
/// Computed/expression columns and columns this best-effort SQL walk can't
/// resolve to a single physical column are skipped -- a false "missing
/// column" on an expression would be worse than not checking it. Every
/// resolvable column must exist, under that name, somewhere in the live
/// per-engine schema.
fn validate_schema_reconciliation(
    fixture: &LiveFixture,
    schemas_root: &Path,
    shape: &QueryShape,
) -> Result<(), FixtureError> {
    let mut declared: Vec<&str> = fixture.expected.query.columns.iter().map(|c| c.name.as_str()).collect();
    for run in &fixture.live.runs {
        for row in &run.rows {
            for column in row.non_null.iter().chain(row.null.iter()) {
                if !declared.contains(&column.as_str()) {
                    declared.push(column.as_str());
                }
            }
        }
    }

    for &engine in &fixture.live.engines {
        let schema_path = schemas_root
            .join(&fixture.live.schema_profile)
            .join(engine.schema_file_name());
        let contents = std::fs::read_to_string(&schema_path).map_err(|source| FixtureError::Io {
            path: schema_path.clone(),
            source,
        })?;
        let dialect = engine.dialect();
        let catalog =
            scythe_core::catalog::Catalog::from_ddl_with_dialect(&[contents.as_str()], &dialect).map_err(|source| {
                FixtureError::LiveSchemaParse {
                    name: fixture.name.clone(),
                    engine,
                    path: schema_path.clone(),
                    source,
                }
            })?;

        for &column in &declared {
            let Some(source) = shape.source_column(column) else {
                continue;
            };
            let found = query_shape::column_exists(&catalog, shape, source);
            if !found {
                return Err(FixtureError::LiveSchemaColumnMissing {
                    name: fixture.name.clone(),
                    engine,
                    column: column.to_string(),
                    physical_column: source.column.to_string(),
                    path: schema_path,
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_schema(schemas_root: &Path, profile: &str, engine: Engine) {
        let dir = schemas_root.join(profile);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(engine.schema_file_name()), "CREATE TABLE t (id INT);").unwrap();
    }

    fn write_fixture(fixtures_root: &Path, file_name: &str, contents: &str) {
        std::fs::write(fixtures_root.join(file_name), contents).unwrap();
    }

    fn minimal_fixture_json(name: &str, query_sql: &str, engines: &str, seed_default: &str) -> String {
        format!(
            r#"{{
  "name": "{name}",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "{query_sql}",
  "expected": {{ "query": {{ "columns": [{{ "name": "id", "nullable": false }}] }} }},
  "live": {{
    "schema_profile": "profile",
    "engines": [{engines}],
    "runs": [
      {{
        "name": "run1",
        "seed": {{ "default": [{seed_default}] }},
        "rows": [{{ "non_null": ["id"], "null": [] }}]
      }}
    ]
  }}
}}"#
        )
    }

    // -- SeedBlock::resolve ---------------------------------------------

    #[test]
    fn seed_resolve_prefers_engine_override_over_default() {
        let seed = SeedBlock {
            default: vec!["INSERT default".to_string()],
            per_engine: [("mssql".to_string(), vec!["INSERT mssql".to_string()])]
                .into_iter()
                .collect(),
        };
        assert_eq!(seed.resolve(Engine::Mssql), Some(&["INSERT mssql".to_string()][..]));
    }

    #[test]
    fn seed_resolve_falls_back_to_default() {
        let seed = SeedBlock {
            default: vec!["INSERT default".to_string()],
            per_engine: Default::default(),
        };
        assert_eq!(
            seed.resolve(Engine::Postgresql),
            Some(&["INSERT default".to_string()][..])
        );
    }

    #[test]
    fn seed_resolve_returns_none_when_neither_is_present() {
        let seed = SeedBlock {
            default: vec![],
            per_engine: Default::default(),
        };
        assert_eq!(seed.resolve(Engine::Postgresql), None);
    }

    #[test]
    fn seed_resolve_treats_an_empty_override_as_unresolved() {
        let seed = SeedBlock {
            default: vec!["INSERT default".to_string()],
            per_engine: [("mssql".to_string(), vec![])].into_iter().collect(),
        };
        assert_eq!(
            seed.resolve(Engine::Mssql),
            None,
            "an explicit empty override must not silently fall back"
        );
    }

    // -- RowExpectation::declared_null ------------------------------------

    #[test]
    fn row_expectation_declared_null_distinguishes_null_non_null_and_unmentioned() {
        let row = RowExpectation {
            non_null: vec!["id".to_string()],
            null: vec!["total".to_string()],
        };
        assert_eq!(row.declared_null("id"), Some(false));
        assert_eq!(row.declared_null("total"), Some(true));
        assert_eq!(row.declared_null("notes"), None);
    }

    // -- Engine ------------------------------------------------------------

    #[test]
    fn engine_schema_file_name_is_engine_dot_sql() {
        assert_eq!(Engine::Postgresql.schema_file_name(), "postgresql.sql");
        assert_eq!(Engine::Mariadb.schema_file_name(), "mariadb.sql");
    }

    // -- load_fixtures: validation rules -----------------------------------

    #[test]
    fn load_fixtures_rejects_name_without_live_prefix() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json(
                "not_prefixed",
                "SELECT id FROM t ORDER BY id",
                r#""postgresql""#,
                r#""INSERT""#,
            ),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(
            matches!(result, Err(FixtureError::MissingLivePrefix { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn load_fixtures_rejects_query_without_order_by() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json("live_x", "SELECT id FROM t", r#""postgresql""#, r#""INSERT""#),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::MissingOrderBy { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_a_query_that_fails_to_parse() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json("live_x", "SELECT FROM WHERE ORDER BY", r#""postgresql""#, r#""INSERT""#),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::QueryParse { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_an_order_by_that_only_appears_inside_a_window_function() {
        // A substring match on "ORDER BY" would be satisfied by
        // `ROW_NUMBER() OVER (ORDER BY id)`, but that's not a top-level
        // ORDER BY -- row order (and therefore ordinal row matching) is
        // still undefined.
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json(
                "live_x",
                "SELECT id, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t",
                r#""postgresql""#,
                r#""INSERT""#,
            ),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::MissingOrderBy { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_accepts_an_order_by_split_across_lines() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json(
                "live_x",
                "SELECT id FROM t ORDER\\n  BY id",
                r#""postgresql""#,
                r#""INSERT""#,
            ),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_engine_with_missing_schema_file() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        // Deliberately do not write a schema file.
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json(
                "live_x",
                "SELECT id FROM t ORDER BY id",
                r#""postgresql""#,
                r#""INSERT""#,
            ),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::MissingSchema { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_run_with_no_resolvable_seed() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json("live_x", "SELECT id FROM t ORDER BY id", r#""postgresql""#, ""),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::MissingSeed { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_duplicate_names() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = minimal_fixture_json(
            "live_dup",
            "SELECT id FROM t ORDER BY id",
            r#""postgresql""#,
            r#""INSERT""#,
        );
        write_fixture(fixtures_root.path(), "a.json", &json);
        write_fixture(fixtures_root.path(), "b.json", &json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::DuplicateName { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_accepts_a_well_formed_fixture() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_schema(schemas_root.path(), "profile", Engine::Mssql);
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json(
                "live_ok",
                "SELECT id FROM t ORDER BY id",
                r#""postgresql", "mssql""#,
                r#""INSERT INTO t VALUES (1)""#,
            ),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path()).expect("well-formed fixture must load");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "live_ok");
    }

    #[test]
    fn load_fixtures_excludes_underscore_prefixed_directories_from_the_glob() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json(
                "live_ok",
                "SELECT id FROM t ORDER BY id",
                r#""postgresql""#,
                r#""INSERT""#,
            ),
        );
        let underscore_dir = fixtures_root.path().join("_schemas");
        std::fs::create_dir_all(&underscore_dir).unwrap();
        // Not a valid LiveFixture -- if the glob ever picked this up, load
        // would fail to parse it.
        std::fs::write(underscore_dir.join("not_a_fixture.json"), "{}").unwrap();

        let result = load_fixtures(fixtures_root.path(), schemas_root.path()).expect("underscore dir must be excluded");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn load_fixtures_loads_normally_when_an_ancestor_of_fixtures_root_starts_with_underscore() {
        // The exclusion check must only look at path components *below*
        // fixtures_root. `dir.join("**/*.json")` glob results carry the
        // full path, ancestors included -- checking the whole path would
        // silently discard every fixture whenever the checkout sits under
        // a `_`-prefixed directory (e.g. `_work`, the exact prefix GitHub
        // Actions self-hosted runners use), which is precisely the
        // silently-examined-nothing failure this crate exists to catch.
        let parent = tempfile::tempdir().unwrap();
        let underscore_ancestor = parent.path().join("_work");
        let fixtures_root = underscore_ancestor.join("testing_data");
        std::fs::create_dir_all(&fixtures_root).unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        write_fixture(
            &fixtures_root,
            "f.json",
            &minimal_fixture_json(
                "live_ok",
                "SELECT id FROM t ORDER BY id",
                r#""postgresql""#,
                r#""INSERT""#,
            ),
        );

        let result = load_fixtures(&fixtures_root, schemas_root.path())
            .expect("an underscore-prefixed ancestor of fixtures_root must not suppress fixtures");
        assert_eq!(
            result.len(),
            1,
            "fixtures_root sits under an underscore-prefixed directory but must still load"
        );
    }

    // -- validate_fixture: emptiness rejections ----------------------------

    #[test]
    fn load_fixtures_rejects_empty_engines() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_fixture(
            fixtures_root.path(),
            "f.json",
            &minimal_fixture_json("live_x", "SELECT id FROM t ORDER BY id", "", r#""INSERT""#),
        );
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::EmptyEngines { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_empty_columns() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id FROM t ORDER BY id",
  "expected": { "query": { "columns": [] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      { "name": "run1", "seed": { "default": ["INSERT"] }, "rows": [{ "non_null": ["id"], "null": [] }] }
    ]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::EmptyColumns { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_empty_runs() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id FROM t ORDER BY id",
  "expected": { "query": { "columns": [{ "name": "id", "nullable": false }] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": []
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::EmptyRuns { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_empty_rows() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id FROM t ORDER BY id",
  "expected": { "query": { "columns": [{ "name": "id", "nullable": false }] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      { "name": "run1", "seed": { "default": ["INSERT"] }, "rows": [] }
    ]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(matches!(result, Err(FixtureError::EmptyRows { .. })), "{result:?}");
    }

    #[test]
    fn load_fixtures_rejects_a_row_missing_a_declared_column() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id, total FROM t ORDER BY id",
  "expected": { "query": { "columns": [{ "name": "id", "nullable": false }, { "name": "total", "nullable": true }] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      { "name": "run1", "seed": { "default": ["INSERT"] }, "rows": [{ "non_null": ["id"], "null": [] }] }
    ]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(
            matches!(result, Err(FixtureError::ColumnMissingFromRow { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn load_fixtures_rejects_an_unknown_seed_engine_key() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id FROM t ORDER BY id",
  "expected": { "query": { "columns": [{ "name": "id", "nullable": false }] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      { "name": "run1", "seed": { "default": ["INSERT"], "postgres": ["INSERT wrong key"] }, "rows": [{ "non_null": ["id"], "null": [] }] }
    ]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(
            matches!(result, Err(FixtureError::UnknownSeedEngine { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn load_fixtures_rejects_a_null_together_group_that_disagrees_on_a_row() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = r#"{
  "name": "live_incoherent",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id, a, b FROM t ORDER BY id",
  "expected": { "query": { "columns": [
    { "name": "id", "nullable": false },
    { "name": "a", "nullable": true },
    { "name": "b", "nullable": true }
  ] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      {
        "name": "run1",
        "seed": { "default": ["INSERT"] },
        "rows": [{ "non_null": ["id", "a"], "null": ["b"] }]
      }
    ],
    "null_together": [["a", "b"]]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(
            matches!(result, Err(FixtureError::NullTogetherViolation { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn load_fixtures_rejects_a_null_together_group_of_one_column() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id FROM t ORDER BY id",
  "expected": { "query": { "columns": [{ "name": "id", "nullable": false }] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      { "name": "run1", "seed": { "default": ["INSERT"] }, "rows": [{ "non_null": ["id"], "null": [] }] }
    ],
    "null_together": [["id"]]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(
            matches!(result, Err(FixtureError::NullTogetherGroupTooSmall { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn load_fixtures_rejects_a_null_together_group_with_an_unknown_column() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        write_schema(schemas_root.path(), "profile", Engine::Postgresql);
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id, a FROM t ORDER BY id",
  "expected": { "query": { "columns": [{ "name": "id", "nullable": false }, { "name": "a", "nullable": true }] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      { "name": "run1", "seed": { "default": ["INSERT"] }, "rows": [{ "non_null": ["id", "a"], "null": [] }] }
    ],
    "null_together": [["a", "totally_typoed_column"]]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(
            matches!(result, Err(FixtureError::NullTogetherUnknownColumn { .. })),
            "{result:?}"
        );
    }

    // -- validate_schema_reconciliation ------------------------------------

    #[test]
    fn load_fixtures_rejects_a_column_missing_from_the_live_schema() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        let dir = schemas_root.path().join("profile");
        std::fs::create_dir_all(&dir).unwrap();
        // The live schema is missing `total`, which the query selects.
        std::fs::write(dir.join("postgresql.sql"), "CREATE TABLE t (id INT NOT NULL);").unwrap();
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id, total FROM t ORDER BY id",
  "expected": { "query": { "columns": [{ "name": "id", "nullable": false }, { "name": "total", "nullable": true }] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      { "name": "run1", "seed": { "default": ["INSERT"] }, "rows": [{ "non_null": ["id"], "null": ["total"] }] }
    ]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(
            matches!(result, Err(FixtureError::LiveSchemaColumnMissing { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn load_fixtures_accepts_an_aliased_column_that_resolves_to_a_real_schema_column() {
        let fixtures_root = tempfile::tempdir().unwrap();
        let schemas_root = tempfile::tempdir().unwrap();
        let dir = schemas_root.path().join("profile");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("postgresql.sql"),
            "CREATE TABLE users (id INT NOT NULL); CREATE TABLE orders (id INT NOT NULL, created_at TIMESTAMPTZ);",
        )
        .unwrap();
        let json = r#"{
  "name": "live_x",
  "category": "nullability_live/test",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT u.id, o.created_at AS order_created_at FROM users u LEFT JOIN orders o ON u.id = o.id ORDER BY u.id",
  "expected": { "query": { "columns": [{ "name": "id", "nullable": false }, { "name": "order_created_at", "nullable": true }] } },
  "live": {
    "schema_profile": "profile",
    "engines": ["postgresql"],
    "runs": [
      { "name": "run1", "seed": { "default": ["INSERT"] }, "rows": [{ "non_null": ["id"], "null": ["order_created_at"] }] }
    ]
  }
}"#;
        write_fixture(fixtures_root.path(), "f.json", json);
        let result = load_fixtures(fixtures_root.path(), schemas_root.path());
        assert!(result.is_ok(), "{result:?}");
    }

    // -- integration: the committed testing_data/nullability_live tree ------

    #[test]
    fn load_fixtures_loads_the_committed_nullability_live_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing_data/nullability_live");
        let schemas_root = root.join("_schemas");
        let fixtures = load_fixtures(&root, &schemas_root).expect("committed fixtures must load cleanly");
        assert!(!fixtures.is_empty(), "expected at least one committed live fixture");
        for fixture in &fixtures {
            assert!(fixture.name.starts_with("live_"));
        }
    }
}
