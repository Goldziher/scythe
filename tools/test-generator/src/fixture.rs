use ahash::{AHashMap, AHashSet};
use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub name: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,

    pub schema_sql: Vec<String>,
    #[serde(default)]
    pub query_sql: Option<String>,

    #[serde(default)]
    pub config: Option<FixtureConfig>,

    pub expected: Expected,

    pub source: Source,

    #[serde(default)]
    pub sqlc_comparison: Option<SqlcComparison>,

    /// The `testing_data/nullability_live/**` seed/run corpus (`schema_profile`, `engines`,
    /// `runs`). Accepted, not modeled or consumed: no code in this repo reads it yet -- a live
    /// fixture runner is separate, larger work than the typo-tolerance fix this field exists to
    /// unblock. Kept opaque (`Value`, not a dedicated struct) on purpose, so `deny_unknown_fields`
    /// on `Fixture` doesn't reject these 16 fixtures while that runner doesn't exist. See #156.
    #[serde(default)]
    pub live: Option<serde_json::Value>,

    /// Populated after loading -- the path on disk this fixture was read from.
    #[serde(skip)]
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureConfig {
    #[serde(default)]
    pub engine: Option<Engine>,
    #[serde(default, rename = "gen")]
    pub generation: Option<GenConfig>,
    #[serde(default)]
    pub type_overrides: Option<Vec<TypeOverride>>,
    #[serde(default)]
    pub naming: Option<NamingConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Postgresql,
    Mysql,
    Sqlite,
    Mssql,
    Oracle,
    Mariadb,
    Redshift,
    Snowflake,
    Duckdb,
}

