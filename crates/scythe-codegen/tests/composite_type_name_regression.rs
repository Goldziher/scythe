//! Regression tests for board #199: a schema-qualified composite's `.` reached the generated
//! type name unsanitized.
//!
//! #136 fixed the identical defect for enums by routing `enum_type_name` through
//! `sanitize_for_identifier` before casing. Composites were never fixed the same way: every
//! backend's `generate_composite_def` called `to_pascal_case(&composite.sql_name)` directly, so
//! `CREATE TYPE app.point AS (...)` reached codegen as `composite.sql_name == "app.point"` and
//! `to_pascal_case("app.point")` split only on `_`, producing `"App.point"` -- a `.` inside
//! `pub struct App.point { ... }` (or the target's equivalent), a syntax error in every backend
//! that shared this call.
//!
//! The fix adds `composite_type_name` beside `enum_type_name` in
//! `scythe-backend/src/naming.rs` (routed through the same `sanitize_for_identifier`) and moves
//! every backend's declaration call through it, so the `.` becomes `_` before casing runs.
//!
//! A second, independent instance of the same defect lived in the five JVM backends
//! (`java_jdbc.rs`, `java_r2dbc.rs`, `kotlin_jdbc.rs`, `kotlin_r2dbc.rs`, `kotlin_exposed.rs`):
//! `composite_field_from_text[_kotlin]` -- the function that renders a nested composite field's
//! `T.fromText(...)` call inside the *outer* composite's own `fromText` method -- called
//! `to_pascal_case(sql_name)` directly too, on the *reference* side rather than the declaration
//! side. Because the two call sites derived the name differently (one sanitized, one not), a
//! composite nested inside another schema-qualified composite would declare
//! `PublicInnerPoint.fromText(...)` but be *called* as `Public.innerPoint.fromText(...)` --
//! declaration and reference disagreeing the same way #164 did, a mismatch that does not compile
//! rather than merely mis-parsing.

