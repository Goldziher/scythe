use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use minijinja::Environment;
use scythe_codegen::backends::get_backend;
use serde::{Deserialize, Serialize};

/// Integration test generator for scythe.
///
/// Renders language-specific integration tests from minijinja templates.
/// Each backend produces: scythe.toml, a test file, and a dependency file.
#[derive(Parser, Debug)]
#[command(name = "integration-test-generator", version, about)]
struct Cli {
    /// Output directory for generated integration test directories.
    #[arg(long, default_value = "integration_tests")]
    output: PathBuf,

    /// Directory containing minijinja templates.
    #[arg(long, default_value = "tools/integration-test-generator/templates")]
    templates: PathBuf,

    /// Skip backends whose output directory already exists.
    #[arg(long)]
    skip_existing: bool,

    /// Only generate for these backends (comma-separated). If empty, generate all.
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    /// Print backend directory names (one per line) and exit without generating anything.
    ///
    /// This is the manifest of record for `build_backends()` — consumers such as
    /// `integration_tests/Taskfile.yaml` should derive their backend lists from this
    /// output instead of hand-maintaining a copy, so the two can never drift.
    #[arg(long)]
    list: bool,
}

#[derive(Debug, Clone, Serialize)]
struct BackendConfig {
    /// Directory name under integration_tests/, e.g. "python-psycopg3".
    name: String,
    /// Language identifier for template selection.
    language: String,
    /// Database engine: "postgresql", "mysql", or "sqlite".
    engine: String,
    /// Driver name used by the scythe backend, e.g. "psycopg3", "asyncpg".
    driver: String,
    /// Environment variable for the database connection string.
    connection_env: String,
    /// The scythe backend identifier (used in scythe.toml gen section).
    backend: String,
    /// Extra options passed to templates (e.g. row_type).
    options: HashMap<String, String>,
}

/// Per-backend overrides for the schema SQL filename within the engine's
/// schema dir. Defaults to "schema.sql" when a backend name is not listed
/// here. Kept as a side table (rather than a `BackendConfig` field) so it
/// doesn't require touching every one of the ~100 `BackendConfig` literals
/// below.
const SCHEMA_FILE_OVERRIDES: &[(&str, &str)] = &[
    ("go-godror-oracle", "schema_full.sql"),
    ("java-jdbc-oracle", "schema_full.sql"),
];

/// Context passed to every template render.
#[derive(Debug, Serialize)]
struct TemplateContext {
    backend_name: String,
    language: String,
    engine: String,
    driver: String,
    connection_env: String,
    backend: String,
    options: HashMap<String, String>,
    /// Relative path from the backend dir to the schema SQL directory.
    schema_dir: String,
    /// Schema SQL filename within `schema_dir`, e.g. "schema.sql" or "schema_full.sql".
    schema_file: String,
    /// Relative path from the backend dir to the queries SQL directory.
    queries_dir: String,
}

impl From<&BackendConfig> for TemplateContext {
    fn from(cfg: &BackendConfig) -> Self {
        let engine_dir = match cfg.engine.as_str() {
            "postgresql" => "pg",
            "mysql" => "mysql",
            "sqlite" => "sqlite",
            "mssql" => "mssql",
            "redshift" => "redshift",
            "snowflake" => "snowflake",
            other => other,
        };
        Self {
            backend_name: cfg.name.clone(),
            language: cfg.language.clone(),
            engine: cfg.engine.clone(),
            driver: cfg.driver.clone(),
            connection_env: cfg.connection_env.clone(),
            backend: cfg.backend.clone(),
            options: cfg.options.clone(),
            schema_dir: format!("../sql/{engine_dir}"),
            schema_file: SCHEMA_FILE_OVERRIDES
                .iter()
                .find(|(name, _)| *name == cfg.name)
                .map(|(_, file)| file.to_string())
                .unwrap_or_else(|| "schema.sql".to_string()),
            queries_dir: format!("../sql/{engine_dir}/queries"),
        }
    }
}

/// Which files to generate for each language.
struct LanguageOutputs {
    test_template: String,
    test_filename: &'static str,
    dep_template: String,
    dep_filename: &'static str,
    /// Additional files (template, filename) pairs.
    extra: Vec<(&'static str, &'static str)>,
}

