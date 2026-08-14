//! Regression tests for two Ruby RBS defects (GH #203, and the Ruby third of GH #198).
//!
//! ## GH #203 -- a `:grouped` query's `.rbs` described a class that does not exist
//!
//! `crates/scythe-cli/src/commands/generate.rs`'s RBS producer used to resolve a `:grouped`
//! query's *flat* `analyzed.columns` and rewrite `QueryCommand::Grouped` to `Many` before
//! handing both to `ruby_rbs.rs`. The generated `.rb` file defines a child `Data.define` plus
//! a parent one with a `children` field (see `ruby_pg.rs`'s `generate_grouped_structs_ruby`
//! call), but the `.rbs` alongside it described one flat class with neither -- `steep check`
//! against real calling code failed. The fix keeps `command` as `Grouped` and carries the
//! parent and child columns in their own `RbsQueryInfo` fields, so the emitter writes both
//! classes.
//!
//! A `scythe-codegen` integration test cannot drive the real producer directly: it is a
//! private function in the separate `scythe-cli` binary crate. So the tests below build an
//! `RbsGenerationContext` the same way that producer does (see `grouped_rbs_context`) and
//! drive the real, public `CodegenBackend::generate_rbs_file` trait method -- the seam this
//! crate's own RBS emitter logic actually runs behind.
//!
//! ## GH #198 (Ruby third) -- the coercion table contradicted the manifest
//!
//! `ruby_pg.rs`, `ruby_mysql2.rs`, and `ruby_trilogy.rs` each had a `ruby_coercion` table with
//! no arm for `decimal` at all, so a `decimal` column (declared `BigDecimal` in every one of
//! those manifests) came back from the generated `.rb` code as whatever raw type the driver
//! returns, while the `.rbs` right next to it said `BigDecimal`. The fix derives the coercion
//! from the manifest's own declared type instead of a parallel hardcoded list. `ruby-sqlite3`
//! declares `decimal = "Float"` and needs no `BigDecimal` at all.

use scythe_backend::naming::{fn_name, row_struct_name};
use scythe_codegen::resolve::{resolve_columns, resolve_params};
use scythe_codegen::{
    CodegenBackend, GeneratedCode, RbsGenerationContext, RbsQueryInfo, generate_with_backend, get_backend,
};
use scythe_core::analyzer::{AnalyzedQuery, analyze};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::{QueryCommand, parse_query_with_dialect};

const GROUPED_SCHEMA: [&str; 2] = [
    "CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL);",
    "CREATE TABLE orders (\
     id SERIAL PRIMARY KEY, \
     user_id INT NOT NULL REFERENCES users (id), \
     total DECIMAL(10, 2) NOT NULL, \
     order_date TIMESTAMP NOT NULL\
     );",
];

const GROUPED_QUERY: &str = "-- @name GetUsersWithOrders\n-- @returns :grouped\n-- @group_by users.id\n\
    SELECT u.id, u.name, u.email, o.id AS order_id, o.total, o.order_date \
    FROM users u JOIN orders o ON o.user_id = u.id;";

const DECIMAL_SCHEMA: &str = "CREATE TABLE orders (id SERIAL PRIMARY KEY, total DECIMAL(10, 2) NOT NULL);";
const DECIMAL_QUERY: &str = "-- @name GetOrder\n-- @returns :one\nSELECT id, total FROM orders WHERE id = $1;";
const NO_DECIMAL_QUERY: &str = "-- @name GetOrderId\n-- @returns :one\nSELECT id FROM orders WHERE id = $1;";

fn backend_for(name: &str, engine: &str) -> Box<dyn CodegenBackend> {
    get_backend(name, engine).unwrap_or_else(|e| panic!("{name}/{engine} must construct: {e}"))
}