impl Engine {
    /// The `scythe.toml` engine string this variant corresponds to, which is
    /// what `SqlDialect::from_str` and `Catalog::with_engine` consume.
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Postgresql => "postgresql",
            Engine::Mysql => "mysql",
            Engine::Sqlite => "sqlite",
            Engine::Mssql => "mssql",
            Engine::Oracle => "oracle",
            Engine::Mariadb => "mariadb",
            Engine::Redshift => "redshift",
            Engine::Snowflake => "snowflake",
            Engine::Duckdb => "duckdb",
        }
    }

    /// The `SqlDialect` variant path to emit, or `None` when the engine is
    /// PostgreSQL proper.
    ///
    /// `None` keeps the generated test on the plain `Catalog::from_ddl` call
    /// it has always used, so adding engine awareness leaves every
    /// PostgreSQL fixture's generated test byte-identical.
    pub fn dialect_path(self) -> Option<&'static str> {
        match self {
            Engine::Postgresql => None,
            Engine::Mysql | Engine::Mariadb => Some("MySQL"),
            Engine::Sqlite => Some("SQLite"),
            Engine::Mssql => Some("MsSql"),
            Engine::Oracle => Some("Oracle"),
            Engine::Snowflake => Some("Snowflake"),
            // Redshift and DuckDB parse as PostgreSQL; only the engine name
            // distinguishes them, and it is what gates nested aggregates.
            Engine::Redshift | Engine::Duckdb => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenConfig {
    #[serde(default)]
    pub target: Option<GenTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenTarget {
    Sqlx,
    TokioPostgres,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeOverride {
    #[serde(default)]
    pub db_type: Option<String>,
    #[serde(default)]
    pub lang_type: Option<String>,
    #[serde(default, rename = "type")]
    pub neutral_type: Option<String>,
    #[serde(default)]
    pub column: Option<String>,
    #[serde(default)]
    pub json: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingConfig {
    #[serde(default)]
    pub enum_style: Option<String>,
    #[serde(default)]
    pub row_suffix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub success: bool,
    #[serde(default)]
    pub catalog: Option<ExpectedCatalog>,
    #[serde(default)]
    pub query: Option<ExpectedQuery>,
    #[serde(default)]
    pub generated_code: Option<AHashMap<String, ExpectedGeneratedCode>>,
    #[serde(default)]
    pub error: Option<ExpectedError>,
    #[serde(default)]
    pub lint: Option<ExpectedLint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedLint {
    #[serde(default)]
    pub violations: Vec<ExpectedLintViolation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedLintViolation {
    pub rule_code: String,
    #[serde(default)]
    pub message_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedCatalog {
    #[serde(default)]
    pub tables: AHashMap<String, ExpectedTable>,
    #[serde(default)]
    pub enums: AHashMap<String, ExpectedEnum>,
    #[serde(default)]
    pub composites: AHashMap<String, ExpectedComposite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedTable {
    pub columns: Vec<ExpectedColumn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedColumn {
    pub name: String,
    pub sql_type: String,
    pub nullable: bool,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub primary_key: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedEnum {
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedComposite {
    pub fields: Vec<ExpectedCompositeField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedCompositeField {
    pub name: String,
    pub sql_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedQuery {
    pub name: String,
    pub command: Command,
    #[serde(default)]
    pub params: Vec<ExpectedParam>,
    #[serde(default)]
    pub columns: Vec<ExpectedReturnColumn>,
    /// Nested-aggregate struct definitions (`json_agg`, `row_to_json`).
    ///
    /// `Option`, not a plain `Vec`: an omitted key means "this fixture says
    /// nothing about nested structs" and asserts nothing, while an explicit
    /// `[]` asserts there are none — which is the whole point of the
    /// dialect-gate and `array_agg`/`string_agg` guardrail fixtures.
    #[serde(default)]
    pub nested_structs: Option<Vec<ExpectedNestedStruct>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedNestedStruct {
    /// snake_case name, as it appears in `NestedStructInfo::name`.
    pub name: String,
    #[serde(default)]
    pub fields: Vec<ExpectedNestedField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedNestedField {
    /// The *raw* SQL column name, which is also the JSON key.
    pub name: String,
    #[serde(rename = "type")]
    pub neutral_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    One,
    Many,
    Exec,
    ExecResult,
    ExecRows,
    Batch,
    Grouped,
}

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Command::One => "one",
            Command::Many => "many",
            Command::Exec => "exec",
            Command::ExecResult => "exec_result",
            Command::ExecRows => "exec_rows",
            Command::Batch => "batch",
            Command::Grouped => "grouped",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedParam {
    pub name: String,
    #[serde(rename = "type")]
    pub neutral_type: String,
    pub nullable: bool,
    #[serde(default)]
    pub position: Option<i64>,
    /// Optional note explaining type/nullability reasoning, mirroring
    /// `ExpectedReturnColumn::note`. Documentation only -- no assertion reads it.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedReturnColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub neutral_type: String,
    pub nullable: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedGeneratedCode {
    #[serde(default)]
    pub row_struct: Option<String>,
    #[serde(default)]
    pub query_fn: Option<String>,
    #[serde(default)]
    pub enum_def: Option<String>,
    #[serde(default)]
    pub model_struct: Option<String>,
    /// Expected `scythe_codegen::GeneratedCode::degraded_nested_structs` entries for
    /// this backend -- the typed record `degrade_unsupported_nested_structs` produces
    /// when a nested-aggregate column (`json_agg`, `row_to_json`) could not become a
    /// real struct for this backend (GH #147).
    ///
    /// `Option`, not a plain `Vec`, for the same reason as
    /// `ExpectedQuery::nested_structs`: an omitted key asserts nothing, while an
    /// explicit `[]` asserts the backend constructed a real nested struct (no
    /// degradation) -- distinct from silence, and the case a string-content
    /// assertion on `row_struct`/`model_struct` cannot express without pinning that
    /// backend's full rendered syntax.
    #[serde(default)]
    pub degraded_nested_structs: Option<Vec<ExpectedNestedStructDegradation>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedNestedStructDegradation {
    /// SQL-level name of the rewritten column, matches
    /// `scythe_codegen::NestedStructDegradation::column`.
    pub column: String,
    /// PascalCase name of the struct the backend declined to generate, matches
    /// `scythe_codegen::NestedStructDegradation::struct_name`.
    pub struct_name: String,
    /// The neutral type the column was rewritten to: `"json"` or `"json_array"`.
    pub fallback_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Sqlc,
    Original,
    SqlcExpanded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SqlcComparison {
    #[serde(default)]
    pub sqlc_behavior: Option<String>,
    #[serde(default)]
    pub scythe_improvement: Option<String>,
}

/// Recursively loads all `.json` fixture files from `dir`, excluding
/// `00-FIXTURE-SCHEMA.json` and any path with an `_`-prefixed directory
/// component *below `dir`* (e.g. `_schemas/`, used by `scythe-conformance`
/// to hold non-fixture data alongside its fixtures) -- such a directory
/// must never be silently picked up as a JSON fixture. The exclusion check
/// is scoped to the path relative to `dir`: `dir` itself carries the full
/// filesystem path, including every ancestor, and checking those too would
/// silently discard every fixture whenever the checkout happens to sit
/// under a `_`-prefixed directory (e.g. `_work`, the prefix GitHub Actions
/// self-hosted runners use). Returns fixtures sorted by (category, name).
pub fn load_fixtures(dir: &Path) -> Result<Vec<Fixture>, Box<dyn std::error::Error>> {
    let pattern = dir.join("**/*.json").to_str().ok_or("non-UTF-8 path")?.to_string();

    let mut fixtures: Vec<Fixture> = Vec::new();

    for entry in glob::glob(&pattern)? {
        let path = entry?;

        if let Some(file_name) = path.file_name().and_then(|n| n.to_str())
            && file_name == "00-FIXTURE-SCHEMA.json"
        {
            continue;
        }

        let relative = path.strip_prefix(dir).unwrap_or(&path);
        if relative
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('_'))
        {
            continue;
        }

        let contents = std::fs::read_to_string(&path)?;
        let mut fixture: Fixture =
            serde_json::from_str(&contents).map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
        fixture.file_path = Some(path.display().to_string());
        fixtures.push(fixture);
    }

    fixtures.sort_by(|a, b| a.category.cmp(&b.category).then_with(|| a.name.cmp(&b.name)));

    let mut seen = AHashSet::new();
    for fixture in &fixtures {
        if !seen.insert(&fixture.name) {
            let first_path = fixtures
                .iter()
                .find(|f| f.name == fixture.name && f.file_path != fixture.file_path)
                .and_then(|f| f.file_path.as_deref())
                .unwrap_or("unknown");
            return Err(format!(
                "duplicate fixture name {:?} in:\n  {}\n  {}",
                fixture.name,
                first_path,
                fixture.file_path.as_deref().unwrap_or("?"),
            )
            .into());
        }
    }

    Ok(fixtures)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(dir: &Path, file_name: &str, name: &str) {
        let json = format!(
            r#"{{
  "name": "{name}",
  "description": "d",
  "category": "smoke",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "expected": {{ "success": true }},
  "source": "original"
}}"#
        );
        std::fs::write(dir.join(file_name), json).unwrap();
    }

    #[test]
    fn load_fixtures_finds_a_well_formed_fixture() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path(), "f.json", "smoke_test");
        let fixtures = load_fixtures(dir.path()).expect("well-formed fixture must load");
        assert_eq!(fixtures.len(), 1);
        assert_eq!(fixtures[0].name, "smoke_test");
    }

    #[test]
    fn load_fixtures_excludes_an_underscore_prefixed_directory_below_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path(), "f.json", "smoke_test");
        let underscore_dir = dir.path().join("_schemas");
        std::fs::create_dir_all(&underscore_dir).unwrap();
        // Not a valid Fixture -- if the glob ever picked this up, load
        // would fail to parse it.
        std::fs::write(underscore_dir.join("not_a_fixture.json"), "{}").unwrap();

        let fixtures = load_fixtures(dir.path()).expect("underscore dir must be excluded");
        assert_eq!(fixtures.len(), 1);
    }

    #[test]
    fn load_fixtures_loads_normally_when_an_ancestor_of_dir_starts_with_underscore() {
        // ~keep The exclusion check must only look at path components *below*
        // `dir`. `dir.join("**/*.json")` glob results carry the full path,
        // ancestors included -- checking the whole path would silently
        // discard every fixture whenever the checkout sits under a
        // `_`-prefixed directory (e.g. `_work`, the exact prefix GitHub
        // Actions self-hosted runners use), which would make every
        // generated test file empty while reporting success.
        let parent = tempfile::tempdir().unwrap();
        let underscore_ancestor = parent.path().join("_work");
        let fixtures_dir = underscore_ancestor.join("testing_data");
        std::fs::create_dir_all(&fixtures_dir).unwrap();
        write_fixture(&fixtures_dir, "f.json", "smoke_test");

        let fixtures =
            load_fixtures(&fixtures_dir).expect("an underscore-prefixed ancestor of dir must not suppress fixtures");
        assert_eq!(
            fixtures.len(),
            1,
            "dir sits under an underscore-prefixed directory but must still load"
        );
    }

    /// Regression for #156: before `#[serde(deny_unknown_fields)]`, misspelling
    /// `expected.query.columns` as `column` silently deserialised to the empty default and the
    /// generated test asserted no column type or nullability at all. It must now be a load
    /// error naming the offending key, not a fixture that loads and tests nothing.
    #[test]
    fn load_fixtures_rejects_a_misspelled_expected_query_key() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{
  "name": "typo_test",
  "description": "d",
  "category": "smoke",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "query_sql": "SELECT id FROM t",
  "expected": {
    "success": true,
    "query": { "name": "GetT", "command": "many", "column": [] }
  },
  "source": "original"
}"#;
        std::fs::write(dir.path().join("f.json"), json).unwrap();

        let error = load_fixtures(dir.path())
            .expect_err("a misspelled `column` key must be rejected, not silently dropped")
            .to_string();
        assert!(
            error.contains("column"),
            "error must name the offending key, got: {error}"
        );
    }

    /// The `testing_data/nullability_live/**` corpus carries a top-level `live` block that no
    /// code in this repo consumes yet (see `Fixture::live`'s doc comment). It must still load
    /// under `deny_unknown_fields` rather than be treated as an unrecognised key.
    #[test]
    fn load_fixtures_accepts_a_live_block_without_rejecting_it() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{
  "name": "live_test",
  "description": "d",
  "category": "smoke",
  "schema_sql": ["CREATE TABLE t (id INT)"],
  "expected": { "success": true },
  "source": "original",
  "live": { "schema_profile": "x", "engines": ["postgresql"], "runs": [] }
}"#;
        std::fs::write(dir.path().join("f.json"), json).unwrap();

        let fixtures = load_fixtures(dir.path()).expect("a declared `live` block must not be rejected");
        assert_eq!(fixtures.len(), 1);
        assert!(
            fixtures[0].live.is_some(),
            "the `live` block must round-trip, not be dropped"
        );
    }
}
