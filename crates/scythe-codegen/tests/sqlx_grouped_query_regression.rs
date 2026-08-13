//! Board #191: `generate_grouped_query_fn` (`sqlx.rs`) reads `row.field_name`
//! off the row type the untyped `sqlx::query!` macro synthesizes -- not off
//! a `#[derive(sqlx::FromRow)]` struct like `sqlx_from_row_rename_regression`
//! covers. That anonymous row's field names come from whatever the *driver*
//! reports as each column's name, not from this backend's `field_name`
//! convention: sqlx-macros-core's `output::parse_ident` (sqlx-macros-core
//! 0.9.0, `src/query/output.rs`) requires that raw name to already be a
//! valid Rust identifier and hard-errors the whole macro expansion
//! otherwise. A quoted, non-identifier column like `"my col"` therefore
//! needs an explicit `AS "my_col"` alias so the name sqlx assigns is the one
//! `row.my_col` reads -- the `#[sqlx(rename = "...")]` fix for the FromRow
//! path does not apply here, because there is no derive to attach it to.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: [&str; 2] = [
    "CREATE TABLE owners (id SERIAL PRIMARY KEY, name TEXT NOT NULL);",
    "CREATE TABLE parts (id SERIAL PRIMARY KEY, owner_id INT NOT NULL REFERENCES owners (id), \"my col\" TEXT);",
];

const QUERY: &str = "-- @name GetOwnersWithParts\n-- @returns :grouped\n-- @group_by owners.id\n\
    SELECT o.id, o.name, p.id AS part_id, p.\"my col\" \
    FROM owners o JOIN parts p ON p.owner_id = o.id;";

fn generate_grouped_query_fn() -> String {
    let backend = get_backend("rust-sqlx", "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&SCHEMA, &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.query_fn.expect("expected a grouped query function")
}

/// The regression itself: the SQL text handed to `sqlx::query!` must alias
/// the non-identifier child column to the same name the generated Rust code
/// reads it back by.
///
/// Before this fix, the SQL embedded `p."my col"` verbatim (no alias) --
/// visible in the generated source, once escaped for the Rust string
/// literal, as `p.\"my col\" FROM owners o JOIN parts p ...` with no `AS`
/// clause anywhere near it -- while the row-construction code below still
/// read `row.my_col`. Because `sqlx::query!` has no explicit record type,
/// the anonymous row's field comes from the raw column name Postgres
/// reports for that expression, which is `my col`, not `my_col`; sqlx's
/// `parse_ident` rejects `my col` outright (it is not a valid Rust
/// identifier), so the macro fails to compile before `row.my_col` is ever
/// type-checked against it.
#[test]
fn rust_sqlx_grouped_query_aliases_a_mangled_child_column() {
    let query_fn = generate_grouped_query_fn();
    assert!(
        query_fn.contains("p.\\\"my col\\\" AS \\\"my_col\\\""),
        "expected the SQL to alias the non-identifier column to the field sqlx's \
         query! macro must report -- `p.\"my col\" AS \"my_col\"` -- but got:\n{query_fn}"
    );
}

/// The row-construction code must read the aliased field back by its
/// sanitized name -- unchanged by this fix, but pinning it here documents
/// that the alias and the read agree on the same spelling.
#[test]
fn rust_sqlx_grouped_query_reads_the_aliased_field_by_its_sanitized_name() {
    let query_fn = generate_grouped_query_fn();
    assert!(
        query_fn.contains("my_col: row.my_col,"),
        "expected the child struct literal to read `row.my_col`:\n{query_fn}"
    );
}

/// A column whose generated field name already matches its SQL name (the
/// group key `o.id`, or the query's own explicit `p.id AS part_id` alias)
/// must not gain a redundant second alias -- only a genuine name mismatch
/// should touch the SQL text.
#[test]
fn rust_sqlx_grouped_query_does_not_alias_columns_that_need_no_rename() {
    let query_fn = generate_grouped_query_fn();
    assert!(
        !query_fn.contains("part_id AS"),
        "part_id already matches its field name and must not be re-aliased:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("o.id AS"),
        "o.id already matches its field name and must not be re-aliased:\n{query_fn}"
    );
}