fn language_outputs(language: &str) -> LanguageOutputs {
    match language {
        "python" => LanguageOutputs {
            test_template: "python.py.jinja".into(),
            test_filename: "test_integration.py",
            dep_template: "pyproject.toml.jinja".into(),
            dep_filename: "pyproject.toml",
            extra: vec![],
        },
        "typescript" => LanguageOutputs {
            test_template: "typescript.ts.jinja".into(),
            test_filename: "test.ts",
            dep_template: "package.json.jinja".into(),
            dep_filename: "package.json",
            extra: vec![
                ("tsconfig.json.jinja", "tsconfig.json"),
                ("pnpm-workspace.yaml.jinja", "pnpm-workspace.yaml"),
            ],
        },
        "go" => LanguageOutputs {
            test_template: "go.go.jinja".into(),
            test_filename: "main.go",
            dep_template: "go.mod.jinja".into(),
            dep_filename: "go.mod",
            extra: vec![],
        },
        "elixir" => LanguageOutputs {
            test_template: "elixir.exs.jinja".into(),
            test_filename: "test/integration_test.exs",
            dep_template: "mix.exs.jinja".into(),
            dep_filename: "mix.exs",
            extra: vec![],
        },
        "ruby" => LanguageOutputs {
            test_template: "ruby.rb.jinja".into(),
            test_filename: "test_integration.rb",
            dep_template: "Gemfile.jinja".into(),
            dep_filename: "Gemfile",
            extra: vec![],
        },
        "php" => LanguageOutputs {
            test_template: "php.php.jinja".into(),
            test_filename: "test_integration.php",
            dep_template: "composer.json.jinja".into(),
            dep_filename: "composer.json",
            extra: vec![],
        },
        "rust" => LanguageOutputs {
            test_template: "rust.rs.jinja".into(),
            test_filename: "src/main.rs",
            dep_template: "Cargo.toml.integration.jinja".into(),
            dep_filename: "Cargo.toml",
            extra: vec![],
        },
        "java" => LanguageOutputs {
            test_template: "java.java.jinja".into(),
            test_filename: "src/main/java/IntegrationTest.java",
            dep_template: "pom.xml.jinja".into(),
            dep_filename: "pom.xml",
            extra: vec![("mvn-jvm-config.jinja", ".mvn/jvm.config")],
        },
        "kotlin" => LanguageOutputs {
            test_template: "kotlin.kt.jinja".into(),
            test_filename: "src/main/kotlin/IntegrationTest.kt",
            dep_template: "build.gradle.kts.jinja".into(),
            dep_filename: "build.gradle.kts",
            extra: vec![("settings.gradle.kts.jinja", "settings.gradle.kts")],
        },
        "csharp" => LanguageOutputs {
            test_template: "csharp.cs.jinja".into(),
            test_filename: "Program.cs",
            dep_template: "csproj.jinja".into(),
            dep_filename: "IntegrationTest.csproj",
            extra: vec![],
        },
        other => {
            eprintln!("warning: unsupported language '{other}', using generic outputs");
            LanguageOutputs {
                test_template: format!("{other}.jinja"),
                test_filename: "test",
                dep_template: "deps.jinja".into(),
                dep_filename: "deps",
                extra: vec![],
            }
        }
    }
}

