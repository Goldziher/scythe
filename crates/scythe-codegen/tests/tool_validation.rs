//! Validate generated code for all backends using real language tools.
//! All tools are expected to be installed.

use scythe_codegen::validation::{validate_structural, validate_with_tools};
use scythe_codegen::{CodegenBackend, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

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

const MYSQL_SCHEMA: &str = "CREATE TABLE users (\
    id INT AUTO_INCREMENT PRIMARY KEY, \
    name VARCHAR(255) NOT NULL, \
    email VARCHAR(255), \
    status VARCHAR(50) NOT NULL DEFAULT 'active', \
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP\
);";

const MYSQL_QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, email, status, created_at FROM users WHERE id = ?;";

const MYSQL_QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, email FROM users ORDER BY name;";

const MYSQL_QUERY_EXEC: &str = "-- @name DeleteUser\n-- @returns :exec\n\
    DELETE FROM users WHERE id = ?;";

fn generate_full_file(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::PostgreSQL)
}

fn generate_full_file_with_options(
    backend_name: &str,
    options: &std::collections::HashMap<String, String>,
) -> String {
    let mut backend = get_backend(backend_name, "postgresql").unwrap();
    backend.apply_options(options).unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::PostgreSQL)
}

fn generate_full_file_mysql(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "mysql").unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::MySQL)
}

fn generate_full_file_from_backend(
    backend_name: &str,
    backend: &dyn CodegenBackend,
    dialect: &SqlDialect,
) -> String {
    let is_mysql = matches!(dialect, SqlDialect::MySQL);
    let schema = if is_mysql { MYSQL_SCHEMA } else { SCHEMA };
    let queries = if is_mysql {
        [MYSQL_QUERY_ONE, MYSQL_QUERY_MANY, MYSQL_QUERY_EXEC]
    } else {
        [QUERY_ONE, QUERY_MANY, QUERY_EXEC]
    };

    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).unwrap();

    let mut full = backend.file_header();
    full.push('\n');

    let class_header = backend.query_class_header();
    let use_class_wrapper = !class_header.is_empty();

    // Collect all generated code first
    let mut all_codes = Vec::new();
    for query_sql in queries {
        let parsed = parse_query_with_dialect(query_sql, dialect).unwrap();
        let analyzed = analyze(&catalog, &parsed).unwrap();
        match generate_with_backend(&analyzed, backend) {
            Ok(code) => all_codes.push(code),
            Err(e) => {
                eprintln!("  codegen error for {backend_name}: {e}");
            }
        }
    }

    if use_class_wrapper {
        // Emit type definitions (enums, structs) first, outside the class
        for code in &all_codes {
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
        }
        // Open the class
        full.push_str(&class_header);
        full.push('\n');
        // Emit query functions inside the class
        for code in &all_codes {
            if let Some(ref s) = code.query_fn {
                full.push_str(s);
                full.push('\n');
            }
        }
        // Close the class via file footer
        let footer = backend.file_footer();
        if !footer.is_empty() {
            full.push_str(&footer);
            full.push('\n');
        }
    } else {
        for code in &all_codes {
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
        let footer = backend.file_footer();
        if !footer.is_empty() {
            full.push_str(&footer);
            full.push('\n');
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

macro_rules! backend_test_with_options {
    ($name:ident, $backend:expr, $($key:expr => $val:expr),+) => {
        #[test]
        fn $name() {
            let mut options = std::collections::HashMap::new();
            $(options.insert($key.to_string(), $val.to_string());)+
            let code = generate_full_file_with_options($backend, &options);
            assert!(
                !code.trim().is_empty(),
                "generated code is empty for {} with options",
                $backend
            );

            // Print generated code for inspection
            eprintln!("\n=== {} (with options) ===\n{}\n=== END ===\n", $backend, code);

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

macro_rules! mysql_backend_test {
    ($name:ident, $backend:expr) => {
        #[test]
        fn $name() {
            let code = generate_full_file_mysql($backend);
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

// --- Default backend tests (PostgreSQL) ---
backend_test!(test_rust_sqlx, "rust-sqlx");
backend_test!(test_rust_tokio_postgres, "rust-tokio-postgres");
backend_test!(test_python_psycopg3, "python-psycopg3");
backend_test!(test_python_asyncpg, "python-asyncpg");
backend_test!(test_typescript_postgres, "typescript-postgres");
backend_test!(test_typescript_pg, "typescript-pg");
backend_test!(test_go_pgx, "go-pgx");
backend_test!(test_java_jdbc, "java-jdbc");
backend_test!(test_java_r2dbc, "java-r2dbc");
backend_test!(test_kotlin_jdbc, "kotlin-jdbc");
backend_test!(test_kotlin_r2dbc, "kotlin-r2dbc");
backend_test!(test_csharp_npgsql, "csharp-npgsql");
backend_test!(test_elixir_postgrex, "elixir-postgrex");
backend_test!(test_elixir_ecto, "elixir-ecto");
backend_test!(test_ruby_pg, "ruby-pg");
backend_test!(test_php_pdo, "php-pdo");
backend_test!(test_php_amphp, "php-amphp");

// --- MySQL backend tests ---
mysql_backend_test!(test_ruby_trilogy, "ruby-trilogy");

// --- Row type variant tests ---
backend_test_with_options!(test_python_psycopg3_pydantic, "python-psycopg3", "row_type" => "pydantic");
backend_test_with_options!(test_python_psycopg3_msgspec, "python-psycopg3", "row_type" => "msgspec");
backend_test_with_options!(test_python_asyncpg_pydantic, "python-asyncpg", "row_type" => "pydantic");
backend_test_with_options!(test_typescript_pg_zod, "typescript-pg", "row_type" => "zod");
backend_test_with_options!(test_typescript_postgres_zod, "typescript-postgres", "row_type" => "zod");
