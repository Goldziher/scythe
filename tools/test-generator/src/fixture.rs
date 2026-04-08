use ahash::{AHashMap, AHashSet};
use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Top-level fixture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Populated after loading -- the path on disk this fixture was read from.
    #[serde(skip)]
    pub file_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Config section
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Postgresql,
    Mysql,
    Sqlite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct NamingConfig {
    #[serde(default)]
    pub enum_style: Option<String>,
    #[serde(default)]
    pub row_suffix: Option<String>,
}

// ---------------------------------------------------------------------------
// Expected section
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
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

// -- Lint -------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedLint {
    #[serde(default)]
    pub violations: Vec<ExpectedLintViolation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedLintViolation {
    pub rule_code: String,
    #[serde(default)]
    pub message_contains: Option<String>,
}

// -- Catalog ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedCatalog {
    #[serde(default)]
    pub tables: AHashMap<String, ExpectedTable>,
    #[serde(default)]
    pub enums: AHashMap<String, ExpectedEnum>,
    #[serde(default)]
    pub composites: AHashMap<String, ExpectedComposite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedTable {
    pub columns: Vec<ExpectedColumn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct ExpectedEnum {
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedComposite {
    pub fields: Vec<ExpectedCompositeField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedCompositeField {
    pub name: String,
    pub sql_type: String,
}

// -- Query ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedQuery {
    pub name: String,
    pub command: Command,
    #[serde(default)]
    pub params: Vec<ExpectedParam>,
    #[serde(default)]
    pub columns: Vec<ExpectedReturnColumn>,
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
pub struct ExpectedParam {
    pub name: String,
    #[serde(rename = "type")]
    pub neutral_type: String,
    pub nullable: bool,
    #[serde(default)]
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedReturnColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub neutral_type: String,
    pub nullable: bool,
    #[serde(default)]
    pub note: Option<String>,
}

// -- Generated Code (keyed by backend name) ---------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedGeneratedCode {
    #[serde(default)]
    pub row_struct: Option<String>,
    #[serde(default)]
    pub query_fn: Option<String>,
    #[serde(default)]
    pub enum_def: Option<String>,
    #[serde(default)]
    pub model_struct: Option<String>,
}

// -- Error ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message_contains: Option<String>,
}

// ---------------------------------------------------------------------------
// Source & sqlc comparison
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Sqlc,
    Original,
    SqlcExpanded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlcComparison {
    #[serde(default)]
    pub sqlc_behavior: Option<String>,
    #[serde(default)]
    pub scythe_improvement: Option<String>,
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Recursively loads all `.json` fixture files from `dir`, excluding
/// `00-FIXTURE-SCHEMA.json`. Returns fixtures sorted by (category, name).
pub fn load_fixtures(dir: &Path) -> Result<Vec<Fixture>, Box<dyn std::error::Error>> {
    let pattern = dir
        .join("**/*.json")
        .to_str()
        .ok_or("non-UTF-8 path")?
        .to_string();

    let mut fixtures: Vec<Fixture> = Vec::new();

    for entry in glob::glob(&pattern)? {
        let path = entry?;

        // Skip the schema file itself.
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str())
            && file_name == "00-FIXTURE-SCHEMA.json"
        {
            continue;
        }

        let contents = std::fs::read_to_string(&path)?;
        let mut fixture: Fixture = serde_json::from_str(&contents)
            .map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
        fixture.file_path = Some(path.display().to_string());
        fixtures.push(fixture);
    }

    fixtures.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.name.cmp(&b.name))
    });

    // Detect duplicate fixture names (globally unique, not just adjacent).
    let mut seen = AHashSet::new();
    for fixture in &fixtures {
        if !seen.insert(&fixture.name) {
            // find the first occurrence for the error message
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
