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
//! The manifests now map the container to a bare `array` in
//! `[types.containers]` and keep the generic form in
//! `[types.docblock_containers]`, which the PHP backends render into
//! `@var`/`@param` tags only. That makes this two claims, not one, and both
//! have to be pinned:
//!
//! 1. a native position never contains generic syntax -- the parse error
//!    above;
//! 2. a docblock does contain the element type -- without it PHPStan reads
//!    `array` as `array<mixed, mixed>`, which is a level-9 finding on every
//!    read out of the value.
//!
//! A whole-file `!contains("array<")` only tests the first, and it passes
//! just as happily when the element type has been thrown away everywhere --
//! which is exactly the state this file used to pin.

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

/// The generated file with every documentation line dropped, leaving only
/// what PHP's parser has to accept as code.
///
/// The whole point of the split is that generic syntax is now *expected* in
/// one of those two halves, so a whole-file substring check can no longer
/// tell a fixed file from a broken one. This is line-based because the PHP
/// backends only ever emit a comment as a whole line (`/**`, ` * ...`,
/// ` */`, or a one-line `/** @var T */`); the panic below is what keeps that
/// assumption from rotting silently into a filter that drops real code or
/// keeps real comments.
fn native_lines(backend_name: &str, code: &str) -> String {
    let mut kept = Vec::new();
    for line in code.lines() {
        let trimmed = line.trim_start();
        let is_comment_line = trimmed.starts_with("/**") || trimmed.starts_with('*');
        if !is_comment_line && (line.contains("/*") || line.contains("*/")) {
            panic!(
                "{backend_name} emitted a comment that is not its own line, so this test can no \
                 longer separate native positions from docblocks:\n{line}"
            );
        }
        if !is_comment_line {
            kept.push(line);
        }
    }
    kept.join("\n")
}

/// The generic type openings on a line, i.e. every `<` directly preceded by
/// an identifier character (`array<`, `list<`, `Foo<`). Deliberately not a
/// search for `array<` alone: the failure being guarded against is generic
/// syntax reaching a native position, whatever the container is named.
fn generic_openings(line: &str) -> Vec<&str> {
    line.match_indices('<')
        .filter(|(index, _)| {
            line[..*index]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
        .map(|(index, _)| {
            let start = line[..index]
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '\\'))
                .map_or(0, |i| i + 1);
            &line[start..index + 1]
        })
        .collect()
}

/// The string assertion always runs; `php -l` runs wherever PHP is installed.
///
/// Splitting them matters: the substring check is what makes this test mean
/// something on a machine with no PHP, and `php -l` is what makes it mean
/// something the substring check cannot express — that the file as a whole
/// still parses.
fn assert_php_file_is_valid(backend_name: &str, code: &str) {
    for line in native_lines(backend_name, code).lines() {
        let openings = generic_openings(line);
        assert!(
            openings.is_empty(),
            "{backend_name} emitted {openings:?} in a native type position, which PHP cannot \
             parse:\n{line}\n\nfull file:\n{code}"
        );
    }

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

/// The other half of the split: the element type the native position cannot
/// hold has to turn up in the docblock.
///
/// Paired deliberately with `php_pdo_array_columns_parse` above, which pins
/// the native halves of these same three columns. Either assertion alone is
/// satisfied by a regression: dropping the docblocks passes the native test,
/// and moving `array<T>` back into `[types.containers]` passes this one.
#[test]
fn php_array_docblocks_carry_the_element_type() {
    for backend_name in ["php-pdo", "php-amphp"] {
        let code = generate_full_file(backend_name, "postgresql", &SqlDialect::PostgreSQL, PG_SCHEMA, PG_QUERY);

        for (docblock, native) in [
            ("/** @var array<string> */", "public array $tags"),
            ("/** @var array<int> */", "public array $counts"),
            ("/** @var ?array<string> */", "public ?array $maybe_tags"),
        ] {
            assert!(
                code.contains(docblock),
                "{backend_name}: expected `{docblock}` above `{native}`; a bare `array` is \
                 `array<mixed, mixed>` to PHPStan:\n{code}"
            );
            assert!(
                code.contains(native),
                "{backend_name}: expected the native position to stay `{native}`:\n{code}"
            );
        }

        // The docblock is above the property, not instead of it.
        assert!(
            code.contains("/** @var array<string> */\n        public array $tags"),
            "{backend_name}: the `@var` docblock must immediately precede the property it \
             describes:\n{code}"
        );

        assert_php_file_is_valid(backend_name, &code);
    }
}

/// The `@param` half, on the route that reaches every dialect.
///
/// `= ANY(...)` synthesises an `array<int32>` parameter, so the element type
/// is available in exactly the position PHP's signature cannot express it.
#[test]
fn php_any_param_docblocks_carry_the_element_type() {
    for backend_name in ["php-pdo", "php-amphp"] {
        let (schema, query) = any_param_fixture(&SqlDialect::PostgreSQL);
        let code = generate_full_file(backend_name, "postgresql", &SqlDialect::PostgreSQL, schema, query);

        assert!(
            code.contains("@param array<int> $ids") || code.contains("@param array<int> $id"),
            "{backend_name}: expected `@param array<int>` for the `= ANY(...)` parameter:\n{code}"
        );
        assert_php_file_is_valid(backend_name, &code);
    }
}

/// A type that needs no docblock must not get one. Otherwise every generated
/// file grows a `/** @var string */` above every string property, which says
/// nothing PHP's own type did not already say.
#[test]
fn php_scalar_columns_get_no_docblock() {
    let code = generate_full_file("php-pdo", "postgresql", &SqlDialect::PostgreSQL, PG_SCHEMA, PG_QUERY);
    assert!(
        !code.contains("/** @var int */"),
        "expected no docblock for a column whose native type is already exact:\n{code}"
    );
}

/// `= ANY(...)` params, one per engine each PHP manifest is reachable through.
///
/// `php-pdo.oracle.toml` is deliberately absent: `PhpPdoBackend::new` has no
/// `oracle` arm and `supported_engines()` omits oracle, so that manifest is
/// not compiled into the binary at all. The manifest-wide test below is what
/// covers it.
///
/// Shared with [`php_any_param_docblocks_carry_the_element_type_on_every_reachable_engine`]
/// so both tests see the same reachable set by construction -- a case added
/// to only one of them would leave the other one silently short.
fn any_param_reachable_cases() -> &'static [(&'static str, &'static str, SqlDialect)] {
    &[
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
    ]
}