use scythe_codegen::{GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::errors::ScytheError;
use scythe_core::parser::parse_query_with_dialect;

fn generate(schema: &str, query: &str, backend_name: &str) -> Result<GeneratedCode, ScytheError> {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[schema], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    generate_with_backend(&analyzed, &*backend)
}

/// A single schema-qualified composite (`public.widget_shape`, no nesting) selected on its own --
/// enough to exercise every backend's declaration-side `generate_composite_def` call, the same
/// call site the top-level enum regression (`enum_and_query_name_regression.rs`) exercises for
/// `enum_type_name`.
const SIMPLE_SCHEMA: &str = "\
    CREATE TYPE public.widget_shape AS (label TEXT, side_count INT); \
    CREATE TABLE widgets (id INT PRIMARY KEY, shape public.widget_shape NOT NULL);";

const SIMPLE_QUERY: &str = "-- @name GetWidget\n-- @returns :one\n\
    SELECT id, shape FROM widgets WHERE id = $1;";

/// Rust: before the fix this line read `pub struct Public.widgetShape {`, which does not parse
/// -- a `.` is not a valid character inside a Rust identifier.
#[test]
fn composite_declaration_sanitizes_schema_qualified_dot_in_rust() {
    let code = generate(SIMPLE_SCHEMA, SIMPLE_QUERY, "rust-sqlx").expect("codegen must succeed");
    let model_struct = code.model_struct.expect("composite definition must be emitted");
    assert!(
        model_struct.contains("pub struct PublicWidgetShape {"),
        "expected the sanitized type name:\n{model_struct}"
    );
    assert!(
        !model_struct.contains("Public.widgetShape") && !model_struct.contains("Public.widget_shape"),
        "the unsanitized, unparseable form must not appear:\n{model_struct}"
    );
}

/// Go: before the fix this line read `type Public.widgetShape struct {`, which does not parse.
#[test]
fn composite_declaration_sanitizes_schema_qualified_dot_in_go() {
    let code = generate(SIMPLE_SCHEMA, SIMPLE_QUERY, "go-pgx").expect("codegen must succeed");
    let model_struct = code.model_struct.expect("composite definition must be emitted");
    assert!(
        model_struct.contains("type PublicWidgetShape struct {"),
        "expected the sanitized type name:\n{model_struct}"
    );
    assert!(
        !model_struct.contains("Public.widgetShape") && !model_struct.contains("Public.widget_shape"),
        "the unsanitized, unparseable form must not appear:\n{model_struct}"
    );
}

/// TypeScript: before the fix this line read `export interface Public.widgetShape {`, which does
/// not parse -- `.` cannot appear in a TypeScript interface name.
#[test]
fn composite_declaration_sanitizes_schema_qualified_dot_in_typescript() {
    let code = generate(SIMPLE_SCHEMA, SIMPLE_QUERY, "typescript-pg").expect("codegen must succeed");
    let model_struct = code.model_struct.expect("composite definition must be emitted");
    assert!(
        model_struct.contains("export interface PublicWidgetShape {"),
        "expected the sanitized type name:\n{model_struct}"
    );
    assert!(
        !model_struct.contains("Public.widgetShape") && !model_struct.contains("Public.widget_shape"),
        "the unsanitized, unparseable form must not appear:\n{model_struct}"
    );
}

/// C#: before the fix this line read `public record Public.widgetShape(`, which does not parse.
#[test]
fn composite_declaration_sanitizes_schema_qualified_dot_in_csharp() {
    let code = generate(SIMPLE_SCHEMA, SIMPLE_QUERY, "csharp-npgsql").expect("codegen must succeed");
    let model_struct = code.model_struct.expect("composite definition must be emitted");
    assert!(
        model_struct.contains("public record PublicWidgetShape("),
        "expected the sanitized type name:\n{model_struct}"
    );
    assert!(
        !model_struct.contains("Public.widgetShape") && !model_struct.contains("Public.widget_shape"),
        "the unsanitized, unparseable form must not appear:\n{model_struct}"
    );
}

/// Python: before the fix this line read `class Public.widgetShape:`, which does not parse --
/// `.` cannot appear in a Python class name.
#[test]
fn composite_declaration_sanitizes_schema_qualified_dot_in_python() {
    let code = generate(SIMPLE_SCHEMA, SIMPLE_QUERY, "python-asyncpg").expect("codegen must succeed");
    let model_struct = code.model_struct.expect("composite definition must be emitted");
    assert!(
        model_struct.contains("class PublicWidgetShape:") || model_struct.contains("class PublicWidgetShape("),
        "expected the sanitized type name:\n{model_struct}"
    );
    assert!(
        !model_struct.contains("Public.widgetShape") && !model_struct.contains("Public.widget_shape"),
        "the unsanitized, unparseable form must not appear:\n{model_struct}"
    );
}

// -- nested composite: declaration and reference must derive the identical sanitized name -----

/// `public.inner_point` is a composite in its own right; `public.widget_outline` embeds it as a
/// field (`origin`), covering the nested-composite reference path
/// `composite_field_from_text[_kotlin]` renders. Both types are schema-qualified so the dot has
/// to be sanitized on *both* the declaration (`generate_composite_def`, fixed alongside every
/// other backend) and the reference (`composite_field_from_text`, a separate call site that had
/// to be fixed independently -- see this file's header) for the two to agree.
///
/// `origin_only` selects `public.inner_point` directly too, so its own definition is emitted for
/// the assertions below to compare against, the same technique
/// `jvm_composite_read_regression.rs` uses.
const NESTED_SCHEMA: &str = "\
    CREATE TYPE public.inner_point AS (x INT, y INT); \
    CREATE TYPE public.widget_outline AS (label TEXT, origin public.inner_point); \
    CREATE TABLE widgets (\
        id INT PRIMARY KEY, \
        outline public.widget_outline NOT NULL, \
        origin_only public.inner_point NOT NULL\
    );";

const NESTED_QUERY: &str = "-- @name GetWidgetOutline\n-- @returns :one\n\
    SELECT id, outline, origin_only FROM widgets WHERE id = $1;";

/// java-jdbc: before the fix, `composite_field_from_text` called `to_pascal_case(sql_name)`
/// directly on the reference side while `generate_composite_def` (now fixed) sanitized the
/// declaration side -- `PublicWidgetOutline.fromText` would call `Public.innerPoint.fromText(...)`,
/// which does not compile, while the sibling declaration read `public record PublicInnerPoint(`.
/// The two must name the same type.
///
/// The negative assertion is deliberately scoped to the `.fromText(` call text, not to every
/// occurrence of the unsanitized spelling: `PublicWidgetOutline`'s *own* field-type annotation
/// (`Public.innerPoint origin` in its record parameter list) is rendered by a different,
/// unfixed call site -- `scythe_backend::types::resolve_type`'s `composite::` branch, which
/// still calls raw `to_pascal_case` -- and is out of scope for this fix (see this crate's
/// commit history / the accompanying report). Asserting the field-type annotation here would
/// pin a still-open, separately-owned defect as if this fix closed it.
#[test]
fn nested_composite_reference_matches_its_own_declaration_in_java_jdbc() {
    let code = generate(NESTED_SCHEMA, NESTED_QUERY, "java-jdbc").expect("codegen must succeed");
    let model_struct = code.model_struct.expect("composite definitions must be emitted");

    assert!(
        model_struct.contains("public record PublicInnerPoint("),
        "the inner composite must declare under its sanitized name:\n{model_struct}"
    );
    assert!(
        model_struct.contains("public record PublicWidgetOutline("),
        "the outer composite must declare under its sanitized name:\n{model_struct}"
    );
    assert!(
        model_struct.contains("PublicInnerPoint.fromText("),
        "the outer composite's fromText must reference the inner composite by its declared \
         (sanitized) name:\n{model_struct}"
    );
    assert!(
        !model_struct.contains("Public.innerPoint.fromText(") && !model_struct.contains("Public.inner_point.fromText("),
        "the unsanitized, unparseable reference form must not appear in the fromText call:\n{model_struct}"
    );
}

/// kotlin-jdbc: same defect, `T::class.java`-adjacent companion-object `fromText` instead of a
/// Java static method. See the java-jdbc test above for why the negative assertion is scoped to
/// the `.fromText(` call rather than the whole composite definition.
#[test]
fn nested_composite_reference_matches_its_own_declaration_in_kotlin_jdbc() {
    let code = generate(NESTED_SCHEMA, NESTED_QUERY, "kotlin-jdbc").expect("codegen must succeed");
    let model_struct = code.model_struct.expect("composite definitions must be emitted");

    assert!(
        model_struct.contains("data class PublicInnerPoint("),
        "the inner composite must declare under its sanitized name:\n{model_struct}"
    );
    assert!(
        model_struct.contains("data class PublicWidgetOutline("),
        "the outer composite must declare under its sanitized name:\n{model_struct}"
    );
    assert!(
        model_struct.contains("PublicInnerPoint.fromText("),
        "the outer composite's fromText must reference the inner composite by its declared \
         (sanitized) name:\n{model_struct}"
    );
    assert!(
        !model_struct.contains("Public.innerPoint.fromText(") && !model_struct.contains("Public.inner_point.fromText("),
        "the unsanitized, unparseable reference form must not appear in the fromText call:\n{model_struct}"
    );
}