/// The hand-maintained half of `build_backends()`: driver, connection env var
/// and per-variant options (row_type, structs_only, etc.) for every project
/// this generator ships. This data cannot come from the manifests directory —
/// a manifest declares only (name, language, engine), not which driver
/// crate/package exercises it or which one-off option combinations are worth
/// their own project — so it stays a literal list.
///
/// What *is* derived, in `build_backends()` below, is completeness: every
/// manifest under `crates/scythe-codegen/manifests` must be reachable from
/// this list (resolved through the real backend resolver, not a re-derived
/// fallback table) or be named in `coverage-exemptions.txt`. That is what
/// keeps a new backend from shipping a manifest with no project (issue #134).
fn backend_variants() -> Vec<BackendConfig> {
    vec![
        BackendConfig {
            name: "python-psycopg3".into(),
            language: "python".into(),
            engine: "postgresql".into(),
            driver: "psycopg3".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "python-psycopg3".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-asyncpg".into(),
            language: "python".into(),
            engine: "postgresql".into(),
            driver: "asyncpg".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "python-asyncpg".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-psycopg3-pydantic".into(),
            language: "python".into(),
            engine: "postgresql".into(),
            driver: "psycopg3".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "python-psycopg3".into(),
            options: HashMap::from([("row_type".into(), "pydantic".into())]),
        },
        BackendConfig {
            name: "python-psycopg3-msgspec".into(),
            language: "python".into(),
            engine: "postgresql".into(),
            driver: "psycopg3".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "python-psycopg3".into(),
            options: HashMap::from([("row_type".into(), "msgspec".into())]),
        },
        BackendConfig {
            name: "python-aiomysql".into(),
            language: "python".into(),
            engine: "mysql".into(),
            driver: "aiomysql".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "python-aiomysql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-aiosqlite".into(),
            language: "python".into(),
            engine: "sqlite".into(),
            driver: "aiosqlite".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "python-aiosqlite".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-pg".into(),
            language: "typescript".into(),
            engine: "postgresql".into(),
            driver: "pg".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "typescript-pg".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-postgres".into(),
            language: "typescript".into(),
            engine: "postgresql".into(),
            driver: "postgres".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "typescript-postgres".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-pg-zod".into(),
            language: "typescript".into(),
            engine: "postgresql".into(),
            driver: "pg".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "typescript-pg".into(),
            options: HashMap::from([("row_type".into(), "zod".into())]),
        },
        BackendConfig {
            name: "typescript-pg-outer-join-unions".into(),
            language: "typescript".into(),
            engine: "postgresql".into(),
            driver: "pg".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "typescript-pg".into(),
            options: HashMap::from([("outer_join_unions".into(), "true".into())]),
        },
        BackendConfig {
            name: "typescript-pg-zod-outer-join-unions".into(),
            language: "typescript".into(),
            engine: "postgresql".into(),
            driver: "pg".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "typescript-pg".into(),
            options: HashMap::from([
                ("row_type".into(), "zod".into()),
                ("outer_join_unions".into(), "true".into()),
            ]),
        },
        BackendConfig {
            name: "typescript-pg-structs-only".into(),
            language: "typescript".into(),
            engine: "postgresql".into(),
            driver: "pg".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "typescript-pg".into(),
            options: HashMap::from([("structs_only".into(), "true".into())]),
        },
        BackendConfig {
            name: "typescript-kysely".into(),
            language: "typescript".into(),
            engine: "postgresql".into(),
            driver: "kysely-pg".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "typescript-kysely".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-kysely-mysql2".into(),
            language: "typescript".into(),
            engine: "mysql".into(),
            driver: "kysely-mysql2".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "typescript-kysely".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-kysely-mariadb".into(),
            language: "typescript".into(),
            engine: "mariadb".into(),
            driver: "kysely-mariadb".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "typescript-kysely".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-kysely-sqlite".into(),
            language: "typescript".into(),
            engine: "sqlite".into(),
            driver: "kysely-sqlite".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "typescript-kysely".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-kysely-mssql".into(),
            language: "typescript".into(),
            engine: "mssql".into(),
            driver: "kysely-mssql".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "typescript-kysely".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-mysql2".into(),
            language: "typescript".into(),
            engine: "mysql".into(),
            driver: "mysql2".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "typescript-mysql2".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-better-sqlite3".into(),
            language: "typescript".into(),
            engine: "sqlite".into(),
            driver: "better-sqlite3".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "typescript-better-sqlite3".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-node-sqlite".into(),
            language: "typescript".into(),
            engine: "sqlite".into(),
            driver: "node-sqlite".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "typescript-node-sqlite".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-wasm-sqlite".into(),
            language: "typescript".into(),
            engine: "sqlite".into(),
            driver: "wasm-sqlite".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "typescript-wasm-sqlite".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "go-pgx".into(),
            language: "go".into(),
            engine: "postgresql".into(),
            driver: "pgx".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "go-pgx".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "go-database-sql-mysql".into(),
            language: "go".into(),
            engine: "mysql".into(),
            driver: "database-sql".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "go-database-sql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "go-database-sql-sqlite".into(),
            language: "go".into(),
            engine: "sqlite".into(),
            driver: "database-sql".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "go-database-sql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "elixir-postgrex".into(),
            language: "elixir".into(),
            engine: "postgresql".into(),
            driver: "postgrex".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "elixir-postgrex".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "elixir-myxql".into(),
            language: "elixir".into(),
            engine: "mysql".into(),
            driver: "myxql".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "elixir-myxql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "elixir-exqlite".into(),
            language: "elixir".into(),
            engine: "sqlite".into(),
            driver: "exqlite".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "elixir-exqlite".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "elixir-ecto".into(),
            language: "elixir".into(),
            engine: "postgresql".into(),
            driver: "ecto".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "elixir-ecto".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-pg".into(),
            language: "ruby".into(),
            engine: "postgresql".into(),
            driver: "pg".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "ruby-pg".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-mysql2".into(),
            language: "ruby".into(),
            engine: "mysql".into(),
            driver: "mysql2".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "ruby-mysql2".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-sqlite3".into(),
            language: "ruby".into(),
            engine: "sqlite".into(),
            driver: "sqlite3".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "ruby-sqlite3".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-trilogy".into(),
            language: "ruby".into(),
            engine: "mysql".into(),
            driver: "trilogy".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "ruby-trilogy".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-pdo".into(),
            language: "php".into(),
            engine: "postgresql".into(),
            driver: "pdo".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "php-pdo".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-pdo-mysql".into(),
            language: "php".into(),
            engine: "mysql".into(),
            driver: "pdo-mysql".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "php-pdo".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-pdo-sqlite".into(),
            language: "php".into(),
            engine: "sqlite".into(),
            driver: "pdo-sqlite".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "php-pdo".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-amphp".into(),
            language: "php".into(),
            engine: "postgresql".into(),
            driver: "amphp".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "php-amphp".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-amphp-mysql".into(),
            language: "php".into(),
            engine: "mysql".into(),
            driver: "amphp".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "php-amphp".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-sqlx".into(),
            language: "rust".into(),
            engine: "postgresql".into(),
            driver: "sqlx".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "rust-sqlx".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-sqlx-mysql".into(),
            language: "rust".into(),
            engine: "mysql".into(),
            driver: "sqlx-mysql".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "rust-sqlx".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-sqlx-sqlite".into(),
            language: "rust".into(),
            engine: "sqlite".into(),
            driver: "sqlx-sqlite".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "rust-sqlx".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-tokio-postgres".into(),
            language: "rust".into(),
            engine: "postgresql".into(),
            driver: "tokio-postgres".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "rust-tokio-postgres".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc".into(),
            language: "java".into(),
            engine: "postgresql".into(),
            driver: "jdbc".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc-mysql".into(),
            language: "java".into(),
            engine: "mysql".into(),
            driver: "jdbc".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc-sqlite".into(),
            language: "java".into(),
            engine: "sqlite".into(),
            driver: "jdbc".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc".into(),
            language: "kotlin".into(),
            engine: "postgresql".into(),
            driver: "jdbc".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc-mysql".into(),
            language: "kotlin".into(),
            engine: "mysql".into(),
            driver: "jdbc".into(),
            connection_env: "MYSQL_URL".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc-sqlite".into(),
            language: "kotlin".into(),
            engine: "sqlite".into(),
            driver: "jdbc".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-r2dbc".into(),
            language: "java".into(),
            engine: "postgresql".into(),
            driver: "r2dbc".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "java-r2dbc".into(),
            options: HashMap::new(),
        },
        // java-r2dbc-sqlite/kotlin-r2dbc-sqlite are intentionally not wired here: no
        // io.r2dbc:r2dbc-sqlite artifact exists on Maven Central (verified 2026-08-14),
        // and the only known community implementation (com.gitee.n__n:r2dbc-sqlite) is
        // distributed via JitPack, not Central, and untested against this generator's
        // SQLite-dialect schema.sql. Substituting r2dbc-h2 is not a like-for-like swap --
        // H2 is a different SQL dialect and would silently run a different database than
        // every other sqlite integration project in this list. See board #212's report.
        BackendConfig {
            name: "kotlin-exposed".into(),
            language: "kotlin".into(),
            engine: "postgresql".into(),
            driver: "exposed".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "kotlin-exposed".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-r2dbc".into(),
            language: "kotlin".into(),
            engine: "postgresql".into(),
            driver: "r2dbc".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "kotlin-r2dbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "csharp-npgsql".into(),
            language: "csharp".into(),
            engine: "postgresql".into(),
            driver: "npgsql".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "csharp-npgsql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "csharp-mysqlconnector".into(),
            language: "csharp".into(),
            engine: "mysql".into(),
            driver: "mysqlconnector".into(),
            connection_env: "DATABASE_URL".into(),
            backend: "csharp-mysqlconnector".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "csharp-microsoft-sqlite".into(),
            language: "csharp".into(),
            engine: "sqlite".into(),
            driver: "microsoft-sqlite".into(),
            connection_env: "SQLITE_PATH".into(),
            backend: "csharp-microsoft-sqlite".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-sqlx-mariadb".into(),
            language: "rust".into(),
            engine: "mariadb".into(),
            driver: "sqlx-mariadb".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "rust-sqlx".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-aiomysql-mariadb".into(),
            language: "python".into(),
            engine: "mariadb".into(),
            driver: "aiomysql".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "python-aiomysql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-mysql2-mariadb".into(),
            language: "typescript".into(),
            engine: "mariadb".into(),
            driver: "mysql2".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "typescript-mysql2".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "go-database-sql-mariadb".into(),
            language: "go".into(),
            engine: "mariadb".into(),
            driver: "database-sql".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "go-database-sql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc-mariadb".into(),
            language: "java".into(),
            engine: "mariadb".into(),
            driver: "jdbc".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc-mariadb".into(),
            language: "kotlin".into(),
            engine: "mariadb".into(),
            driver: "jdbc".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "csharp-mysqlconnector-mariadb".into(),
            language: "csharp".into(),
            engine: "mariadb".into(),
            driver: "mysqlconnector".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "csharp-mysqlconnector".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "elixir-myxql-mariadb".into(),
            language: "elixir".into(),
            engine: "mariadb".into(),
            driver: "myxql".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "elixir-myxql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-mysql2-mariadb".into(),
            language: "ruby".into(),
            engine: "mariadb".into(),
            driver: "mysql2".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "ruby-mysql2".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-trilogy-mariadb".into(),
            language: "ruby".into(),
            engine: "mariadb".into(),
            driver: "trilogy".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "ruby-trilogy".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-pdo-mariadb".into(),
            language: "php".into(),
            engine: "mariadb".into(),
            driver: "pdo-mysql".into(),
            connection_env: "MARIADB_URL".into(),
            backend: "php-pdo".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-oracledb-oracle".into(),
            language: "python".into(),
            engine: "oracle".into(),
            driver: "oracledb".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "python-oracledb".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-oracledb-oracle".into(),
            language: "typescript".into(),
            engine: "oracle".into(),
            driver: "oracledb".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "typescript-oracledb".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "go-godror-oracle".into(),
            language: "go".into(),
            engine: "oracle".into(),
            driver: "godror".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "go-godror".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc-oracle".into(),
            language: "java".into(),
            engine: "oracle".into(),
            driver: "jdbc".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc-oracle".into(),
            language: "kotlin".into(),
            engine: "oracle".into(),
            driver: "jdbc".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "csharp-oracle".into(),
            language: "csharp".into(),
            engine: "oracle".into(),
            driver: "oracle".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "csharp-oracle".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "elixir-jamdb-oracle".into(),
            language: "elixir".into(),
            engine: "oracle".into(),
            driver: "jamdb".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "elixir-jamdb".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-oci8-oracle".into(),
            language: "ruby".into(),
            engine: "oracle".into(),
            driver: "oci8".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "ruby-oci8".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-sibyl-oracle".into(),
            language: "rust".into(),
            engine: "oracle".into(),
            driver: "sibyl".into(),
            connection_env: "ORACLE_URL".into(),
            backend: "rust-sibyl".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-tiberius-mssql".into(),
            language: "rust".into(),
            engine: "mssql".into(),
            driver: "tiberius".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "rust-tiberius".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-pyodbc-mssql".into(),
            language: "python".into(),
            engine: "mssql".into(),
            driver: "pyodbc".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "python-pyodbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-mssql".into(),
            language: "typescript".into(),
            engine: "mssql".into(),
            driver: "mssql".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "typescript-mssql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "go-database-sql-mssql".into(),
            language: "go".into(),
            engine: "mssql".into(),
            driver: "database-sql".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "go-database-sql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc-mssql".into(),
            language: "java".into(),
            engine: "mssql".into(),
            driver: "jdbc".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc-mssql".into(),
            language: "kotlin".into(),
            engine: "mssql".into(),
            driver: "jdbc".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "csharp-sqlclient-mssql".into(),
            language: "csharp".into(),
            engine: "mssql".into(),
            driver: "sqlclient".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "csharp-sqlclient".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "elixir-tds-mssql".into(),
            language: "elixir".into(),
            engine: "mssql".into(),
            driver: "tds".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "elixir-tds".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-tiny-tds-mssql".into(),
            language: "ruby".into(),
            engine: "mssql".into(),
            driver: "tiny-tds".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "ruby-tiny-tds".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-pdo-mssql".into(),
            language: "php".into(),
            engine: "mssql".into(),
            driver: "pdo-mssql".into(),
            connection_env: "MSSQL_URL".into(),
            backend: "php-pdo".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-sqlx-redshift".into(),
            language: "rust".into(),
            engine: "redshift".into(),
            driver: "sqlx".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "rust-sqlx".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "rust-tokio-postgres-redshift".into(),
            language: "rust".into(),
            engine: "redshift".into(),
            driver: "tokio-postgres".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "rust-tokio-postgres".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-psycopg3-redshift".into(),
            language: "python".into(),
            engine: "redshift".into(),
            driver: "psycopg3".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "python-psycopg3".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-asyncpg-redshift".into(),
            language: "python".into(),
            engine: "redshift".into(),
            driver: "asyncpg".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "python-asyncpg".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-pg-redshift".into(),
            language: "typescript".into(),
            engine: "redshift".into(),
            driver: "pg".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "typescript-pg".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-postgres-redshift".into(),
            language: "typescript".into(),
            engine: "redshift".into(),
            driver: "postgres".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "typescript-postgres".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-kysely-redshift".into(),
            language: "typescript".into(),
            engine: "redshift".into(),
            driver: "kysely-pg".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "typescript-kysely".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "go-pgx-redshift".into(),
            language: "go".into(),
            engine: "redshift".into(),
            driver: "pgx".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "go-pgx".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc-redshift".into(),
            language: "java".into(),
            engine: "redshift".into(),
            driver: "jdbc".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc-redshift".into(),
            language: "kotlin".into(),
            engine: "redshift".into(),
            driver: "jdbc".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "csharp-npgsql-redshift".into(),
            language: "csharp".into(),
            engine: "redshift".into(),
            driver: "npgsql".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "csharp-npgsql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "elixir-postgrex-redshift".into(),
            language: "elixir".into(),
            engine: "redshift".into(),
            driver: "postgrex".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "elixir-postgrex".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "ruby-pg-redshift".into(),
            language: "ruby".into(),
            engine: "redshift".into(),
            driver: "pg".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "ruby-pg".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-pdo-redshift".into(),
            language: "php".into(),
            engine: "redshift".into(),
            driver: "pdo".into(),
            connection_env: "REDSHIFT_URL".into(),
            backend: "php-pdo".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-snowflake".into(),
            language: "python".into(),
            engine: "snowflake".into(),
            driver: "snowflake".into(),
            connection_env: "SNOWFLAKE_URL".into(),
            backend: "python-snowflake".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-snowflake".into(),
            language: "typescript".into(),
            engine: "snowflake".into(),
            driver: "snowflake".into(),
            connection_env: "SNOWFLAKE_URL".into(),
            backend: "typescript-snowflake".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "go-gosnowflake".into(),
            language: "go".into(),
            engine: "snowflake".into(),
            driver: "database-sql".into(),
            connection_env: "SNOWFLAKE_URL".into(),
            backend: "go-gosnowflake".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc-snowflake".into(),
            language: "java".into(),
            engine: "snowflake".into(),
            driver: "jdbc".into(),
            connection_env: "SNOWFLAKE_URL".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc-snowflake".into(),
            language: "kotlin".into(),
            engine: "snowflake".into(),
            driver: "jdbc".into(),
            connection_env: "SNOWFLAKE_URL".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "csharp-snowflake".into(),
            language: "csharp".into(),
            engine: "snowflake".into(),
            driver: "snowflake".into(),
            connection_env: "SNOWFLAKE_URL".into(),
            backend: "csharp-snowflake".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "php-pdo-snowflake".into(),
            language: "php".into(),
            engine: "snowflake".into(),
            driver: "pdo".into(),
            connection_env: "SNOWFLAKE_URL".into(),
            backend: "php-pdo".into(),
            options: HashMap::new(),
        },
        // DuckDB (issue #126): embedded, no service container. Modelled on the
        // sqlite variants above, not the postgres ones -- DuckDB is the other
        // embedded engine here.
        BackendConfig {
            name: "go-database-sql-duckdb".into(),
            language: "go".into(),
            engine: "duckdb".into(),
            driver: "database-sql".into(),
            connection_env: "DUCKDB_PATH".into(),
            backend: "go-database-sql".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "java-jdbc-duckdb".into(),
            language: "java".into(),
            engine: "duckdb".into(),
            driver: "jdbc".into(),
            connection_env: "DUCKDB_PATH".into(),
            backend: "java-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "kotlin-jdbc-duckdb".into(),
            language: "kotlin".into(),
            engine: "duckdb".into(),
            driver: "jdbc".into(),
            connection_env: "DUCKDB_PATH".into(),
            backend: "kotlin-jdbc".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "python-duckdb".into(),
            language: "python".into(),
            engine: "duckdb".into(),
            driver: "duckdb".into(),
            connection_env: "DUCKDB_PATH".into(),
            backend: "python-duckdb".into(),
            options: HashMap::new(),
        },
        BackendConfig {
            name: "typescript-duckdb".into(),
            language: "typescript".into(),
            engine: "duckdb".into(),
            driver: "duckdb".into(),
            connection_env: "DUCKDB_PATH".into(),
            backend: "typescript-duckdb".into(),
            options: HashMap::new(),
        },
    ]
}

