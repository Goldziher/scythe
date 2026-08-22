//! Regression tests for board #204: `python-psycopg3` and `python-asyncpg` declared a
//! nullable composite column (e.g. `home_address: WidgetAddress | None`) and a nullable enum
//! column (e.g. `state: Status | None`) on the generated row dataclass, but read both straight
//! through from the raw row with no conversion (`home_address=row[1]`, `state=row[4]`). Neither
//! driver constructs our generated type on its own:
//!
//! - psycopg3 has no adapter for a user-defined composite -- it hands back the driver's raw
//!   *text form* (`"(a,b,c)"`) as a plain `str`, and no conversion at all for a user-defined
//!   enum, also a plain `str`.
//! - asyncpg *does* decode a composite column natively, but into its own `asyncpg.Record`
//!   (tuple-like, keyed by the composite's attribute names) -- not our generated
//!   dataclass/BaseModel/Struct. An enum column decodes to a plain `str` (asyncpg registers enum
//!   types as a scalar `TEXTOID` codec; verified by reading
//!   `asyncpg/protocol/codecs/base.pyx`'s `DataCodecConfig.add_types`, not from memory).
//!
//! The fix routes both column kinds through explicit conversions:
//!
//! - psycopg3: `T._from_text(...)`, a generated classmethod that parses PostgreSQL's composite
//!   text-form escaping rules by hand (mirrors `java_jdbc.rs`'s `fromText`/`parseCompositeFields`
//!   -- an empty unquoted field is SQL NULL, a field needing quoting is wrapped in `"` with `"`/
//!   `\` backslash-escaped inside).
//! - asyncpg: `T._from_record(...)`, a generated classmethod that reads the already-decoded
//!   `asyncpg.Record` by attribute name -- no text parsing needed, since the driver already
//!   decoded every scalar sub-field to its native Python type.
//! - both: a nullable enum column reads as `None if raw is None else T(raw)` -- `T` is a `str,
//!   Enum` subclass, so `T(raw)` is its value-lookup constructor; calling it unguarded on `None`
//!   raises `ValueError` instead of producing the column's actual SQL NULL.

use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// `inner_thing` covers int32/string/bool composite-field conversion; `widget_address` embeds
/// it as a field (nested-composite recursion) and also carries its own scalar fields. `status`
/// is a plain enum, selected both nullable and non-nullable. `tag_only`/`state_required` select
/// `inner_thing`/`status` directly (not only nested/as a sibling column) the same way
/// `jvm_composite_read_regression.rs`'s `SCHEMA` does, for the same reason: `analyzed.composites`
/// is built by scanning selected columns, not by walking into another composite's own fields.
const SCHEMA: &str = "CREATE TYPE status AS ENUM ('active', 'inactive'); \
    CREATE TYPE inner_thing AS (n INTEGER, label TEXT, flag BOOLEAN); \
    CREATE TYPE widget_address AS (street TEXT, city TEXT, tag inner_thing); \
    CREATE TABLE widgets (\
    id SERIAL PRIMARY KEY, \
    home_address widget_address, \
    home_address_required widget_address NOT NULL, \
    tag_only inner_thing NOT NULL, \
    state status, \
    state_required status NOT NULL\
);";

const QUERY: &str = "-- @name GetWidget\n-- @returns :one\n\
    SELECT id, home_address, home_address_required, tag_only, state, state_required \
    FROM widgets WHERE id = $1;";

/// `:many` sibling of [`QUERY`], to exercise the `r`-named loop variable used by both
/// backends' list-returning read path (a separate code path from `:one`'s `row`).
const QUERY_MANY: &str = "-- @name ListWidgets\n-- @returns :many\n\
    SELECT id, home_address, home_address_required, tag_only, state, state_required \
    FROM widgets;";

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

// -- psycopg3: composite columns route through `_from_text`, never a raw `row[i]` -------------

#[test]
fn psycopg3_reads_composite_columns_through_from_text_not_raw_index() {
    let code = generated_text("python-psycopg3", QUERY);
    assert_contains(
        "python-psycopg3",
        &code,
        "home_address=WidgetAddress._from_text(row[1])",
        "psycopg3 hands back the composite's raw text form as a str; it must be parsed, not \
         assigned straight into the `WidgetAddress`-typed field",
    );
    assert_contains(
        "python-psycopg3",
        &code,
        "home_address_required=WidgetAddress._from_text(row[2])",
        "a NOT NULL composite column needs the same conversion as a nullable one",
    );
    assert_contains(
        "python-psycopg3",
        &code,
        "tag_only=InnerThing._from_text(row[3])",
        "a composite selected directly, not only nested, must read the same way",
    );
    assert_absent(
        "python-psycopg3",
        &code,
        "home_address=row[1]",
        "the defective form: a raw str assigned to a field declared WidgetAddress | None -- \
         board #204's 'str' object has no attribute 'street'",
    );
}