fn analyze_query(schema: &[&str], query: &str) -> AnalyzedQuery {
    let catalog = Catalog::from_ddl_with_dialect(schema, &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    analyze(&catalog, &parsed).expect("query must analyze")
}

/// Build the `RbsGenerationContext` for a single `:grouped` query exactly the way
/// `generate_rbs_if_supported`'s `QueryCommand::Grouped` branch does.
fn grouped_rbs_context(analyzed: &AnalyzedQuery, backend: &dyn CodegenBackend) -> RbsGenerationContext {
    let manifest = backend.manifest();
    let group_by = analyzed.group_by.as_ref().expect("query must have @group_by");

    let parent_cols = resolve_columns(&group_by.parent_columns, manifest, &[]).expect("parent columns resolve");
    let child_cols = resolve_columns(&group_by.child_columns, manifest, &[]).expect("child columns resolve");
    let params = resolve_params(&analyzed.params, manifest, &[], "").expect("params resolve");

    RbsGenerationContext {
        queries: vec![RbsQueryInfo {
            func_name: fn_name(&analyzed.name, &manifest.naming),
            struct_name: Some(row_struct_name(&analyzed.name, &manifest.naming)),
            columns: parent_cols,
            child_columns: child_cols,
            params,
            command: QueryCommand::Grouped,
        }],
        enums: vec![],
    }
}

/// Build the `RbsGenerationContext` for a single non-grouped (`:one`/`:many`) query exactly
/// the way `generate_rbs_if_supported`'s other branch does.
fn flat_rbs_context(analyzed: &AnalyzedQuery, backend: &dyn CodegenBackend) -> RbsGenerationContext {
    let manifest = backend.manifest();
    let columns = resolve_columns(&analyzed.columns, manifest, &[]).expect("columns resolve");
    let params = resolve_params(&analyzed.params, manifest, &[], "").expect("params resolve");

    RbsGenerationContext {
        queries: vec![RbsQueryInfo {
            func_name: fn_name(&analyzed.name, &manifest.naming),
            struct_name: Some(row_struct_name(&analyzed.name, &manifest.naming)),
            columns,
            child_columns: Vec::new(),
            params,
            command: analyzed.command.clone(),
        }],
        enums: vec![],
    }
}

/// The `.rb` file `scythe generate` would write: header (via `file_header_for_results`, so a
/// conditional `require` is exercised the same way it is in production) through footer.
fn full_rb_file(backend: &dyn CodegenBackend, code: &GeneratedCode) -> String {
    let all = std::slice::from_ref(code);
    let mut out = backend.file_header_for_results(all);
    out.push('\n');
    for text in [&code.enum_def, &code.model_struct, &code.row_struct, &code.query_fn]
        .into_iter()
        .flatten()
    {
        out.push_str(text);
        out.push('\n');
    }
    out.push_str(&backend.file_footer());
    out
}

#[test]
fn grouped_query_rbs_declares_child_before_parent_with_children_reader() {
    let analyzed = analyze_query(&GROUPED_SCHEMA, GROUPED_QUERY);

    for (backend_name, engine) in [("ruby-pg", "postgresql"), ("ruby-sqlite3", "sqlite")] {
        let backend = backend_for(backend_name, engine);
        let context = grouped_rbs_context(&analyzed, &*backend);
        let rbs = backend
            .generate_rbs_file(&context)
            .unwrap_or_else(|| panic!("{backend_name} must emit RBS"));

        assert!(
            rbs.contains("class GetUsersWithOrdersChildRow"),
            "{backend_name}: missing child class; got:\n{rbs}"
        );
        assert!(
            rbs.contains("class GetUsersWithOrdersRow"),
            "{backend_name}: missing parent class; got:\n{rbs}"
        );
        assert!(
            rbs.contains("attr_reader children: Array[GetUsersWithOrdersChildRow]"),
            "{backend_name}: parent class must declare a `children` reader typed to the child \
             class; got:\n{rbs}"
        );
        assert!(
            rbs.contains("def self.get_users_with_orders: ") && rbs.contains("-> Array[GetUsersWithOrdersRow]"),
            "{backend_name}: query method must return `Array[<parent>]`; got:\n{rbs}"
        );

        let child_pos = rbs
            .find("class GetUsersWithOrdersChildRow")
            .expect("child class position");
        let parent_pos = rbs.find("class GetUsersWithOrdersRow").expect("parent class position");
        assert!(
            child_pos < parent_pos,
            "{backend_name}: child class must be defined before the parent class (avoids a \
             forward reference); got:\n{rbs}"
        );
    }
}

#[test]
fn grouped_query_child_column_types_are_not_lost_in_the_split() {
    let analyzed = analyze_query(&GROUPED_SCHEMA, GROUPED_QUERY);
    let backend = backend_for("ruby-pg", "postgresql");
    let context = grouped_rbs_context(&analyzed, &*backend);
    let rbs = backend.generate_rbs_file(&context).expect("ruby-pg must emit RBS");

    // ~keep Parent columns. The parent/child labels are the point of this test:
    // #203 was the RBS describing one flat class, so which side each assertion
    // belongs to is what distinguishes a passing split from a regressed one.
    assert!(rbs.contains("attr_reader id: Integer"), "got:\n{rbs}");
    assert!(rbs.contains("attr_reader name: String"), "got:\n{rbs}");
    // ~keep The child half of the parent/child split above: without this label the
    // preceding comment's claim that the labels distinguish a passing split from a
    // regressed one has no counterpart. The `decimal` -> `BigDecimal` assertion is
    // the one that motivated #198.
    assert!(rbs.contains("attr_reader order_id: Integer"), "got:\n{rbs}");
    assert!(rbs.contains("attr_reader total: BigDecimal"), "got:\n{rbs}");
    assert!(rbs.contains("attr_reader order_date:"), "got:\n{rbs}");
}

#[test]
fn pg_mysql2_and_trilogy_decimal_columns_coerce_to_bigdecimal_matching_the_rbs() {
    let analyzed = analyze_query(&[DECIMAL_SCHEMA], DECIMAL_QUERY);

    for (backend_name, engine) in [
        ("ruby-pg", "postgresql"),
        ("ruby-mysql2", "mysql"),
        ("ruby-trilogy", "mysql"),
    ] {
        let backend = backend_for(backend_name, engine);
        let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
        let query_fn = code.query_fn.as_deref().expect("query fn");
        assert!(
            query_fn.contains(".to_d"),
            "{backend_name}: `total` (decimal -> BigDecimal) must be coerced with `.to_d`; got:\n{query_fn}"
        );

        let full_rb = full_rb_file(&*backend, &code);
        assert!(
            full_rb.contains("require \"bigdecimal/util\""),
            "{backend_name}: a file whose generated code calls `.to_d` must require \
             `bigdecimal/util`; got:\n{full_rb}"
        );

        let rbs_context = flat_rbs_context(&analyzed, &*backend);
        let rbs = backend.generate_rbs_file(&rbs_context).expect("must emit RBS");
        assert!(
            rbs.contains("attr_reader total: BigDecimal"),
            "{backend_name}: `.rbs` must declare `total` as `BigDecimal`, matching the `.rb` \
             coercion above; got:\n{rbs}"
        );
        // No `library "bigdecimal"` counterpart: `library` is Steepfile syntax, not RBS
        // declaration syntax, and emitting it makes `rbs parse` reject the whole file with
        // "cannot start a declaration". A signature may name `BigDecimal` freely.
        assert!(
            !rbs.contains("library "),
            "{backend_name}: an `.rbs` must not carry a `library` directive -- it is not RBS \
             declaration syntax and makes the file unparseable; got:\n{rbs}"
        );
    }
}

#[test]
fn sqlite3_decimal_column_coerces_to_float_never_bigdecimal() {
    let analyzed = analyze_query(&[DECIMAL_SCHEMA], DECIMAL_QUERY);
    let backend = backend_for("ruby-sqlite3", "sqlite");

    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    let query_fn = code.query_fn.as_deref().expect("query fn");
    assert!(
        query_fn.contains(".to_f"),
        "sqlite3: `total` (decimal -> Float) must be coerced with `.to_f`; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains(".to_d"),
        "sqlite3 never declares BigDecimal, so it must never coerce with `.to_d`; got:\n{query_fn}"
    );

    let rbs_context = flat_rbs_context(&analyzed, &*backend);
    let rbs = backend.generate_rbs_file(&rbs_context).expect("must emit RBS");
    assert!(
        rbs.contains("attr_reader total: Float"),
        "sqlite3: `.rbs` must declare `total` as `Float`, matching the `.rb` coercion above; got:\n{rbs}"
    );
    assert!(
        !rbs.contains("BigDecimal"),
        "sqlite3 never declares BigDecimal; got:\n{rbs}"
    );
}

/// `require "bigdecimal/util"` must be emitted only when the generated code for *this*
/// query actually needs it -- not unconditionally for every file a `BigDecimal`-capable
/// backend ever produces.
#[test]
fn bigdecimal_directives_are_not_emitted_for_a_query_with_no_decimal_column() {
    let analyzed = analyze_query(&[DECIMAL_SCHEMA], NO_DECIMAL_QUERY);
    let backend = backend_for("ruby-pg", "postgresql");

    let code = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    let full_rb = full_rb_file(&*backend, &code);
    assert!(
        !full_rb.contains("bigdecimal"),
        "a query selecting no decimal column must not require bigdecimal/util; got:\n{full_rb}"
    );

    let rbs_context = flat_rbs_context(&analyzed, &*backend);
    let rbs = backend.generate_rbs_file(&rbs_context).expect("must emit RBS");
    assert!(
        !rbs.contains("BigDecimal"),
        "a query selecting no decimal column must not reference BigDecimal in its `.rbs`; got:\n{rbs}"
    );
}
