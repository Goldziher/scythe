//! Regression tests for board #225: `ruby-oci8` assigned an Oracle LOB column's value
//! straight through, but oci8 hands back a lazy `OCI8::CLOB`/`OCI8::NCLOB`/`OCI8::BLOB`
//! locator, not a materialized `String` -- the row struct declares `String` (the manifest's
//! mapping for both the `string` and `bytes` neutral types), so a caller comparing against
//! the row's `notes` field got `#<OCI8::CLOB:0x...>` instead of its text contents.
//!
//! `neutral_type` alone cannot distinguish a LOB from its non-LOB counterpart -- CLOB and
//! VARCHAR2 both resolve to `"string"`, BLOB and RAW both resolve to `"bytes"` -- so the fix
//! matches on `sql_type`, the raw schema-declared type, exactly the seam `rust_sibyl.rs`
//! already uses for the identical CLOB-vs-VARCHAR2 problem in the sibling Oracle backend (see
//! `rust_sibyl.rs::emit_row_get`'s doc comment and
//! `test_clob_column_reads_via_lob_locator_not_row_get_string`).
//!
//! These tests pin the emitted `read_lob(...)` wrapper at every column-read site: the
//! `RETURNING ... INTO` path (`cursor[N]`), the plain fetch path (`row[N]`) for `:one`,
//! `:opt`, and `:many`, and the grouped-query fold's parent/child struct construction. They
//! also pin that a non-LOB column (`VARCHAR2`/`NUMBER`) is never wrapped, and that the
//! `read_lob` class method is only emitted into the file header when a query actually needs
//! it.
//!
//! ruby-oci8 needs the Oracle Instant Client, which does not ship for macOS ARM64, so none of
//! this is verified against a running driver -- only that the generator emits the intended
//! Ruby source shape. See `ruby_oci8.rs`'s `READ_LOB_METHOD` doc comment for the vendored gem
//! source evidence backing the wrapper's own logic (nil passthrough, `#read` semantics).

use scythe_codegen::{CodegenBackend, generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery, GroupByConfig};
use scythe_core::parser::QueryCommand;

fn backend() -> Box<dyn CodegenBackend> {
    get_backend("ruby-oci8", "oracle").expect("ruby-oci8/oracle must construct")
}

/// `id INTEGER` (never wrapped) alongside `notes CLOB` and `payload BLOB` (both must be
/// wrapped), matching `integration_tests/sql/oracle/schema_full.sql`'s `orders.notes` and
/// `attachments.payload` columns that triggered the original CI failure.
fn lob_query(command: QueryCommand) -> AnalyzedQuery {
    AnalyzedQuery::build(|query| {
        query.name = "GetOrdersByUser".to_string();
        query.command = command;
        query.sql = "SELECT id, notes, payload FROM orders".to_string();
        query.columns = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                sql_type: "integer".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "notes".to_string(),
                sql_type: "clob".to_string(),
                neutral_type: "string".to_string(),
                nullable: true,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "payload".to_string(),
                sql_type: "blob".to_string(),
                neutral_type: "bytes".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
    })
}

/// A query with no LOB column at all -- a `VARCHAR2` (`sql_type = "varchar2"`) has the same
/// `neutral_type` (`"string"`) as a `CLOB`, so this is the case that actually distinguishes
/// `sql_type`-matching from a bare `neutral_type` check.
fn non_lob_query(command: QueryCommand) -> AnalyzedQuery {
    AnalyzedQuery::build(|query| {
        query.name = "GetItem".to_string();
        query.command = command;
        query.sql = "SELECT id, label FROM items".to_string();
        query.columns = vec![
            AnalyzedColumn {
                name: "id".to_string(),
                sql_type: "integer".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            },
            AnalyzedColumn {
                name: "label".to_string(),
                sql_type: "varchar2".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                ..Default::default()
            },
        ];
    })
}

#[test]
fn many_wraps_clob_and_blob_columns_but_not_the_integer_column() {
    let query = lob_query(QueryCommand::Many);
    let generated = generate_with_backend(&query, &*backend()).expect("codegen must succeed");
    let query_fn = generated.query_fn.expect(":many must produce a query fn");

    assert!(
        query_fn.contains("notes: read_lob(row[1])"),
        "CLOB column must be read via read_lob; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("payload: read_lob(row[2])"),
        "BLOB column must be read via read_lob; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("id: row[0]"),
        "non-LOB column must be read raw, not wrapped; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("id: read_lob"),
        "non-LOB column must never be wrapped in read_lob; got:\n{query_fn}"
    );
}

#[test]
fn one_and_opt_plain_fetch_wrap_lob_columns() {
    for command in [QueryCommand::One, QueryCommand::Opt] {
        let query = lob_query(command.clone());
        let generated = generate_with_backend(&query, &*backend()).expect("codegen must succeed");
        let query_fn = generated.query_fn.expect("must produce a query fn");

        assert!(
            query_fn.contains("notes: read_lob(row[1])"),
            "{command:?}: CLOB column must be read via read_lob; got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("payload: read_lob(row[2])"),
            "{command:?}: BLOB column must be read via read_lob; got:\n{query_fn}"
        );
    }
}

