//! Regression tests for board #196: every JVM backend read a PostgreSQL composite column
//! through `rs.getObject(col, T.class)` / `row.get(col, T.class)`. That overload compiles --
//! `getObject(String, Class<T>)`/`Row.get(String, Class<T>)` are both real driver methods -- but
//! neither pgjdbc nor r2dbc-postgresql has a type map registered for a user-defined composite,
//! so the call throws at runtime the first time a query actually returns one:
//! `PSQLException: conversion to class T from java.lang.String not supported` (pgjdbc) / the
//! equivalent r2dbc-postgresql decode failure.
//!
//! The fix parses PostgreSQL's composite *text form* (`"(a,b,c)"`, what the driver hands back
//! for an unmapped type when asked for `String`) by hand, through a `fromText` static
//! factory/companion method emitted onto the generated record/data class alongside a private
//! `parseCompositeFields` helper that implements the format's escaping rules:
//!
//! - an empty, unquoted field is SQL NULL;
//! - a field containing a comma, paren, quote, backslash, or leading/trailing space (or the
//!   empty string) is wrapped in double quotes, with `"` and `\` backslash-escaped inside;
//! - every other field is unquoted and taken literally.
//!
//! A nested composite's own text form always contains parens, so it always arrives at the outer
//! level as one already-unescaped quoted field -- ready for that type's own `fromText` to parse
//! recursively. See `src/backends/java_jdbc.rs`'s `JAVA_PARSE_COMPOSITE_FIELDS_METHOD` (and its
//! four language/driver siblings) for the implementation this file exercises.
//!
//! `jvm_reader_type_regression.rs` used to pin the *defective* shape as correct (`rs.getObject
//! ("home_address", WidgetAddress.class)` present, asserted with `assert_contains`) in three
//! tests; those assertions are inverted alongside this file landing -- see that file's updated
//! doc comments.

use scythe_codegen::provenance;
use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// `inner_thing` is a composite in its own right (covering `int32`/`string`/`bool` field
/// conversion and the NULL/empty-quoted-string escaping split); `widget_address` embeds it as a
/// field, covering the nested-composite recursion. `tag_only` selects `inner_thing` directly (not
/// only as a nested field) so its own `CompositeInfo` is collected too -- `analyzed.composites`
/// is built by scanning selected *columns*, not by walking into a composite's own fields, so a
/// composite reachable only as another composite's field never gets its definition emitted. That
/// gap is real but pre-existing and out of scope here; this schema sidesteps it rather than
/// exercising it.
const SCHEMA: &str = "CREATE TYPE widget_state AS ENUM ('ready', 'blocked'); \
    CREATE TYPE inner_thing AS (n INTEGER, label TEXT, flag BOOLEAN, state widget_state); \
    CREATE TYPE widget_address AS (street TEXT, city TEXT, tag inner_thing); \
    CREATE TABLE widgets (\
    id SERIAL PRIMARY KEY, \
    home_address widget_address, \
    home_address_required widget_address NOT NULL, \
    tag_only inner_thing NOT NULL, \
    state_only widget_state NOT NULL\
);";

const QUERY: &str = "-- @name GetWidget\n-- @returns :one\n\
    SELECT id, home_address, home_address_required, tag_only, state_only FROM widgets WHERE id = $1;";

fn generate(backend: &dyn CodegenBackend) -> GeneratedCode {
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    generate_with_backend(&analyzed, backend).expect("codegen must succeed")
}

fn backend_for(name: &str) -> Box<dyn CodegenBackend> {
    get_backend(name, "postgresql").expect("backend must support postgresql")
}