#[test]
fn psycopg3_composite_from_text_returns_none_for_a_null_column() {
    let code = generated_text("python-psycopg3", QUERY);
    assert_contains(
        "python-psycopg3",
        &code,
        "def _from_text(cls, text: str | None) -> \"WidgetAddress | None\":",
        "the classmethod must accept and be able to return None",
    );
    assert_contains(
        "python-psycopg3",
        &code,
        "if text is None:\n            return None",
        "a SQL NULL composite column (psycopg3's row[i] is None) must produce None, never an \
         all-default WidgetAddress built from an empty/garbage parse",
    );
}

#[test]
fn psycopg3_nested_composite_field_recurses_through_inner_from_text() {
    let code = generated_text("python-psycopg3", QUERY);
    assert_contains(
        "python-psycopg3",
        &code,
        "tag=None if f[2] is None else InnerThing._from_text(f[2]),",
        "a nullable composite-typed field must preserve NULL and recurse through the inner \
         type's own _from_text for non-NULL values",
    );
}

#[test]
fn psycopg3_composite_scalar_fields_are_converted_not_left_as_strings() {
    let code = generated_text("python-psycopg3", QUERY);
    assert_contains(
        "python-psycopg3",
        &code,
        "n=None if f[0] is None else int(f[0]),",
        "a nullable int32 composite field (`n`) must preserve NULL and parse non-NULL text",
    );
    assert_contains(
        "python-psycopg3",
        &code,
        "flag=None if f[2] is None else f[2] == \"t\",",
        "a nullable boolean field must preserve NULL; PostgreSQL's non-NULL text output is \
         \"t\"/\"f\", not \"True\"/\"False\"",
    );
}

#[test]
fn psycopg3_parse_composite_fields_implements_the_escaping_rules() {
    let code = generated_text("python-psycopg3", QUERY);
    assert_contains(
        "python-psycopg3",
        &code,
        "is_null = len(chars) == 0",
        "an empty *unquoted* field is SQL NULL -- the case a quoted empty string \"\" must not hit",
    );
    assert_contains(
        "python-psycopg3",
        &code,
        "fields.append(None if is_null else \"\".join(chars))",
        "the parsed token is None for a NULL field, never the literal string \"None\"",
    );
}

// -- psycopg3/asyncpg: a nullable enum column must not construct from None --------------------

#[test]
fn psycopg3_nullable_enum_column_guards_none_before_constructing() {
    let code = generated_text("python-psycopg3", QUERY);
    assert_contains(
        "python-psycopg3",
        &code,
        "state=None if row[4] is None else Status(row[4])",
        "Status(None) raises ValueError -- a NULL state column must produce None, not a crash \
         or (worse) a coincidentally-truthy default member",
    );
    assert_contains(
        "python-psycopg3",
        &code,
        "state_required=Status(row[5])",
        "a NOT NULL enum column still needs the value-lookup constructor -- it must not stay \
         the raw str psycopg3 hands back",
    );
}

#[test]
fn asyncpg_nullable_enum_column_guards_none_before_constructing() {
    let code = generated_text("python-asyncpg", QUERY);
    assert_contains(
        "python-asyncpg",
        &code,
        "state=None if row[\"state\"] is None else Status(row[\"state\"])",
        "asyncpg decodes a user enum to a plain str; a NULL column must produce None, not \
         Status(None) raising ValueError",
    );
    assert_contains(
        "python-asyncpg",
        &code,
        "state_required=Status(row[\"state_required\"])",
        "a NOT NULL enum column still needs the value-lookup constructor",
    );
}

// -- asyncpg: composite columns route through `_from_record`, never the raw `Record` ----------

#[test]
fn asyncpg_reads_composite_columns_through_from_record_not_raw_record() {
    let code = generated_text("python-asyncpg", QUERY);
    assert_contains(
        "python-asyncpg",
        &code,
        "home_address=WidgetAddress._from_record(row[\"home_address\"])",
        "asyncpg decodes a composite column to its own Record, not WidgetAddress; it must be \
         wrapped, not assigned straight into the WidgetAddress-typed field",
    );
    assert_contains(
        "python-asyncpg",
        &code,
        "home_address_required=WidgetAddress._from_record(row[\"home_address_required\"])",
        "a NOT NULL composite column needs the same conversion as a nullable one",
    );
    assert_contains(
        "python-asyncpg",
        &code,
        "tag_only=InnerThing._from_record(row[\"tag_only\"])",
        "a composite selected directly, not only nested, must read the same way",
    );
    assert_absent(
        "python-asyncpg",
        &code,
        "home_address=row[\"home_address\"]",
        "the defective form: an asyncpg.Record assigned to a field declared WidgetAddress | \
         None -- board #204's 'tuple-like, looks plausible, is not the declared type'",
    );
}

