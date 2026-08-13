//! Regression tests for GH #202: five Elixir codegen defects, none of which
//! had any test coverage before this fix.
//!
//! 1. `elixir-postgrex` (and every other Elixir driver backend, since they
//!    all share the identical `generate_grouped_structs` shape) emitted
//!    `defstruct [, :children]` -- a syntax error -- whenever a `:grouped`
//!    query's parent column set was empty. The core analyzer now rejects the
//!    one way this used to be reachable through real SQL (an alias-qualified
//!    `@group_by` matching no column, see
//!    `crates/scythe-codegen/tests/generated/test_errors.rs`), so these tests
//!    construct the empty-parent shape directly against the public codegen
//!    API -- the same technique every backend file's own `make_grouped_query`
//!    helper already uses -- to keep the backend-local defensive guard
//!    pinned regardless of what the analyzer does or does not still allow.
//! 2. Five of the six Elixir backends (`elixir-ecto` was already correct)
//!    rendered an enum-typed parameter's `@spec` as the bare module alias
//!    (`UserStatus`), which is a *literal atom* type in typespec position,
//!    not the module's `t()` -- a guaranteed Dialyzer contract violation
//!    against the `String.t()` value actually passed at every call site.
//! 3. `elixir-ecto` emitted byte-for-byte `elixir-postgrex` output --
//!    `Postgrex.query/3`, `Postgrex.conn()`, a `conn` parameter -- despite
//!    its own manifest describing it as "targeting Ecto with
//!    Ecto.Adapters.SQL". It also nested every struct under
//!    `Scythe.Queries.*` (via `file_header`) while postgrex left them
//!    top-level (via `query_class_header`), a second, silent API difference
//!    between two backends meant to be interchangeable.
//! 4. `elixir-exqlite` leaked the `Exqlite.Sqlite3` prepared statement on
//!    every error path except the outermost `prepare` failure: `release` was
//!    reachable only from the success branch.
//! 5. `elixir_tds.rs`'s `tds_param_type_atom` had no `:binary` or `:time`
//!    arm, so a `VARBINARY`/`TIME` parameter was declared to the TDS wire
//!    protocol as `:string`.

use scythe_codegen::validation::{ToolValidation, strict_mode_enabled, validate_structural, validate_with_tools};
use scythe_codegen::{generate_with_backend, get_backend, provenance};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedParam, AnalyzedQuery, GroupByConfig, analyze};
use scythe_core::catalog::Catalog;
use scythe_core::dialect::SqlDialect;
use scythe_core::parser::{QueryCommand, parse_query_with_dialect};

/// Every `elixir-*` backend paired with an engine `get_backend` accepts for
/// it. Kept in one place so a new Elixir backend is automatically covered by
/// every test below that iterates it, rather than silently exempt.
const ALL_ELIXIR_BACKENDS: &[(&str, &str)] = &[
    ("elixir-postgrex", "postgresql"),
    ("elixir-ecto", "postgresql"),
    ("elixir-myxql", "mysql"),
    ("elixir-exqlite", "sqlite"),
    ("elixir-jamdb", "oracle"),
    ("elixir-tds", "mssql"),
];

/// The five backends that lacked the `enum::` -> `String.t()` `@spec` special
/// case before this fix. `elixir-ecto` is deliberately excluded: it already
/// had the correct behavior (`elixir_ecto.rs:125-134`, pre-fix), and is
/// covered separately so a regression there is not masked by the same loop
/// that pins the other five.
const PREVIOUSLY_BUGGY_ENUM_SPEC_BACKENDS: &[(&str, &str)] = &[
    ("elixir-postgrex", "postgresql"),
    ("elixir-myxql", "mysql"),
    ("elixir-exqlite", "sqlite"),
    ("elixir-jamdb", "oracle"),
    ("elixir-tds", "mssql"),
];

