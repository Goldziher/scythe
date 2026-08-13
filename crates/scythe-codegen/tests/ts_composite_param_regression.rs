//! Regression tests for board #179: `typescript-postgres`/`javascript-postgres`
//! could not bind a composite-typed parameter.
//!
//! Root cause chain: a query parameter whose neutral type is `composite::{name}`
//! reached `generate_query_fn`'s placeholder rewrite
//! (`crates/scythe-codegen/src/backends/typescript_postgres.rs`), which used to
//! splice every parameter the same way -- `${field_name}` -- straight into the
//! postgres.js tagged template. postgres.js's `sql` tag only binds values it has
//! a serializer for (scalars, arrays, `sql.json`, ...); a plain object typed as
//! a Postgres composite is not one of them, so `tsc` rejected the argument as
//! not assignable to `ParameterOrFragment<never>` (TS2345), cascading into a
//! TS1320 on the `await` (see `scripts/torture-expected-failures.txt`'s
//! now-deleted Class 3 entry, and the CHANGELOG entry this fix responds to).
//!
//! The fix builds a Postgres row-constructor literal instead of binding the
//! whole object: `ROW(${value.field1}, ${value.field2})::composite_name`, one
//! `${}` binding per *scalar* field (each of which postgres.js already knows
//! how to serialize), with `ROW(`, the separators, and `)::composite_name`
//! spliced in as literal SQL text around them -- see `pg_composite_bind_expr`
//! and `pg_bind_expr` in `typescript_postgres.rs`.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo};
use scythe_core::parser::QueryCommand;

/// A composite type with two fields, one of which (`zip_code`) is not
/// already camelCase -- so a test that only used single-word fields could
/// not tell a correct per-field camelCase rename from a coincidence.
fn address_composite() -> CompositeInfo {
    CompositeInfo {
        sql_name: "address".to_string(),
        fields: vec![
            CompositeFieldInfo {
                name: "street".to_string(),
                neutral_type: "string".to_string(),
            },
            CompositeFieldInfo {
                name: "zip_code".to_string(),
                neutral_type: "string".to_string(),
            },
        ],
    }
}

/// A single `:exec` insert taking one composite-typed parameter -- the
/// torture schema's `CreateWidget` shape, minus the `RETURNING` clause and
/// the other nine columns that shape needs, since neither is relevant to
/// binding.
fn composite_param_query() -> AnalyzedQuery {
    AnalyzedQuery::build(|query| {
        query.name = "CreateWidget".to_string();
        query.command = QueryCommand::Exec;
        query.sql = "INSERT INTO widgets (home_address) VALUES ($1)".to_string();
        query.columns = vec![];
        query.params = vec![AnalyzedParam {
            name: "home_address".to_string(),
            neutral_type: "composite::address".to_string(),
            nullable: true,
            position: 1,
        }];
        query.composites = vec![address_composite()];
    })
}

const EXPECTED_ROW_LITERAL: &str = "ROW(${home_address.street}, ${home_address.zipCode})::address";

/// The regression itself: `typescript-postgres` must bind the composite
/// through a row-constructor literal, one `${}` per scalar field, never the
/// whole object as a single binding.
#[test]
fn typescript_postgres_binds_a_composite_param_as_a_row_constructor() {
    let backend = get_backend("typescript-postgres", "postgresql").expect("typescript-postgres supports postgresql");
    let query = composite_param_query();
    let result = generate_with_backend(&query, &*backend).expect("codegen must not fail on a composite param");
    let query_fn = result.query_fn.expect("Exec command must produce a query fn");

    assert!(
        query_fn.contains(EXPECTED_ROW_LITERAL),
        "expected the row-constructor literal `{EXPECTED_ROW_LITERAL}`; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("${home_address}"),
        "must not bind the whole composite object as a single postgres.js parameter -- that is \
         exactly #179 (TS2345, not assignable to ParameterOrFragment<never>); got:\n{query_fn}"
    );
}

/// Same fix, `javascript-postgres` (js_mode) side: the JSDoc-typed query
/// functions go through the identical placeholder rewrite.
#[test]
fn javascript_postgres_binds_a_composite_param_as_a_row_constructor() {
    let backend = get_backend("javascript-postgres", "postgresql").expect("javascript-postgres supports postgresql");
    let query = composite_param_query();
    let result = generate_with_backend(&query, &*backend).expect("codegen must not fail on a composite param");
    let query_fn = result.query_fn.expect("Exec command must produce a query fn");

    assert!(
        query_fn.contains(EXPECTED_ROW_LITERAL),
        "expected the row-constructor literal `{EXPECTED_ROW_LITERAL}`; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("${home_address}"),
        "must not bind the whole composite object as a single postgres.js parameter; got:\n{query_fn}"
    );
}

/// A `:batch` query whose only parameter is composite-typed binds each item
/// as a whole (`${item}` today) -- the same defect one level up, since the
/// composite is now the entire batch item rather than one field of it.
#[test]
fn typescript_postgres_binds_a_single_composite_batch_item_as_a_row_constructor() {
    let backend = get_backend("typescript-postgres", "postgresql").expect("typescript-postgres supports postgresql");
    let mut query = composite_param_query();
    query.command = QueryCommand::Batch;
    let result = generate_with_backend(&query, &*backend).expect("codegen must not fail on a composite batch item");
    let query_fn = result.query_fn.expect("Batch command must produce a query fn");

    let expected = "ROW(${item.street}, ${item.zipCode})::address";
    assert!(
        query_fn.contains(expected),
        "expected the row-constructor literal `{expected}` for the batch item; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("${item}"),
        "must not bind the whole composite item as a single postgres.js parameter -- the pre-fix \
         `${{item}}` binding; got:\n{query_fn}"
    );
}

/// A `:batch` query with a composite param alongside another param exercises
/// the multi-parameter batch path (`batch_item_sql`), which reads each
/// field off `item.<field>` rather than the bare parameter name.
#[test]
fn typescript_postgres_binds_a_composite_field_in_a_multi_param_batch_item() {
    let backend = get_backend("typescript-postgres", "postgresql").expect("typescript-postgres supports postgresql");
    let mut query = composite_param_query();
    query.command = QueryCommand::Batch;
    query.sql = "INSERT INTO widgets (kind, home_address) VALUES ($1, $2)".to_string();
    query.params = vec![
        AnalyzedParam {
            name: "kind".to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            position: 1,
        },
        AnalyzedParam {
            name: "home_address".to_string(),
            neutral_type: "composite::address".to_string(),
            nullable: true,
            position: 2,
        },
    ];
    let result = generate_with_backend(&query, &*backend).expect("codegen must not fail on a composite param");
    let query_fn = result.query_fn.expect("Batch command must produce a query fn");

    let expected = "ROW(${item.home_address.street}, ${item.home_address.zipCode})::address";
    assert!(
        query_fn.contains(expected),
        "expected the row-constructor literal `{expected}` reading the composite field off the \
         batch item; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("${item.home_address}"),
        "must not bind the whole composite field as a single postgres.js parameter; got:\n{query_fn}"
    );
}
