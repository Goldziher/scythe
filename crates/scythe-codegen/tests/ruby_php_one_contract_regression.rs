//! Regression tests for board #192, ruby-*/php-* families: `:one` never errors on a missing
//! row.
//!
//! `:one` means "exactly one row, error if absent"; `:opt` means "zero or one row, return
//! nil/null". Every ruby-* and php-* `generate_query_fn` used to fold
//! `QueryCommand::One | QueryCommand::Opt` into a single match arm that always returned
//! nil/null on a missing row -- correct for `:opt`, but silently wrong for `:one`, which must
//! raise/throw instead. This mirrors `crates/scythe-codegen/tests/opt_command_regression.rs`'s
//! `query_fn_for`/`one_column_query` helpers (duplicated here rather than imported: each
//! integration-test binary is its own crate, so there is nothing to import from).
//!
//! Ruby raises `Queries::RecordNotFound` (a `StandardError` subclass -- see
//! `ruby_rbs.rs::RECORD_NOT_FOUND_CLASS`, this project's ruby-conventions: "specific
//! exceptions inheriting StandardError"). PHP throws `RecordNotFoundException` (a
//! `RuntimeException` subclass -- see `php_common.rs::RECORD_NOT_FOUND_EXCEPTION_CLASS`, this
//! project's php-conventions: "specific exceptions extending RuntimeException").
//!
//! `ruby-rbs` is covered separately in the last section: it is not reachable through
//! `get_backend` (see `opt_command_regression.rs`'s note on `ALL_BACKEND_NAMES`), so its
//! `:one`/`:opt` signature split is exercised through `CodegenBackend::generate_rbs_file`
//! directly, the same seam `ruby_rbs_regression.rs` uses.

use scythe_backend::naming::{fn_name, row_struct_name};
use scythe_codegen::resolve::{resolve_columns, resolve_params};
use scythe_codegen::{CodegenBackend, RbsGenerationContext, RbsQueryInfo, generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery};
use scythe_core::parser::QueryCommand;

/// A minimal single-column, param-free, non-`RETURNING` query: it exercises only the
/// `QueryCommand::One`/`QueryCommand::Opt` branch every backend's `generate_query_fn` has,
/// without touching any RETURNING-clause, enum, or composite special case.
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

fn query_fn_for(backend_name: &str, engine: &str, command: QueryCommand) -> String {
    let backend = get_backend(backend_name, engine)
        .unwrap_or_else(|error| panic!("{backend_name}/{engine}: backend must support engine: {error}"));
    let query = one_column_query(command.clone());
    let generated = generate_with_backend(&query, &*backend)
        .unwrap_or_else(|error| panic!("{backend_name}/{engine}: codegen failed for {command:?}: {error}"));
    generated
        .query_fn
        .unwrap_or_else(|| panic!("{backend_name}/{engine}: {command:?} produced no query fn"))
}

// ---------------------------------------------------------------------
// ruby-*: `raise RecordNotFound, "get_item: no row found"` for :one,
// `return nil` (unchanged) for :opt.
// ---------------------------------------------------------------------

const RUBY_RAISE: &str = "raise RecordNotFound, \"get_item: no row found\"";

struct RubyCase {
    backend: &'static str,
    engine: &'static str,
}

const RUBY_CASES: &[RubyCase] = &[
    RubyCase {
        backend: "ruby-pg",
        engine: "postgresql",
    },
    RubyCase {
        backend: "ruby-mysql2",
        engine: "mysql",
    },
    RubyCase {
        backend: "ruby-sqlite3",
        engine: "sqlite",
    },
    RubyCase {
        backend: "ruby-trilogy",
        engine: "mysql",
    },
    RubyCase {
        backend: "ruby-tiny-tds",
        engine: "mssql",
    },
    RubyCase {
        backend: "ruby-oci8",
        engine: "oracle",
    },
];

#[test]
fn ruby_one_raises_record_not_found_and_never_returns_nil() {
    for case in RUBY_CASES {
        let one_fn = query_fn_for(case.backend, case.engine, QueryCommand::One);
        assert!(
            one_fn.contains(RUBY_RAISE),
            "{}: :one must raise RecordNotFound on a missing row; got:\n{one_fn}",
            case.backend
        );
        assert!(
            !one_fn.contains("return nil"),
            "{}: :one must not return nil on a missing row; got:\n{one_fn}",
            case.backend
        );
    }
}

#[test]
fn ruby_opt_still_returns_nil_and_never_raises_record_not_found() {
    for case in RUBY_CASES {
        let opt_fn = query_fn_for(case.backend, case.engine, QueryCommand::Opt);
        assert!(
            opt_fn.contains("return nil"),
            "{}: :opt must keep returning nil on a missing row; got:\n{opt_fn}",
            case.backend
        );
        assert!(
            !opt_fn.contains("raise RecordNotFound"),
            "{}: :opt must not raise RecordNotFound; got:\n{opt_fn}",
            case.backend
        );
    }
}

