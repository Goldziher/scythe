//! Regression tests for #198: a manifest declares a type, and a hardcoded
//! table in the backend disagrees with it, in two unrelated languages.
//!
//! 1. **PHP casts contradicted the PHP manifests.** `php_cast` matched on
//!    `neutral_type` and hardcoded `json` and `decimal` into the
//!    `"(string) "` arm, which held only by coincidence: `php-pdo.toml`
//!    declares `json = "array"` while `php-pdo.mysql.toml` declares
//!    `json = "string"`, and `php-pdo.sqlite.toml` declares
//!    `decimal = "float"` while every other PHP manifest declares it
//!    `"string"`. A `json` column was therefore declared `array` in its
//!    promoted property and cast to `(string)` in `fromRow`, and a SQLite
//!    `decimal` was declared `float` and cast to `(string)`. The fix reads
//!    `ResolvedColumn::lang_type` -- the manifest's own declaration for the
//!    same column -- instead of a second table, exactly as
//!    `csharp_reader_type_regression.rs` pins for the C# reader accessors.
//!
//! 2. **Go's import block ignored the manifest entirely.**
//!    `manifests/go-pgx.toml` declares five `[imports.rules]` entries
//!    (`time.`, `decimal.`, `uuid.`, `json.`, `netip.`), but nothing in
//!    `scythe-codegen`'s `src/` ever read `.imports`. `go_common::go_file_header`
//!    hardcoded `"time"` and `github.com/shopspring/decimal` as the only two
//!    conditional imports, so a `uuid`, `json`, or `inet` column emitted
//!    `uuid.UUID` / `json.RawMessage` / `netip.Addr` with no import for any
//!    of them -- a file `go build` rejects with "undefined: uuid" (and, per
//!    the generated header's own advice, `goimports -w .` cannot fix an
//!    import goimports has no way to know the file needs `github.com/...`
//!    for). The fix drives the import block from `manifest.imports.rules`.

