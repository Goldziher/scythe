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

fn generate_full_file_with_options(backend_name: &str, options: &std::collections::HashMap<String, String>) -> String {
    let mut backend = get_backend(backend_name, "postgresql").unwrap();
    backend.apply_options(options).unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::PostgreSQL)
}

fn generate_full_file_mysql(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "mysql").unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::MySQL)
}

fn generate_full_file_from_backend(backend_name: &str, backend: &dyn CodegenBackend, dialect: &SqlDialect) -> String {
    let is_mysql = matches!(dialect, SqlDialect::MySQL);
    let schema = if is_mysql { MYSQL_SCHEMA } else { SCHEMA };
    let queries = if is_mysql {
        [MYSQL_QUERY_ONE, MYSQL_QUERY_MANY, MYSQL_QUERY_EXEC]
    } else {
        [QUERY_ONE, QUERY_MANY, QUERY_EXEC]
    };

    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).unwrap();

    let class_header = backend.query_class_header();
    let use_class_wrapper = !class_header.is_empty();

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

    let mut full = backend.file_header_for_results(&all_codes);
    full.push('\n');

    if use_class_wrapper {
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
        full.push_str(&class_header);
        full.push('\n');
        for code in &all_codes {
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

            eprintln!("\n=== {} ===\n{}\n=== END ===\n", $backend, code);

            let structural_errors = validate_structural(&code, $backend);
            assert!(
                structural_errors.is_empty(),
                "{} structural: {:?}",
                $backend,
                structural_errors
            );

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

            eprintln!("\n=== {} (with options) ===\n{}\n=== END ===\n", $backend, code);

            let structural_errors = validate_structural(&code, $backend);
            assert!(
                structural_errors.is_empty(),
                "{} structural: {:?}",
                $backend,
                structural_errors
            );

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

            eprintln!("\n=== {} ===\n{}\n=== END ===\n", $backend, code);

            let structural_errors = validate_structural(&code, $backend);
            assert!(
                structural_errors.is_empty(),
                "{} structural: {:?}",
                $backend,
                structural_errors
            );

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

fn generate_full_file_duckdb(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "duckdb").unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::PostgreSQL)
}

macro_rules! duckdb_backend_test {
    ($name:ident, $backend:expr) => {
        #[test]
        fn $name() {
            let code = generate_full_file_duckdb($backend);
            assert!(
                !code.trim().is_empty(),
                "generated code is empty for {}",
                $backend
            );

            eprintln!("\n=== {} ===\n{}\n=== END ===\n", $backend, code);

            let structural_errors = validate_structural(&code, $backend);
            assert!(
                structural_errors.is_empty(),
                "{} structural: {:?}",
                $backend,
                structural_errors
            );

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
backend_test!(test_java_r2dbc, "java-r2dbc");
backend_test!(test_kotlin_jdbc, "kotlin-jdbc");
backend_test!(test_kotlin_r2dbc, "kotlin-r2dbc");
backend_test!(test_csharp_npgsql, "csharp-npgsql");
backend_test!(test_elixir_postgrex, "elixir-postgrex");
backend_test!(test_elixir_ecto, "elixir-ecto");
backend_test!(test_ruby_pg, "ruby-pg");
backend_test!(test_php_pdo, "php-pdo");
backend_test!(test_php_amphp, "php-amphp");
backend_test!(test_kotlin_exposed, "kotlin-exposed");

duckdb_backend_test!(test_python_duckdb, "python-duckdb");
duckdb_backend_test!(test_typescript_duckdb, "typescript-duckdb");

mysql_backend_test!(test_ruby_trilogy, "ruby-trilogy");

backend_test_with_options!(test_python_psycopg3_pydantic, "python-psycopg3", "row_type" => "pydantic");
backend_test_with_options!(test_python_psycopg3_msgspec, "python-psycopg3", "row_type" => "msgspec");
backend_test_with_options!(test_python_asyncpg_pydantic, "python-asyncpg", "row_type" => "pydantic");
backend_test_with_options!(test_typescript_pg_zod, "typescript-pg", "row_type" => "zod");
backend_test_with_options!(test_typescript_postgres_zod, "typescript-postgres", "row_type" => "zod");

const SCHEMA_UUID_JSONB: &str = "CREATE TABLE items (\
    id UUID PRIMARY KEY, \
    name TEXT NOT NULL, \
    metadata JSONB\
);";

const QUERY_UUID: &str = "-- @name GetItem\n-- @returns :one\n\
    SELECT id, name, metadata FROM items WHERE id = $1;";

fn generate_header_for_uuid_jsonb_schema(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").unwrap();
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA_UUID_JSONB], &SqlDialect::PostgreSQL).unwrap();
    let parsed = parse_query_with_dialect(QUERY_UUID, &SqlDialect::PostgreSQL).unwrap();
    let analyzed = analyze(&catalog, &parsed).unwrap();
    let _ = generate_with_backend(&analyzed, &*backend).unwrap();
    backend.file_header()
}

fn generate_header_for_uuid_jsonb_schema_mysql(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "mysql").unwrap();
    backend.file_header()
}

