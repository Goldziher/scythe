//! Regression tests for the PHP backends' array type, exercised through the
//! real parse -> analyze -> codegen pipeline and then handed to `php -l`.
//!
//! Every `php-*.toml` manifest used to map the `array` container to
//! `array<{T}>`. PHP has no generic syntax in the *native* type position, so
//! anything that resolved to an array reached the generated file as a parse
//! error:
//!
//! ```text
//! public array<string> $tags,
//! PHP Parse error: syntax error, unexpected token "<", expecting variable
//! ```
//!
//! Two independent routes reach that container, which is why the fix had to
//! cover every PHP manifest rather than just the PostgreSQL ones:
//!
//! 1. An array *column* (`TEXT[]`, `JSONB[]`), PostgreSQL-only.
//! 2. An `= ANY(?)` *parameter*, which the analyzer synthesises as
//!    `array<T>` in every dialect — including the ones whose SQL has no array
//!    type at all. There the broken type landed in the function signature.
//!
//! The manifests now map the container to a bare `array`. The element type
//! survives in `neutral_type` for any backend that wants to render a
//! PHPStan-style `array<T>` docblock separately from the native type; today
//! both positions share `full_type`, so the docblock is the less precise
//! `array`. Making the docblock generic again is a backend change, not a
//! manifest one.

use std::path::PathBuf;

use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_structural, validate_with_tools};
use scythe_codegen::{generate_with_backend, get_backend, provenance};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// PostgreSQL is the only dialect here with real array *columns*.
const PG_SCHEMA: &str = "CREATE TABLE orders (\
    id SERIAL PRIMARY KEY, \
    tags TEXT[] NOT NULL, \
    counts INTEGER[] NOT NULL, \
    payload JSONB[] NOT NULL, \
    maybe_tags TEXT[]\
);";

const PG_QUERY: &str = "-- @name GetOrder\n-- @returns :one\n\
    SELECT id, tags, counts, payload, maybe_tags FROM orders WHERE id = $1;";

/// The second route: `= ANY(...)` makes the analyzer synthesise an
/// `array<int>` parameter regardless of dialect, so the broken type reached
/// the *signature* even on engines with no array type of their own.
fn any_param_fixture(dialect: &SqlDialect) -> (&'static str, &'static str) {
    match dialect {
        SqlDialect::MySQL => (
            "CREATE TABLE items (id INT PRIMARY KEY, name VARCHAR(255) NOT NULL);",
            "-- @name FindItems\n-- @returns :many\nSELECT id, name FROM items WHERE id = ANY(?);",
        ),
        SqlDialect::SQLite => (
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
            "-- @name FindItems\n-- @returns :many\nSELECT id, name FROM items WHERE id = ANY(?);",
        ),
        SqlDialect::MsSql => (
            "CREATE TABLE items (id INT PRIMARY KEY, name NVARCHAR(255) NOT NULL);",
            "-- @name FindItems\n-- @returns :many\nSELECT id, name FROM items WHERE id = ANY(@p1);",
        ),
        _ => (
            "CREATE TABLE items (id SERIAL PRIMARY KEY, name TEXT NOT NULL);",
            "-- @name FindItems\n-- @returns :many\nSELECT id, name FROM items WHERE id = ANY($1);",
        ),
    }
}

/// Assemble the exact bytes `scythe generate` would write, so `php -l` sees a
/// complete file (provenance header included) rather than a fragment.
fn generate_full_file(backend_name: &str, engine: &str, dialect: &SqlDialect, schema: &str, query: &str) -> String {
    let backend = get_backend(backend_name, engine).expect("backend must support engine");
    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, dialect).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");

    let all_codes = vec![code];
    let mut full = backend.file_header_for_results(&all_codes);
    full.push('\n');
    for code in &all_codes {
        for s in [&code.enum_def, &code.model_struct, &code.row_struct]
            .into_iter()
            .flatten()
        {
            full.push_str(s);
            full.push('\n');
        }
    }
    let class_header = backend.query_class_header();
    if !class_header.is_empty() {
        full.push_str(&class_header);
        full.push('\n');
    }
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

    provenance::assemble_file(
        &backend.file_preamble(),
        &provenance::header_line(
            &*backend,
            env!("CARGO_PKG_VERSION"),
            engine,
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &full,
    )
}