/// The `[backend]` fields this generator needs out of a manifest TOML file.
/// Mirrors `ManifestBackend` in
/// `tools/integration-test-generator/tests/coverage_completeness.rs` — kept
/// as a separate, minimal struct (rather than depending on
/// `scythe_backend::manifest::BackendManifest`) because this is the disk-read
/// half of the check, not the resolved half; the resolved half goes through
/// `get_backend` instead, below.
#[derive(Debug, Deserialize)]
struct ManifestFile {
    backend: ManifestBackendMeta,
}

#[derive(Debug, Deserialize)]
struct ManifestBackendMeta {
    name: String,
    engine: String,
}

/// Every manifest that ships, keyed by the (name, engine) pair the backend
/// resolver uses. This is the disk-derived side of the completeness check:
/// `build_backends()` cannot ship a project for a manifest this function does
/// not find.
fn discover_manifest_pairs(manifests_dir: &Path) -> BTreeSet<(String, String)> {
    let entries =
        fs::read_dir(manifests_dir).unwrap_or_else(|err| panic!("reading {}: {err}", manifests_dir.display()));
    let mut pairs = BTreeSet::new();
    for entry in entries {
        let path = entry
            .unwrap_or_else(|err| panic!("reading manifest entry: {err}"))
            .path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let contents = fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
        let parsed: ManifestFile =
            toml::from_str(&contents).unwrap_or_else(|err| panic!("parsing manifest {}: {err}", path.display()));
        pairs.insert((parsed.backend.name, parsed.backend.engine));
    }
    assert!(
        !pairs.is_empty(),
        "no manifests found under {}",
        manifests_dir.display()
    );
    pairs
}

