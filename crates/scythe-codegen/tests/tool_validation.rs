//! Validate generated code for all backends using real language tools.
//! All tools are expected to be installed.

use scythe_codegen::provenance;
use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_structural, validate_with_tools};
use scythe_codegen::{CodegenBackend, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

// ~keep #146: the PostgreSQL schema is the one shared by every full
// generate-and-compile round in this file except the MySQL/SQLite/MSSQL/
// Oracle/Snowflake ones below (see each dialect's own schema for why those
// stay SERIAL/TEXT/VARCHAR/TIMESTAMP-only -- PostgreSQL is the only dialect
// here with a real array element type and `CREATE TYPE`). Before this it
// carried zero container or user-defined types, so nothing in this file's
// ~20 PostgreSQL backend tests -- the strongest gate in the project -- ever
// asked a real compiler to accept an array, an enum, an array of enums, a
// composite, a `uuid`, or a `jsonb` column. `role`/`roles`/`tags`/
// `home_address`/`external_id`/`metadata` below exist to close exactly that
// gap; every one of them is selected by `QUERY_ONE`, the query every
// PostgreSQL backend test in this file runs, so widening the schema without
// also widening the query would have added dead columns no generated file
// ever reaches.
const SCHEMA: &str = "CREATE TYPE user_role AS ENUM ('member', 'admin'); \
    CREATE TYPE user_address AS (street TEXT, city TEXT, zip TEXT); \
    CREATE TABLE users (\
    id SERIAL PRIMARY KEY, \
    name TEXT NOT NULL, \
    email TEXT, \
    status TEXT NOT NULL DEFAULT 'active', \
    role user_role NOT NULL DEFAULT 'member', \
    roles user_role[], \
    tags TEXT[], \
    home_address user_address, \
    external_id UUID, \
    metadata JSONB, \
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
);";

const QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, email, status, role, roles, tags, home_address, external_id, metadata, \
    created_at FROM users WHERE id = $1;";

const QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, email FROM users ORDER BY name;";

const QUERY_EXEC: &str = "-- @name DeleteUser\n-- @returns :exec\n\
    DELETE FROM users WHERE id = $1;";

// ~keep #146: MySQL has no `CREATE TYPE` and no array element type, but it
// does have an inline `ENUM(...)` column -- a real user-defined-type family
// this file's MySQL-backed tests (ruby-trilogy, csharp-mysqlconnector,
// elixir-myxql, javascript-mysql2) had no coverage of at all. `role` is
// selected by `MYSQL_QUERY_ONE` for the same reason `role` was added to the
// PostgreSQL `QUERY_ONE` above -- an unselected column proves nothing.
const MYSQL_SCHEMA: &str = "CREATE TABLE users (\
    id INT AUTO_INCREMENT PRIMARY KEY, \
    name VARCHAR(255) NOT NULL, \
    email VARCHAR(255), \
    status VARCHAR(50) NOT NULL DEFAULT 'active', \
    role ENUM('member', 'admin') NOT NULL DEFAULT 'member', \
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP\
);";

const MYSQL_QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, email, status, role, created_at FROM users WHERE id = ?;";

const MYSQL_QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, email FROM users ORDER BY name;";

const MYSQL_QUERY_EXEC: &str = "-- @name DeleteUser\n-- @returns :exec\n\
    DELETE FROM users WHERE id = ?;";

const SQLITE_SCHEMA: &str = "CREATE TABLE users (\
    id INTEGER PRIMARY KEY AUTOINCREMENT, \
    name TEXT NOT NULL, \
    email TEXT, \
    status TEXT NOT NULL DEFAULT 'active', \
    created_at TEXT NOT NULL\
);";

const SQLITE_QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, email, status, created_at FROM users WHERE id = ?;";

const SQLITE_QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, email FROM users ORDER BY name;";

const SQLITE_QUERY_EXEC: &str = "-- @name DeleteUser\n-- @returns :exec\n\
    DELETE FROM users WHERE id = ?;";

const MSSQL_SCHEMA: &str = "CREATE TABLE users (\
    id INT IDENTITY(1,1) PRIMARY KEY, \
    name NVARCHAR(255) NOT NULL, \
    email NVARCHAR(255), \
    status NVARCHAR(50) NOT NULL DEFAULT 'active', \
    created_at DATETIME2 NOT NULL\
);";

const MSSQL_QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, email, status, created_at FROM users WHERE id = @p1;";

const MSSQL_QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, email FROM users ORDER BY name;";