/// `RETURNING ... INTO` path: `:one`/`:opt` against a DML statement bind the output columns
/// and read them back through `cursor[N]`, not `row[N]`.
#[test]
fn returning_into_path_wraps_lob_columns_read_through_cursor() {
    for command in [QueryCommand::One, QueryCommand::Opt] {
        let query = AnalyzedQuery::build(|query| {
            query.name = "InsertOrder".to_string();
            query.command = command.clone();
            query.sql = "INSERT INTO orders (notes) VALUES (:notes) RETURNING id, notes".to_string();
            query.columns = vec![
                AnalyzedColumn {
                    name: "id".to_string(),
                    sql_type: "integer".to_string(),
                    neutral_type: "int32".to_string(),
                    nullable: false,
                    ..Default::default()
                },
                AnalyzedColumn {
                    name: "notes".to_string(),
                    sql_type: "clob".to_string(),
                    neutral_type: "string".to_string(),
                    nullable: true,
                    ..Default::default()
                },
            ];
        });
        let generated = generate_with_backend(&query, &*backend()).expect("codegen must succeed");
        let query_fn = generated.query_fn.expect("must produce a query fn");

        assert!(
            query_fn.contains("notes: read_lob(cursor[2])"),
            "{command:?}: RETURNING INTO CLOB column must be read via read_lob(cursor[N]); got:\n{query_fn}"
        );
        assert!(
            query_fn.contains("id: cursor[1]"),
            "{command:?}: RETURNING INTO non-LOB column must be read raw; got:\n{query_fn}"
        );
    }
}

fn make_grouped_query() -> AnalyzedQuery {
    let parent_cols = vec![
        AnalyzedColumn {
            name: "id".to_string(),
            sql_type: "integer".to_string(),
            neutral_type: "int32".to_string(),
            nullable: false,
            ..Default::default()
        },
        AnalyzedColumn {
            name: "notes".to_string(),
            sql_type: "clob".to_string(),
            neutral_type: "string".to_string(),
            nullable: true,
            ..Default::default()
        },
    ];
    let child_cols = vec![
        AnalyzedColumn {
            name: "attachment_id".to_string(),
            sql_type: "integer".to_string(),
            neutral_type: "int32".to_string(),
            nullable: false,
            ..Default::default()
        },
        AnalyzedColumn {
            name: "payload".to_string(),
            sql_type: "blob".to_string(),
            neutral_type: "bytes".to_string(),
            nullable: false,
            ..Default::default()
        },
    ];
    let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
    AnalyzedQuery::build(|aq| {
        aq.name = "GetOrdersWithAttachments".to_string();
        aq.command = QueryCommand::Grouped;
        aq.sql = "SELECT o.id, o.notes, a.id AS attachment_id, a.payload\n\
                  FROM orders o\n\
                  JOIN attachments a ON a.order_id = o.id"
            .to_string();
        aq.columns = all_cols;
        aq.group_by = Some(GroupByConfig {
            table: "orders".to_string(),
            key_column: "id".to_string(),
            parent_columns: parent_cols,
            child_columns: child_cols,
        });
    })
}

#[test]
fn grouped_fold_wraps_lob_columns_in_both_parent_and_child_but_not_the_grouping_key() {
    let query = make_grouped_query();
    let generated = generate_with_backend(&query, &*backend()).expect("codegen must succeed");
    let query_fn = generated.query_fn.expect("grouped query must produce a query fn");

    assert!(
        query_fn.contains("notes: read_lob(row[1]),"),
        "parent CLOB column must be wrapped; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("payload: read_lob(row[3]),"),
        "child BLOB column must be wrapped; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("id: row[0],"),
        "parent non-LOB column must be read raw; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("key = row[0]"),
        "the grouping key itself must be read raw, never through read_lob -- wrapping it \
         would call #read a second time on the same handle used for the id field above, and \
         a LOB's read position is EOF after the first call; got:\n{query_fn}"
    );
}

#[test]
fn file_header_only_declares_read_lob_when_a_query_actually_uses_it() {
    let backend = backend();

    let lob_generated = generate_with_backend(&lob_query(QueryCommand::Many), &*backend).expect("codegen");
    let header_with_lob = backend.file_header_for_results(std::slice::from_ref(&lob_generated));
    assert!(
        header_with_lob.contains("def self.read_lob(value)"),
        "a file whose generated code calls read_lob must declare it; got:\n{header_with_lob}"
    );

    let plain_generated = generate_with_backend(&non_lob_query(QueryCommand::Many), &*backend).expect("codegen");
    let header_without_lob = backend.file_header_for_results(std::slice::from_ref(&plain_generated));
    assert!(
        !header_without_lob.contains("read_lob"),
        "a file with no LOB column must not declare the unused read_lob helper; got:\n{header_without_lob}"
    );
    assert_eq!(
        header_without_lob,
        backend.file_header(),
        "with no LOB usage, file_header_for_results must fall back to the plain file_header"
    );
}

#[test]
fn non_lob_string_column_is_never_wrapped_despite_matching_neutral_type() {
    // VARCHAR2 and CLOB share the same neutral_type ("string"); only sql_type distinguishes
    // them. This is the case a neutral_type-only check would get wrong. ~keep
    let query = non_lob_query(QueryCommand::Many);
    let generated = generate_with_backend(&query, &*backend()).expect("codegen must succeed");
    let query_fn = generated.query_fn.expect(":many must produce a query fn");

    assert!(
        query_fn.contains("label: row[1]"),
        "VARCHAR2 column must be read raw, not wrapped; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("read_lob"),
        "a query touching no LOB column must never reference read_lob; got:\n{query_fn}"
    );
}
