//! End-to-end regression tests for the shared SQL-literal escaping layer
//! (`scythe_codegen::sql_literal`), exercised through the real
//! parse -> analyze -> codegen pipeline rather than the escape functions in
//! isolation (those are unit-tested directly in `sql_literal.rs`).
//!
//! This closes issues #176 (the Kotlin injection), #179, and #150: every
//! backend here previously spliced raw SQL into a host string literal with
//! no escaping at all. Each test below feeds the pipeline SQL containing the
//! exact trigger shape the issue reported and asserts the *escaped* form
//! reaches the generated file, never the raw one.
//!
//! Where a real compiler is on `PATH`, the generated (or a minimal
//! standalone) file is actually compiled or syntax-checked, mirroring
//! `tool_validation.rs`'s `generate_full_file_from_backend` assembly so the
//! bytes handed to the compiler are the same ones `scythe generate` would
//! write. A missing tool skips that one assertion rather than failing the
//! test -- consistent with `validation::ToolOutcome::Missing` elsewhere in
//! this crate -- since this suite must still pass on a machine that only has
//! some of the eight language toolchains installed.

use std::process::Command;

use scythe_codegen::{GeneratedCode, generate_with_backend, get_backend, provenance};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: &str = "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL);";

/// Assemble one query's generated output into a complete file, mirroring
/// `tool_validation.rs`'s `generate_full_file_from_backend` -- real
/// compilers must see the same bytes `scythe generate` would write, not a
/// hand-trimmed fragment.
fn generate_full_file(backend_name: &str, query_sql: &str) -> String {
    generate_full_file_for_engine(backend_name, "postgresql", &SqlDialect::PostgreSQL, SCHEMA, query_sql)
}

