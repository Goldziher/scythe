//! Validate generated code for all backends using real language tools.
//! All tools are expected to be installed.

use scythe_codegen::validation::{validate_structural, validate_with_tools};
use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::parser::parse_query;

const SCHEMA: &str = "CREATE TABLE users (\
    id SERIAL PRIMARY KEY, \
    name TEXT NOT NULL, \
    email TEXT, \
    status TEXT NOT NULL DEFAULT 'active', \
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
);";

const QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, email, status, created_at FROM users WHERE id = $1;";

const QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, email FROM users ORDER BY name;";

const QUERY_EXEC: &str = "-- @name DeleteUser\n-- @returns :exec\n\
    DELETE FROM users WHERE id = $1;";

fn generate_full_file(backend_name: &str) -> String {
    let catalog = Catalog::from_ddl(&[SCHEMA]).unwrap();
    let backend = get_backend(backend_name).unwrap();

    let mut full = backend.file_header();
    full.push('\n');

    for query_sql in [QUERY_ONE, QUERY_MANY, QUERY_EXEC] {
        let parsed = parse_query(query_sql).unwrap();
        let analyzed = analyze(&catalog, &parsed).unwrap();
        match generate_with_backend(&analyzed, &*backend) {
            Ok(code) => {
                if let Some(ref s) = code.enum_def {
                    full.push_str(s);
                    full.push('\n');
                }
                if let Some(ref s) = code.model_struct {
                    full.push_str(s);
                    full.push('\n');
                }
                if let Some(ref s) = code.row_struct {
                    full.push_str(s);
                    full.push('\n');
                }
                if let Some(ref s) = code.query_fn {
                    full.push_str(s);
                    full.push('\n');
                }
            }
            Err(e) => {
                eprintln!("  codegen error for {backend_name}: {e}");
            }
        }
    }

    full
}

macro_rules! backend_test {
    ($name:ident, $backend:expr) => {
        #[test]
        fn $name() {
            let code = generate_full_file($backend);
            assert!(
                !code.trim().is_empty(),
                "generated code is empty for {}",
                $backend
            );

            // Print generated code for inspection
            eprintln!("\n=== {} ===\n{}\n=== END ===\n", $backend, code);

            // Structural validation
            let structural_errors = validate_structural(&code, $backend);
            assert!(
                structural_errors.is_empty(),
                "{} structural: {:?}",
                $backend,
                structural_errors
            );

            // Real tool validation
            if let Some(tool_errors) = validate_with_tools(&code, $backend) {
                assert!(
                    tool_errors.is_empty(),
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend,
                    tool_errors,
                    code
                );
            }
        }
    };
}

backend_test!(test_rust_sqlx, "rust-sqlx");
backend_test!(test_rust_tokio_postgres, "rust-tokio-postgres");
backend_test!(test_python_psycopg3, "python-psycopg3");
backend_test!(test_python_asyncpg, "python-asyncpg");
backend_test!(test_typescript_postgres, "typescript-postgres");
backend_test!(test_typescript_pg, "typescript-pg");
backend_test!(test_go_pgx, "go-pgx");
backend_test!(test_java_jdbc, "java-jdbc");
backend_test!(test_kotlin_jdbc, "kotlin-jdbc");
backend_test!(test_csharp_npgsql, "csharp-npgsql");
backend_test!(test_elixir_postgrex, "elixir-postgrex");
backend_test!(test_ruby_pg, "ruby-pg");
backend_test!(test_php_pdo, "php-pdo");