/// The whole generated fragment for a backend: enum, composite, row struct and query function
/// concatenated -- mirrors `jvm_reader_type_regression.rs`'s `generated_text`.
fn generated_text(backend_name: &str) -> String {
    let code = generate(&*backend_for(backend_name));
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

/// Assemble the same bytes `scythe generate` writes, so a compiler is handed a whole file rather
/// than a fragment. Mirrors `jvm_reader_type_regression.rs`'s `generated_file`.
fn generated_file(backend_name: &str) -> String {
    let backend = backend_for(backend_name);
    let code = generate(&*backend);
    let all = std::slice::from_ref(&code);

    let mut body = backend.file_header_for_results(all);
    body.push('\n');

    let class_header = backend.query_class_header();
    if !class_header.is_empty() {
        for text in [&code.enum_def, &code.model_struct, &code.row_struct]
            .into_iter()
            .flatten()
        {
            body.push_str(text);
            body.push('\n');
        }
        body.push_str(&class_header);
        body.push('\n');
        if let Some(ref text) = code.query_fn {
            body.push_str(text);
            body.push('\n');
        }
    } else {
        for text in [&code.enum_def, &code.model_struct, &code.row_struct, &code.query_fn]
            .into_iter()
            .flatten()
        {
            body.push_str(text);
            body.push('\n');
        }
    }
    let footer = backend.file_footer();
    if !footer.is_empty() {
        body.push_str(&footer);
        body.push('\n');
    }

    provenance::assemble_file(
        &backend.file_preamble(),
        &provenance::header_line(
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

// -- every backend: the type-map-less accessor must be gone for composite columns ------------

#[test]
fn java_jdbc_reads_composite_columns_through_text_form_not_getobject() {
    let code = generated_text("java-jdbc");
    for column in ["home_address", "home_address_required"] {
        assert_contains(
            "java-jdbc",
            &code,
            &format!("WidgetAddress.fromText(rs.getString(\"{column}\"))"),
            "pgjdbc has no type map for WidgetAddress; the text form must be parsed by hand",
        );
        assert_absent(
            "java-jdbc",
            &code,
            &format!("rs.getObject(\"{column}\", WidgetAddress.class)"),
            "this throws PSQLException at runtime against real pgjdbc -- board #196",
        );
    }
    assert_contains(
        "java-jdbc",
        &code,
        "InnerThing.fromText(rs.getString(\"tag_only\"))",
        "a composite selected directly, not only nested, must read the same way",
    );
}

#[test]
fn kotlin_jdbc_reads_composite_columns_through_text_form_not_getobject() {
    let code = generated_text("kotlin-jdbc");
    for column in ["home_address", "home_address_required"] {
        assert_contains(
            "kotlin-jdbc",
            &code,
            &format!("WidgetAddress.fromText(rs.getString(\"{column}\"))"),
            "pgjdbc has no type map for WidgetAddress; the text form must be parsed by hand",
        );
        assert_absent(
            "kotlin-jdbc",
            &code,
            &format!("rs.getObject(\"{column}\", WidgetAddress::class.java)"),
            "this throws PSQLException at runtime against real pgjdbc -- board #196",
        );
    }
    // ~keep Unlike every other nullable type in this backend, a composite column must not grow a
    // `homeAddressValue`/wasNull() preamble pair -- `fromText` is already null-safe, and the
    // broken preamble arm would have emitted `rs.getObject(col, WidgetAddress::class.java)` for
    // exactly this column (see the fallback tail of `write_kt_nullable_preamble`).
    assert_absent(
        "kotlin-jdbc",
        &code,
        "home_addressValue",
        "composite columns must not route through the generic nullable preamble at all",
    );
}

#[test]
fn kotlin_exposed_reads_composite_columns_through_text_form_not_getobject() {
    let code = generated_text("kotlin-exposed");
    for column in ["home_address", "home_address_required"] {
        assert_contains(
            "kotlin-exposed",
            &code,
            &format!("WidgetAddress.fromText(rs.getString(\"{column}\"))"),
            "Exposed's exec block hands out a plain java.sql.ResultSet -- same defect, same fix",
        );
        assert_absent(
            "kotlin-exposed",
            &code,
            &format!("rs.getObject(\"{column}\", WidgetAddress::class.java)"),
            "this throws PSQLException at runtime against real pgjdbc -- board #196",
        );
    }
}

#[test]
fn java_r2dbc_reads_composite_columns_through_text_form_not_row_get_class() {
    let code = generated_text("java-r2dbc");
    for column in ["home_address", "home_address_required"] {
        assert_contains(
            "java-r2dbc",
            &code,
            &format!("WidgetAddress.fromText(row.get(\"{column}\", String.class))"),
            "r2dbc-postgresql has no codec for WidgetAddress; the text form must be parsed by hand",
        );
        assert_absent(
            "java-r2dbc",
            &code,
            &format!("row.get(\"{column}\", WidgetAddress.class)"),
            "an unregistered composite class is driver-codec-dependent and throws at runtime",
        );
    }
}

#[test]
fn kotlin_r2dbc_reads_composite_columns_through_text_form_not_row_get_class() {
    let code = generated_text("kotlin-r2dbc");
    for column in ["home_address", "home_address_required"] {
        assert_contains(
            "kotlin-r2dbc",
            &code,
            &format!("WidgetAddress.fromText(row.get(\"{column}\", String::class.java))"),
            "r2dbc-postgresql has no codec for WidgetAddress; the text form must be parsed by hand",
        );
        assert_absent(
            "kotlin-r2dbc",
            &code,
            &format!("row.get(\"{column}\", WidgetAddress::class.java)"),
            "an unregistered composite class is driver-codec-dependent and throws at runtime",
        );
    }
}

// -- nested composites recurse through the inner type's own `fromText` -----------------------

#[test]
fn java_jdbc_nested_composite_field_recurses_through_inner_fromtext() {
    let code = generated_text("java-jdbc");
    assert_contains(
        "java-jdbc",
        &code,
        "public record WidgetAddress(@Nullable String street, @Nullable String city, @Nullable InnerThing tag) {",
        "the nested field is declared as the inner composite's own record type",
    );
    assert_contains(
        "java-jdbc",
        &code,
        "InnerThing.fromText(f.get(2))",
        "a composite-typed field must recurse through the inner type's own fromText, not attempt \
         to parse the (already-unescaped) nested text form itself",
    );
}

#[test]
fn kotlin_jdbc_nested_composite_field_recurses_through_inner_fromtext() {
    let code = generated_text("kotlin-jdbc");
    assert_contains(
        "kotlin-jdbc",
        &code,
        "val tag: InnerThing?,",
        "the nested field is declared as the inner composite's own data class type",
    );
    assert_contains(
        "kotlin-jdbc",
        &code,
        "f[2]?.let { value -> InnerThing.fromText(value)!! }",
        "a composite-typed field must recurse through the inner type's own fromText",
    );
}

// -- NULL: the whole point of a nullable composite column ------------------------------------

#[test]
fn java_jdbc_composite_fromtext_returns_null_for_a_null_column() {
    let code = generated_text("java-jdbc");
    assert_contains(
        "java-jdbc",
        &code,
        "public static WidgetAddress fromText(String text) {\n        if (text == null) {\n            return null;\n        }",
        "getString returns null for a SQL NULL composite column; fromText must hand that straight back",
    );
    assert_contains(
        "java-jdbc",
        &code,
        "public static InnerThing fromText(String text) {\n        if (text == null) {\n            return null;\n        }",
        "every composite's fromText needs the same null guard, not only the outer one",
    );
}

#[test]
fn kotlin_jdbc_composite_fromtext_returns_null_for_a_null_column() {
    let code = generated_text("kotlin-jdbc");
    assert_contains(
        "kotlin-jdbc",
        &code,
        "fun fromText(text: String?): WidgetAddress? {\n            if (text == null) {\n                return null\n            }",
        "rs.getString returns null for a SQL NULL composite column; fromText must hand that back",
    );
}

// -- the escaping rules themselves: NULL-vs-empty-string, quoting, backslash escapes ----------

#[test]
fn java_jdbc_parse_composite_fields_implements_the_escaping_rules() {
    let code = generated_text("java-jdbc");
    assert_contains(
        "java-jdbc",
        &code,
        "isNull = field.length() == 0;",
        "an empty *unquoted* field is SQL NULL -- the case a quoted empty string \"\" must not hit",
    );
    assert_contains(
        "java-jdbc",
        &code,
        "if (c == '\\\\' && i + 1 < n) {",
        "a backslash inside a quoted field escapes the next character (\\\" or \\\\), per the format",
    );
    assert_contains(
        "java-jdbc",
        &code,
        "fields.add(isNull ? null : field.toString());",
        "the parsed token is null for a NULL field, never the literal string \"null\"",
    );
}

#[test]
fn kotlin_jdbc_parse_composite_fields_implements_the_escaping_rules() {
    let code = generated_text("kotlin-jdbc");
    assert_contains(
        "kotlin-jdbc",
        &code,
        "isNull = field.isEmpty()",
        "an empty *unquoted* field is SQL NULL -- the case a quoted empty string \"\" must not hit",
    );
    assert_contains(
        "kotlin-jdbc",
        &code,
        "if (c == '\\\\' && i + 1 < n) {",
        "a backslash inside a quoted field escapes the next character (\\\" or \\\\), per the format",
    );
    assert_contains(
        "kotlin-jdbc",
        &code,
        "fields.add(if (isNull) null else field.toString())",
        "the parsed token is null for a NULL field, never the literal string \"null\"",
    );
}

// -- scalar field conversion: int/bool must not stay raw strings ------------------------------

#[test]
fn java_jdbc_composite_scalar_fields_are_converted_not_left_as_strings() {
    let code = generated_text("java-jdbc");
    assert_contains(
        "java-jdbc",
        &code,
        "Integer.parseInt(f.get(0))",
        "an int32 composite field (`n`) must be parsed, not assigned the raw text token",
    );
    assert_contains(
        "java-jdbc",
        &code,
        "\"t\".equals(f.get(2))",
        "PostgreSQL's boolean text output is \"t\"/\"f\", not \"true\"/\"false\"",
    );
}

#[test]
fn kotlin_jdbc_composite_scalar_fields_are_converted_not_left_as_strings() {
    let code = generated_text("kotlin-jdbc");
    assert_contains(
        "kotlin-jdbc",
        &code,
        "f[0]?.let { value -> value.toInt() }",
        "an int32 composite field (`n`) must be parsed, not assigned the raw text token",
    );
    assert_contains(
        "kotlin-jdbc",
        &code,
        "f[2]?.let { value -> value == \"t\" }",
        "PostgreSQL's boolean text output is \"t\"/\"f\", not \"true\"/\"false\"",
    );
}

#[test]
fn kotlin_composite_nullable_fields_use_null_safe_base_type_conversions() {
    for backend in ["kotlin-jdbc", "kotlin-r2dbc", "kotlin-exposed"] {
        let code = generated_text(backend);
        assert_contains(
            backend,
            &code,
            "val n: Int?,",
            "PostgreSQL composite attributes are nullable unless live metadata proves otherwise",
        );
        assert_contains(
            backend,
            &code,
            "f[0]?.let { value -> value.toInt() }",
            "a NULL scalar token must remain null instead of reaching a parser",
        );
        assert_contains(
            backend,
            &code,
            "f[2]?.let { value -> InnerThing.fromText(value)!! }",
            "a NULL nested-composite token must remain null instead of reaching fromText",
        );
        assert_contains(
            backend,
            &code,
            "f[3]?.let { value -> WidgetState.fromValue(value) }",
            "enum conversion must use the non-null base type and skip NULL tokens",
        );
        assert_absent(
            backend,
            &code,
            "WidgetState?.fromValue",
            "a nullable declaration spelling is not a valid Kotlin static receiver",
        );
    }
}

// -- the compilers, additively (mirrors jvm_reader_type_regression.rs) ------------------------

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
fn java_jdbc_composite_text_form_file_compiles() {
    assert_file_compiles("java-jdbc", generated_file("java-jdbc"));
}

#[test]
fn java_r2dbc_composite_text_form_file_compiles() {
    assert_file_compiles("java-r2dbc", generated_file("java-r2dbc"));
}

#[test]
fn kotlin_jdbc_composite_text_form_file_compiles() {
    assert_file_compiles("kotlin-jdbc", generated_file("kotlin-jdbc"));
}