#[test]
fn test_python_psycopg3_header_contains_uuid_and_any_imports() {
    let header = generate_header_for_uuid_jsonb_schema("python-psycopg3");
    eprintln!("psycopg3 header:\n{}", header);
    assert!(
        header.contains("import uuid  # noqa: F401"),
        "psycopg3 header missing `import uuid  # noqa: F401`\nHeader:\n{}",
        header
    );
    assert!(
        header.contains("from typing import Any  # noqa: F401"),
        "psycopg3 header missing `from typing import Any  # noqa: F401`\nHeader:\n{}",
        header
    );
}

#[test]
fn test_python_asyncpg_header_contains_uuid_and_any_imports() {
    let header = generate_header_for_uuid_jsonb_schema("python-asyncpg");
    eprintln!("asyncpg header:\n{}", header);
    assert!(
        header.contains("import uuid  # noqa: F401"),
        "asyncpg header missing `import uuid  # noqa: F401`\nHeader:\n{}",
        header
    );
    assert!(
        header.contains("from typing import Any  # noqa: F401"),
        "asyncpg header missing `from typing import Any  # noqa: F401`\nHeader:\n{}",
        header
    );
}

#[test]
fn test_python_aiomysql_header_contains_any_but_not_uuid_import() {
    let header = generate_header_for_uuid_jsonb_schema_mysql("python-aiomysql");
    eprintln!("aiomysql header:\n{}", header);
    assert!(
        header.contains("from typing import Any  # noqa: F401"),
        "aiomysql header missing `from typing import Any  # noqa: F401`\nHeader:\n{}",
        header
    );
    assert!(
        !header.contains("import uuid"),
        "aiomysql header should NOT contain `import uuid` (uuid maps to str)\nHeader:\n{}",
        header
    );
}

#[test]
fn test_php_pdo_default_namespace() {
    let code = generate_full_file("php-pdo");
    assert!(
        code.contains("namespace App\\Generated;"),
        "php-pdo default header must contain 'namespace App\\Generated;', got:\n{}",
        &code[..code.len().min(300)]
    );
}

#[test]
fn test_php_pdo_custom_namespace() {
    let mut options = std::collections::HashMap::new();
    options.insert("namespace".to_string(), "App\\Database\\Generated".to_string());
    let code = generate_full_file_with_options("php-pdo", &options);
    assert!(
        code.contains("namespace App\\Database\\Generated;"),
        "php-pdo custom namespace header must contain 'namespace App\\Database\\Generated;', got:\n{}",
        &code[..code.len().min(300)]
    );
    assert!(
        !code.contains("namespace App\\Generated;"),
        "php-pdo custom namespace header must not contain the default 'namespace App\\Generated;'"
    );
}

#[test]
fn test_php_pdo_empty_namespace() {
    let mut options = std::collections::HashMap::new();
    options.insert("namespace".to_string(), String::new());
    let code = generate_full_file_with_options("php-pdo", &options);
    assert!(
        !code.contains("namespace "),
        "php-pdo empty namespace header must not contain any 'namespace ' line, got:\n{}",
        &code[..code.len().min(300)]
    );
    assert!(
        code.contains("<?php"),
        "php-pdo empty namespace header must still contain '<?php'"
    );
    assert!(
        code.contains("declare(strict_types=1);"),
        "php-pdo empty namespace header must still contain 'declare(strict_types=1);'"
    );
    assert!(
        code.contains("// Auto-generated by scythe. Do not edit."),
        "php-pdo empty namespace header must still contain the auto-generated comment"
    );
}

#[test]
fn test_php_amphp_default_namespace() {
    let code = generate_full_file("php-amphp");
    assert!(
        code.contains("namespace App\\Generated;"),
        "php-amphp default header must contain 'namespace App\\Generated;', got:\n{}",
        &code[..code.len().min(300)]
    );
}

#[test]
fn test_php_amphp_custom_namespace() {
    let mut options = std::collections::HashMap::new();
    options.insert("namespace".to_string(), "App\\Database\\Generated".to_string());
    let code = generate_full_file_with_options("php-amphp", &options);
    assert!(
        code.contains("namespace App\\Database\\Generated;"),
        "php-amphp custom namespace header must contain 'namespace App\\Database\\Generated;', got:\n{}",
        &code[..code.len().min(300)]
    );
    assert!(
        !code.contains("namespace App\\Generated;"),
        "php-amphp custom namespace header must not contain the default 'namespace App\\Generated;'"
    );
}

