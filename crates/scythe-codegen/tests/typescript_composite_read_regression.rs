//! Regression tests for board #204: `typescript-pg`, `typescript-postgres`, and
//! `typescript-kysely` declared a nullable composite column on the generated row `interface`
//! (e.g. `homeAddress: WidgetAddress | null`), but read it with nothing more than a
//! compile-time cast (`row.home_address as WidgetAddress | null`, or the driver's own generic
//! type parameter, e.g. `client.query<GetWidgetRow>(...)`). Neither `pg` nor `postgres.js`
//! registers a decoder for a user-defined composite type -- verified by reading each vendored
//! driver's own source under `integration_tests/*/node_modules`, not from memory: `pg` has no
//! composite/record type-parser table, and postgres.js's `src/types.js` has no
//! `record`/`typtype`/composite handling at all. Both hand back PostgreSQL's raw composite
//! *text form* (`"(a,b,c)"`), a plain `string`. A TypeScript `as T` cast is compile-time only,
//! so at runtime `row.home_address` is still that string, and `row.home_address.street` reads
//! `undefined` -- silently, with no thrown error the way Python's attribute access would give.
//!
//! The fix parses the text form by hand through a generated `parse{Name}` function (mirrors
//! `java_jdbc.rs`'s `fromText`/`parseCompositeFields` -- see that file's doc comment for the
//! escaping rules), routed into every read path (`:one`/`:opt`/`:many`, both `field_case`
//! settings, and `:grouped`) via each backend's own `ts_composite_aware_row_access`.
//!
//! A generated `enum::` column needs no equivalent fix: `generate_enum_def` emits a
//! string-literal union (`export type Status = "active" | "inactive"`), which has no runtime
//! representation distinct from the driver's raw string, so the existing `as Status` cast is
//! already correct at runtime -- confirmed by inspecting `generate_enum_def` directly, not
//! assumed.

use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// `inner_thing` covers int32/string/bool composite-field conversion; `widget_address` embeds
/// it as a field (nested-composite recursion). `tag_only` selects `inner_thing` directly, not
/// only nested, the same way `jvm_composite_read_regression.rs`'s schema does and for the same
/// reason: `analyzed.composites` is built by scanning selected columns, not by walking into
/// another composite's own fields.
const SCHEMA: &str = "CREATE TYPE inner_thing AS (n INTEGER, label TEXT, flag BOOLEAN); \
    CREATE TYPE widget_address AS (street TEXT, city TEXT, tag inner_thing); \
    CREATE TABLE widgets (\
    id SERIAL PRIMARY KEY, \
    home_address widget_address, \
    home_address_required widget_address NOT NULL, \
    tag_only inner_thing NOT NULL\
);";

const QUERY: &str = "-- @name GetWidget\n-- @returns :one\n\
    SELECT id, home_address, home_address_required, tag_only FROM widgets WHERE id = $1;";

const QUERY_MANY: &str = "-- @name ListWidgets\n-- @returns :many\n\
    SELECT id, home_address, home_address_required, tag_only FROM widgets;";

