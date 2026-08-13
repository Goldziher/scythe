//! Regression tests for array-typed columns on the JVM backends.
//!
//! #192 was closed once already, but the fix was a degradation: the JVM
//! manifests were changed to declare every `array<T>` column as plain `String`
//! (see the removed comment this file used to quote: "arrays are declared in
//! their text form ... exactly like the range container"), and this file's
//! assertions were rewritten to pin that string-typed spelling as if it were
//! correct. It was not -- an array column's *value* is not a string, and no
//! caller wanted `"{a,b,c}"` back. The degradation made a compile failure go
//! away without giving callers a usable value.
//!
//! The real fix restores a typed `List<T>` declaration and a matching reader:
//!
//! - **JDBC** (`java-jdbc`, `kotlin-jdbc`, `kotlin-exposed`, which shares the
//!   JDBC `ResultSet`): `rs.getArray(col)` returns a `java.sql.Array`; its
//!   `.getArray()` (Java) / `.array` (Kotlin) is cast to a reference-type
//!   array (`Object[]`/`Array<*>` -- JDBC never returns a primitive array for
//!   an SQL array column) and each element is cast to the declared element
//!   type. A NULL column's `getArray()` returns `null`, so nullable array
//!   columns get the same guarded preamble a nullable enum column already
//!   has.
//! - **R2DBC** (`java-r2dbc`, `kotlin-r2dbc`): the driver hands back its own
//!   native array shape (`String[]`/`Array<String>`, not a `java.sql.Array`),
//!   read via `row.get(col, T[].class)` / `row.get(col, Array<T>::class.java)`
//!   and converted to a `List<T>`.
//!
//! Java alone needs a boxing step the manifest's `{T}` substitution cannot
//! provide: `[types.scalars]` deliberately spells `int32` as the unboxed
//! `int` (right for a plain field), and `List<int>` is not legal Java. The
//! Java backends re-resolve the element type and box it themselves (see
//! `java_array_element_type` in `java_jdbc.rs`/`java_r2dbc.rs`) rather than
//! trusting `col.lang_type`/`col.full_type`. Kotlin needs no such step:
//! `List<Int>`/`List<Boolean>` are legal Kotlin on their own, so the manifest
//! change alone is correct for `kotlin-jdbc`/`kotlin-r2dbc`/`kotlin-exposed`.

use scythe_codegen::{CodegenBackend, generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

/// Every neutral array spelling reachable from PostgreSQL DDL that used to
/// render as `List<...>` and get degraded to `String`: a JSON element type, a
/// string one, a boxed-in-name-only primitive one (`int32` -> unboxed `int`
/// unless the backend boxes it), and a nullable one.
const SCHEMA: &str = "CREATE TABLE events (\
    id INTEGER PRIMARY KEY, \
    payload JSON[] NOT NULL, \
    payload_opt JSONB[], \
    tags TEXT[] NOT NULL, \
    counts INTEGER[] NOT NULL, \
    flags BOOLEAN[] NOT NULL\
);";

const QUERY: &str = "-- @name GetEvent\n-- @returns :one\n\
    SELECT id, payload, payload_opt, tags, counts, flags FROM events WHERE id = $1;";

fn generate_row_struct(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    generate_row_struct_with(&*backend)
}

fn generate_row_struct_with(backend: &dyn CodegenBackend) -> String {
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, backend).expect("codegen must succeed");
    code.row_struct.expect("expected a row struct")
}

fn generate_query_fn(backend_name: &str) -> String {
    let backend = get_backend(backend_name, "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&[SCHEMA], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.query_fn.expect("expected a query fn")
}

// -- java-jdbc ---------------------------------------------------------------

#[test]
fn java_jdbc_declares_array_columns_as_boxed_lists() {
    let row_struct = generate_row_struct("java-jdbc");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Integer"),
        ("flags", "Boolean"),
    ] {
        let declared = format!("List<{elem}> {field}");
        assert!(
            row_struct.contains(&declared),
            "java-jdbc: expected `{field}` declared as `{declared}`; got:\n{row_struct}"
        );
    }
    assert!(
        row_struct.contains("List<String> payload_opt"),
        "java-jdbc: nullable array column must still declare List<String>, not String; got:\n{row_struct}"
    );
}

