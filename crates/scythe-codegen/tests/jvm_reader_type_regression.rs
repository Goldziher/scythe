//! Regression tests for the JVM backends' column readers (#191, #192, #213,
//! #214).
//!
//! Two distinct defects, both rooted in the same design: each JVM backend
//! resolved a column's accessor from its own hardcoded table of type
//! spellings, independent of the manifest that decided the *declared* type.
//!
//! 1. **The reader did not compile.** Every one of those tables ended in an
//!    untyped fallback -- `rs.getObject(col)` on `java-jdbc`/`kotlin-jdbc`/
//!    `kotlin-exposed`, `row.get(col, Object.class)` / `row.get(col,
//!    Any::class.java)` on the R2DBC pair. Their static types (`Object`,
//!    `Any!`) are assignable to nothing, so every column outside the table
//!    produced a file the compiler rejected. Confirmed live against real
//!    `javac`/`kotlinc` on a composite column (`incompatible types: Object
//!    cannot be converted to ReproAddress`) and on `kotlin-jdbc`'s `uuid`
//!    (`argument type mismatch: actual type is 'Any!', but 'UUID' was
//!    expected`). `java-r2dbc` had a third, independent failure: it emitted
//!    top-level `public record`s and bare `public static` methods with no
//!    enclosing class, which is not a legal Java compilation unit at all.
//!
//! 2. **The reader corrupted NULLs.** JDBC's primitive getters return
//!    `0`/`false` for SQL NULL and require a following `wasNull()` to tell
//!    that apart from a real zero. `kotlin-exposed` had no `wasNull()`
//!    handling whatsoever, so a nullable `Int?`/`Boolean?`/`Double?` column
//!    silently became `0`/`false` instead of null. Separately, all three JDBC
//!    backends read a *nullable* enum column as
//!    `Status.valueOf(rs.getString(col).uppercase())`, which throws on
//!    exactly the value a nullable enum column exists to hold.
//!
//! The fix removes the second table: a reference-typed column is read through
//! the class-taking accessor overload (`rs.getObject(col, T.class)`,
//! `row.get(col, T.class)`) with `T` derived from the declared type, so
//! declaration and reader cannot drift. See
//! `src/backends/jvm_common.rs`.
//!
//! Every assertion below runs unconditionally on the generated text. The real
//! `javac`/`kotlinc` compilations at the bottom are *additive*: they prove the
//! text claims add up to a file the compiler accepts, and they are the only
//! thing that would have caught the `java-r2dbc` missing-class-wrapper defect,
//! which no substring check would have looked for.