/// Parses the `[manifests]` section of `coverage-exemptions.txt`: lines
/// shaped `<name>|<engine> : <reason>`. Deliberately reads the same file
/// `tests/coverage_completeness.rs` reads (rather than a second, generator-only
/// opt-out list), because both are asserting the same fact — a manifest with
/// no project — and two allowlists for one fact is exactly the drift issue
/// #134 exists to close.
fn load_manifest_exemptions(exemptions_path: &Path) -> HashMap<String, String> {
    let contents = fs::read_to_string(exemptions_path)
        .unwrap_or_else(|err| panic!("reading {}: {err}", exemptions_path.display()));
    let mut in_section = false;
    let mut entries = HashMap::new();
    for raw in contents.lines() {
        let line = raw.trim();
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_section = header == "manifests";
            continue;
        }
        if line.is_empty() || line.starts_with('#') || !in_section {
            continue;
        }
        let Some((key, reason)) = line.split_once(':') else {
            panic!("exemption line in [manifests] is not '<key> : <reason>': {line}");
        };
        let (key, reason) = (key.trim().to_string(), reason.trim().to_string());
        assert!(!reason.is_empty(), "exemption '{key}' in [manifests] has no reason");
        entries.insert(key, reason);
    }
    entries
}

/// Cross-checks `backend_variants()` against the manifests directory and
/// `coverage-exemptions.txt`, failing the build in both directions — a
/// manifest with no covering variant and no exemption, or an exemption whose
/// manifest is covered now — the same ratchet `torture-expected-failures.txt`
/// uses. See `backend_variants()`'s doc comment for why the entries
/// themselves cannot be generated from the manifest directly.
fn validate_manifest_coverage(variants: &[BackendConfig], manifests_dir: &Path, exemptions_path: &Path) {
    let shipped = discover_manifest_pairs(manifests_dir);

    // A variant names the (backend, engine) it *asks* for, but the resolver
    // can answer with a manifest declared under a different engine (e.g.
    // csharp-mysqlconnector on mariadb resolves to the mysql manifest, there
    // being no mariadb manifest for it). So coverage is whatever the real
    // resolver reaches, not a literal pair match -- re-deriving that
    // fallback table here would reintroduce the "two derivations, never
    // cross-checked" shape this check exists to kill.
    let mut covered: BTreeSet<(String, String)> = BTreeSet::new();
    for variant in variants {
        let backend = get_backend(&variant.backend, &variant.engine).unwrap_or_else(|err| {
            panic!(
                "backend_variants() entry '{}' requests backend '{}' on engine '{}', which does not \
                 resolve to any manifest: {err}",
                variant.name, variant.backend, variant.engine
            )
        });
        let meta = &backend.manifest().backend;
        covered.insert((meta.name.clone(), meta.engine.clone()));
    }

    let exemptions = load_manifest_exemptions(exemptions_path);
    let mut failures = Vec::new();

    for (name, engine) in shipped.difference(&covered) {
        let key = format!("{name}|{engine}");
        if !exemptions.contains_key(&key) {
            failures.push(format!(
                "manifest '{name}' (engine '{engine}') has no entry in backend_variants() and no \
                 exemption in {}",
                exemptions_path.display()
            ));
        }
    }
    for key in exemptions.keys() {
        let Some((name, engine)) = key.split_once('|') else {
            failures.push(format!(
                "malformed exemption key '{key}' in {}",
                exemptions_path.display()
            ));
            continue;
        };
        if covered.contains(&(name.to_string(), engine.to_string())) {
            failures.push(format!(
                "exemption '{key}' in {} is stale -- it is covered now, delete the line",
                exemptions_path.display()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "manifest coverage check failed:\n{}",
        failures.iter().map(|f| format!("  {f}")).collect::<Vec<_>>().join("\n")
    );
}

/// Repo root, derived from the compiled-in manifest dir rather than the
/// process's current directory, so the coverage check below is correct
/// regardless of where the binary is invoked from.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate is two levels below the repo root")
        .to_path_buf()
}

/// The list `main()` actually uses: `backend_variants()`, validated against
/// `crates/scythe-codegen/manifests` and `coverage-exemptions.txt` so a
/// manifest can never ship silently uncovered (issue #134).
fn build_backends() -> Vec<BackendConfig> {
    let variants = backend_variants();
    let root = repo_root();
    validate_manifest_coverage(
        &variants,
        &root.join("crates/scythe-codegen/manifests"),
        &root.join("tools/integration-test-generator/coverage-exemptions.txt"),
    );
    variants
}

fn load_templates(env: &mut Environment<'_>, templates_dir: &Path) -> Result<(), String> {
    if !templates_dir.is_dir() {
        return Err(format!(
            "templates directory does not exist: {}",
            templates_dir.display()
        ));
    }

    let entries = fs::read_dir(templates_dir).map_err(|err| format!("reading templates dir: {err}"))?;

    for entry in entries {
        let entry = entry.map_err(|err| format!("reading template entry: {err}"))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("jinja") {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("invalid template filename: {}", path.display()))?
                .to_string();
            let content =
                fs::read_to_string(&path).map_err(|err| format!("reading template {}: {err}", path.display()))?;
            env.add_template_owned(name, content)
                .map_err(|err| format!("parsing template {}: {err}", path.display()))?;
        }
    }

    Ok(())
}

fn render_template(env: &Environment<'_>, template_name: &str, context: &TemplateContext) -> Result<String, String> {
    let tmpl = env
        .get_template(template_name)
        .map_err(|err| format!("template '{template_name}' not found: {err}"))?;
    let mut output = tmpl
        .render(context)
        .map_err(|err| format!("rendering '{template_name}': {err}"))?;
    if !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let backends = build_backends();

    if cli.list {
        for backend in &backends {
            if !cli.only.is_empty() && !cli.only.contains(&backend.name) {
                continue;
            }
            println!("{}", backend.name);
        }
        return Ok(());
    }

    let mut env = Environment::new();
    load_templates(&mut env, &cli.templates)?;

    let mut generated = 0u32;
    let mut skipped = 0u32;

    for backend in &backends {
        if !cli.only.is_empty() && !cli.only.contains(&backend.name) {
            continue;
        }

        let output_dir = cli.output.join(&backend.name);
        if cli.skip_existing && output_dir.exists() {
            eprintln!("skip (exists): {}", backend.name);
            skipped += 1;
            continue;
        }

        let outputs = language_outputs(&backend.language);
        let context = TemplateContext::from(backend);

        if env.get_template(&outputs.test_template).is_err() {
            eprintln!(
                "warning: skipping {} — template '{}' not found",
                backend.name, outputs.test_template
            );
            skipped += 1;
            continue;
        }

        fs::create_dir_all(&output_dir)?;

        let scythe_toml = render_template(&env, "scythe.toml.jinja", &context)?;
        fs::write(output_dir.join("scythe.toml"), scythe_toml)?;

        let test_path = output_dir.join(outputs.test_filename);
        if let Some(parent) = test_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let test_content = render_template(&env, &outputs.test_template, &context)?;
        fs::write(&test_path, test_content)?;

        if env.get_template(&outputs.dep_template).is_ok() {
            let dep_content = render_template(&env, &outputs.dep_template, &context)?;
            fs::write(output_dir.join(outputs.dep_filename), dep_content)?;
        }

        for (tmpl, filename) in &outputs.extra {
            if env.get_template(tmpl).is_ok() {
                let content = render_template(&env, tmpl, &context)?;
                if content.trim().is_empty() {
                    continue;
                }
                let extra_path = output_dir.join(filename);
                if let Some(parent) = extra_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(extra_path, content)?;
            }
        }

        println!("generated: {}", backend.name);
        generated += 1;
    }

    println!(
        "\ndone: {generated} generated, {skipped} skipped, {} total backends",
        backends.len()
    );

    Ok(())
}
