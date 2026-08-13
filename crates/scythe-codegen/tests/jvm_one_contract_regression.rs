//! Regression tests for the JVM family half of the `:one`/`:opt` contract bug
//! this repo tracks as #192: `:one` means "exactly one row, error if absent";
//! `:opt` means "zero or one row, null/None/empty if absent". Before this
//! fix, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, and
//! `kotlin-exposed` all matched `QueryCommand::One | QueryCommand::Opt` in a
//! single shared arm and rendered byte-identical code for both -- `:opt`'s
//! own permissive shape was correct, but `:one` silently inherited it and
//! never errored on a missing row. `crates/scythe-codegen/tests/
//! opt_command_regression.rs` (owned by another agent) tracks this at the
//! census level via `KNOWN_UNDIFFERENTIATED_BACKENDS`; this file asserts the
//! actual fixed shape for the five JVM backends directly, mirroring that
//! file's `query_fn_for`/`one_column_query` helpers.
//!
//! The chosen idiom differs by backend family:
//! - `java-jdbc`/`kotlin-jdbc`/`kotlin-exposed` are blocking JDBC callers:
//!   `:one` drops the nullable return shape and throws
//!   `NoSuchElementException` (`java.util.NoSuchElementException` on the Java
//!   side; `kotlin.NoSuchElementException`, in Kotlin's default imports, on
//!   the Kotlin side) when the query returns no rows.
//! - `java-r2dbc`/`kotlin-r2dbc` are reactive: a missing row is not a thrown
//!   exception at call time, it is an error signal on the publisher. `:one`
//!   chains `.switchIfEmpty(Mono.error(...))` onto the row-producing `Mono`
//!   instead of throwing synchronously; `kotlin-r2dbc` additionally swaps its
//!   terminal `awaitFirstOrNull()` for `awaitFirst()` so the now-non-nullable
//!   `suspend fun` return type is honest about never receiving `null`.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery};
use scythe_core::parser::QueryCommand;

/// A minimal single-column, param-free, non-`RETURNING` query -- mirrors
/// `opt_command_regression.rs`'s fixture of the same name so both files
/// exercise the exact same `QueryCommand::One`/`QueryCommand::Opt` branch
/// shape.
fn one_column_query(command: QueryCommand) -> AnalyzedQuery {
    AnalyzedQuery::build(|query| {
        query.name = "GetItem".to_string();
        query.command = command;
        query.sql = "SELECT value FROM t WHERE id = 1".to_string();
        query.columns = vec![AnalyzedColumn {
            name: "value".to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            sql_type: "string".to_string(),
            ..Default::default()
        }];
    })
}

fn query_fn_for(backend_name: &str, command: QueryCommand) -> String {
    let backend = get_backend(backend_name, "postgresql")
        .unwrap_or_else(|error| panic!("{backend_name}: backend must support postgresql: {error}"));
    let query = one_column_query(command.clone());
    let generated = generate_with_backend(&query, &*backend)
        .unwrap_or_else(|error| panic!("{backend_name}: codegen failed for {command:?}: {error}"));
    generated
        .query_fn
        .unwrap_or_else(|| panic!("{backend_name}: {command:?} produced no query fn"))
}

fn assert_contains(backend: &str, command: QueryCommand, code: &str, needle: &str) {
    assert!(
        code.contains(needle),
        "{backend} {command:?}: expected `{needle}`;\ngenerated:\n{code}"
    );
}

fn assert_absent(backend: &str, command: QueryCommand, code: &str, needle: &str) {
    assert!(
        !code.contains(needle),
        "{backend} {command:?}: `{needle}` must not appear;\ngenerated:\n{code}"
    );
}

// ---------------------------------------------------------------------
// java-jdbc / kotlin-jdbc / kotlin-exposed: blocking, throw NoSuchElementException
// ---------------------------------------------------------------------