#[test]
fn java_jdbc_reads_non_nullable_array_columns_through_getarray() {
    let row_struct = generate_row_struct("java-jdbc");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Integer"),
        ("flags", "Boolean"),
    ] {
        let expr = format!(
            "java.util.Arrays.stream((Object[]) rs.getArray(\"{field}\").getArray()).map(v -> ({elem}) v).collect(java.util.stream.Collectors.toList())"
        );
        assert!(
            row_struct.contains(&expr),
            "java-jdbc: expected `{field}` read with `{expr}`; got:\n{row_struct}"
        );
    }
}

#[test]
fn java_jdbc_null_guards_a_nullable_array_column() {
    let row_struct = generate_row_struct("java-jdbc");
    assert!(
        row_struct.contains("var payload_optSqlArray = rs.getArray(\"payload_opt\");"),
        "java-jdbc: nullable array must extract the java.sql.Array into a local first; got:\n{row_struct}"
    );
    assert!(
        row_struct.contains(
            "List<String> payload_opt = payload_optSqlArray == null ? null : java.util.Arrays.stream((Object[]) payload_optSqlArray.getArray()).map(v -> (String) v).collect(java.util.stream.Collectors.toList());"
        ),
        "java-jdbc: nullable array must null-check before calling .getArray(); got:\n{row_struct}"
    );
}

// -- java-r2dbc ---------------------------------------------------------------

#[test]
fn java_r2dbc_declares_array_columns_as_boxed_lists() {
    let row_struct = generate_row_struct("java-r2dbc");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Integer"),
        ("flags", "Boolean"),
    ] {
        let declared = format!("List<{elem}> {field}");
        assert!(
            row_struct.contains(&declared),
            "java-r2dbc: expected `{field}` declared as `{declared}`; got:\n{row_struct}"
        );
    }
}

#[test]
fn java_r2dbc_query_fn_reads_array_columns_through_the_native_array_shape() {
    let query_fn = generate_query_fn("java-r2dbc");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Integer"),
        ("flags", "Boolean"),
    ] {
        let get_expr = format!("row.get(\"{field}\", {elem}[].class)");
        let full_expr = format!("{get_expr} == null ? null : java.util.Arrays.asList({get_expr})");
        assert!(
            query_fn.contains(&full_expr),
            "java-r2dbc: expected `{field}` read with `{full_expr}`; got:\n{query_fn}"
        );
    }
}

// -- kotlin-jdbc ---------------------------------------------------------------

#[test]
fn kotlin_jdbc_declares_array_columns_as_lists() {
    let row_struct = generate_row_struct("kotlin-jdbc");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Int"),
        ("flags", "Boolean"),
    ] {
        let declared = format!("val {field}: List<{elem}>,");
        assert!(
            row_struct.contains(&declared),
            "kotlin-jdbc: expected `{field}` declared as `{declared}`; got:\n{row_struct}"
        );
    }
    assert!(
        row_struct.contains("val payload_opt: List<String>?,"),
        "kotlin-jdbc: nullable array column must declare List<String>?, not String?; got:\n{row_struct}"
    );
}

#[test]
fn kotlin_jdbc_reads_non_nullable_array_columns_through_getarray() {
    let query_fn = generate_query_fn("kotlin-jdbc");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Int"),
        ("flags", "Boolean"),
    ] {
        let expr = format!("(rs.getArray(\"{field}\").array as Array<*>).map {{ it as {elem} }}");
        assert!(
            query_fn.contains(&expr),
            "kotlin-jdbc: expected `{field}` read with `{expr}`; got:\n{query_fn}"
        );
    }
}

#[test]
fn kotlin_jdbc_null_guards_a_nullable_array_column() {
    let query_fn = generate_query_fn("kotlin-jdbc");
    assert!(
        query_fn.contains("val payload_optSqlArray = rs.getArray(\"payload_opt\")"),
        "kotlin-jdbc: nullable array must extract the java.sql.Array into a local first; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains(
            "val payload_opt = if (payload_optSqlArray == null) null else (payload_optSqlArray.array as Array<*>).map { it as String }"
        ),
        "kotlin-jdbc: nullable array must null-check before calling .array; got:\n{query_fn}"
    );
}

// -- kotlin-exposed ------------------------------------------------------------

