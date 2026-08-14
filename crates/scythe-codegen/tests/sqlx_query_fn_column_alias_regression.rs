//! Board #195 / #194: board #191 (see `sqlx_grouped_query_regression.rs`)
//! gave `generate_grouped_query_fn` an explicit `AS "field_name"` alias for a
//! non-identifier column, but `generate_query_fn` -- the plain, non-grouped
//! `:one`/`:many`/`:opt` path -- kept selecting such a column unaliased.
//! sqlx-macros-core's
//! `output::parse_ident` (sqlx-macros-core 0.9.0, `src/query/output.rs:408`)
//! requires a driver-reported column name to already be a valid Rust
//! identifier and hard-errors macro expansion otherwise; `output::quote_query_as`
//! (`output.rs:202`) then builds the row with `#out_ty { #ident: #var_name }`,
//! where `#ident` is that same driver-reported name, so even a column whose
//! name *is* a valid identifier but differs in shape from this backend's own
//! `field_name` (case, or `sanitize_field_names` reshaping) fails against a
//! struct-literal field that isn't spelled that way.
//!
//! A second, independent defect lived in the enum-column aliasing this path
//! already had (`rewrite_sql_for_enums`, since folded into the same shared
//! rewrite as the fix for the above): its alias was hand-written as
//! `\\\"...\\\"` -- Rust source for a literal backslash followed by a quote
//! -- and then passed through `escape_rust_string`, which escapes that
//! backslash and quote *again*. The SQL text sqlx actually receives at
//! compile time ends up with a literal backslash sqlx never asked for.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::analyze;
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::parse_query_with_dialect;

const SCHEMA: [&str; 2] = [
    "CREATE TYPE widget_status AS ENUM ('active', 'inactive');",
    "CREATE TABLE widgets (id SERIAL PRIMARY KEY, \"my col\" TEXT, status widget_status NOT NULL);",
];

const QUERY: &str = "-- @name GetWidget\n-- @returns :one\nSELECT id, \"my col\", status FROM widgets WHERE id = $1;";

fn generate_query_fn() -> String {
    let backend = get_backend("rust-sqlx", "postgresql").expect("backend must support postgresql");
    let catalog = Catalog::from_ddl_with_dialect(&SCHEMA, &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(QUERY, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    code.query_fn.expect("expected a query function")
}

/// The board #195 regression: a plain (non-grouped) `:one` query selecting a
/// non-identifier column must alias it in the SQL handed to `query_as!`.
///
/// Before this fix, `generate_query_fn` ran the SQL only through
/// `rewrite_sql_for_enums`, which never looks at non-enum columns at all, so
/// `"my col"` reached `sqlx::query_as!` unaliased. The old code's `query_fn`
/// does not contain this substring at all -- no `AS` clause is emitted for
/// `"my col"` anywhere.
#[test]
fn rust_sqlx_query_fn_aliases_a_mangled_column() {
    let query_fn = generate_query_fn();
    assert!(
        query_fn.contains("\\\"my col\\\" AS \\\"my_col\\\""),
        "expected the SQL to alias the non-identifier column to the field sqlx's \
         query_as! macro must report -- `\"my col\" AS \"my_col\"` -- but got:\n{query_fn}"
    );
}

/// The board #194 regression: the enum-column alias must reach sqlx as a
/// plain, single-escaped `AS "status: WidgetStatus"` -- one backslash ahead
/// of each quote in the generated Rust source, which is what a Rust string
/// literal needs to decode back to a bare `"` at compile time.
///
/// The old `rewrite_sql_for_enums` hand-wrote a backslash-quote pair
/// (already a real backslash + quote at runtime) and then ran the whole SQL
/// through `escape_rust_string`, which escapes that same backslash and quote
/// a second time. The old code's `query_fn` does not contain this
/// single-escaped substring at all -- the backslash ahead of each quote
/// comes out tripled instead of single, so a real, unwanted backslash lands
/// in the SQL text sqlx actually receives at compile time.
#[test]
fn rust_sqlx_query_fn_enum_alias_is_not_double_escaped() {
    let query_fn = generate_query_fn();
    assert!(
        query_fn.contains("status AS \\\"status: WidgetStatus\\\""),
        "expected a single-escaped enum alias -- `status AS \"status: WidgetStatus\"` -- \
         but got:\n{query_fn}"
    );
}