#[test]
fn java_jdbc_one_returns_the_bare_row_type_and_throws_on_a_missing_row() {
    let code = query_fn_for("java-jdbc", QueryCommand::One);
    assert_contains(
        "java-jdbc",
        QueryCommand::One,
        &code,
        "public static GetItemRow getItem(Connection conn) throws SQLException {",
    );
    assert_absent("java-jdbc", QueryCommand::One, &code, "@Nullable GetItemRow getItem");
    assert_contains(
        "java-jdbc",
        QueryCommand::One,
        &code,
        "throw new java.util.NoSuchElementException(\"getItem: no rows returned\");",
    );
    assert_absent("java-jdbc", QueryCommand::One, &code, "return null;");
}

#[test]
fn java_jdbc_opt_keeps_the_nullable_shape_and_never_throws() {
    let code = query_fn_for("java-jdbc", QueryCommand::Opt);
    assert_contains(
        "java-jdbc",
        QueryCommand::Opt,
        &code,
        "public static @Nullable GetItemRow getItem(Connection conn) throws SQLException {",
    );
    assert_contains("java-jdbc", QueryCommand::Opt, &code, "return null;");
    assert_absent("java-jdbc", QueryCommand::Opt, &code, "NoSuchElementException");
}

#[test]
fn kotlin_jdbc_one_returns_the_bare_row_type_and_throws_on_a_missing_row() {
    let code = query_fn_for("kotlin-jdbc", QueryCommand::One);
    assert_contains(
        "kotlin-jdbc",
        QueryCommand::One,
        &code,
        "fun getItem(conn: Connection): GetItemRow {",
    );
    assert_absent("kotlin-jdbc", QueryCommand::One, &code, ": GetItemRow?");
    assert_contains(
        "kotlin-jdbc",
        QueryCommand::One,
        &code,
        "throw NoSuchElementException(\"getItem: no rows returned\")",
    );
    assert_absent("kotlin-jdbc", QueryCommand::One, &code, "                null\n");
}

#[test]
fn kotlin_jdbc_opt_keeps_the_nullable_shape_and_never_throws() {
    let code = query_fn_for("kotlin-jdbc", QueryCommand::Opt);
    assert_contains(
        "kotlin-jdbc",
        QueryCommand::Opt,
        &code,
        "fun getItem(conn: Connection): GetItemRow? {",
    );
    assert_contains("kotlin-jdbc", QueryCommand::Opt, &code, "                null\n");
    assert_absent("kotlin-jdbc", QueryCommand::Opt, &code, "NoSuchElementException");
}

#[test]
fn kotlin_exposed_one_returns_the_bare_row_type_and_throws_on_a_missing_row() {
    let code = query_fn_for("kotlin-exposed", QueryCommand::One);
    assert_contains(
        "kotlin-exposed",
        QueryCommand::One,
        &code,
        "fun getItem(): GetItemRow =",
    );
    assert_absent("kotlin-exposed", QueryCommand::One, &code, ": GetItemRow? =");
    // The elvis fallback sits on the *outer* `exec(...)` call, not inside the
    // `rs.next()` if/else -- `Transaction.exec(sql) { rs -> T }` returns `T?`
    // regardless of what the lambda's branches do, so a throw nested inside
    // the lambda would not change the static nullability of the call it's
    // nested in and `: GetItemRow` (no `?`) would not type-check against it.
    // The if/else's own `null` branch is therefore identical between :one
    // and :opt; only this trailing fallback differs.
    assert_contains(
        "kotlin-exposed",
        QueryCommand::One,
        &code,
        "} ?: throw NoSuchElementException(\"getItem: no rows returned\")",
    );
}

#[test]
fn kotlin_exposed_opt_keeps_the_nullable_shape_and_never_throws() {
    let code = query_fn_for("kotlin-exposed", QueryCommand::Opt);
    assert_contains(
        "kotlin-exposed",
        QueryCommand::Opt,
        &code,
        "fun getItem(): GetItemRow? =",
    );
    assert_absent("kotlin-exposed", QueryCommand::Opt, &code, "NoSuchElementException");
    assert_absent("kotlin-exposed", QueryCommand::Opt, &code, "?: throw");
}

// ---------------------------------------------------------------------
// java-r2dbc / kotlin-r2dbc: reactive, error the publisher instead of throwing
// ---------------------------------------------------------------------