/// A `:grouped` query whose parent column set is empty -- the shape that used
/// to reach `defstruct [, :children]`.
///
/// Built directly against `AnalyzedQuery`/`GroupByConfig` rather than through
/// `parse_query_with_dialect` + `analyze`: the core analyzer now turns the one
/// realistic way this arose (an alias-qualified `@group_by` matching no
/// column) into an `INVALID_ANNOTATION` error before codegen ever runs (see
/// `test_group_by_uses_a_query_alias` in `tests/generated/test_errors.rs`),
/// so a real-SQL fixture could no longer exercise the backend-local guard at
/// all. `generate_with_backend` never asserts `parent_columns` is non-empty
/// on the way in (`crates/scythe-codegen/src/lib.rs` resolves it, checks
/// field-name collisions against it, and hands it straight to
/// `generate_grouped_structs` -- nothing in between rejects zero elements),
/// so this input is exactly what a backend's `generate_grouped_structs` must
/// tolerate regardless of how core reaches it.
fn grouped_query_with_empty_parent_columns() -> AnalyzedQuery {
    let child_cols = vec![AnalyzedColumn {
        name: "order_id".to_string(),
        neutral_type: "int32".to_string(),
        nullable: false,
        ..Default::default()
    }];
    AnalyzedQuery::build(|aq| {
        aq.name = "GetUsersWithOrders".to_string();
        aq.command = QueryCommand::Grouped;
        aq.sql = "SELECT o.id AS order_id FROM users u JOIN orders o ON o.user_id = u.id".to_string();
        aq.columns = child_cols.clone();
        aq.group_by = Some(GroupByConfig {
            table: "users".to_string(),
            key_column: "id".to_string(),
            parent_columns: vec![],
            child_columns: child_cols,
        });
    })
}

#[test]
fn empty_grouped_parent_never_produces_defstruct_with_a_leading_comma() {
    for (backend_name, engine) in ALL_ELIXIR_BACKENDS {
        let backend = get_backend(backend_name, engine).expect("backend must support its own engine");
        let query = grouped_query_with_empty_parent_columns();
        let result = generate_with_backend(&query, &*backend).expect("codegen must succeed on an empty parent set");
        let row_struct = result.row_struct.expect("a :grouped query always emits a row_struct");

        assert!(
            !row_struct.contains("defstruct [,"),
            "{backend_name}: emitted the syntactically invalid `defstruct [,` for an empty grouped \
             parent; got:\n{row_struct}"
        );
        assert!(
            row_struct.contains("defstruct [:children]"),
            "{backend_name}: expected the empty-parent case to fall back to `defstruct [:children]`; \
             got:\n{row_struct}"
        );
    }
}

/// A single-param `:one` query, parameterized on the param's neutral type --
/// enough to exercise `@spec` rendering without going through the parser.
fn one_param_query(param_neutral_type: &str) -> AnalyzedQuery {
    AnalyzedQuery::build(|aq| {
        aq.name = "FindUserByStatus".to_string();
        aq.command = QueryCommand::One;
        aq.sql = "SELECT id FROM users WHERE status = $1".to_string();
        aq.columns = vec![AnalyzedColumn {
            name: "id".to_string(),
            neutral_type: "int32".to_string(),
            nullable: false,
            ..Default::default()
        }];
        aq.params = vec![AnalyzedParam {
            name: "status".to_string(),
            neutral_type: param_neutral_type.to_string(),
            nullable: false,
            position: 1,
        }];
    })
}

#[test]
fn enum_param_gets_string_t_spec_not_a_bare_module_alias() {
    for (backend_name, engine) in PREVIOUSLY_BUGGY_ENUM_SPEC_BACKENDS {
        let backend = get_backend(backend_name, engine).expect("backend must support its own engine");
        let query = one_param_query("enum::user_status");
        let result = generate_with_backend(&query, &*backend).expect("codegen must succeed");
        let query_fn = result.query_fn.expect("a :one query always emits a query_fn");

        assert!(
            query_fn.contains("String.t())"),
            "{backend_name}: expected the enum parameter's `@spec` type to be `String.t()`; \
             got:\n{query_fn}"
        );
        assert!(
            !query_fn.contains("UserStatus)"),
            "{backend_name}: the enum parameter's `@spec` still names the bare module alias, which \
             typespecs read as the literal atom `:\"Elixir.UserStatus\"`, not the module's `t()`; \
             got:\n{query_fn}"
        );
    }
}