/// The string assertion always runs; `php -l` runs wherever PHP is installed.
///
/// Splitting them matters: the substring check is what makes this test mean
/// something on a machine with no PHP, and `php -l` is what makes it mean
/// something the substring check cannot express — that the file as a whole
/// still parses.
fn assert_php_file_is_valid(backend_name: &str, code: &str) {
    assert!(
        !code.contains("array<"),
        "{backend_name} emitted a generic `array<...>`, which PHP cannot parse in a native type \
         position:\n{code}"
    );

    let structural_errors = validate_structural(code, backend_name);
    assert!(
        structural_errors.is_empty(),
        "{backend_name} structural: {structural_errors:?}\n\n{code}"
    );

    let validation = validate_with_tools(code, backend_name);
    if strict_mode_enabled() {
        assert!(
            !matches!(validation, ToolValidation::Unsupported) && validation.fully_checked(),
            "{backend_name} has no working PHP validator, so this test would pass without ever \
             parsing the file"
        );
    }
    if let Err(tool_errors) = validation.into_result() {
        panic!("{backend_name} tool validation: {tool_errors:?}\n\n{code}");
    }
}

#[test]
fn php_pdo_array_columns_parse() {
    let code = generate_full_file("php-pdo", "postgresql", &SqlDialect::PostgreSQL, PG_SCHEMA, PG_QUERY);
    assert!(
        code.contains("public array $tags"),
        "expected a bare native `array` for the TEXT[] column:\n{code}"
    );
    assert!(
        code.contains("public ?array $maybe_tags"),
        "expected `?array` for the nullable TEXT[] column:\n{code}"
    );
    assert_php_file_is_valid("php-pdo", &code);
}

#[test]
fn php_amphp_array_columns_parse() {
    let code = generate_full_file("php-amphp", "postgresql", &SqlDialect::PostgreSQL, PG_SCHEMA, PG_QUERY);
    assert_php_file_is_valid("php-amphp", &code);
}

/// `= ANY(...)` params, one per engine each PHP manifest is reachable through.
///
/// `php-pdo.oracle.toml` is deliberately absent: `PhpPdoBackend::new` has no
/// `oracle` arm and `supported_engines()` omits oracle, so that manifest is
/// not compiled into the binary at all. The manifest-wide test below is what
/// covers it.
#[test]
fn php_any_params_parse_on_every_reachable_engine() {
    let cases: &[(&str, &str, SqlDialect)] = &[
        ("php-pdo", "postgresql", SqlDialect::PostgreSQL),
        ("php-pdo", "mysql", SqlDialect::MySQL),
        ("php-pdo", "mariadb", SqlDialect::MySQL),
        ("php-pdo", "sqlite", SqlDialect::SQLite),
        ("php-pdo", "mssql", SqlDialect::MsSql),
        ("php-pdo", "redshift", SqlDialect::PostgreSQL),
        ("php-pdo", "snowflake", SqlDialect::PostgreSQL),
        ("php-amphp", "postgresql", SqlDialect::PostgreSQL),
        ("php-amphp", "mysql", SqlDialect::MySQL),
        ("php-amphp", "mariadb", SqlDialect::MySQL),
    ];

    for (backend_name, engine, dialect) in cases {
        let (schema, query) = any_param_fixture(dialect);
        let code = generate_full_file(backend_name, engine, dialect, schema, query);
        assert!(
            code.contains("array $ids") || code.contains("array $id"),
            "{backend_name}/{engine}: expected an `array` parameter from `= ANY(...)`:\n{code}"
        );
        assert_php_file_is_valid(backend_name, &code);
    }
}

/// The two tests above can only reach manifests that some backend constructor
/// selects. This one covers all nine files on disk, including
/// `php-pdo.oracle.toml`, which nothing compiles in.
#[test]
fn no_php_manifest_declares_a_generic_container_type() {
    let manifests_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("manifests");
    let mut checked = 0usize;

    for entry in std::fs::read_dir(&manifests_dir).expect("manifests dir must exist") {
        let path = entry.expect("readable dir entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if !name.starts_with("php-") || !name.ends_with(".toml") {
            continue;
        }
        checked += 1;

        let text = std::fs::read_to_string(&path).expect("manifest must be readable");
        let containers = text
            .split("[types.containers]")
            .nth(1)
            .unwrap_or_else(|| panic!("{name} has no [types.containers] section"))
            .split("\n[")
            .next()
            .unwrap_or_default();

        assert!(
            !containers.contains('<'),
            "{name} declares a generic container type; PHP has no generics in a native type \
             position, so this reaches the generated file as a parse error:\n{containers}"
        );
    }

    // Without this the test passes vacuously the day the manifests move or are
    // renamed -- a zero-match glob is not evidence of anything.
    assert!(
        checked >= 9,
        "expected at least 9 php-*.toml manifests under {}, found {checked} -- the glob has gone \
         stale and this test is no longer checking anything",
        manifests_dir.display()
    );
}