#[test]
fn kotlin_exposed_declares_array_columns_as_lists() {
    let row_struct = generate_row_struct("kotlin-exposed");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Int"),
        ("flags", "Boolean"),
    ] {
        let declared = format!("val {field}: List<{elem}>,");
        assert!(
            row_struct.contains(&declared),
            "kotlin-exposed: expected `{field}` declared as `{declared}`; got:\n{row_struct}"
        );
    }
}

#[test]
fn kotlin_exposed_reads_non_nullable_array_columns_through_getarray() {
    let query_fn = generate_query_fn("kotlin-exposed");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Int"),
        ("flags", "Boolean"),
    ] {
        let expr = format!("(rs.getArray(\"{field}\").array as Array<*>).map {{ it as {elem} }}");
        assert!(
            query_fn.contains(&expr),
            "kotlin-exposed: expected `{field}` read with `{expr}`; got:\n{query_fn}"
        );
    }
}

// -- kotlin-r2dbc ---------------------------------------------------------------

#[test]
fn kotlin_r2dbc_declares_array_columns_as_lists() {
    let row_struct = generate_row_struct("kotlin-r2dbc");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Int"),
        ("flags", "Boolean"),
    ] {
        let declared = format!("val {field}: List<{elem}>,");
        assert!(
            row_struct.contains(&declared),
            "kotlin-r2dbc: expected `{field}` declared as `{declared}`; got:\n{row_struct}"
        );
    }
}

#[test]
fn kotlin_r2dbc_query_fn_reads_array_columns_through_the_native_array_shape() {
    let query_fn = generate_query_fn("kotlin-r2dbc");
    for (field, elem) in [
        ("payload", "String"),
        ("tags", "String"),
        ("counts", "Int"),
        ("flags", "Boolean"),
    ] {
        let expr = format!("row.get(\"{field}\", Array<{elem}>::class.java).toList()");
        assert!(
            query_fn.contains(&expr),
            "kotlin-r2dbc: expected `{field}` read with `{expr}`; got:\n{query_fn}"
        );
    }
}

// -- shared: never regress to the untyped fallback ----------------------------

/// The JDBC-family readers (`java-jdbc`, `kotlin-jdbc`, `kotlin-exposed`) must
/// never fall back to a bare, class-less `getObject`/`getString` for an array
/// column -- that untyped accessor (or the plain-text degradation) is exactly
/// what this file exists to keep out.
#[test]
fn jdbc_backends_never_read_array_columns_as_plain_strings() {
    for backend_name in ["java-jdbc", "kotlin-jdbc", "kotlin-exposed"] {
        let row_struct = generate_row_struct(backend_name);
        for field in ["payload", "payload_opt", "tags", "counts", "flags"] {
            let get_object = format!("getObject(\"{field}\")");
            assert!(
                !row_struct.contains(&get_object),
                "{backend_name}: array column `{field}` is still read with `{get_object}`, whose \
                 static type is Object/Any; got:\n{row_struct}"
            );
            let get_string = format!("getString(\"{field}\")");
            assert!(
                !row_struct.contains(&get_string),
                "{backend_name}: array column `{field}` must not be read as plain text; got:\n{row_struct}"
            );
        }
    }
}

// -- shared: keep at least one typed-array backend in the set ----------------

/// The set this file exists to keep honest: every JVM backend with a real
/// array *column* (`SCHEMA`'s `tags TEXT[] NOT NULL`), independent of which
/// individual assertions above happen to cover it.
const JVM_ARRAY_BACKENDS: &[&str] = &[
    "java-jdbc",
    "java-r2dbc",
    "kotlin-jdbc",
    "kotlin-exposed",
    "kotlin-r2dbc",
];

/// One entry in [`KNOWN_DEGRADED_ARRAY_BACKENDS`]: a backend name paired with
/// why its array columns currently render as an untyped scalar rather than a
/// `List<T>`, so the list doubles as its own documentation -- same shape as
/// `scythe_codegen::lib::tests::KNOWN_DIVERGENT_BACKENDS`.
struct DegradedArrayBackend {
    backend: &'static str,
    reason: &'static str,
}