#[test]
fn java_r2dbc_one_errors_the_publisher_on_a_missing_row() {
    let code = query_fn_for("java-r2dbc", QueryCommand::One);
    assert_contains(
        "java-r2dbc",
        QueryCommand::One,
        &code,
        "public static Mono<GetItemRow> getItem(ConnectionFactory cf) {",
    );
    assert_contains(
        "java-r2dbc",
        QueryCommand::One,
        &code,
        ".switchIfEmpty(Mono.error(new java.util.NoSuchElementException(\"getItem: no rows returned\")));",
    );
}

#[test]
fn java_r2dbc_opt_never_errors_the_publisher_on_a_missing_row() {
    let code = query_fn_for("java-r2dbc", QueryCommand::Opt);
    assert_contains(
        "java-r2dbc",
        QueryCommand::Opt,
        &code,
        "public static Mono<GetItemRow> getItem(ConnectionFactory cf) {",
    );
    assert_absent("java-r2dbc", QueryCommand::Opt, &code, "switchIfEmpty");
    assert_absent("java-r2dbc", QueryCommand::Opt, &code, "NoSuchElementException");
}

#[test]
fn kotlin_r2dbc_one_returns_the_bare_row_type_and_errors_the_publisher() {
    let code = query_fn_for("kotlin-r2dbc", QueryCommand::One);
    assert_contains(
        "kotlin-r2dbc",
        QueryCommand::One,
        &code,
        "suspend fun getItem(cf: ConnectionFactory): GetItemRow {",
    );
    assert_absent("kotlin-r2dbc", QueryCommand::One, &code, ": GetItemRow? {");
    assert_contains(
        "kotlin-r2dbc",
        QueryCommand::One,
        &code,
        ".switchIfEmpty(Mono.error(java.util.NoSuchElementException(\"getItem: no rows returned\")))",
    );
    // The row-collector's own terminal call, distinct from the unrelated
    // `Mono.from(cf.create()).awaitFirst()` connection-acquire line every
    // branch (One and Opt alike) already has -- a bare `.awaitFirst()`/
    // `awaitFirstOrNull()` substring check would match that line regardless
    // of which path this test is exercising.
    assert_contains("kotlin-r2dbc", QueryCommand::One, &code, "            .awaitFirst()");
    assert_absent("kotlin-r2dbc", QueryCommand::One, &code, "}.awaitFirstOrNull()");
}

#[test]
fn kotlin_r2dbc_opt_keeps_the_nullable_shape_and_never_errors_the_publisher() {
    let code = query_fn_for("kotlin-r2dbc", QueryCommand::Opt);
    assert_contains(
        "kotlin-r2dbc",
        QueryCommand::Opt,
        &code,
        "suspend fun getItem(cf: ConnectionFactory): GetItemRow? {",
    );
    // The row-collector's own terminal call -- see the :one test's comment on
    // why this is not a bare `awaitFirstOrNull()` substring check.
    assert_contains("kotlin-r2dbc", QueryCommand::Opt, &code, "}.awaitFirstOrNull()");
    assert_absent("kotlin-r2dbc", QueryCommand::Opt, &code, "switchIfEmpty");
    assert_absent("kotlin-r2dbc", QueryCommand::Opt, &code, "NoSuchElementException");
    assert_absent("kotlin-r2dbc", QueryCommand::Opt, &code, "            .awaitFirst()");
}

// ---------------------------------------------------------------------
// Cross-backend: :one and :opt must never render identical code
// ---------------------------------------------------------------------

#[test]
fn every_jvm_backend_renders_different_code_for_one_and_opt() {
    for backend in [
        "java-jdbc",
        "java-r2dbc",
        "kotlin-jdbc",
        "kotlin-r2dbc",
        "kotlin-exposed",
    ] {
        let one_fn = query_fn_for(backend, QueryCommand::One);
        let opt_fn = query_fn_for(backend, QueryCommand::Opt);
        assert_ne!(
            one_fn, opt_fn,
            "{backend}: :one and :opt must render different code -- identical code means :one \
             silently inherited :opt's permissiveness and never errors on a missing row"
        );
    }
}
