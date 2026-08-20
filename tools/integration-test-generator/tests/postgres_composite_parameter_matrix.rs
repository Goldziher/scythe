use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const EXPECTED_PROJECTS: &[&str] = &[
    "csharp-npgsql",
    "elixir-ecto",
    "elixir-postgrex",
    "go-pgx",
    "java-jdbc",
    "java-r2dbc",
    "kotlin-exposed",
    "kotlin-jdbc",
    "kotlin-jdbc-ext",
    "kotlin-r2dbc",
    "php-amphp",
    "php-pdo",
    "php-pdo-namespace",
    "python-asyncpg",
    "python-psycopg3",
    "python-psycopg3-msgspec",
    "python-psycopg3-pydantic",
    "ruby-pg",
    "rust-sqlx",
    "rust-sqlx-nested-json",
    "rust-tokio-postgres",
    "typescript-kysely",
    "typescript-kysely-camel",
    "typescript-pg",
    "typescript-pg-camel",
    "typescript-pg-outer-join-unions",
    "typescript-pg-structs-only",
    "typescript-pg-zod",
    "typescript-pg-zod-outer-join-unions",
    "typescript-postgres",
];

const EXPECTED_BACKENDS: &[&str] = &[
    "csharp-npgsql",
    "elixir-ecto",
    "elixir-postgrex",
    "go-pgx",
    "java-jdbc",
    "java-r2dbc",
    "kotlin-exposed",
    "kotlin-jdbc",
    "kotlin-r2dbc",
    "php-amphp",
    "php-pdo",
    "python-asyncpg",
    "python-psycopg3",
    "ruby-pg",
    "rust-sqlx",
    "rust-tokio-postgres",
    "typescript-kysely",
    "typescript-pg",
    "typescript-postgres",
];

const DECLARATIONS_ONLY_PROJECT: &str = "typescript-pg-structs-only";
const NAMESPACE_ONLY_PROJECT: &str = "php-pdo-namespace";
const QUERY_SYMBOL: &str = "RoundTripUserAddress";
const EXPECTED_PROJECT_COUNT: usize = 30;
const EXPECTED_BACKEND_COUNT: usize = 19;
const EXPECTED_QUERY_CAPABLE_COUNT: usize = 29;
const EXPECTED_LIVE_HARNESS_COUNT: usize = 28;

#[derive(Deserialize)]
struct ScytheConfig {
    sql: Vec<SqlBlock>,
}

#[derive(Deserialize)]
struct SqlBlock {
    engine: String,
    #[serde(rename = "gen")]
    generators: Vec<Generator>,
}

#[derive(Deserialize)]
struct Generator {
    backend: String,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate is two levels below the repository root")
        .to_path_buf()
}

fn postgres_projects() -> BTreeMap<String, String> {
    let integration_dir = repo_root().join("integration_tests");
    let mut projects = BTreeMap::new();
    for entry in fs::read_dir(&integration_dir).expect("read integration_tests") {
        let path = entry.expect("integration_tests entry").path();
        let config_path = path.join("scythe.toml");
        if !config_path.is_file() {
            continue;
        }
        let config: ScytheConfig =
            toml::from_str(&fs::read_to_string(&config_path).expect("read scythe.toml")).expect("parse scythe.toml");
        for block in config.sql.iter().filter(|block| block.engine == "postgresql") {
            for generator in &block.generators {
                let project = path
                    .file_name()
                    .expect("project directory name")
                    .to_string_lossy()
                    .into_owned();
                assert!(
                    projects.insert(project.clone(), generator.backend.clone()).is_none(),
                    "PostgreSQL project {project} declares more than one generator"
                );
            }
        }
    }
    projects
}

fn project_files(project: &str) -> Vec<PathBuf> {
    let root = repo_root().join("integration_tests").join(project);
    let mut pending = vec![root];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap_or_else(|error| panic!("read {}: {error}", directory.display())) {
            let path = entry.expect("project entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}

fn project_contains(project: &str, needle: &str) -> bool {
    project_files(project).into_iter().any(|path| {
        fs::read_to_string(path)
            .map(|contents| contents.contains(needle))
            .unwrap_or(false)
    })
}

fn harness_contains(project: &str, needle: &str) -> bool {
    project_files(project).into_iter().any(|path| {
        if path.components().any(|component| component.as_os_str() == "generated")
            || path.file_name().is_some_and(|name| name == "queries.rs")
        {
            return false;
        }
        fs::read_to_string(path)
            .map(|contents| contents.contains(needle))
            .unwrap_or(false)
    })
}

#[test]
fn postgres_composite_parameter_matrix_has_exact_inventory() {
    let projects = postgres_projects();
    let actual_projects: BTreeSet<&str> = projects.keys().map(String::as_str).collect();
    let expected_projects: BTreeSet<&str> = EXPECTED_PROJECTS.iter().copied().collect();
    assert_eq!(
        actual_projects, expected_projects,
        "PostgreSQL integration project inventory drifted"
    );

    let actual_backends: BTreeSet<&str> = projects.values().map(String::as_str).collect();
    let expected_backends: BTreeSet<&str> = EXPECTED_BACKENDS.iter().copied().collect();
    assert_eq!(
        actual_backends, expected_backends,
        "PostgreSQL backend inventory drifted"
    );
    assert_eq!(
        projects.len(),
        EXPECTED_PROJECT_COUNT,
        "project count must remain explicit"
    );
    assert_eq!(
        actual_backends.len(),
        EXPECTED_BACKEND_COUNT,
        "backend count must remain explicit"
    );
}

#[test]
fn every_postgres_project_covers_the_composite_parameter_query() {
    let mut live_harnesses = 0;
    let projects = postgres_projects();
    for project in projects.keys() {
        assert!(
            project_contains(project, QUERY_SYMBOL),
            "{project} does not generate {QUERY_SYMBOL}"
        );
        if project == DECLARATIONS_ONLY_PROJECT || project == NAMESPACE_ONLY_PROJECT {
            assert!(
                !harness_contains(project, QUERY_SYMBOL),
                "{project} must remain runtime-free"
            );
            continue;
        }
        live_harnesses += 1;
        assert!(
            harness_contains(project, QUERY_SYMBOL),
            "{project} does not execute {QUERY_SYMBOL}"
        );
    }
    assert_eq!(
        projects.len() - 1,
        EXPECTED_QUERY_CAPABLE_COUNT,
        "query-capable project count drifted"
    );
    assert_eq!(
        live_harnesses, EXPECTED_LIVE_HARNESS_COUNT,
        "live composite harness count drifted"
    );
}