use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_structural, validate_with_tools};
use scythe_codegen::{generate_with_backend, get_backend, provenance};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// Assemble the exact bytes `scythe generate` would write, provenance header
/// included, so the assertions below see a whole file rather than a
/// fragment. Mirrors `csharp_reader_type_regression.rs`'s helper of the same
/// name -- every regression test file in this crate builds its own copy
/// rather than sharing one, since integration test binaries do not share
/// code across files.
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
        for section in [&code.enum_def, &code.model_struct, &code.row_struct]
            .into_iter()
            .flatten()
        {
            full.push_str(section);
            full.push('\n');
        }
    }
    let class_header = backend.query_class_header();
    if !class_header.is_empty() {
        full.push_str(&class_header);
        full.push('\n');
    }
    for code in &all_codes {
        if let Some(ref query_fn) = code.query_fn {
            full.push_str(query_fn);
            full.push('\n');
        }
    }
    let footer = backend.file_footer();
    if !footer.is_empty() {
        full.push_str(&footer);
        full.push('\n');
    }
    let post = backend.post_footer();
    if !post.is_empty() {
        full.push('\n');
        full.push_str(&post);
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

/// Structural validation always runs; the real syntax checker (`mago` for
/// PHP via `poly`, `gofmt` for Go) runs wherever it is installed, and
/// `strict_mode_enabled()` (set in CI) turns "the checker never ran" itself
/// into a failure so this cannot pass vacuously on a machine with neither
/// tool.
fn assert_generated_file_is_valid(backend_name: &str, code: &str) {
    let structural_errors = validate_structural(code, backend_name);
    assert!(
        structural_errors.is_empty(),
        "{backend_name} structural: {structural_errors:?}\n\n{code}"
    );

    let validation = validate_with_tools(code, backend_name);
    if strict_mode_enabled() {
        assert!(
            matches!(validation, ToolValidation::Unsupported) || validation.fully_checked(),
            "{backend_name} has a validator that reported no checker actually ran"
        );
    }
    if let Err(tool_errors) = validation.into_result() {
        panic!("{backend_name} tool validation: {tool_errors:?}\n\n{code}");
    }
}

const PG_JSON_SCHEMA: &str =
    "CREATE TABLE widgets (id SERIAL PRIMARY KEY, payload JSONB NOT NULL, maybe_payload JSONB);";
const PG_JSON_QUERY: &str =
    "-- @name GetWidget\n-- @returns :one\nSELECT id, payload, maybe_payload FROM widgets WHERE id = $1;";

const MYSQL_JSON_SCHEMA: &str = "CREATE TABLE widgets (id INT PRIMARY KEY, payload JSON NOT NULL);";
const MYSQL_JSON_QUERY: &str = "-- @name GetWidget\n-- @returns :one\nSELECT id, payload FROM widgets WHERE id = ?;";

const SQLITE_DECIMAL_SCHEMA: &str = "CREATE TABLE orders (id INTEGER PRIMARY KEY, total DECIMAL(10, 2) NOT NULL);";
const SQLITE_DECIMAL_QUERY: &str = "-- @name GetOrder\n-- @returns :one\nSELECT id, total FROM orders WHERE id = ?;";

const PG_DECIMAL_SCHEMA: &str = "CREATE TABLE orders (id SERIAL PRIMARY KEY, total DECIMAL(10, 2) NOT NULL);";
const PG_DECIMAL_QUERY: &str = "-- @name GetOrder\n-- @returns :one\nSELECT id, total FROM orders WHERE id = $1;";

/// `php-pdo.toml` and `php-amphp.toml` (both PostgreSQL) declare
/// `json = "array"`, so the property is `public array $payload` -- a value
/// no PHP scalar cast can produce from PDO's raw JSON text. The fix decodes
/// it instead of casting it to the string type the old table assumed.
#[test]
fn php_json_column_is_decoded_when_the_manifest_declares_it_an_array() {
    for backend_name in ["php-pdo", "php-amphp"] {
        let code = generate_full_file(
            backend_name,
            "postgresql",
            &SqlDialect::PostgreSQL,
            PG_JSON_SCHEMA,
            PG_JSON_QUERY,
        );

        assert!(
            code.contains("public array $payload") && code.contains("public ?array $maybe_payload"),
            "{backend_name}: expected the manifest's declared `array` property to survive:\n{code}"
        );
        assert!(
            code.contains("json_decode($row['payload'], true)"),
            "{backend_name}: a column the manifest declares `array` must be decoded, not cast to \
             `(string)` -- that is exactly the disagreement #198 exists to fix:\n{code}"
        );
        assert!(
            !code.contains("(string) $row['payload']"),
            "{backend_name}: must not cast a manifest-declared `array` column to `(string)`:\n{code}"
        );
        assert!(
            code.contains("$row['maybe_payload'] !== null ? json_decode($row['maybe_payload'], true) : null"),
            "{backend_name}: the nullable JSON column must keep its null guard around the decode:\n{code}"
        );

        assert_generated_file_is_valid(backend_name, &code);
    }
}

/// The other half of the same defect: `php-pdo.mysql.toml` and
/// `php-amphp.mysql.toml` declare `json = "string"`, so on those manifests a
/// `json_decode` would disagree with the declared type just as badly as the
/// old unconditional `(string)` cast disagreed with `php-pdo.toml`. The cast
/// must track whichever type *this* manifest declared.
#[test]
fn php_json_column_stays_a_string_when_the_manifest_declares_it_a_string() {
    for backend_name in ["php-pdo", "php-amphp"] {
        let code = generate_full_file(
            backend_name,
            "mysql",
            &SqlDialect::MySQL,
            MYSQL_JSON_SCHEMA,
            MYSQL_JSON_QUERY,
        );

        assert!(
            code.contains("public string $payload"),
            "{backend_name}/mysql: expected the manifest's declared `string` property:\n{code}"
        );
        assert!(
            code.contains("(string) $row['payload']"),
            "{backend_name}/mysql: a column the manifest declares `string` must be cast to \
             `(string)`:\n{code}"
        );
        assert!(
            !code.contains("json_decode"),
            "{backend_name}/mysql: must not decode a column the manifest declares `string`:\n{code}"
        );

        assert_generated_file_is_valid(backend_name, &code);
    }
}

/// `php-pdo.sqlite.toml` is the one PHP manifest that declares
/// `decimal = "float"`; every other PHP manifest declares it `"string"`. The
/// old table cast every `decimal` column to `(string)` regardless, which
/// disagreed with the SQLite property's declared `float` type.
#[test]
fn php_pdo_sqlite_decimal_column_is_cast_to_float() {
    let code = generate_full_file(
        "php-pdo",
        "sqlite",
        &SqlDialect::SQLite,
        SQLITE_DECIMAL_SCHEMA,
        SQLITE_DECIMAL_QUERY,
    );

    assert!(
        code.contains("public float $total"),
        "expected the manifest's declared `float` property for a SQLite decimal column:\n{code}"
    );
    assert!(
        code.contains("(float) $row['total']"),
        "a column the manifest declares `float` must be cast to `(float)`, not `(string)`:\n{code}"
    );
    assert!(
        !code.contains("(string) $row['total']"),
        "must not cast a manifest-declared `float` column to `(string)`:\n{code}"
    );

    assert_generated_file_is_valid("php-pdo", &code);
}

/// Every other PHP manifest still declares `decimal = "string"`, so the
/// `(string)` cast is correct there -- this pins that the fix did not throw
/// the working case away while chasing the SQLite one.
#[test]
fn php_pdo_postgresql_decimal_column_stays_a_string() {
    let code = generate_full_file(
        "php-pdo",
        "postgresql",
        &SqlDialect::PostgreSQL,
        PG_DECIMAL_SCHEMA,
        PG_DECIMAL_QUERY,
    );

    assert!(
        code.contains("public string $total"),
        "expected the manifest's declared `string` property for a PostgreSQL decimal column:\n{code}"
    );
    assert!(
        code.contains("(string) $row['total']"),
        "a column the manifest declares `string` must still be cast to `(string)`:\n{code}"
    );

    assert_generated_file_is_valid("php-pdo", &code);
}

const GO_PGX_SCHEMA: &str = "CREATE TABLE widgets (\
    id SERIAL PRIMARY KEY, \
    identifier UUID NOT NULL, \
    payload JSONB NOT NULL, \
    ip INET NOT NULL\
);";
const GO_PGX_QUERY: &str =
    "-- @name GetWidget\n-- @returns :one\nSELECT id, identifier, payload, ip FROM widgets WHERE id = $1;";

/// The defect this file exists for on the Go side (#198): `go-pgx.toml`
/// declares `uuid = "uuid.UUID"`, `json = "json.RawMessage"`, and
/// `inet = "netip.Addr"`, each backed by an `[imports.rules]` entry, but
/// `go_file_header` only ever knew about `"time."` and `"decimal."`. A query
/// selecting all three columns must now pull in all three imports -- the
/// generated file did not compile before this fix, with no import for any of
/// `uuid.UUID`, `json.RawMessage`, or `netip.Addr`.
#[test]
fn go_pgx_emits_the_imports_uuid_json_and_inet_columns_need() {
    let code = generate_full_file(
        "go-pgx",
        "postgresql",
        &SqlDialect::PostgreSQL,
        GO_PGX_SCHEMA,
        GO_PGX_QUERY,
    );

    assert!(
        code.contains("Identifier uuid.UUID")
            && code.contains("Payload json.RawMessage")
            && code.contains("Ip netip.Addr"),
        "expected all three manifest-declared types in the row struct:\n{code}"
    );

    for expected_import in ["\"github.com/google/uuid\"", "\"encoding/json\"", "\"net/netip\""] {
        assert!(
            code.contains(expected_import),
            "go-pgx: expected the import {expected_import} for a column the manifest declares to \
             need it -- a column typed from that package with no import for it does not compile:\n{code}"
        );
    }

    assert_generated_file_is_valid("go-pgx", &code);
}

const GO_MYSQL_DATETIME_SCHEMA: &str = "CREATE TABLE events (id INT PRIMARY KEY, occurred_at DATETIME NOT NULL);";
const GO_MYSQL_DATETIME_QUERY: &str =
    "-- @name GetEvent\n-- @returns :one\nSELECT id, occurred_at FROM events WHERE id = ?;";

/// `go-database-sql.mysql.toml` declares only a `"time."` rule (`decimal`,
/// `uuid`, `json`, and `inet` all resolve to a Go primitive or `string`
/// there, so it needs nothing else). This pins that manifest-driven imports
/// still work for the one rule every non-`go-pgx` manifest declares, not
/// only for `go-pgx`'s five.
#[test]
fn go_database_sql_mysql_still_emits_time_import_for_a_datetime_column() {
    let code = generate_full_file(
        "go-database-sql",
        "mysql",
        &SqlDialect::MySQL,
        GO_MYSQL_DATETIME_SCHEMA,
        GO_MYSQL_DATETIME_QUERY,
    );

    assert!(
        code.contains("OccurredAt time.Time"),
        "expected a `time.Time` field for the DATETIME column:\n{code}"
    );
    assert!(
        code.contains("\"time\""),
        "go-database-sql/mysql: expected the `time` import for a `time.Time` field:\n{code}"
    );

    assert_generated_file_is_valid("go-database-sql", &code);
}

const GO_SQLITE_DATETIME_SCHEMA: &str = "CREATE TABLE events (id INTEGER PRIMARY KEY, occurred_at DATETIME NOT NULL);";
const GO_SQLITE_DATETIME_QUERY: &str =
    "-- @name GetEvent\n-- @returns :one\nSELECT id, occurred_at FROM events WHERE id = ?;";

/// `go-database-sql.sqlite.toml` declares an empty `[imports.rules]` table
/// (SQLite's `datetime`/`decimal`/`uuid`/`json`/`inet` all resolve to
/// `string`/`float64`, needing no import at all). This is the "manifest
/// declares no rules" half of the regression: a backend whose manifest has
/// nothing in `[imports.rules]` must not regress to emitting no imports
/// whatsoever -- `"context"` and `"database/sql"`, the unconditional ones,
/// must still be there.
#[test]
fn go_database_sql_sqlite_emits_no_time_import_and_keeps_the_unconditional_ones() {
    let code = generate_full_file(
        "go-database-sql",
        "sqlite",
        &SqlDialect::SQLite,
        GO_SQLITE_DATETIME_SCHEMA,
        GO_SQLITE_DATETIME_QUERY,
    );

    assert!(
        code.contains("OccurredAt string"),
        "sqlite's `datetime` maps to `string`, not `time.Time`:\n{code}"
    );
    assert!(
        !code.contains("\"time\""),
        "go-database-sql/sqlite: a schema with no temporal column typed `time.Time` must not import \
         `time`:\n{code}"
    );
    assert!(
        code.contains("\"context\"") && code.contains("\"database/sql\""),
        "go-database-sql/sqlite: the unconditional imports must survive a manifest with no \
         `[imports.rules]` entries:\n{code}"
    );

    assert_generated_file_is_valid("go-database-sql", &code);
}