/// Ratcheting allowlist, same discipline as
/// `scythe_codegen::lib::tests::KNOWN_DIVERGENT_BACKENDS`: a backend not
/// listed here whose `tags` column renders as a bare scalar (`String`, not
/// `List<String>`) is a regression -- and not a hypothetical one. It is
/// exactly the defect #192 was closed on once already: the JVM manifests
/// were changed to declare every array column `String`, and the assertions
/// in this file were rewritten to pin that degraded spelling as if it were
/// correct (see the file-level comment). A backend listed here whose `tags`
/// column has started to render `List<T>` is a stale entry: delete the line.
///
/// It is empty today. [`jvm_array_backend_set_keeps_at_least_one_typed_member`]
/// below is what turns "empty" into an enforced invariant instead of an
/// incidental one: entries may be added here as individual backends turn out
/// to have a real reason they cannot express a typed array, but the list can
/// never grow to cover all five, because the day it does, this file's
/// per-backend assertions have nothing left to check but agreement between
/// backends that all discarded the same information.
const KNOWN_DEGRADED_ARRAY_BACKENDS: &[DegradedArrayBackend] = &[];

/// `true` when `backend_name`'s row struct declares the non-nullable
/// `TEXT[]` column `tags` as a typed `List<String>`; `false` when it
/// declares a bare `String` instead. Panics on any other shape -- a third
/// spelling means this classifier itself has gone stale and must not
/// silently miscount either direction.
fn declares_tags_as_typed_list(backend_name: &str, row_struct: &str) -> bool {
    let java_typed = row_struct.contains("List<String> tags");
    let java_degraded = row_struct.contains("String tags");
    let kotlin_typed = row_struct.contains("val tags: List<String>,");
    let kotlin_degraded = row_struct.contains("val tags: String,");

    match (java_typed, java_degraded, kotlin_typed, kotlin_degraded) {
        (true, false, false, false) => true,
        (false, false, true, false) => true,
        (false, true, false, false) => false,
        (false, false, false, true) => false,
        _ => panic!(
            "{backend_name}: the `tags` column matched neither exactly one known typed spelling \
             nor exactly one known degraded spelling -- this classifier has gone stale and must \
             be taught the new shape before it can be trusted again; got:\n{row_struct}"
        ),
    }
}

/// Guard for #164: a test whose whole backend set has degraded still passes
/// if it only ever compares those backends to each other, because they all
/// agree. This makes that failure mode impossible to reach silently for
/// [`JVM_ARRAY_BACKENDS`] -- every backend not in
/// [`KNOWN_DEGRADED_ARRAY_BACKENDS`] must still render a typed `List<T>`,
/// every backend in that allowlist must actually be degraded, and the set as
/// a whole can never end up with zero typed members even if every backend
/// were (wrongly) added to the allowlist at once.
#[test]
fn jvm_array_backend_set_keeps_at_least_one_typed_member() {
    let mut regressions = Vec::new();
    let mut stale_entries = Vec::new();
    let mut typed_backends = Vec::new();

    for &backend_name in JVM_ARRAY_BACKENDS {
        let row_struct = generate_row_struct(backend_name);
        let typed = declares_tags_as_typed_list(backend_name, &row_struct);
        let listed = KNOWN_DEGRADED_ARRAY_BACKENDS
            .iter()
            .find(|note| note.backend == backend_name);

        match (typed, listed) {
            (false, None) => regressions.push(backend_name),
            (true, Some(_)) => stale_entries.push(backend_name),
            (true, None) => typed_backends.push(backend_name),
            // Confirmed still degraded for the reason on file -- visible
            // with `--nocapture`.
            (false, Some(note)) => eprintln!("expected degradation, {backend_name}: {}", note.reason),
        }
    }

    let mut failures = Vec::new();
    for name in &regressions {
        failures.push(format!(
            "REGRESSION: {name}: `tags` renders as a bare scalar and {name} is not in \
             KNOWN_DEGRADED_ARRAY_BACKENDS -- an array column's value is not a string; this is \
             the exact defect #192 was closed on once already by pinning the degraded spelling \
             as correct instead of fixing it"
        ));
    }
    for name in &stale_entries {
        failures.push(format!(
            "STALE ALLOWLIST: {name}: listed in KNOWN_DEGRADED_ARRAY_BACKENDS but `tags` now \
             renders a typed `List<T>` -- delete its entry"
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    assert!(
        !typed_backends.is_empty(),
        "every backend in JVM_ARRAY_BACKENDS is listed in KNOWN_DEGRADED_ARRAY_BACKENDS -- this \
         file's per-backend assertions would then only ever confirm that degraded backends agree \
         with each other about opaque text, exactly the vacuous equivalence #164 exists to catch; \
         keep at least one entry off the allowlist"
    );
}