const MSSQL_QUERY_EXEC: &str = "-- @name DeleteUser\n-- @returns :exec\n\
    DELETE FROM users WHERE id = @p1;";

const ORACLE_SCHEMA: &str = "CREATE TABLE users (\
    id NUMBER(10) PRIMARY KEY, \
    name VARCHAR2(255) NOT NULL, \
    email VARCHAR2(255), \
    status VARCHAR2(50) DEFAULT 'active' NOT NULL, \
    created_at TIMESTAMP NOT NULL\
);";

const ORACLE_QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, email, status, created_at FROM users WHERE id = :1;";

const ORACLE_QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, email FROM users ORDER BY name;";

const ORACLE_QUERY_EXEC: &str = "-- @name DeleteUser\n-- @returns :exec\n\
    DELETE FROM users WHERE id = :1;";

const SNOWFLAKE_SCHEMA: &str = "CREATE TABLE users (\
    id INTEGER PRIMARY KEY, \
    name VARCHAR(255) NOT NULL, \
    email VARCHAR(255), \
    status VARCHAR(50) NOT NULL DEFAULT 'active', \
    created_at TIMESTAMP_NTZ NOT NULL\
);";

const SNOWFLAKE_QUERY_ONE: &str = "-- @name GetUser\n-- @returns :one\n\
    SELECT id, name, email, status, created_at FROM users WHERE id = ?;";

const SNOWFLAKE_QUERY_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, email FROM users ORDER BY name;";

const SNOWFLAKE_QUERY_EXEC: &str = "-- @name DeleteUser\n-- @returns :exec\n\
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

fn generate_full_file_sqlite(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "sqlite").unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::SQLite)
}

fn generate_full_file_mssql(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "mssql").unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::MsSql)
}

fn generate_full_file_oracle(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "oracle").unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::Oracle)
}

fn generate_full_file_snowflake(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "snowflake").unwrap();
    generate_full_file_from_backend(backend_name, &*backend, &SqlDialect::Snowflake)
}

fn generate_full_file_from_backend(backend_name: &str, backend: &dyn CodegenBackend, dialect: &SqlDialect) -> String {
    // ~keep The engine alias is carried alongside the fixtures purely so the
    // provenance header this harness emits below says something truthful;
    // nothing about the languages' acceptance of that line depends on it.
    let (schema, queries, engine) = match dialect {
        SqlDialect::MySQL => (
            MYSQL_SCHEMA,
            [MYSQL_QUERY_ONE, MYSQL_QUERY_MANY, MYSQL_QUERY_EXEC],
            "mysql",
        ),
        SqlDialect::SQLite => (
            SQLITE_SCHEMA,
            [SQLITE_QUERY_ONE, SQLITE_QUERY_MANY, SQLITE_QUERY_EXEC],
            "sqlite",
        ),
        SqlDialect::MsSql => (
            MSSQL_SCHEMA,
            [MSSQL_QUERY_ONE, MSSQL_QUERY_MANY, MSSQL_QUERY_EXEC],
            "mssql",
        ),
        SqlDialect::Oracle => (
            ORACLE_SCHEMA,
            [ORACLE_QUERY_ONE, ORACLE_QUERY_MANY, ORACLE_QUERY_EXEC],
            "oracle",
        ),
        SqlDialect::Snowflake => (
            SNOWFLAKE_SCHEMA,
            [SNOWFLAKE_QUERY_ONE, SNOWFLAKE_QUERY_MANY, SNOWFLAKE_QUERY_EXEC],
            "snowflake",
        ),
        _ => (SCHEMA, [QUERY_ONE, QUERY_MANY, QUERY_EXEC], "postgresql"),
    };

    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).unwrap();

    let class_header = backend.query_class_header();
    let use_class_wrapper = !class_header.is_empty();

    // ~keep Swallowing a codegen error here (and letting the loop continue with
    // whatever queries did succeed) let a broken backend pass this harness as
    // long as at least one of the three queries still produced a struct and a
    // function -- the empty-file backstop below can never catch it, since
    // `provenance::assemble_file` always prepends a non-empty header line
    // regardless of how many query bodies actually made it into `all_codes`.
    // A partial failure must fail the test loudly, naming the backend and the
    // query that broke, not get logged to a captured stderr no one reads.
    let mut all_codes = Vec::new();
    for query_sql in queries {
        let parsed = parse_query_with_dialect(query_sql, dialect).unwrap();
        let analyzed = analyze(&catalog, &parsed).unwrap();
        let code = generate_with_backend(&analyzed, backend)
            .unwrap_or_else(|e| panic!("codegen error for backend {backend_name} on query {query_sql:?}: {e}"));
        all_codes.push(code);
    }

    // ~keep Only the *body* is accumulated here. The preamble and the provenance
    // header line are added by `provenance::assemble_file` at the bottom of
    // this function, in the one place that owns their ordering — the same
    // call `scythe-cli`'s `assemble_output` makes.
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

    // ~keep The point of this harness is that `php -l`, `ruby -c`, `gofmt`,
    // `python -m py_compile`, `tsc`, and `kotlinc` below are handed the bytes
    // a real `scythe generate` writes — provenance header included. Without
    // it, nothing anywhere proves that the header sits in a position each
    // language actually accepts: the structural checks in `validation.rs` are
    // substring matches (`code.contains("<?php")`) that pass regardless of
    // what precedes them, and the ordering assertions in `scythe-cli` are
    // Rust string comparisons that no language tool ever sees. PHP is the
    // sharpest case — `declare(strict_types=1);` must be the file's first
    // *statement*, so only a real `php -l` can confirm a preceding comment is
    // allowed there.
    //
    // The version, engine, schema, and queries values are placeholders: the
    // header's legality depends on its comment prefix and its position, not
    // on its field values.
    provenance::assemble_file(
        &backend.file_preamble(),
        &provenance::header_line(
            backend,
            env!("CARGO_PKG_VERSION"),
            engine,
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &full,
    )
}