fn generate(backend: &dyn CodegenBackend, sql: &str) -> GeneratedCode {
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(sql, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    generate_with_backend(&analyzed, backend).expect("codegen must succeed")
}

fn backend_for(name: &str) -> Box<dyn CodegenBackend> {
    get_backend(name, "postgresql").expect("backend must support postgresql")
}

fn generated_text(backend_name: &str, sql: &str) -> String {
    let code = generate(&*backend_for(backend_name), sql);
    let mut out = String::new();
    for text in [&code.enum_def, &code.model_struct, &code.row_struct, &code.query_fn]
        .into_iter()
        .flatten()
    {
        out.push_str(text);
        out.push('\n');
    }
    out
}

fn generated_file(backend_name: &str, sql: &str) -> String {
    let backend = backend_for(backend_name);
    let code = generate(&*backend, sql);
    let all = std::slice::from_ref(&code);

    let mut body = backend.file_header_for_results(all);
    body.push('\n');
    for text in [&code.enum_def, &code.model_struct, &code.row_struct, &code.query_fn]
        .into_iter()
        .flatten()
    {
        body.push_str(text);
        body.push('\n');
    }
    let footer = backend.file_footer();
    if !footer.is_empty() {
        body.push_str(&footer);
        body.push('\n');
    }

    scythe_codegen::provenance::assemble_file(
        &backend.file_preamble(),
        &scythe_codegen::provenance::header_line(
            &*backend,
            env!("CARGO_PKG_VERSION"),
            "postgresql",
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &body,
    )
}

fn assert_contains(backend: &str, code: &str, needle: &str, why: &str) {
    assert!(
        code.contains(needle),
        "{backend}: expected `{needle}` ({why});\ngenerated:\n{code}"
    );
}

fn assert_absent(backend: &str, code: &str, needle: &str, why: &str) {
    assert!(
        !code.contains(needle),
        "{backend}: `{needle}` must not appear ({why});\ngenerated:\n{code}"
    );
}

// -- pg: composite columns route through parse{Name}, never a bare cast -----------------------

#[test]
fn pg_reads_composite_columns_through_parse_fn_not_bare_cast() {
    let code = generated_text("typescript-pg", QUERY);
    assert_contains(
        "typescript-pg",
        &code,
        "home_address: parseWidgetAddress(row.home_address) as WidgetAddress | null,",
        "pg hands back the composite's raw text form as a string; it must be parsed, not \
         merely cast to WidgetAddress | null",
    );
    assert_contains(
        "typescript-pg",
        &code,
        "home_address_required: parseWidgetAddress(row.home_address_required) as WidgetAddress,",
        "a NOT NULL composite column needs the same conversion as a nullable one",
    );
    assert_contains(
        "typescript-pg",
        &code,
        "tag_only: parseInnerThing(row.tag_only) as InnerThing,",
        "a composite selected directly, not only nested, must read the same way",
    );
}

#[test]
fn pg_composite_parse_fn_returns_null_for_a_null_column() {
    let code = generated_text("typescript-pg", QUERY);
    assert_contains(
        "typescript-pg",
        &code,
        "export function parseWidgetAddress(raw: unknown): WidgetAddress | null {",
        "the function must accept the driver's untyped value and be able to return null",
    );
    assert_contains(
        "typescript-pg",
        &code,
        "if (raw === null || raw === undefined) {\n\t\treturn null;\n\t}",
        "a SQL NULL composite column must produce null, never an all-default WidgetAddress \
         built from an empty/garbage parse",
    );
}

#[test]
fn pg_nested_composite_field_recurses_through_inner_parse_fn() {
    let code = generated_text("typescript-pg", QUERY);
    assert_contains(
        "typescript-pg",
        &code,
        "tag: f[2] === null ? null : parseInnerThing(f[2]) as InnerThing,",
        "a composite-typed field must recurse through the inner type's own parse function, not \
         attempt to parse the (already-unescaped) nested text form itself",
    );
}

#[test]
fn pg_composite_scalar_fields_are_converted_not_left_as_strings() {
    let code = generated_text("typescript-pg", QUERY);
    assert_contains(
        "typescript-pg",
        &code,
        "n: f[0] === null ? null : Number(f[0]),",
        "an int32 composite field (`n`) must be parsed, not assigned the raw text token",
    );
    assert_contains(
        "typescript-pg",
        &code,
        "flag: f[2] === null ? null : f[2] === \"t\",",
        "PostgreSQL's boolean text output is \"t\"/\"f\", not \"true\"/\"false\"",
    );
}

#[test]
fn pg_parse_composite_fields_implements_the_escaping_rules() {
    let code = generated_text("typescript-pg", QUERY);
    assert_contains(
        "typescript-pg",
        &code,
        "isNull = chars.length === 0;",
        "an empty *unquoted* field is SQL NULL -- the case a quoted empty string \"\" must not hit",
    );
    assert_contains(
        "typescript-pg",
        &code,
        "fields.push(isNull ? null : chars);",
        "the parsed token is null for a NULL field, never the literal string \"null\"",
    );
}

/// `field_case = "snake"` (the default) previously returned `rows[0]` untouched -- no per-field
/// reconstruction at all, so a composite column had no read path to fix short of this one.
#[test]
fn pg_snake_mode_one_command_still_converts_composite_columns() {
    let code = generated_text("typescript-pg", QUERY);
    assert_contains(
        "typescript-pg",
        &code,
        "return {\n\t\t...row,\n\t\thome_address: parseWidgetAddress(row.home_address) as WidgetAddress | null,",
        "the default (snake) field_case must spread the row and override just the composite \
         fields, not trust the bare client.query<GetWidgetRow> generic",
    );
}

#[test]
fn pg_snake_mode_many_command_converts_composite_columns_too() {
    let code = generated_text("typescript-pg", QUERY_MANY);
    assert_contains(
        "typescript-pg",
        &code,
        "return rows.map((row) => ({",
        ":many under snake field_case must map every row through the composite override too",
    );
    assert_contains(
        "typescript-pg",
        &code,
        "home_address: parseWidgetAddress(row.home_address) as WidgetAddress | null,",
        ":many's per-row override must apply the same conversion as :one's",
    );
}

// -- postgres.js: same defect, bracket-access row shape -----------------------------------------

#[test]
fn postgres_reads_composite_columns_through_parse_fn_not_bare_cast() {
    let code = generated_text("typescript-postgres", QUERY);
    assert_contains(
        "typescript-postgres",
        &code,
        "home_address: parseWidgetAddress(row['home_address']) as WidgetAddress | null,",
        "postgres.js hands back the composite's raw text form as a string; it must be parsed",
    );
    assert_contains(
        "typescript-postgres",
        &code,
        "tag_only: parseInnerThing(row['tag_only']) as InnerThing,",
        "a composite selected directly, not only nested, must read the same way",
    );
}

#[test]
fn postgres_composite_parse_fn_returns_null_for_a_null_column() {
    let code = generated_text("typescript-postgres", QUERY);
    assert_contains(
        "typescript-postgres",
        &code,
        "export function parseWidgetAddress(raw: unknown): WidgetAddress | null {",
        "the function must accept the driver's untyped value and be able to return null",
    );
    assert_contains(
        "typescript-postgres",
        &code,
        "if (raw === null || raw === undefined) {\n\t\treturn null;\n\t}",
        "a SQL NULL composite column must produce null",
    );
}

// -- kysely: same defect; generate_query_fn previously ignored `columns` entirely --------------

#[test]
fn kysely_reads_composite_columns_through_parse_fn_not_bare_cast() {
    let code = generated_text("typescript-kysely", QUERY);
    assert_contains(
        "typescript-kysely",
        &code,
        "home_address: parseWidgetAddress(row.home_address) as WidgetAddress | null,",
        "kysely's sql tag hands back the composite's raw text form as a string, unconverted by \
         the query builder; it must be parsed, not merely cast",
    );
    assert_absent(
        "typescript-kysely",
        &code,
        "\treturn row;\n}",
        "the pre-fix form: trusting sql<GetWidgetRow>`...`.execute(db) blindly, which is how a \
         composite column's fields silently read undefined at runtime",
    );
}

#[test]
fn kysely_composite_parse_fn_returns_null_for_a_null_column() {
    let code = generated_text("typescript-kysely", QUERY);
    assert_contains(
        "typescript-kysely",
        &code,
        "export function parseWidgetAddress(raw: unknown): WidgetAddress | null {",
        "the function must accept the driver's untyped value and be able to return null",
    );
    assert_contains(
        "typescript-kysely",
        &code,
        "if (raw === null || raw === undefined) {\n\t\treturn null;\n\t}",
        "a SQL NULL composite column must produce null",
    );
}

// -- the compilers, additively (mirrors jvm_composite_read_regression.rs) ---------------------

fn assert_file_compiles(backend_name: &str, file: String) {
    let validation = validate_with_tools(&file, backend_name);

    assert_ne!(
        validation,
        ToolValidation::Unsupported,
        "{backend_name} lost its tool validator; the compile half of this test is now vacuous"
    );
    for tool in validation.tools_run() {
        eprintln!("  {backend_name}: `{tool}` ran against the generated file");
    }
    for tool in validation.missing_tools() {
        eprintln!("  {backend_name}: `{tool}` is not on PATH -- the compile half went unchecked");
    }
    if strict_mode_enabled() {
        assert!(
            validation.fully_checked(),
            "{backend_name}: tool validation reports nothing actually checked the code"
        );
    }
    if let Err(errors) = validation.into_result() {
        panic!("{backend_name} tool validation: {errors:?}\n\nGenerated file:\n{file}");
    }
}

#[test]
fn pg_composite_read_file_compiles() {
    assert_file_compiles("typescript-pg", generated_file("typescript-pg", QUERY));
}

#[test]
fn postgres_composite_read_file_compiles() {
    assert_file_compiles("typescript-postgres", generated_file("typescript-postgres", QUERY));
}

#[test]
fn kysely_composite_read_file_compiles() {
    assert_file_compiles("typescript-kysely", generated_file("typescript-kysely", QUERY));
}