/// The one realistic route to an enum parameter through the real pipeline:
/// `WHERE status = $1` against an enum-typed column, the same shape already
/// checked into `integration_tests/sql/pg/{schema.sql,queries/users.sql}` as
/// `ListActiveUsers` (which currently generates and runs against a live
/// Postgres). Runs on `elixir-postgrex` only -- `CREATE TYPE ... AS ENUM` is
/// PostgreSQL-specific syntax, so this is the one Elixir backend it reaches
/// without a dialect-specific enum fixture.
#[test]
fn enum_where_clause_param_gets_string_t_spec_through_the_real_pipeline() {
    let schema = "CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned'); \
        CREATE TABLE users (id SERIAL PRIMARY KEY, name TEXT NOT NULL, status user_status NOT NULL DEFAULT 'active');";
    let query = "-- @name ListActiveUsers\n-- @returns :many\nSELECT id, name FROM users WHERE status = $1;";

    let catalog = Catalog::from_ddl_with_dialect(&[schema], &SqlDialect::PostgreSQL).expect("schema must parse");
    let parsed = parse_query_with_dialect(query, &SqlDialect::PostgreSQL).expect("query must parse");
    let analyzed = analyze(&catalog, &parsed).expect("query must analyze");

    let backend = get_backend("elixir-postgrex", "postgresql").expect("postgrex must support postgresql");
    let result = generate_with_backend(&analyzed, &*backend).expect("codegen must succeed");
    let query_fn = result.query_fn.expect("a :many query always emits a query_fn");

    assert!(
        query_fn.contains("String.t())"),
        "expected the `status` parameter's `@spec` type to be `String.t()`; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("UserStatus)"),
        "the `status` parameter's `@spec` still names the bare enum module alias; got:\n{query_fn}"
    );
}

/// A single-param `:many` query with no group-by, used for the ecto shape
/// checks below -- same construction technique as [`one_param_query`].
fn many_param_query() -> AnalyzedQuery {
    AnalyzedQuery::build(|aq| {
        aq.name = "ListUsers".to_string();
        aq.command = QueryCommand::Many;
        aq.sql = "SELECT id FROM users WHERE status = $1".to_string();
        aq.columns = vec![AnalyzedColumn {
            name: "id".to_string(),
            neutral_type: "int32".to_string(),
            nullable: false,
            ..Default::default()
        }];
        aq.params = vec![AnalyzedParam {
            name: "status".to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            position: 1,
        }];
    })
}

fn batch_query() -> AnalyzedQuery {
    AnalyzedQuery::build(|aq| {
        aq.name = "CreateUser".to_string();
        aq.command = QueryCommand::Batch;
        aq.sql = "INSERT INTO users (name) VALUES ($1)".to_string();
        aq.columns = vec![];
        aq.params = vec![AnalyzedParam {
            name: "name".to_string(),
            neutral_type: "string".to_string(),
            nullable: false,
            position: 1,
        }];
    })
}