#[test]
fn ruby_one_and_opt_render_different_code() {
    for case in RUBY_CASES {
        let one_fn = query_fn_for(case.backend, case.engine, QueryCommand::One);
        let opt_fn = query_fn_for(case.backend, case.engine, QueryCommand::Opt);
        assert_ne!(
            one_fn, opt_fn,
            "{}: :one and :opt must render different code -- identical code is exactly the \
             fold that let :one silently inherit :opt's permissiveness",
            case.backend
        );
    }
}

// ---------------------------------------------------------------------
// php-*: `throw new RecordNotFoundException('getItem: no row found')` and a
// non-nullable return type for :one; `?{Struct}` and `null` on a missing
// row (unchanged) for :opt.
// ---------------------------------------------------------------------

const PHP_THROW: &str = "throw new RecordNotFoundException('getItem: no row found')";

struct PhpCase {
    backend: &'static str,
    engine: &'static str,
}

const PHP_CASES: &[PhpCase] = &[
    PhpCase {
        backend: "php-pdo",
        engine: "postgresql",
    },
    PhpCase {
        backend: "php-amphp",
        engine: "postgresql",
    },
];

#[test]
fn php_one_throws_record_not_found_exception_and_never_returns_null() {
    for case in PHP_CASES {
        let one_fn = query_fn_for(case.backend, case.engine, QueryCommand::One);
        assert!(
            one_fn.contains(PHP_THROW),
            "{}: :one must throw RecordNotFoundException on a missing row; got:\n{one_fn}",
            case.backend
        );
        assert!(
            one_fn.contains("): GetItemRow {"),
            "{}: :one's declared return type must be the bare row type, not nullable; got:\n{one_fn}",
            case.backend
        );
        assert!(
            !one_fn.contains("?GetItemRow"),
            "{}: :one must not declare a nullable return type; got:\n{one_fn}",
            case.backend
        );
    }
}

#[test]
fn php_opt_still_returns_null_and_never_throws_record_not_found_exception() {
    for case in PHP_CASES {
        let opt_fn = query_fn_for(case.backend, case.engine, QueryCommand::Opt);
        assert!(
            opt_fn.contains("): ?GetItemRow {"),
            "{}: :opt must keep declaring a nullable return type; got:\n{opt_fn}",
            case.backend
        );
        assert!(
            opt_fn.contains("null"),
            "{}: :opt must keep returning null on a missing row; got:\n{opt_fn}",
            case.backend
        );
        assert!(
            !opt_fn.contains("RecordNotFoundException"),
            "{}: :opt must not throw RecordNotFoundException; got:\n{opt_fn}",
            case.backend
        );
    }
}

#[test]
fn php_one_and_opt_render_different_code() {
    for case in PHP_CASES {
        let one_fn = query_fn_for(case.backend, case.engine, QueryCommand::One);
        let opt_fn = query_fn_for(case.backend, case.engine, QueryCommand::Opt);
        assert_ne!(
            one_fn, opt_fn,
            "{}: :one and :opt must render different code -- identical code is exactly the \
             fold that let :one silently inherit :opt's permissiveness",
            case.backend
        );
    }
}

// ---------------------------------------------------------------------
// ruby-rbs: the `.rbs` signature must agree with the `.rb` runtime shape
// above -- `:one` non-nullable, `:opt` nullable.
// ---------------------------------------------------------------------

/// Build the `RbsGenerationContext` for a single non-grouped query, mirroring
/// `ruby_rbs_regression.rs`'s `flat_rbs_context` (a private helper in a separate test binary,
/// so duplicated here rather than imported).
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

#[test]
fn ruby_rbs_one_signature_is_non_nullable_and_opt_stays_nullable() {
    let backend = get_backend("ruby-pg", "postgresql").expect("ruby-pg/postgresql must construct");

    let one_query = one_column_query(QueryCommand::One);
    let one_context = flat_rbs_context(&one_query, &*backend);
    let one_rbs = backend
        .generate_rbs_file(&one_context)
        .expect("ruby-pg must emit RBS for :one");
    assert!(
        one_rbs.contains("def self.get_item: (PG::Connection) -> GetItemRow\n"),
        ":one's RBS signature must return the bare row type, matching the `.rb` code's \
         raise-on-missing-row shape; got:\n{one_rbs}"
    );
    assert!(
        !one_rbs.contains("GetItemRow?"),
        ":one's RBS signature must not be nullable; got:\n{one_rbs}"
    );

    let opt_query = one_column_query(QueryCommand::Opt);
    let opt_context = flat_rbs_context(&opt_query, &*backend);
    let opt_rbs = backend
        .generate_rbs_file(&opt_context)
        .expect("ruby-pg must emit RBS for :opt");
    assert!(
        opt_rbs.contains("def self.get_item: (PG::Connection) -> GetItemRow?"),
        ":opt's RBS signature must stay nullable; got:\n{opt_rbs}"
    );
}