use scythe_codegen::provenance;
use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_with_tools};
use scythe_codegen::{CodegenBackend, GeneratedCode, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// A composite column, a uuid column, a non-null and a nullable enum column,
/// one nullable column per JDBC primitive getter, and a naive-datetime column
/// (whose `LocalDateTime` the R2DBC backends used to read as `LocalDate`,
/// because `contains("LocalDate")` matched first).
const SCHEMA: &str = "CREATE TYPE widget_status AS ENUM ('active', 'inactive'); \
    CREATE TYPE widget_address AS (street TEXT, city TEXT); \
    CREATE TABLE widgets (\
    id SERIAL PRIMARY KEY, \
    name TEXT NOT NULL, \
    home_address widget_address, \
    external_id UUID NOT NULL, \
    status widget_status NOT NULL, \
    optional_status widget_status, \
    nullable_int INTEGER, \
    nullable_bigint BIGINT, \
    nullable_bool BOOLEAN, \
    nullable_double DOUBLE PRECISION, \
    payload BYTEA, \
    scheduled_at TIMESTAMP\
);";

/// One query, selecting every column. Deliberately a single query: the
/// composite type's record/data-class definition is emitted once per query
/// that selects a composite column, and de-duplicating those across queries
/// belongs to the assembly step in `scythe-cli`, not to any backend here.
const QUERY: &str = "-- @name GetWidget\n-- @returns :one\n\
    SELECT id, name, home_address, external_id, status, optional_status, \
    nullable_int, nullable_bigint, nullable_bool, nullable_double, payload, \
    scheduled_at FROM widgets WHERE id = $1;";

/// The nullable columns whose Java/Kotlin type is a JVM primitive, paired with
/// the JDBC getter that returns `0`/`false` -- not null -- for SQL NULL.
const NULLABLE_PRIMITIVES: [(&str, &str); 4] = [
    ("nullable_int", "getInt"),
    ("nullable_bigint", "getLong"),
    ("nullable_bool", "getBoolean"),
    ("nullable_double", "getDouble"),
];

/// A `:grouped` fixture. `:grouped` is the one command whose row construction
/// each backend writes in a *different* place from `:one`/`:many` (a fold over
/// an intermediate buffer), so a reader fix applied to only the simple paths
/// would still leave this one broken.
const GROUPED_SCHEMA: [&str; 2] = [
    "CREATE TYPE widget_status AS ENUM ('active', 'inactive'); \
     CREATE TABLE owners (\
     id SERIAL PRIMARY KEY, \
     name TEXT NOT NULL, \
     external_id UUID NOT NULL, \
     optional_status widget_status, \
     nullable_int INTEGER\
     );",
    "CREATE TABLE parts (\
     id SERIAL PRIMARY KEY, \
     owner_id INT NOT NULL REFERENCES owners (id), \
     part_uuid UUID NOT NULL, \
     part_status widget_status\
     );",
];

const GROUPED_QUERY: &str = "-- @name GetOwnersWithParts\n-- @returns :grouped\n-- @group_by owners.id\n\
    SELECT o.id, o.name, o.external_id, o.optional_status, o.nullable_int, \
    p.id AS part_id, p.part_uuid, p.part_status \
    FROM owners o JOIN parts p ON p.owner_id = o.id;";

fn generate(backend: &dyn CodegenBackend) -> GeneratedCode {
    generate_from(backend, &[SCHEMA], QUERY)
}

fn generate_from(backend: &dyn CodegenBackend, schema: &[&str], query: &str) -> GeneratedCode {
    let catalog = Catalog::from_ddl_with_dialect(schema, &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    generate_with_backend(&analyzed, backend).expect("codegen must succeed")
}

fn backend_for(name: &str) -> Box<dyn CodegenBackend> {
    get_backend(name, "postgresql").expect("backend must support postgresql")
}

/// The whole generated fragment for a backend: enum, composite, row struct and
/// query function concatenated. Which of the four a given backend puts a
/// reader in differs (`java-jdbc` puts it in the row record's
/// `fromResultSet`, `java-r2dbc` in the query function), so the text
/// assertions look at all of it rather than guessing.
fn generated_text(backend_name: &str) -> String {
    concatenate(generate(&*backend_for(backend_name)))
}

/// The same, for the `:grouped` fixture.
fn generated_grouped_text(backend_name: &str) -> String {
    concatenate(generate_from(
        &*backend_for(backend_name),
        &GROUPED_SCHEMA,
        GROUPED_QUERY,
    ))
}

fn concatenate(code: GeneratedCode) -> String {
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

/// Assemble the same bytes `scythe generate` writes, so a compiler is handed a
/// whole file rather than a fragment.
fn generated_file(backend_name: &str) -> String {
    let backend = backend_for(backend_name);
    assemble(&*backend, generate(&*backend))
}

/// The same, for the `:grouped` fixture.
fn generated_grouped_file(backend_name: &str) -> String {
    let backend = backend_for(backend_name);
    let code = generate_from(&*backend, &GROUPED_SCHEMA, GROUPED_QUERY);
    assemble(&*backend, code)
}

fn assemble(backend: &dyn CodegenBackend, code: GeneratedCode) -> String {
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
            backend,
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

// -- #213 / #214: the reader's static type must match the declared type -----

#[test]
fn java_jdbc_reads_composite_and_uuid_columns_through_a_class_literal() {
    let code = generated_text("java-jdbc");
    assert_contains(
        "java-jdbc",
        &code,
        "rs.getObject(\"home_address\", WidgetAddress.class)",
        "a composite column declared `WidgetAddress` cannot be read with the Object-typed getObject",
    );
    assert_contains(
        "java-jdbc",
        &code,
        "rs.getObject(\"external_id\", java.util.UUID.class)",
        "a uuid column declared `java.util.UUID` cannot be read with the Object-typed getObject",
    );
    assert_absent(
        "java-jdbc",
        &code,
        "rs.getObject(\"home_address\")",
        "the untyped overload returns Object, which is assignable to nothing",
    );
    assert_absent(
        "java-jdbc",
        &code,
        "rs.getObject(\"external_id\")",
        "same untyped overload",
    );
}

#[test]
fn kotlin_jdbc_reads_composite_and_uuid_columns_through_a_class_literal() {
    let code = generated_text("kotlin-jdbc");
    assert_contains(
        "kotlin-jdbc",
        &code,
        "rs.getObject(\"home_address\", WidgetAddress::class.java)",
        "a composite column declared `WidgetAddress?` cannot be read with the Any!-typed getObject",
    );
    assert_contains(
        "kotlin-jdbc",
        &code,
        "rs.getObject(\"external_id\", java.util.UUID::class.java)",
        "the confirmed #214 instance: `external_id = rs.getObject(...)` was `Any!`, `UUID` expected",
    );
    assert_absent(
        "kotlin-jdbc",
        &code,
        "rs.getObject(\"external_id\")",
        "the untyped overload returns Any!, which Kotlin refuses to pass as UUID",
    );
    assert_absent(
        "kotlin-jdbc",
        &code,
        "rs.getObject(\"home_address\")",
        "same untyped overload",
    );
}

#[test]
fn kotlin_exposed_reads_composite_and_uuid_columns_through_a_class_literal() {
    let code = generated_text("kotlin-exposed");
    assert_contains(
        "kotlin-exposed",
        &code,
        "rs.getObject(\"home_address\", WidgetAddress::class.java)",
        "Exposed's exec block hands out a plain java.sql.ResultSet, so the same rule applies",
    );
    assert_contains(
        "kotlin-exposed",
        &code,
        "rs.getObject(\"external_id\", java.util.UUID::class.java)",
        "uuid must not fall through to the untyped accessor",
    );
    assert_absent(
        "kotlin-exposed",
        &code,
        "rs.getObject(\"home_address\")",
        "untyped overload",
    );
    assert_absent(
        "kotlin-exposed",
        &code,
        "rs.getObject(\"external_id\")",
        "untyped overload",
    );
    // The temporal columns took the same fallback here, unlike on kotlin-jdbc
    // where a temporal-specific branch already handled them.
    assert_contains(
        "kotlin-exposed",
        &code,
        "rs.getObject(\"scheduled_at\", java.time.LocalDateTime::class.java)",
        "a datetime column declared `LocalDateTime?` was also read with the untyped accessor",
    );
}

#[test]
fn java_r2dbc_row_get_uses_the_declared_class_never_object() {
    let code = generated_text("java-r2dbc");
    assert_contains(
        "java-r2dbc",
        &code,
        "row.get(\"home_address\", WidgetAddress.class)",
        "Row.get is generic in its class argument; only the declared class yields an assignable value",
    );
    assert_contains(
        "java-r2dbc",
        &code,
        "row.get(\"status\", WidgetStatus.class)",
        "enum columns fell through to Object.class too",
    );
    assert_absent(
        "java-r2dbc",
        &code,
        "Object.class",
        "the fallback that made every composite and enum column a compile error",
    );
    assert_contains(
        "java-r2dbc",
        &code,
        "row.get(\"scheduled_at\", java.time.LocalDateTime.class)",
        "`contains(\"LocalDate\")` matched LocalDateTime first, reading a datetime as a date",
    );
}

#[test]
fn kotlin_r2dbc_row_get_uses_the_declared_class_never_any() {
    let code = generated_text("kotlin-r2dbc");
    assert_contains(
        "kotlin-r2dbc",
        &code,
        "row.get(\"home_address\", WidgetAddress::class.java)",
        "only the declared class yields a value assignable to the declared field",
    );
    assert_contains(
        "kotlin-r2dbc",
        &code,
        "row.get(\"status\", WidgetStatus::class.java)",
        "enum columns fell through to Any::class.java too",
    );
    assert_absent(
        "kotlin-r2dbc",
        &code,
        "Any::class.java",
        "the fallback whose Any! result is assignable to nothing",
    );
    assert_contains(
        "kotlin-r2dbc",
        &code,
        "row.get(\"scheduled_at\", java.time.LocalDateTime::class.java)",
        "`contains(\"LocalDate\")` matched LocalDateTime first, reading a datetime as a date",
    );
    // `Int::class.java` is `int.class`, the *primitive* Class object; a driver
    // that hands back boxed values never matches it. The trailing `)` keeps
    // this from matching the correct `Int::class.javaObjectType)`.
    for primitive in ["Int", "Long", "Short", "Boolean", "Byte", "Float", "Double"] {
        assert_absent(
            "kotlin-r2dbc",
            &code,
            &format!("{primitive}::class.java)"),
            "a primitive Class object is not what an R2DBC driver returns; ::class.javaObjectType is",
        );
    }
    assert_contains(
        "kotlin-r2dbc",
        &code,
        "row.get(\"nullable_bool\", Boolean::class.javaObjectType)",
        "boolean was the inconsistent one: it asked for the primitive Class where Int did not",
    );
}

// -- #191: java-r2dbc emitted no enclosing class ---------------------------

#[test]
fn java_r2dbc_wraps_its_output_in_a_single_public_class() {
    let file = generated_file("java-r2dbc");
    assert_contains(
        "java-r2dbc",
        &file,
        "public class Queries {",
        "top-level records plus bare `public static` methods is not a legal Java compilation unit",
    );
    let opens = file.matches("public class Queries {").count();
    assert_eq!(
        opens, 1,
        "java-r2dbc: expected exactly one class wrapper, found {opens};\ngenerated:\n{file}"
    );
    assert!(
        file.trim_end().ends_with('}'),
        "java-r2dbc: the class wrapper must be closed by the file footer;\ngenerated:\n{file}"
    );
}

// -- #192: NULL must survive the reader ------------------------------------

/// `java-jdbc` already paired every nullable primitive with `wasNull()`; this
/// pins that it stays that way, and is the shape `kotlin-exposed` was missing
/// entirely.
#[test]
fn java_jdbc_guards_every_nullable_primitive_with_was_null() {
    let code = generated_text("java-jdbc");
    for (field, getter) in NULLABLE_PRIMITIVES {
        assert_contains(
            "java-jdbc",
            &code,
            &format!("var {field}Raw = rs.{getter}(\"{field}\");"),
            "the raw read must be captured before wasNull() is consulted",
        );
        assert_contains(
            "java-jdbc",
            &code,
            &format!("{field} = rs.wasNull() ? null : {field}Raw;"),
            "without this the SQL NULL becomes 0/false",
        );
    }
}

#[test]
fn kotlin_jdbc_guards_every_nullable_primitive_with_was_null() {
    let code = generated_text("kotlin-jdbc");
    for (field, getter) in NULLABLE_PRIMITIVES {
        assert_contains(
            "kotlin-jdbc",
            &code,
            &format!("val {field}Value = rs.{getter}(\"{field}\")"),
            "the raw read must be captured before wasNull() is consulted",
        );
        assert_contains(
            "kotlin-jdbc",
            &code,
            &format!("val {field} = if (rs.wasNull()) null else {field}Value"),
            "without this the SQL NULL becomes 0/false",
        );
    }
}

/// The `kotlin-exposed` half of #192: this backend read every nullable
/// primitive column inline with `rs.getInt(...)`/`rs.getBoolean(...)` and had
/// no `wasNull()` call anywhere in its output.
#[test]
fn kotlin_exposed_guards_every_nullable_primitive_with_was_null() {
    let code = generated_text("kotlin-exposed");
    for (field, getter) in NULLABLE_PRIMITIVES {
        assert_contains(
            "kotlin-exposed",
            &code,
            &format!("val {field}Value = rs.{getter}(\"{field}\")"),
            "the raw read must be captured before wasNull() is consulted",
        );
        assert_contains(
            "kotlin-exposed",
            &code,
            &format!("val {field} = if (rs.wasNull()) null else {field}Value"),
            "without this the SQL NULL becomes 0/false",
        );
        assert_absent(
            "kotlin-exposed",
            &code,
            &format!("{field} = rs.{getter}(\"{field}\")"),
            "reading a nullable primitive straight into the constructor is the NULL-corrupting form",
        );
    }
}

/// A nullable enum column read as `Status.valueOf(rs.getString(col)...)`
/// throws `NullPointerException` on a NULL, because `getString` returns null
/// and the conversion dereferences it. All three JDBC backends did this.
#[test]
fn jdbc_backends_null_guard_a_nullable_enum_column() {
    let java = generated_text("java-jdbc");
    assert_contains(
        "java-jdbc",
        &java,
        "optional_statusRaw == null ? null : WidgetStatus.valueOf(optional_statusRaw.toUpperCase());",
        "a nullable enum needs the raw string checked before valueOf",
    );
    assert_absent(
        "java-jdbc",
        &java,
        "WidgetStatus.valueOf(rs.getString(\"optional_status\").toUpperCase())",
        "the unguarded inline form throws NullPointerException on a NULL column",
    );
    // The NOT NULL enum keeps the direct conversion -- the guard is scoped to
    // nullable columns, not applied to everything.
    assert_contains(
        "java-jdbc",
        &java,
        "WidgetStatus.valueOf(rs.getString(\"status\").toUpperCase())",
        "a NOT NULL enum column needs no guard and must not grow one",
    );

    for backend in ["kotlin-jdbc", "kotlin-exposed"] {
        let code = generated_text(backend);
        assert_contains(
            backend,
            &code,
            "if (optional_statusValue == null) null else WidgetStatus.valueOf(optional_statusValue.uppercase())",
            "a nullable enum needs the raw string checked before valueOf",
        );
        assert_absent(
            backend,
            &code,
            "WidgetStatus.valueOf(rs.getString(\"optional_status\").uppercase())",
            "the unguarded inline form throws on a NULL column",
        );
        assert_contains(
            backend,
            &code,
            "WidgetStatus.valueOf(rs.getString(\"status\").uppercase())",
            "a NOT NULL enum column needs no guard and must not grow one",
        );
    }
}

// -- the compilers, additively ---------------------------------------------

/// Compile the assembled file with the backend's real tool.
///
/// Additive to the assertions above, never a substitute: a missing tool is a
/// skip outside strict mode (and a failure inside it, via `into_result`), so
/// the string checks stay the floor. `fully_checked` is asserted in strict
/// mode for the same reason `tests/tool_validation.rs` asserts it -- a
/// validator that dispatched to zero checkers would otherwise pass here
/// silently.
fn assert_compiles(backend_name: &str) {
    assert_file_compiles(backend_name, generated_file(backend_name));
}

fn assert_grouped_compiles(backend_name: &str) {
    assert_file_compiles(backend_name, generated_grouped_file(backend_name));
}

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
fn java_jdbc_composite_uuid_and_nullable_file_compiles() {
    assert_compiles("java-jdbc");
}

#[test]
fn java_r2dbc_composite_uuid_and_nullable_file_compiles() {
    assert_compiles("java-r2dbc");
}

#[test]
fn kotlin_jdbc_composite_uuid_and_nullable_file_compiles() {
    assert_compiles("kotlin-jdbc");
}

// -- :grouped, whose row construction lives on a separate code path ---------

/// `java-r2dbc`'s `:grouped` path buffered each row into a `new Object[]{...}`
/// and closed the expression with a bare `});`, leaving the enclosing
/// `.flatMap(` open: `error: ')' or ',' expected`. That is a plain syntax
/// defect, unrelated to any reader, and it made *every* `:grouped` query this
/// backend emitted unbuildable.
#[test]
fn java_r2dbc_grouped_closes_its_row_buffer_lambda() {
    let code = generated_grouped_text("java-r2dbc");
    assert_contains(
        "java-r2dbc",
        &code,
        "}));",
        "three closers are needed: the array initializer, `result.map(`, and `.flatMap(`",
    );
    assert_absent(
        "java-r2dbc",
        &code,
        "java.util.UUID.class),\n                }",
        "a trailing comma before the array initializer's close is not valid here either",
    );
}

/// The `:grouped` fold reads every column a second time, on a code path none
/// of the `:one`/`:many` assertions above touch.
#[test]
fn grouped_row_folds_use_typed_readers_on_every_jvm_backend() {
    for (backend, uuid_read) in [
        ("java-jdbc", "rs.getObject(\"external_id\", java.util.UUID.class)"),
        (
            "kotlin-jdbc",
            "rs.getObject(\"external_id\", java.util.UUID::class.java)",
        ),
        (
            "kotlin-exposed",
            "rs.getObject(\"external_id\", java.util.UUID::class.java)",
        ),
        ("java-r2dbc", "row.get(\"external_id\", java.util.UUID.class)"),
        ("kotlin-r2dbc", "row.get(\"external_id\", java.util.UUID::class.java)"),
    ] {
        let code = generated_grouped_text(backend);
        assert_contains(backend, &code, uuid_read, "the grouped fold reads columns too");
        assert_absent(
            backend,
            &code,
            "rs.getObject(\"external_id\")",
            "the untyped accessor must not survive on the grouped path",
        );
    }
}

/// The `kotlin-exposed` grouped fold had the same missing `wasNull()` as its
/// `:one`/`:many` paths, on both the parent and the child row.
#[test]
fn kotlin_exposed_grouped_fold_guards_nullable_columns() {
    let code = generated_grouped_text("kotlin-exposed");
    assert_contains(
        "kotlin-exposed",
        &code,
        "val nullable_intValue = rs.getInt(\"nullable_int\")",
        "the parent row's nullable primitive needs the same wasNull() pairing",
    );
    assert_contains(
        "kotlin-exposed",
        &code,
        "val nullable_int = if (rs.wasNull()) null else nullable_intValue",
        "without this the SQL NULL becomes 0",
    );
    assert_contains(
        "kotlin-exposed",
        &code,
        "if (part_statusValue == null) null else WidgetStatus.valueOf(part_statusValue.uppercase())",
        "the child row's nullable enum needs the null guard too",
    );
}

#[test]
fn java_jdbc_grouped_file_compiles() {
    assert_grouped_compiles("java-jdbc");
}

#[test]
fn java_r2dbc_grouped_file_compiles() {
    assert_grouped_compiles("java-r2dbc");
}

#[test]
fn kotlin_jdbc_grouped_file_compiles() {
    assert_grouped_compiles("kotlin-jdbc");
}