fn generate_full_file_for_engine(
    backend_name: &str,
    engine: &str,
    dialect: &SqlDialect,
    schema: &str,
    query_sql: &str,
) -> String {
    let backend = get_backend(backend_name, engine)
        .unwrap_or_else(|e| panic!("backend {backend_name} must support engine {engine}: {e}"));
    let catalog = Catalog::from_ddl_with_dialect(&[schema], dialect).expect("schema must parse");
    let parsed = parse_query_with_dialect(query_sql, dialect).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    let codes = [code];

    let class_header = backend.query_class_header();
    let mut full = backend.file_header_for_results(&codes);
    full.push('\n');

    if class_header.is_empty() {
        push_bodies(&mut full, &codes);
    } else {
        for code in &codes {
            push_defs(&mut full, code);
        }
        full.push_str(&class_header);
        full.push('\n');
        for code in &codes {
            if let Some(ref s) = code.query_fn {
                full.push_str(s);
                full.push('\n');
            }
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

fn push_defs(out: &mut String, code: &GeneratedCode) {
    if let Some(ref s) = code.enum_def {
        out.push_str(s);
        out.push('\n');
    }
    if let Some(ref s) = code.model_struct {
        out.push_str(s);
        out.push('\n');
    }
    if let Some(ref s) = code.row_struct {
        out.push_str(s);
        out.push('\n');
    }
}

fn push_bodies(out: &mut String, codes: &[GeneratedCode]) {
    for code in codes {
        push_defs(out, code);
        if let Some(ref s) = code.query_fn {
            out.push_str(s);
            out.push('\n');
        }
    }
}

/// Write `code` to a fresh temp file with the given extension and return its
/// path. Each call gets a unique name so parallel `cargo test` threads never
/// collide.
fn write_temp(code: &str, ext: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("scythe_sql_literal_regression_{n}{ext}"));
    std::fs::write(&path, code).expect("failed to write temp file");
    path
}

/// Run `tool` with `args`; panics with the tool's output if it exits
/// non-zero. Returns `false` (skip) rather than panicking when `tool` is not
/// on `PATH`, so this suite still passes on a machine missing that one
/// toolchain -- see the module doc comment.
fn compiles_or_skipped(tool: &str, probe_arg: &str, args: &[&str]) {
    if Command::new(tool).arg(probe_arg).output().is_err() {
        eprintln!("skipping {tool} check: not on PATH");
        return;
    }
    let output = Command::new(tool)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{tool}: could not be executed: {e}"));
    if !output.status.success() {
        panic!(
            "{tool} rejected escaped output:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

// ---------------------------------------------------------------------
// #176: Kotlin non-raw string interpolation
// ---------------------------------------------------------------------

const KOTLIN_176_QUERY: &str = "-- @name FindUser\n-- @returns :one\n\
    SELECT id, name FROM users WHERE name = $1 AND name != 'literal-$name-marker';";

/// Direct regression test for #176 through the real backend, not just the
/// escape function: a query whose SQL contains `$name` -- exactly matching
/// an in-scope parameter's field name -- must not produce Kotlin source
/// that interpolates it.
#[test]
fn kotlin_jdbc_176_injection_is_neutralized() {
    let file = generate_full_file("kotlin-jdbc", KOTLIN_176_QUERY);
    assert!(
        !file.contains("literal-$name-marker"),
        "unescaped $name must never reach Kotlin source (this is the #176 injection):\n{file}"
    );
    assert!(
        file.contains("literal-\\$name-marker"),
        "expected \\$name (Kotlin's escape for a literal dollar sign):\n{file}"
    );

    let path = write_temp(&file, ".kt");
    compiles_or_skipped(
        "kotlinc",
        "-version",
        &[
            path.to_str().unwrap(),
            "-d",
            std::env::temp_dir().join("scythe_kotlinc_out").to_str().unwrap(),
        ],
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn kotlin_r2dbc_176_injection_is_neutralized() {
    let file = generate_full_file("kotlin-r2dbc", KOTLIN_176_QUERY);
    assert!(!file.contains("literal-$name-marker"), "got:\n{file}");
    assert!(file.contains("literal-\\$name-marker"), "got:\n{file}");
}

#[test]
fn kotlin_exposed_176_injection_is_neutralized() {
    let file = generate_full_file("kotlin-exposed", KOTLIN_176_QUERY);
    assert!(!file.contains("literal-$name-marker"), "got:\n{file}");
    assert!(file.contains("literal-\\$name-marker"), "got:\n{file}");
}

// ---------------------------------------------------------------------
// Ruby / Elixir #{...} interpolation
// ---------------------------------------------------------------------

const HASH_BRACE_QUERY: &str = "-- @name FindUser\n-- @returns :one\n\
    SELECT id, name FROM users WHERE name = $1 AND name != 'literal-#{evil}-marker';";

#[test]
fn ruby_pg_hash_brace_interpolation_is_neutralized() {
    let file = generate_full_file("ruby-pg", HASH_BRACE_QUERY);
    assert!(
        !file.contains("literal-#{evil}-marker"),
        "unescaped #{{}} must never reach Ruby source:\n{file}"
    );
    assert!(file.contains("literal-\\#{evil}-marker"), "got:\n{file}");

    let path = write_temp(&file, ".rb");
    compiles_or_skipped("ruby", "--version", &["-c", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn elixir_postgrex_hash_brace_interpolation_is_neutralized() {
    let file = generate_full_file("elixir-postgrex", HASH_BRACE_QUERY);
    assert!(
        !file.contains("literal-#{evil}-marker"),
        "unescaped #{{}} must never reach Elixir source:\n{file}"
    );
    assert!(file.contains("literal-\\#{evil}-marker"), "got:\n{file}");

    let path = write_temp(&file, ".exs");
    compiles_or_skipped(
        "elixirc",
        "--version",
        &["-o", std::env::temp_dir().to_str().unwrap(), path.to_str().unwrap()],
    );
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------
// PHP $-interpolation (double-quoted strings interpolate; switched to
// single-quoted, which never does)
// ---------------------------------------------------------------------

const PHP_DOLLAR_QUERY: &str = "-- @name FindUser\n-- @returns :one\n\
    SELECT id, name FROM users WHERE name = $1 AND name != 'literal-$pdo-marker';";

#[test]
fn php_pdo_dollar_interpolation_is_neutralized() {
    let file = generate_full_file("php-pdo", PHP_DOLLAR_QUERY);
    // A PHP *double*-quoted literal here would interpolate $pdo -- the
    // connection handle parameter -- into the SQL text. The fix routes SQL
    // through a single-quoted literal instead, which performs no
    // interpolation, so the raw text survives verbatim and inertly.
    assert!(
        file.contains("->prepare('") && file.contains("literal-$pdo-marker"),
        "SQL must be emitted as a single-quoted (non-interpolating) PHP literal:\n{file}"
    );
    assert!(
        !file.contains("->prepare(\""),
        "SQL must not be spliced into a double-quoted (interpolating) PHP literal:\n{file}"
    );

    let path = write_temp(&file, ".php");
    compiles_or_skipped("php", "--version", &["-l", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------
// Quote / backslash: breaks non-interpolating host literals outright
// (issue #150's "loud case" and #179's silent-corruption case)
// ---------------------------------------------------------------------

const QUOTE_BACKSLASH_QUERY: &str = "-- @name FindUser\n-- @returns :one\n\
    SELECT id, name FROM users WHERE name = $1 AND name != 'has \"quotes\" and a\\backslash and a\\_b%';";

#[test]
fn go_pgx_quote_and_backslash_are_escaped() {
    let file = generate_full_file("go-pgx", QUOTE_BACKSLASH_QUERY);
    assert!(file.contains("\\\"quotes\\\""), "got:\n{file}");
    assert!(file.contains("a\\\\backslash"), "got:\n{file}");

    let path = write_temp(&file, ".go");
    compiles_or_skipped("gofmt", "-h", &["-e", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn python_psycopg3_quote_and_backslash_are_escaped() {
    let file = generate_full_file("python-psycopg3", QUOTE_BACKSLASH_QUERY);
    assert!(file.contains("\\\"quotes\\\""), "got:\n{file}");
    assert!(file.contains("a\\\\backslash"), "got:\n{file}");

    let path = write_temp(&file, ".py");
    compiles_or_skipped("python3", "--version", &["-m", "py_compile", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn csharp_npgsql_quote_is_doubled_and_backslash_stays_literal() {
    let file = generate_full_file("csharp-npgsql", QUOTE_BACKSLASH_QUERY);
    // Verbatim (@"...") strings double an embedded quote and treat
    // backslash as an ordinary character -- both correct for SQL text.
    assert!(file.contains("\"\"quotes\"\""), "got:\n{file}");
    assert!(file.contains("a\\backslash"), "got:\n{file}");
    assert!(
        file.contains("NpgsqlCommand(@\""),
        "SQL must use a verbatim string literal:\n{file}"
    );
}

// ---------------------------------------------------------------------
// Rust raw-string delimiter widening (#179's `"#` raw-string-breaking case)
// ---------------------------------------------------------------------

const RUST_HASH_QUOTE_QUERY: &str = "-- @name FindUser\n-- @returns :one\n\
    SELECT id, name FROM users WHERE name = $1 AND name != 'ends with a hash-quote \"# sequence';";

#[test]
fn tokio_postgres_raw_string_widens_delimiter_for_hash_quote() {
    let file = generate_full_file("rust-tokio-postgres", RUST_HASH_QUOTE_QUERY);
    assert!(
        file.contains("r##\"") && file.contains("\"##"),
        "a `\"#` in the SQL must widen the raw-string delimiter:\n{file}"
    );
    assert!(
        file.contains("ends with a hash-quote \"# sequence"),
        "the payload itself must survive verbatim inside the widened literal:\n{file}"
    );
}

#[test]
fn rust_tiberius_raw_string_widens_delimiter_for_hash_quote() {
    const MSSQL_SCHEMA: &str = "CREATE TABLE users (id INT IDENTITY(1,1) PRIMARY KEY, name NVARCHAR(255) NOT NULL);";
    const MSSQL_QUERY: &str = "-- @name FindUser\n-- @returns :one\n\
        SELECT id, name FROM users WHERE name = @p1 AND name != 'ends with a hash-quote \"# sequence';";
    let file = generate_full_file_for_engine("rust-tiberius", "mssql", &SqlDialect::MsSql, MSSQL_SCHEMA, MSSQL_QUERY);
    assert!(file.contains("r##\"") && file.contains("\"##"), "got:\n{file}");
}

/// Minimal standalone Rust and Java snippets, compiled directly, proving the
/// literal-building helpers produce syntactically valid output without
/// depending on external crates/packages the full generated files reference
/// (`tokio-postgres`, JDBC driver JARs) that are not resolvable by a bare
/// `rustc`/`javac` invocation outside a full build.
#[test]
fn rust_raw_string_literal_compiles_standalone() {
    let sql = "ends with a hash-quote \"# sequence and a 'quote' and a \\backslash";
    let literal = scythe_codegen::sql_literal::rust_raw_string_literal(sql);
    let source = format!("fn main() {{ let sql = {literal}; assert_eq!(sql, {literal:?}); }}");
    let path = write_temp(&source, ".rs");
    let out = std::env::temp_dir().join("scythe_rustc_check_out");
    compiles_or_skipped(
        "rustc",
        "--version",
        &["--edition", "2024", "-o", out.to_str().unwrap(), path.to_str().unwrap()],
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn java_string_literal_compiles_standalone() {
    let sql = "has \"quotes\" and a\\backslash and a\\_b% and\ta tab";
    let literal = scythe_codegen::sql_literal::escape_java_string(sql);
    let source = format!(
        "public class ScytheEscapeCheck {{ public static void main(String[] a) {{ String sql = \"{literal}\"; System.out.println(sql); }} }}"
    );
    let dir = std::env::temp_dir().join("scythe_javac_check");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("ScytheEscapeCheck.java");
    std::fs::write(&path, &source).expect("failed to write temp file");
    compiles_or_skipped(
        "javac",
        "-version",
        &["-d", dir.to_str().unwrap(), path.to_str().unwrap()],
    );
    let _ = std::fs::remove_file(&path);
}