#[test]
fn test_php_amphp_empty_namespace() {
    let mut options = std::collections::HashMap::new();
    options.insert("namespace".to_string(), String::new());
    let code = generate_full_file_with_options("php-amphp", &options);
    assert!(
        !code.contains("namespace "),
        "php-amphp empty namespace header must not contain any 'namespace ' line, got:\n{}",
        &code[..code.len().min(300)]
    );
    assert!(
        code.contains("<?php"),
        "php-amphp empty namespace header must still contain '<?php'"
    );
    assert!(
        code.contains("declare(strict_types=1);"),
        "php-amphp empty namespace header must still contain 'declare(strict_types=1);'"
    );
    assert!(
        code.contains("// Auto-generated by scythe. Do not edit."),
        "php-amphp empty namespace header must still contain the auto-generated comment"
    );
}

#[test]
fn test_kotlin_jdbc_extension_functions_signature() {
    let mut options = std::collections::HashMap::new();
    options.insert("extension_functions".to_string(), "true".to_string());
    let code = generate_full_file_with_options("kotlin-jdbc", &options);

    assert!(
        code.contains("fun Connection."),
        "kotlin-jdbc ext: expected 'fun Connection.' in output\n\nGenerated:\n{code}"
    );
    assert!(
        !code.contains("conn: Connection"),
        "kotlin-jdbc ext: unexpected 'conn: Connection' param\n\nGenerated:\n{code}"
    );
}

#[test]
fn test_kotlin_jdbc_extension_functions_expression_body() {
    let mut options = std::collections::HashMap::new();
    options.insert("extension_functions".to_string(), "true".to_string());
    let code = generate_full_file_with_options("kotlin-jdbc", &options);

    assert!(
        code.contains("): List<") && code.contains("> =\n    this.prepareStatement"),
        "kotlin-jdbc ext: expected expression body for :many with 'this.prepareStatement'\n\nGenerated:\n{code}"
    );
    assert!(
        code.contains("? =\n    this.prepareStatement"),
        "kotlin-jdbc ext: expected expression body for :one with 'this.prepareStatement'\n\nGenerated:\n{code}"
    );
}

#[test]
fn test_kotlin_jdbc_extension_functions_exec_block_body() {
    let mut options = std::collections::HashMap::new();
    options.insert("extension_functions".to_string(), "true".to_string());
    let code = generate_full_file_with_options("kotlin-jdbc", &options);

    assert!(
        code.contains(") {\n    this.prepareStatement"),
        "kotlin-jdbc ext: expected block body for :exec with 'this.prepareStatement'\n\nGenerated:\n{code}"
    );
}

#[test]
fn test_kotlin_jdbc_extension_functions_default_off() {
    let code = generate_full_file("kotlin-jdbc");
    assert!(
        !code.contains("fun Connection."),
        "kotlin-jdbc default: unexpected 'fun Connection.' when extension_functions=false\n\nGenerated:\n{code}"
    );
    assert!(
        code.contains("conn: Connection"),
        "kotlin-jdbc default: expected 'conn: Connection' param\n\nGenerated:\n{code}"
    );
}

#[test]
fn test_kotlin_r2dbc_extension_functions_signature() {
    let mut options = std::collections::HashMap::new();
    options.insert("extension_functions".to_string(), "true".to_string());
    let code = generate_full_file_with_options("kotlin-r2dbc", &options);

    assert!(
        code.contains("suspend fun Connection.") || code.contains("fun Connection."),
        "kotlin-r2dbc ext: expected 'fun Connection.' or 'suspend fun Connection.' in output\n\nGenerated:\n{code}"
    );
    assert!(
        !code.contains("cf: ConnectionFactory"),
        "kotlin-r2dbc ext: unexpected 'cf: ConnectionFactory' param\n\nGenerated:\n{code}"
    );
    assert!(
        !code.contains("cf.create()"),
        "kotlin-r2dbc ext: unexpected 'cf.create()' in body\n\nGenerated:\n{code}"
    );
}

#[test]
fn test_kotlin_r2dbc_extension_functions_header() {
    let mut options = std::collections::HashMap::new();
    options.insert("extension_functions".to_string(), "true".to_string());
    let code = generate_full_file_with_options("kotlin-r2dbc", &options);

    assert!(
        code.contains("import io.r2dbc.spi.Connection\n"),
        "kotlin-r2dbc ext: expected 'import io.r2dbc.spi.Connection' in header\n\nGenerated:\n{code}"
    );
    assert!(
        !code.contains("import io.r2dbc.spi.ConnectionFactory"),
        "kotlin-r2dbc ext: unexpected 'ConnectionFactory' import in header\n\nGenerated:\n{code}"
    );
}

#[test]
fn test_kotlin_r2dbc_extension_functions_default_off() {
    let code = generate_full_file("kotlin-r2dbc");
    assert!(
        code.contains("cf: ConnectionFactory"),
        "kotlin-r2dbc default: expected 'cf: ConnectionFactory' param\n\nGenerated:\n{code}"
    );
    assert!(
        !code.contains("fun Connection."),
        "kotlin-r2dbc default: unexpected 'fun Connection.' when extension_functions=false\n\nGenerated:\n{code}"
    );
    assert!(
        code.contains("cf.create()"),
        "kotlin-r2dbc default: expected 'cf.create()' in body\n\nGenerated:\n{code}"
    );
}