#[test]
fn asyncpg_composite_from_record_returns_none_for_a_null_column() {
    let code = generated_text("python-asyncpg", QUERY);
    assert_contains(
        "python-asyncpg",
        &code,
        "def _from_record(cls, record: Any) -> \"WidgetAddress | None\":",
        "the classmethod must accept and be able to return None",
    );
    assert_contains(
        "python-asyncpg",
        &code,
        "if record is None:\n            return None",
        "a SQL NULL composite column must produce None, never an all-default WidgetAddress",
    );
}

#[test]
fn asyncpg_nested_composite_field_recurses_through_inner_from_record() {
    let code = generated_text("python-asyncpg", QUERY);
    assert_contains(
        "python-asyncpg",
        &code,
        "tag=None if record[\"tag\"] is None else InnerThing._from_record(record[\"tag\"]),",
        "a nullable composite-typed field must preserve NULL and recurse through the inner \
         type's own _from_record for non-NULL values",
    );
}

/// asyncpg decodes every non-NULL scalar sub-field of a composite to its native Python type
/// already, while nullable fields still need to preserve `None`
/// (verified from `asyncpg/protocol/codecs/base.pyx`'s `decode_composite`, which dispatches
/// through each attribute's own element codec) -- re-parsing `n`/`flag` as text, the way
/// psycopg3 must, would be redundant *and* wrong (`int("3")` on an already-`int` value is fine,
/// but there is nothing to strip; the point of this test is that no text-parsing helper is
/// emitted at all for asyncpg).
#[test]
fn asyncpg_composite_scalar_fields_pass_through_the_drivers_native_decode() {
    let code = generated_text("python-asyncpg", QUERY);
    assert_contains(
        "python-asyncpg",
        &code,
        "n=None if record[\"n\"] is None else record[\"n\"],",
        "a nullable int32 field must preserve NULL; asyncpg already decodes non-NULL values \
         to native Python ints",
    );
    assert_contains(
        "python-asyncpg",
        &code,
        "flag=None if record[\"flag\"] is None else record[\"flag\"],",
        "a nullable bool field must preserve NULL; asyncpg already decodes non-NULL values \
         to native Python bools",
    );
    assert_absent(
        "python-asyncpg",
        &code,
        "_parse_composite_fields",
        "asyncpg needs no PostgreSQL composite-text-form parser -- the driver already decodes \
         binary composite wire format; emitting one would be dead code copied from psycopg3",
    );
}

// -- the `:many` path (`r`-named loop variable) gets the same conversions ---------------------

#[test]
fn psycopg3_many_command_converts_composite_and_enum_columns_too() {
    let code = generated_text("python-psycopg3", QUERY_MANY);
    assert_contains(
        "python-psycopg3",
        &code,
        "home_address=WidgetAddress._from_text(r[1])",
        "the :many path loops over `r`, not `row`, but must apply the same conversion",
    );
    assert_contains(
        "python-psycopg3",
        &code,
        "state=None if r[4] is None else Status(r[4])",
        "the :many path's nullable enum column must be guarded the same way as :one's",
    );
}

#[test]
fn asyncpg_many_command_converts_composite_and_enum_columns_too() {
    let code = generated_text("python-asyncpg", QUERY_MANY);
    assert_contains(
        "python-asyncpg",
        &code,
        "home_address=WidgetAddress._from_record(r[\"home_address\"])",
        "the :many path loops over `r`, not `row`, but must apply the same conversion",
    );
    assert_contains(
        "python-asyncpg",
        &code,
        "state=None if r[\"state\"] is None else Status(r[\"state\"])",
        "the :many path's nullable enum column must be guarded the same way as :one's",
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
fn psycopg3_composite_enum_read_file_compiles() {
    assert_file_compiles("python-psycopg3", generated_file("python-psycopg3", QUERY));
}

#[test]
fn asyncpg_composite_enum_read_file_compiles() {
    assert_file_compiles("python-asyncpg", generated_file("python-asyncpg", QUERY));
}
