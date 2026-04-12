use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use minijinja::Environment;
use serde::Serialize;

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
    /// Relative path from the backend dir to the queries SQL directory.
    queries_dir: String,
}

impl From<&BackendConfig> for TemplateContext {
    fn from(cfg: &BackendConfig) -> Self {
        let engine_dir = match cfg.engine.as_str() {
            "postgresql" => "pg",
            "mysql" => "mysql",
            "sqlite" => "sqlite",
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
            extra: vec![("tsconfig.json.jinja", "tsconfig.json")],
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
            extra: vec![],
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

fn build_backends() -> Vec<BackendConfig> {
    vec![
        // --- Python ---
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
        // --- TypeScript ---
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
        // --- Go ---
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
        // --- Elixir ---
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
        // --- Ruby ---
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
        // --- PHP ---
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
            backend: "php-pdo-sqlite".into(),
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
        // --- Rust ---
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
        // --- Java ---
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
        // --- Kotlin ---
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
        // --- C# ---
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
        // --- MariaDB ---
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
    ]
}

fn load_templates(env: &mut Environment<'_>, templates_dir: &Path) -> Result<(), String> {
    if !templates_dir.is_dir() {
        return Err(format!(
            "templates directory does not exist: {}",
            templates_dir.display()
        ));
    }

    let entries =
        fs::read_dir(templates_dir).map_err(|err| format!("reading templates dir: {err}"))?;

    for entry in entries {
        let entry = entry.map_err(|err| format!("reading template entry: {err}"))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("jinja") {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("invalid template filename: {}", path.display()))?
                .to_string();
            let content = fs::read_to_string(&path)
                .map_err(|err| format!("reading template {}: {err}", path.display()))?;
            env.add_template_owned(name, content)
                .map_err(|err| format!("parsing template {}: {err}", path.display()))?;
        }
    }

    Ok(())
}

fn render_template(
    env: &Environment<'_>,
    template_name: &str,
    context: &TemplateContext,
) -> Result<String, String> {
    let tmpl = env
        .get_template(template_name)
        .map_err(|err| format!("template '{template_name}' not found: {err}"))?;
    let mut output = tmpl
        .render(context)
        .map_err(|err| format!("rendering '{template_name}': {err}"))?;
    // Ensure trailing newline for POSIX compliance.
    if !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let mut env = Environment::new();
    load_templates(&mut env, &cli.templates)?;

    let backends = build_backends();

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

        // Check that the required templates exist before creating the directory.
        if env.get_template(&outputs.test_template).is_err() {
            eprintln!(
                "warning: skipping {} — template '{}' not found",
                backend.name, outputs.test_template
            );
            skipped += 1;
            continue;
        }

        fs::create_dir_all(&output_dir)?;

        // Render scythe.toml
        let scythe_toml = render_template(&env, "scythe.toml.jinja", &context)?;
        fs::write(output_dir.join("scythe.toml"), scythe_toml)?;

        // Render test file (create parent dirs for nested paths like test/foo.exs)
        let test_path = output_dir.join(outputs.test_filename);
        if let Some(parent) = test_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let test_content = render_template(&env, &outputs.test_template, &context)?;
        fs::write(&test_path, test_content)?;

        // Render dependency file
        if env.get_template(&outputs.dep_template).is_ok() {
            let dep_content = render_template(&env, &outputs.dep_template, &context)?;
            fs::write(output_dir.join(outputs.dep_filename), dep_content)?;
        }

        // Render extra files
        for (tmpl, filename) in &outputs.extra {
            if env.get_template(tmpl).is_ok() {
                let content = render_template(&env, tmpl, &context)?;
                fs::write(output_dir.join(filename), content)?;
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