#[test]
fn ecto_backend_emits_ecto_not_a_postgrex_clone() {
    let backend = get_backend("elixir-ecto", "postgresql").expect("ecto must support postgresql");

    let many = generate_with_backend(&many_param_query(), &*backend).expect("codegen must succeed");
    let query_fn = many.query_fn.expect("a :many query always emits a query_fn");
    assert!(
        query_fn.contains("Ecto.Adapters.SQL.query(repo,"),
        "elixir-ecto must call Ecto.Adapters.SQL.query, not Postgrex.query directly; got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("Postgrex.query"),
        "elixir-ecto must not call Postgrex.query directly; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("Ecto.Repo.t()"),
        "elixir-ecto's @spec must take an Ecto.Repo.t(), not a raw Postgrex.conn(); got:\n{query_fn}"
    );
    assert!(
        !query_fn.contains("Postgrex.conn()"),
        "elixir-ecto's @spec must not reference Postgrex.conn(); got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("def list_users(repo"),
        "elixir-ecto's query functions must take a `repo` parameter, not a raw `conn`; got:\n{query_fn}"
    );

    let batch = generate_with_backend(&batch_query(), &*backend).expect("codegen must succeed");
    let batch_fn = batch.query_fn.expect("a :batch query always emits a query_fn");
    assert!(
        batch_fn.contains("repo.transaction(fn ->"),
        "elixir-ecto's batch function must use Ecto.Repo.transaction/1 via the repo module, not \
         Postgrex.transaction; got:\n{batch_fn}"
    );
    assert!(
        !batch_fn.contains("Postgrex.transaction"),
        "elixir-ecto must not call Postgrex.transaction directly; got:\n{batch_fn}"
    );
    assert!(
        batch_fn.contains("repo.rollback(err)"),
        "elixir-ecto's batch rollback must go through Ecto.Repo.rollback/1 via the repo module, not \
         DBConnection.rollback; got:\n{batch_fn}"
    );
    assert!(
        !batch_fn.contains("DBConnection.rollback"),
        "elixir-ecto must not call DBConnection.rollback directly; got:\n{batch_fn}"
    );
}

/// The grouped path is a separate code path (`generate_grouped_query_fn`)
/// from the flat one exercised above, and needs its own pin.
#[test]
fn ecto_grouped_query_fn_also_emits_ecto() {
    let backend = get_backend("elixir-ecto", "postgresql").expect("ecto must support postgresql");
    let parent_cols = vec![AnalyzedColumn {
        name: "id".to_string(),
        neutral_type: "int32".to_string(),
        nullable: false,
        ..Default::default()
    }];
    let child_cols = vec![AnalyzedColumn {
        name: "order_id".to_string(),
        neutral_type: "int32".to_string(),
        nullable: false,
        ..Default::default()
    }];
    let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
    let query = AnalyzedQuery::build(|aq| {
        aq.name = "GetUsersWithOrders".to_string();
        aq.command = QueryCommand::Grouped;
        aq.sql = "SELECT u.id, o.id AS order_id FROM users u JOIN orders o ON o.user_id = u.id".to_string();
        aq.columns = all_cols;
        aq.group_by = Some(GroupByConfig {
            table: "users".to_string(),
            key_column: "id".to_string(),
            parent_columns: parent_cols,
            child_columns: child_cols,
        });
    });

    let result = generate_with_backend(&query, &*backend).expect("codegen must succeed");
    let query_fn = result.query_fn.expect("a :grouped query always emits a query_fn");

    assert!(
        query_fn.contains("Ecto.Adapters.SQL.query(repo,"),
        "elixir-ecto's grouped query function must call Ecto.Adapters.SQL.query; got:\n{query_fn}"
    );
    assert!(
        query_fn.contains("def get_users_with_orders(repo) do"),
        "elixir-ecto's grouped query function must take a `repo` parameter; got:\n{query_fn}"
    );
}

/// Same silent difference the issue calls out: postgrex leaves row structs
/// top-level (`query_class_header`), the pre-fix ecto backend nested them
/// under `Scythe.Queries.*` (`file_header`). A struct's own generated text
/// never carries that nesting -- it comes from which of the two hooks the
/// backend implements -- so this pins the hooks directly rather than the
/// struct text.
#[test]
fn ecto_and_postgrex_agree_on_where_struct_definitions_nest() {
    let ecto = get_backend("elixir-ecto", "postgresql").expect("ecto must support postgresql");
    let postgrex = get_backend("elixir-postgrex", "postgresql").expect("postgrex must support postgresql");

    assert_eq!(
        ecto.file_header(),
        postgrex.file_header(),
        "elixir-ecto and elixir-postgrex must agree on file_header (both empty), or their row \
         structs nest differently under Scythe.Queries"
    );
    assert!(
        !ecto.query_class_header().is_empty(),
        "elixir-ecto must wrap query functions (not struct definitions) in Scythe.Queries via \
         query_class_header, matching elixir-postgrex"
    );
    assert_eq!(
        ecto.query_class_header(),
        postgrex.query_class_header(),
        "elixir-ecto and elixir-postgrex must use the same Scythe.Queries wrapper"
    );
}

/// One `AnalyzedQuery` per `elixir-exqlite` command shape, each exercising a
/// different `generate_query_fn` branch that prepares a statement.
fn exqlite_query(command: QueryCommand) -> AnalyzedQuery {
    AnalyzedQuery::build(|aq| {
        aq.name = "TouchUser".to_string();
        aq.command = command.clone();
        aq.sql = "UPDATE users SET name = $1 WHERE id = $2".to_string();
        aq.columns = if matches!(command, QueryCommand::One | QueryCommand::Opt | QueryCommand::Many) {
            vec![AnalyzedColumn {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                ..Default::default()
            }]
        } else {
            vec![]
        };
        aq.params = vec![
            AnalyzedParam {
                name: "name".to_string(),
                neutral_type: "string".to_string(),
                nullable: false,
                position: 1,
            },
            AnalyzedParam {
                name: "id".to_string(),
                neutral_type: "int32".to_string(),
                nullable: false,
                position: 2,
            },
        ];
    })
}

/// Every exit path of a `with {:ok, stmt} <- prepare(...) do ... end` that
/// binds and steps a statement must release it -- success, a bind failure,
/// and (where the shape has one) a step failure. Before this fix `release`
/// was reachable from exactly one of those: the success path. This counts
/// occurrences rather than asserting a fixed number, because the point being
/// pinned is "more than the one success-path call", not the exact code shape.
#[test]
fn exqlite_releases_the_prepared_statement_on_every_exit_path() {
    let backend = get_backend("elixir-exqlite", "sqlite").expect("exqlite must support sqlite");

    for command in [
        QueryCommand::One,
        QueryCommand::Many,
        QueryCommand::Exec,
        QueryCommand::ExecResult,
        QueryCommand::Batch,
    ] {
        let query = exqlite_query(command.clone());
        let result = generate_with_backend(&query, &*backend).expect("codegen must succeed");
        let query_fn = result.query_fn.expect("every command shape here emits a query_fn");

        let release_count = query_fn.matches("Exqlite.Sqlite3.release(conn, stmt)").count();
        assert!(
            release_count > 1,
            "{command}: expected `release` to be reachable from more than the single success path \
             (found {release_count} call(s)) -- a bind or step failure would leak the prepared \
             statement; got:\n{query_fn}"
        );
    }
}

#[test]
fn exqlite_grouped_query_fn_also_releases_on_the_bind_failure_path() {
    let backend = get_backend("elixir-exqlite", "sqlite").expect("exqlite must support sqlite");
    let parent_cols = vec![AnalyzedColumn {
        name: "id".to_string(),
        neutral_type: "int32".to_string(),
        nullable: false,
        ..Default::default()
    }];
    let child_cols = vec![AnalyzedColumn {
        name: "order_id".to_string(),
        neutral_type: "int32".to_string(),
        nullable: false,
        ..Default::default()
    }];
    let all_cols = [parent_cols.clone(), child_cols.clone()].concat();
    let query = AnalyzedQuery::build(|aq| {
        aq.name = "GetUsersWithOrders".to_string();
        aq.command = QueryCommand::Grouped;
        aq.sql = "SELECT u.id, o.id AS order_id FROM users u JOIN orders o ON o.user_id = u.id".to_string();
        aq.columns = all_cols;
        aq.group_by = Some(GroupByConfig {
            table: "users".to_string(),
            key_column: "id".to_string(),
            parent_columns: parent_cols,
            child_columns: child_cols,
        });
    });

    let result = generate_with_backend(&query, &*backend).expect("codegen must succeed");
    let query_fn = result.query_fn.expect("a :grouped query always emits a query_fn");

    let release_count = query_fn.matches("Exqlite.Sqlite3.release(conn, stmt)").count();
    assert!(
        release_count > 1,
        "expected `release` to be reachable from more than the single success path (found \
         {release_count} call(s)); got:\n{query_fn}"
    );
}

/// Assemble the exact bytes `scythe generate` would write for a single query,
/// so the real `elixirc` compiler (via [`validate_with_tools`]) sees a
/// complete, syntactically self-contained file rather than a bare fragment.
/// Mirrors `generate_full_file` in `php_array_type_regression.rs`.
fn generate_full_file(backend_name: &str, engine: &str, query: &AnalyzedQuery) -> String {
    let backend = get_backend(backend_name, engine).expect("backend must support its own engine");
    let code = generate_with_backend(query, &*backend).expect("codegen must succeed");

    let all_codes = vec![code];
    let mut full = backend.file_header_for_results(&all_codes);
    full.push('\n');
    for code in &all_codes {
        for s in [&code.enum_def, &code.model_struct, &code.row_struct]
            .into_iter()
            .flatten()
        {
            full.push_str(s);
            full.push('\n');
        }
    }
    let class_header = backend.query_class_header();
    if !class_header.is_empty() {
        full.push_str(&class_header);
        full.push('\n');
    }
    for code in &all_codes {
        if let Some(ref s) = code.query_fn {
            full.push_str(s);
            full.push('\n');
        }
    }
    let footer = backend.file_footer();
    if !footer.is_empty() {
        full.push_str(&footer);
        full.push('\n');
    }

    provenance::assemble_file(
        &backend.file_preamble(),
        &provenance::header_line(
            &*backend,
            env!("CARGO_PKG_VERSION"),
            engine,
            "sch1:0123456789abcdef",
            "q1:fedcba9876543210",
        ),
        &full,
    )
}

/// Structural check always runs; the real `elixirc` compiler runs wherever it
/// is installed (gated the same way every other backend's tool validation
/// is -- see [`strict_mode_enabled`]).
fn assert_elixir_file_is_valid(backend_name: &str, code: &str) {
    let structural_errors = validate_structural(code, backend_name);
    assert!(
        structural_errors.is_empty(),
        "{backend_name} structural: {structural_errors:?}\n\n{code}"
    );

    let validation = validate_with_tools(code, backend_name);
    if strict_mode_enabled() {
        assert!(
            !matches!(validation, ToolValidation::Unsupported) && validation.fully_checked(),
            "{backend_name} has no working elixirc validator, so this test would pass without ever \
             compiling the file"
        );
    }
    if let Err(tool_errors) = validation.into_result() {
        panic!("{backend_name} tool validation: {tool_errors:?}\n\n{code}");
    }
}

/// Every exqlite command shape, run through the real `elixirc` compiler: the
/// restructured `with`/`case` nesting has to still be valid Elixir, not just
/// release the statement on more paths.
#[test]
fn exqlite_restructured_release_paths_still_compile() {
    for command in [
        QueryCommand::One,
        QueryCommand::Many,
        QueryCommand::Exec,
        QueryCommand::ExecResult,
        QueryCommand::Batch,
    ] {
        let query = exqlite_query(command.clone());
        let code = generate_full_file("elixir-exqlite", "sqlite", &query);
        assert_elixir_file_is_valid("elixir-exqlite", &code);
    }
}

/// The rewritten ecto output, run through the real `elixirc` compiler.
#[test]
fn ecto_rewrite_compiles() {
    for query in [many_param_query(), batch_query()] {
        let code = generate_full_file("elixir-ecto", "postgresql", &query);
        assert_elixir_file_is_valid("elixir-ecto", &code);
    }
}

/// `tds_param_type_atom` itself is private to `elixir_tds.rs` and is unit
/// tested there directly (`test_tds_param_type_atom_every_neutral_type`).
/// This pins the same fix from the public codegen surface: a `bytes` or
/// `time`/`time_tz` neutral-typed parameter must reach the generated
/// `%Tds.Parameter{}` literal as `:binary`/`:time`, not fall through to the
/// `:string` default the way it did before #202.
#[test]
fn tds_binary_and_time_params_are_not_declared_as_string() {
    let backend = get_backend("elixir-tds", "mssql").expect("tds must support mssql");

    for (neutral_type, expected_atom) in [("bytes", ":binary"), ("time", ":time"), ("time_tz", ":time")] {
        let query = one_param_query(neutral_type);
        let result = generate_with_backend(&query, &*backend).expect("codegen must succeed");
        let query_fn = result.query_fn.expect("a :one query always emits a query_fn");

        assert!(
            query_fn.contains(&format!("type: {expected_atom}}}")),
            "neutral type `{neutral_type}` must produce `type: {expected_atom}`, not fall through \
             to `:string`; got:\n{query_fn}"
        );
    }
}