/// Guard against a validator that is wired up but checks nothing.
///
/// `validate_with_tools` returning `Attempted(vec![])` -- a match arm that
/// builds an empty outcome list, e.g. a future backend added to an existing
/// prefix match without adding its own checker call -- reports zero errors
/// and zero missing tools. `into_result`, even in strict mode, therefore
/// treats it exactly like a `Ran` outcome that found nothing: a clean pass.
/// That is the same "ran vs. never ran" confusion `ToolValidation` exists to
/// prevent in the first place (see `fully_checked`'s own doc comment),
/// reached through a different bug -- and it is the reason `fully_checked`
/// needs a caller here rather than staying something only its own unit tests
/// exercise.
///
/// Gated on strict mode, same as `into_result`'s missing-tool handling:
/// outside strict mode a validator that only ran a subset of its checkers
/// (because one is not installed) is expected, so `fully_checked` legitimately
/// returns `false` on every laptop that doesn't have every tool. `Unsupported`
/// is exempt for the same reason `into_result_with_strictness` exempts it --
/// tracked by the inventory test below instead of failing here with no fix
/// available.
fn assert_tool_validation_is_not_vacuous(validation: &ToolValidation, backend: &str, code: &str) {
    if !strict_mode_enabled() {
        return;
    }
    let unsupported = matches!(validation, ToolValidation::Unsupported);
    assert!(
        unsupported || validation.fully_checked(),
        "{backend} tool validation ran but `fully_checked()` reports nothing actually checked \
         the code -- a validator that dispatches to zero checkers passes vacuously.\n\n\
         Generated code:\n{code}"
    );
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

            let validation = validate_with_tools(&code, $backend);
            assert_tool_validation_is_not_vacuous(&validation, $backend, &code);
            if let Err(tool_errors) = validation.into_result() {
                panic!(
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend, tool_errors, code
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

            let validation = validate_with_tools(&code, $backend);
            assert_tool_validation_is_not_vacuous(&validation, $backend, &code);
            if let Err(tool_errors) = validation.into_result() {
                panic!(
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend, tool_errors, code
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

            let validation = validate_with_tools(&code, $backend);
            assert_tool_validation_is_not_vacuous(&validation, $backend, &code);
            if let Err(tool_errors) = validation.into_result() {
                panic!(
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend, tool_errors, code
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

            let validation = validate_with_tools(&code, $backend);
            assert_tool_validation_is_not_vacuous(&validation, $backend, &code);
            if let Err(tool_errors) = validation.into_result() {
                panic!(
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend, tool_errors, code
                );
            }
        }
    };
}

macro_rules! sqlite_backend_test {
    ($name:ident, $backend:expr) => {
        #[test]
        fn $name() {
            let code = generate_full_file_sqlite($backend);
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

            let validation = validate_with_tools(&code, $backend);
            assert_tool_validation_is_not_vacuous(&validation, $backend, &code);
            if let Err(tool_errors) = validation.into_result() {
                panic!(
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend, tool_errors, code
                );
            }
        }
    };
}

macro_rules! mssql_backend_test {
    ($name:ident, $backend:expr) => {
        #[test]
        fn $name() {
            let code = generate_full_file_mssql($backend);
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

            let validation = validate_with_tools(&code, $backend);
            assert_tool_validation_is_not_vacuous(&validation, $backend, &code);
            if let Err(tool_errors) = validation.into_result() {
                panic!(
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend, tool_errors, code
                );
            }
        }
    };
}

macro_rules! oracle_backend_test {
    ($name:ident, $backend:expr) => {
        #[test]
        fn $name() {
            let code = generate_full_file_oracle($backend);
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

            let validation = validate_with_tools(&code, $backend);
            assert_tool_validation_is_not_vacuous(&validation, $backend, &code);
            if let Err(tool_errors) = validation.into_result() {
                panic!(
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend, tool_errors, code
                );
            }
        }
    };
}

macro_rules! snowflake_backend_test {
    ($name:ident, $backend:expr) => {
        #[test]
        fn $name() {
            let code = generate_full_file_snowflake($backend);
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

            let validation = validate_with_tools(&code, $backend);
            assert_tool_validation_is_not_vacuous(&validation, $backend, &code);
            if let Err(tool_errors) = validation.into_result() {
                panic!(
                    "{} tool validation: {:?}\n\nGenerated code:\n{}",
                    $backend, tool_errors, code
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
backend_test!(test_typescript_kysely, "typescript-kysely");
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

sqlite_backend_test!(test_typescript_better_sqlite3, "typescript-better-sqlite3");
sqlite_backend_test!(test_typescript_node_sqlite, "typescript-node-sqlite");
sqlite_backend_test!(test_typescript_wasm_sqlite, "typescript-wasm-sqlite");

duckdb_backend_test!(test_python_duckdb, "python-duckdb");
duckdb_backend_test!(test_typescript_duckdb, "typescript-duckdb");

mysql_backend_test!(test_ruby_trilogy, "ruby-trilogy");
mysql_backend_test!(test_csharp_mysqlconnector, "csharp-mysqlconnector");
mysql_backend_test!(test_elixir_myxql, "elixir-myxql");

sqlite_backend_test!(test_csharp_microsoft_sqlite, "csharp-microsoft-sqlite");
sqlite_backend_test!(test_elixir_exqlite, "elixir-exqlite");

mssql_backend_test!(test_typescript_mssql, "typescript-mssql");
mssql_backend_test!(test_csharp_sqlclient, "csharp-sqlclient");
mssql_backend_test!(test_elixir_tds, "elixir-tds");
mssql_backend_test!(test_python_pyodbc, "python-pyodbc");
mssql_backend_test!(test_ruby_tiny_tds, "ruby-tiny-tds");
mssql_backend_test!(test_rust_tiberius, "rust-tiberius");

oracle_backend_test!(test_typescript_oracledb, "typescript-oracledb");
oracle_backend_test!(test_csharp_oracle, "csharp-oracle");
oracle_backend_test!(test_elixir_jamdb, "elixir-jamdb");
oracle_backend_test!(test_python_oracledb, "python-oracledb");
oracle_backend_test!(test_go_godror, "go-godror");
oracle_backend_test!(test_ruby_oci8, "ruby-oci8");
oracle_backend_test!(test_rust_sibyl, "rust-sibyl");

snowflake_backend_test!(test_typescript_snowflake, "typescript-snowflake");
snowflake_backend_test!(test_csharp_snowflake, "csharp-snowflake");
snowflake_backend_test!(test_python_snowflake, "python-snowflake");
snowflake_backend_test!(test_go_gosnowflake, "go-gosnowflake");

backend_test_with_options!(test_python_psycopg3_pydantic, "python-psycopg3", "row_type" => "pydantic");
backend_test_with_options!(test_python_psycopg3_msgspec, "python-psycopg3", "row_type" => "msgspec");
backend_test_with_options!(test_python_asyncpg_pydantic, "python-asyncpg", "row_type" => "pydantic");
backend_test_with_options!(test_typescript_pg_zod, "typescript-pg", "row_type" => "zod");
backend_test_with_options!(test_typescript_kysely_zod, "typescript-kysely", "row_type" => "zod");
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
        code.contains("// scythe:provenance"),
        "php-pdo empty namespace header must still contain the provenance comment"
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
        code.contains("// scythe:provenance"),
        "php-amphp empty namespace header must still contain the provenance comment"
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
    // ~keep This asserted `"? =\n"` until #192 -- a nullable return, which is
    // `:opt`'s shape, not `:one`'s. The fixture's only single-row query is
    // `:one`, so the assertion was pinning the very fold #192 fixed: `:one`
    // inheriting `:opt`'s permissiveness. It must stay non-nullable.
    assert!(
        code.contains("): GetUserRow =\n    this.prepareStatement"),
        "kotlin-jdbc ext: expected a non-nullable expression body for :one with 'this.prepareStatement'\n\nGenerated:\n{code}"
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

// -- javascript-* (JSDoc emit mode, #81) -------------------------------------
//
// These backends reuse `generate_full_file*`/`backend_test!` above for
// their `:one`/`:many`/`:exec` coverage (each dialect's schema already has
// a nullable `email` column, so that path is exercised for free). The tests
// below add the two cases #81's release plan calls out explicitly and that
// produced real defects during development: a nullable column emitting
// `{T | null}` (never JSDoc's bracket-optional or `?`-suffix syntax), and a
// `:grouped` query (where the mysql2/better-sqlite3 untyped-return defects
// lived). Both are run through `validate_with_tools`, which for a
// `javascript-*` backend name now shells out to real `node --check` and
// (if present) real `tsc --checkJs --strict` -- see `validate_javascript_tools`
// in `src/validation.rs`.

backend_test!(test_javascript_pg, "javascript-pg");
backend_test!(test_javascript_postgres, "javascript-postgres");
mysql_backend_test!(test_javascript_mysql2, "javascript-mysql2");
sqlite_backend_test!(test_javascript_better_sqlite3, "javascript-better-sqlite3");

const JS_MODE_PG_SCHEMA: [&str; 2] = [
    "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, bio TEXT);",
    "CREATE TABLE orders (id SERIAL PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), total NUMERIC);",
];
const JS_MODE_PG_ONE: &str = "-- @name GetUserById\n-- @returns :one\n\
    SELECT id, name, bio FROM users WHERE id = $1;";

const JS_MODE_MYSQL_SCHEMA: [&str; 2] = [
    "CREATE TABLE users (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(255) NOT NULL, bio TEXT);",
    "CREATE TABLE orders (id INT AUTO_INCREMENT PRIMARY KEY, user_id INT NOT NULL, total DECIMAL(10,2));",
];
const JS_MODE_MYSQL_ONE: &str = "-- @name GetUserById\n-- @returns :one\n\
    SELECT id, name, bio FROM users WHERE id = ?;";

const JS_MODE_SQLITE_SCHEMA: [&str; 2] = [
    "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, bio TEXT);",
    "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, total REAL);",
];
const JS_MODE_SQLITE_ONE: &str = "-- @name GetUserById\n-- @returns :one\n\
    SELECT id, name, bio FROM users WHERE id = ?;";

const JS_MODE_SNOWFLAKE_SCHEMA: [&str; 2] = [
    "CREATE TABLE users (id INTEGER PRIMARY KEY, name VARCHAR(255) NOT NULL, bio VARCHAR(255));",
    "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, total NUMERIC);",
];
const JS_MODE_SNOWFLAKE_ONE: &str = "-- @name GetUserById\n-- @returns :one\n\
    SELECT id, name, bio FROM users WHERE id = ?;";

// ~keep `:many` is the one command whose JSDoc cast is not a straight
// `/** @type {T} */ (expr)`: a driver whose row-fetch returns a concrete
// record type rather than `unknown` cannot be asserted directly to a row
// interface, and the backend has to route the cast through an intermediate
// `unknown`. Nothing checked that against real `tsc` until this query
// existed -- the fixture built only `:one` and `:grouped`, so every
// `javascript-*` backend's `:many` cast was pinned by hand-written string
// assertions alone. Placeholder-free, like the grouped query below, so one
// spelling covers all three engines.
const JS_MODE_MANY: &str = "-- @name ListUsers\n-- @returns :many\n\
    SELECT id, name, bio FROM users;";

// No dialect-specific placeholders (no WHERE clause), so one grouped query
// covers all three engines.
const JS_MODE_GROUPED: &str = "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
    SELECT u.id, u.name, o.id AS order_id, o.total\n\
    FROM users u\n\
    JOIN orders o ON o.user_id = u.id;";

/// Generate a full file for a `javascript-*` backend covering a nullable
/// column (`users.bio`, via `one_sql`) and a `:grouped` query
/// (`JS_MODE_GROUPED`) in one file, the same way `generate_full_file_from_backend`
/// assembles `:one`/`:many`/`:exec` output above.
fn generate_js_mode_nullable_and_grouped_file(
    backend_name: &str,
    engine: &str,
    dialect: &SqlDialect,
    schema: &[&str],
    one_sql: &str,
) -> String {
    let backend = get_backend(backend_name, engine).unwrap();
    let catalog = Catalog::from_ddl_with_dialect(schema, dialect).unwrap();

    let mut all_codes = Vec::new();
    for query_sql in [one_sql, JS_MODE_MANY, JS_MODE_GROUPED] {
        let parsed = parse_query_with_dialect(query_sql, dialect).unwrap();
        let analyzed = analyze(&catalog, &parsed).unwrap();
        let code = generate_with_backend(&analyzed, &*backend).unwrap();
        all_codes.push(code);
    }

    let mut full = backend.file_header_for_results(&all_codes);
    full.push('\n');
    for code in &all_codes {
        if let Some(ref s) = code.row_struct {
            full.push_str(s);
            full.push('\n');
        }
        if let Some(ref s) = code.query_fn {
            full.push_str(s);
            full.push('\n');
        }
    }
    full
}

/// Assert the JSDoc row-struct property for `field` is `{T | null}` -- never
/// JSDoc's bracket-optional (`[field]`) or `?`-suffix optional syntax, which
/// mean the property may be *absent* rather than present-and-`null`.
fn assert_js_mode_nullable_property_is_type_or_null(code: &str, field: &str) {
    assert!(
        code.contains(&format!("| null}} {field}")),
        "expected a `{{T | null}}` JSDoc property for '{field}'; got:\n{code}"
    );
    assert!(
        !code.contains(&format!("[{field}]")),
        "must never use bracket-optional syntax for '{field}'; got:\n{code}"
    );
    assert!(
        !code.contains(&format!("{field}?")),
        "must never use `?`-suffix optional syntax for '{field}'; got:\n{code}"
    );
}

/// Run the real `node`/`tsc` tool validation (`validate_with_tools`) and
/// fail loudly if it ran and found errors.
///
/// Outside strict mode an uninstalled `node` or `tsc` is not a test failure
/// -- requiring every contributor to have both would be the wrong tradeoff
/// -- but the skip is printed at this call site so `cargo test --
/// --nocapture` makes it impossible to mistake for a pass. Under
/// `SCYTHE_VALIDATE_STRICT=1`, which CI sets after installing both,
/// `into_result` turns that same skip into a failure.
///
/// This wrapper predates `ToolValidation` and hand-rolled the same
/// skip-is-not-a-pass distinction for these two tools only; it now defers to
/// the type so there is one mechanism rather than two.
fn assert_js_mode_tool_validation_passes(backend_name: &str, code: &str) {
    let validation = validate_with_tools(code, backend_name);

    for tool in validation.tools_run() {
        eprintln!("  {backend_name}: `{tool}` ran against the generated code");
    }
    for tool in validation.missing_tools() {
        eprintln!("  {backend_name}: `{tool}` is not on PATH -- whatever it would have caught went unchecked");
    }

    if let Err(errors) = validation.into_result() {
        panic!("{backend_name} tool validation: {errors:?}\n\nGenerated code:\n{code}");
    }
}

#[test]
fn test_javascript_pg_grouped_and_nullable_pass_real_tools() {
    let code = generate_js_mode_nullable_and_grouped_file(
        "javascript-pg",
        "postgresql",
        &SqlDialect::PostgreSQL,
        &JS_MODE_PG_SCHEMA,
        JS_MODE_PG_ONE,
    );
    eprintln!("\n=== javascript-pg (nullable + grouped) ===\n{code}\n=== END ===\n");
    assert_js_mode_nullable_property_is_type_or_null(&code, "bio");
    assert!(
        code.contains("@typedef {object} GetUsersWithOrdersRow"),
        "expected the grouped parent typedef; got:\n{code}"
    );
    assert_js_mode_tool_validation_passes("javascript-pg", &code);
}

#[test]
fn test_javascript_postgres_grouped_and_nullable_pass_real_tools() {
    let code = generate_js_mode_nullable_and_grouped_file(
        "javascript-postgres",
        "postgresql",
        &SqlDialect::PostgreSQL,
        &JS_MODE_PG_SCHEMA,
        JS_MODE_PG_ONE,
    );
    eprintln!("\n=== javascript-postgres (nullable + grouped) ===\n{code}\n=== END ===\n");
    assert_js_mode_nullable_property_is_type_or_null(&code, "bio");
    assert!(
        code.contains("@typedef {object} GetUsersWithOrdersRow"),
        "expected the grouped parent typedef; got:\n{code}"
    );
    assert_js_mode_tool_validation_passes("javascript-postgres", &code);
}

#[test]
fn test_javascript_mysql2_grouped_and_nullable_pass_real_tools() {
    let code = generate_js_mode_nullable_and_grouped_file(
        "javascript-mysql2",
        "mysql",
        &SqlDialect::MySQL,
        &JS_MODE_MYSQL_SCHEMA,
        JS_MODE_MYSQL_ONE,
    );
    eprintln!("\n=== javascript-mysql2 (nullable + grouped) ===\n{code}\n=== END ===\n");
    assert_js_mode_nullable_property_is_type_or_null(&code, "bio");
    assert!(
        code.contains("@typedef {object} GetUsersWithOrdersRow"),
        "expected the grouped parent typedef; got:\n{code}"
    );
    assert_js_mode_tool_validation_passes("javascript-mysql2", &code);
}

#[test]
fn test_javascript_better_sqlite3_grouped_and_nullable_pass_real_tools() {
    let code = generate_js_mode_nullable_and_grouped_file(
        "javascript-better-sqlite3",
        "sqlite",
        &SqlDialect::SQLite,
        &JS_MODE_SQLITE_SCHEMA,
        JS_MODE_SQLITE_ONE,
    );
    eprintln!("\n=== javascript-better-sqlite3 (nullable + grouped) ===\n{code}\n=== END ===\n");
    assert_js_mode_nullable_property_is_type_or_null(&code, "bio");
    assert!(
        code.contains("@typedef {object} GetUsersWithOrdersRow"),
        "expected the grouped parent typedef; got:\n{code}"
    );
    assert_js_mode_tool_validation_passes("javascript-better-sqlite3", &code);
}

#[test]
fn test_javascript_node_sqlite_grouped_and_nullable_pass_real_tools() {
    let code = generate_js_mode_nullable_and_grouped_file(
        "javascript-node-sqlite",
        "sqlite",
        &SqlDialect::SQLite,
        &JS_MODE_SQLITE_SCHEMA,
        JS_MODE_SQLITE_ONE,
    );
    eprintln!("\n=== javascript-node-sqlite (nullable + grouped) ===\n{code}\n=== END ===\n");
    assert_js_mode_nullable_property_is_type_or_null(&code, "bio");
    assert!(
        code.contains("@typedef {object} GetUsersWithOrdersRow"),
        "expected the grouped parent typedef; got:\n{code}"
    );
    assert_js_mode_tool_validation_passes("javascript-node-sqlite", &code);
}

#[test]
fn test_javascript_wasm_sqlite_grouped_and_nullable_pass_real_tools() {
    let code = generate_js_mode_nullable_and_grouped_file(
        "javascript-wasm-sqlite",
        "sqlite",
        &SqlDialect::SQLite,
        &JS_MODE_SQLITE_SCHEMA,
        JS_MODE_SQLITE_ONE,
    );
    eprintln!("\n=== javascript-wasm-sqlite (nullable + grouped) ===\n{code}\n=== END ===\n");
    assert_js_mode_nullable_property_is_type_or_null(&code, "bio");
    assert!(
        code.contains("@typedef {object} GetUsersWithOrdersRow"),
        "expected the grouped parent typedef; got:\n{code}"
    );
    assert_js_mode_tool_validation_passes("javascript-wasm-sqlite", &code);
}

#[test]
fn test_javascript_snowflake_grouped_and_nullable_pass_real_tools() {
    let code = generate_js_mode_nullable_and_grouped_file(
        "javascript-snowflake",
        "snowflake",
        &SqlDialect::Snowflake,
        &JS_MODE_SNOWFLAKE_SCHEMA,
        JS_MODE_SNOWFLAKE_ONE,
    );
    eprintln!("\n=== javascript-snowflake (nullable + grouped) ===\n{code}\n=== END ===\n");
    assert_js_mode_nullable_property_is_type_or_null(&code, "bio");
    assert!(
        code.contains("@typedef {object} GetUsersWithOrdersRow"),
        "expected the grouped parent typedef; got:\n{code}"
    );
    assert_js_mode_tool_validation_passes("javascript-snowflake", &code);
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

/// Inventory of the backends `validate_with_tools` cannot check with any real
/// tool, because no validator has been written for their language.
///
/// This is the other half of #98. Strict mode catches a checker that fell out
/// of the CI image; nothing catches a language that never had one, because
/// `validate_with_tools` returning `Unsupported` is indistinguishable from a
/// pass at a call site that only looks at the error list. That was `_ => None`
/// before `ToolValidation` existed, and it made 18 of this file's backend
/// tests unconditionally green.
///
/// The list is asserted in both directions on purpose:
///
/// * adding a backend in one of these languages without noticing it ships
///   entirely unverified fails here, and
/// * writing a validator for one of them also fails here, prompting whoever
///   did it to shorten the list.
///
/// Rust is the sharpest entry: `validate_structural` returns `vec![]` for the
/// four `rust-*` backends too, so their generated code is currently checked by
/// nothing whatsoever in this file.
#[test]
fn backends_with_no_tool_validator_are_a_known_and_shrinking_set() {
    const NO_TOOL_VALIDATOR: &[&str] = &[
        // C#: every backend here references a NuGet-only driver (Npgsql,
        // MySqlConnector, Microsoft.Data.Sqlite, Microsoft.Data.SqlClient,
        // Oracle.ManagedDataAccess.Core, Snowflake.Data.Client). `using
        // Npgsql;` with no compiled `Npgsql.dll` on the reference path is a
        // hard `CS0246` -- unlike Elixir's soft "module not available"
        // warning below -- so there is no stub-free path through `dotnet
        // build`/`csc`, and six distinct driver APIs is too much surface to
        // hand-stub credibly.
        "csharp-npgsql",
        "csharp-mysqlconnector",
        "csharp-microsoft-sqlite",
        "csharp-sqlclient",
        "csharp-oracle",
        "csharp-snowflake",
        // Rust: `rust-sqlx` expands `sqlx::query_as!`/`sqlx::query!` at
        // compile time, needing either a live database or an `SQLX_OFFLINE`
        // `.sqlx` query cache. `rust-tokio-postgres`/`rust-tiberius`/
        // `rust-sibyl` reference their driver crates by fully-qualified path
        // with no `use` to stub around, so resolving them needs `--extern`
        // pointing at real compiled `.rlib`s -- a full Cargo dependency graph
        // per backend. Only two of the four have a `syn::parse_file` fallback:
        // the generated suite gates that call on `rust-sqlx` and
        // `rust-tokio-postgres` by name, and `crates/scythe-cli/tests/compile_check.rs`
        // reaches `rust-sqlx` alone because it calls `scythe_codegen::generate`,
        // which is pinned to that backend. `rust-tiberius` and `rust-sibyl` are
        // syntax-checked nowhere -- see #229. Do not restore the claim that all
        // four are covered without a test that actually parses their output.
        "rust-sqlx",
        "rust-tokio-postgres",
        "rust-tiberius",
        "rust-sibyl",
        // `kotlin-exposed` needs the Exposed DSL framework (`transaction { }`,
        // `exec(sql, args) { rs -> }`, a `*ColumnType` per scalar);
        // `kotlin-r2dbc` needs `kotlinx.coroutines.flow.Flow` plus the
        // `awaitFirst`/`asFlow` suspend bridges, whose stubs would have to
        // reproduce the coroutines compiler plugin's view of `suspend`.
        // `kotlin-jdbc` itself has a real `kotlinc` validator -- see
        // `validate_kotlin_tools` -- because it touches nothing but
        // `java.sql`/`java.math`/`java.time`. `java-r2dbc` has a real `javac`
        // one now too: R2DBC SPI and Reactor turned out to be stubbable
        // faithfully (`tests/java_stubs/`) where the coroutines runtime is
        // not.
        "kotlin-r2dbc",
        "kotlin-exposed",
    ];

    for backend in NO_TOOL_VALIDATOR {
        assert_eq!(
            validate_with_tools("", backend),
            ToolValidation::Unsupported,
            "{backend} is listed as having no tool validator, but one now answers for it --              remove it from NO_TOOL_VALIDATOR"
        );
    }

    // The languages that do have one must not silently regress into this set.
    const HAS_TOOL_VALIDATOR: &[&str] = &[
        "python-psycopg3",
        "typescript-pg",
        "javascript-pg",
        "go-pgx",
        "ruby-pg",
        "php-pdo",
        "java-jdbc",
        "java-r2dbc",
        "kotlin-jdbc",
        "elixir-postgrex",
        "elixir-ecto",
        "elixir-myxql",
        "elixir-exqlite",
        "elixir-tds",
        "elixir-jamdb",
    ];

    for backend in HAS_TOOL_VALIDATOR {
        assert_ne!(
            validate_with_tools("", backend),
            ToolValidation::Unsupported,
            "{backend} had a tool validator and no longer does"
        );
    }
}
