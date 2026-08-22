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
use scythe_core::analyzer::{AnalyzedParam, AnalyzedQuery, CompositeFieldInfo, CompositeInfo, analyze};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::{QueryCommand, parse_query_with_dialect};

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
                nullable: false,
            },
            CompositeFieldInfo {
                name: "zip_code".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
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
            source_relation: None,
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

#[test]
fn typescript_pg_encodes_a_composite_param_as_postgres_text() {
    let backend = get_backend("typescript-pg", "postgresql").expect("typescript-pg supports postgresql");
    let query = composite_param_query();
    let composite_def = backend
        .generate_composite_def(&query.composites[0])
        .expect("composite definition must be generated");
    let result = generate_with_backend(&query, &*backend).expect("codegen must not fail on a composite param");
    let query_fn = result.query_fn.expect("Exec command must produce a query fn");

    assert!(
        query_fn.contains("[encodeAddress(home_address)]"),
        "must encode the composite before binding it; got:\n{query_fn}"
    );
    assert!(
        composite_def.contains("function encodeAddress(value: Address | null): string | null"),
        "must generate a nullable whole-composite encoder; got:\n{composite_def}"
    );
    assert!(
        composite_def.contains("replaceAll(\"\\\\\", \"\\\\\\\\\").replaceAll('\\\"', '\\\"\\\"')"),
        "encoder must escape backslashes and quotes; got:\n{composite_def}"
    );
    assert!(
        composite_def.contains("return `(${encode(value.street)},${encode(value.zipCode)})`;"),
        "encoder must interpolate encoded field values; got:\n{composite_def}"
    );
}

#[test]
fn typescript_kysely_encodes_a_composite_param_as_postgres_text() {
    let backend = get_backend("typescript-kysely", "postgresql").expect("typescript-kysely supports postgresql");
    let query = composite_param_query();
    let composite_def = backend
        .generate_composite_def(&query.composites[0])
        .expect("composite definition must be generated");
    let result = generate_with_backend(&query, &*backend).expect("codegen must not fail on a composite param");
    let query_fn = result.query_fn.expect("Exec command must produce a query fn");

    assert!(
        query_fn.contains("${encodeAddress(home_address)}"),
        "must encode the composite before binding it; got:\n{query_fn}"
    );
    assert!(
        composite_def.contains("function encodeAddress(value: Address | null): string | null"),
        "must generate a nullable whole-composite encoder; got:\n{composite_def}"
    );
    assert!(
        composite_def.contains("return `(${encode(value.street)},${encode(value.zipCode)})`;"),
        "encoder must interpolate encoded field values; got:\n{composite_def}"
    );
}

#[test]
fn typescript_kysely_encodes_a_single_composite_batch_item() {
    let backend = get_backend("typescript-kysely", "postgresql").expect("typescript-kysely supports postgresql");
    let mut query = composite_param_query();
    query.command = QueryCommand::Batch;
    let result = generate_with_backend(&query, &*backend).expect("codegen must not fail on a composite batch item");
    let query_fn = result.query_fn.expect("Batch command must produce a query fn");

    assert!(
        query_fn.contains("${encodeAddress(item)}"),
        "must encode each composite batch item; got:\n{query_fn}"
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
            source_relation: None,
        },
        AnalyzedParam {
            name: "home_address".to_string(),
            neutral_type: "composite::address".to_string(),
            nullable: true,
            position: 2,
            source_relation: None,
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

/// #225: the four tests above build `AnalyzedQuery` by hand, with `query.composites` set
/// directly (see `composite_param_query` above). That hand-populated field is exactly the state
/// the real bug prevented from existing -- every test above passed while the actual pipeline was
/// broken, because none of them exercise the code that is supposed to populate `composites` in
/// the first place. The real defect lived one stage upstream, in the analyzer: `composite_worklist`
/// in `crates/scythe-core/src/analyzer/mod.rs` seeded itself from `columns` and
/// `nested_field_types` but never from `params`, so a composite bound only as a query parameter
/// (never returned as a column, e.g. the torture schema's `CreateWidget` INSERT) never reached
/// `analyzed.composites` at all, and `pg_composite_bind_expr` took its silent whole-object
/// fallback. This test runs the full `Catalog::from_ddl` -> `parse_query_with_dialect` -> `analyze`
/// -> `generate_with_backend` pipeline -- the only way to prove the analyzer itself, not a test
/// fixture standing in for it, produces the composite.
#[test]
fn analyzer_populates_composites_for_a_param_only_composite() {
    let schema = "\
        CREATE TYPE torture_address AS (street TEXT, city TEXT, zip TEXT); \
        CREATE TABLE torture_widgets (\
            widget_id SERIAL PRIMARY KEY, \
            home_address torture_address, \
            scheduled_at TIMESTAMP NOT NULL DEFAULT NOW()\
        );";
    let query = "-- @name CreateWidget\n-- @returns :one\n\
        INSERT INTO torture_widgets (home_address) VALUES ($1) RETURNING widget_id, scheduled_at;";

    let catalog = Catalog::from_ddl_with_dialect(&[schema], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");
    assert!(
        analyzed.composites.iter().any(|c| c.sql_name == "torture_address"),
        "the analyzer must collect a composite reachable only through a query param, not just \
         through columns; got composites:\n{:?}",
        analyzed.composites
    );

    let backend = get_backend("typescript-postgres", "postgresql").expect("typescript-postgres supports postgresql");
    let result = generate_with_backend(&analyzed, &*backend).expect("codegen must not fail on a composite param");
    let query_fn = result
        .query_fn
        .expect("query with a RETURNING clause must produce a query fn");

    let expected = "ROW(${home_address.street}, ${home_address.city}, ${home_address.zip})::torture_address";
    assert!(
        query_fn.contains(expected),
        "expected the row-constructor literal `{expected}`; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("${home_address}"),
        "must not bind the whole composite object as a single postgres.js parameter -- that is \
         the #225 fallback firing because analyzed.composites was empty; got:\n{query_fn}"
    );
}