#[test]
fn php_any_params_parse_on_every_reachable_engine() {
    for (backend_name, engine, dialect) in any_param_reachable_cases() {
        let (schema, query) = any_param_fixture(dialect);
        let code = generate_full_file(backend_name, engine, dialect, schema, query);
        assert!(
            code.contains("array $ids") || code.contains("array $id"),
            "{backend_name}/{engine}: expected an `array` parameter from `= ANY(...)`:\n{code}"
        );
        assert_php_file_is_valid(backend_name, &code);
    }
}

/// The other half of [`php_any_params_parse_on_every_reachable_engine`],
/// across the same reachable set: that test only pins the native `array`
/// parameter every one of these ten pairs degrades to. Before this test,
/// the element type surviving in `@param array<int>` was pinned only for
/// `postgresql`, by [`php_any_param_docblocks_carry_the_element_type`] --
/// the other nine (backend, engine) pairs could lose the element type
/// entirely and every existing test would still pass, because the native
/// side is the only thing any of them checked on those nine. That is
/// exactly the shape #164 exists to catch: a whole set that only ever
/// degrades, verified to agree with itself and nothing else.
#[test]
fn php_any_param_docblocks_carry_the_element_type_on_every_reachable_engine() {
    for (backend_name, engine, dialect) in any_param_reachable_cases() {
        let (schema, query) = any_param_fixture(dialect);
        let code = generate_full_file(backend_name, engine, dialect, schema, query);
        assert!(
            code.contains("@param array<int> $ids") || code.contains("@param array<int> $id"),
            "{backend_name}/{engine}: expected `@param array<int>` to survive for the \
             `= ANY(...)` parameter, not just the degraded native `array`:\n{code}"
        );
        assert_php_file_is_valid(backend_name, &code);
    }
}

/// The body of one TOML table, up to the next table header.
///
/// Anchored on a whole header line rather than a bare substring: with
/// `[types.containers]` and `[types.docblock_containers]` both present, and
/// each one named in the other's comments, a substring match picks up prose.
fn manifest_section<'a>(text: &'a str, header: &str, name: &str) -> &'a str {
    let (_, after) = text
        .split_once(&format!("\n{header}\n"))
        .unwrap_or_else(|| panic!("{name} has no {header} section"));
    after.split("\n[").next().unwrap_or_default()
}

/// Every `php-*.toml` on disk, read as (file name, contents).
///
/// The count assertion lives here rather than in each caller because without
/// it the whole family passes vacuously the day the manifests move or are
/// renamed -- a zero-match glob is not evidence of anything.
fn php_manifests() -> Vec<(String, String)> {
    let manifests_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("manifests");
    let mut found = Vec::new();

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
        let text = std::fs::read_to_string(&path).expect("manifest must be readable");
        found.push((name, text));
    }

    assert!(
        found.len() >= 9,
        "expected at least 9 php-*.toml manifests under {}, found {} -- the glob has gone stale \
         and these tests are no longer checking anything",
        manifests_dir.display(),
        found.len()
    );
    found
}

/// The generated-code tests above can only reach manifests that some backend
/// constructor selects. This one covers all nine files on disk, including
/// `php-pdo.oracle.toml`, which nothing compiles in.
#[test]
fn no_php_manifest_declares_a_generic_container_type() {
    for (name, text) in php_manifests() {
        let containers = manifest_section(&text, "[types.containers]", &name);
        assert!(
            !containers.contains('<'),
            "{name} declares a generic container type; PHP has no generics in a native type \
             position, so this reaches the generated file as a parse error:\n{containers}"
        );
    }
}

/// The counterpart, over the same nine files: a manifest that dropped the
/// generic form altogether would satisfy the test above and silently cost
/// every array column its element type.
#[test]
fn every_php_manifest_keeps_the_element_type_in_a_docblock_container() {
    for (name, text) in php_manifests() {
        let docblock = manifest_section(&text, "[types.docblock_containers]", &name);
        assert!(
            docblock.contains("array = \"array<{T}>\""),
            "{name} has no generic `array` mapping in [types.docblock_containers], so PHPStan \
             sees `array<mixed, mixed>` for every array column:\n{docblock}"
        );
    }
}
