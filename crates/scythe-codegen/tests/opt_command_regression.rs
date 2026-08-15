//! Regression tests for GH #197: `-- @returns :opt` must be distinguishable
//! from `-- @returns :one` in generated code.
//!
//! `:one` means "exactly one row, error if absent"; `:opt` means "zero or one
//! row, return null/None/Option". Across the backend registry, the vast
//! majority of `generate_query_fn` implementations match
//! `QueryCommand::One | QueryCommand::Opt => { .. }` in a single arm, so the
//! two commands render byte-identical code. Which contract silently wins
//! depends on which behaviour that shared arm happens to implement:
//!
//! - `rust-tiberius` implemented the arm as "exactly one row, `.expect()` if
//!   absent" -- so `:opt` panicked at runtime on a missing row, the exact
//!   case `:opt` exists to handle without an error.
//! - Most backends implemented the arm as "zero or one row, return
//!   null/None" -- so `:opt`'s own output was correct and `:one` silently
//!   inherited its permissiveness, never erroring on a missing row.
//!
//! An earlier revision of this comment claimed that second bullet covered
//! "csharp-*, python-*, elixir-*, go-*, java-*, kotlin-*, php-*, ruby-*,
//! typescript-*, rust-sibyl". **That was wrong for go-\* and elixir-\***, 10
//! of the 53 entries this list used to hold, and the error was found only
//! when someone re-derived each family's arm from the code while fixing it:
//!
//! - `go_database_sql.rs` emitted `err := row.Scan(..); return r, err`, and
//!   `sql.ErrNoRows` propagates through that untouched -- so `:one` was
//!   already correct and `:opt` was the broken one, erroring on a
//!   legitimately absent row.
//! - Every `elixir_*.rs` emitted `{:error, :not_found}` for both commands,
//!   the same inversion.
//! - `go_godror.rs` really did fold the permissive way, unlike its three Go
//!   siblings -- so even "the family behaves alike" was not safe to assume.
//!
//! The lesson is the one the `rust-sqlx` note below records independently: a
//! ratchet can only check pass/fail, never *why*, so a stated reason nobody
//! re-derives rots while the gate stays green. Re-derive before trusting a
//! reason in this file, and prefer quoting the emitted code over describing
//! it.
//!
//! `rust-tokio-postgres` was already correct throughout and is the reference
//! shape: `query_one` for `:one` (errors on a missing row through the
//! driver's own `Result`, never panics) and `query_opt` for `:opt` (returns
//! `Option<Row>`, mapped into `Option<Struct>`).
//!
//! Every backend has since been fixed, so `KNOWN_UNDIFFERENTIATED_BACKENDS`
//! is empty. It is kept, rather than deleted with its last entry, because
//! the ratchet still fails in both directions and an empty list is what
//! makes a new fold show up as a regression instead of as the status quo.

use scythe_codegen::{generate_with_backend, get_backend};
use scythe_core::analyzer::{AnalyzedColumn, AnalyzedQuery};
use scythe_core::parser::QueryCommand;

/// A minimal single-column, param-free, non-`RETURNING` query: it exercises
/// only the `QueryCommand::One`/`QueryCommand::Opt` branch every backend's
/// `generate_query_fn` has, without touching any of the RETURNING-clause,
/// enum, or composite special cases that would need per-backend fixtures.
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
// rust-tiberius: the panic fix (crates/scythe-codegen/src/backends/rust_tiberius.rs)
// ---------------------------------------------------------------------

/// The bug itself: `stream.into_row().await?.expect("expected one row")` was
/// emitted for both `:one` and `:opt`. For `:opt` that `.expect()` panics at
/// runtime the moment the row is legitimately absent -- the one case `:opt`
/// is supposed to hand back as `None` rather than fail on.
#[test]
fn rust_tiberius_opt_returns_option_and_does_not_panic_on_missing_row() {
    let opt_fn = query_fn_for("rust-tiberius", "mssql", QueryCommand::Opt);

    assert!(
        opt_fn.contains("Result<Option<GetItemRow>, tiberius::error::Error>"),
        "rust-tiberius :opt must return an Option-wrapped row type; got:\n{opt_fn}"
    );
    assert!(
        !opt_fn.contains(".expect(\"expected one row\")"),
        "rust-tiberius :opt must not use .expect() on a possibly-absent row -- that panics \
         at runtime for the exact case :opt exists to handle (issue #197):\n{opt_fn}"
    );
    assert!(
        opt_fn.contains(".transpose()"),
        "rust-tiberius :opt is expected to map the optional row through `from_row` and fold it \
         with `.transpose()` into Result<Option<Row>, Error>, so an absent row becomes `Ok(None)` \
         rather than a panic; got:\n{opt_fn}"
    );
}

/// `:one` keeps its pre-existing "exactly one row, `.expect()` if absent"
/// shape -- this fix only stops `:opt` from being folded into it. Whether
/// `:one` panicking (rather than returning an `Err`) on a missing row is
/// itself desirable is a separate question, out of scope for #197.
#[test]
fn rust_tiberius_one_is_unchanged_by_the_opt_fix() {
    let one_fn = query_fn_for("rust-tiberius", "mssql", QueryCommand::One);

    assert!(
        one_fn.contains("-> Result<GetItemRow, tiberius::error::Error>"),
        "rust-tiberius :one must keep returning the bare row type, not Option-wrapped; got:\n{one_fn}"
    );
    assert!(
        one_fn.contains(".expect(\"expected one row\")"),
        "rust-tiberius :one's exactly-one-row behaviour must be unchanged by the :opt fix; \
         got:\n{one_fn}"
    );
}

/// The regression itself, phrased the way #197 named it: before this fix,
/// `:opt` on rust-tiberius rendered byte-identical code to `:one`, which is
/// exactly what made it panic instead of returning `None`.
#[test]
fn rust_tiberius_opt_and_one_generate_different_code() {
    let one_fn = query_fn_for("rust-tiberius", "mssql", QueryCommand::One);
    let opt_fn = query_fn_for("rust-tiberius", "mssql", QueryCommand::Opt);

    assert_ne!(
        one_fn, opt_fn,
        "rust-tiberius must generate different code for :one and :opt -- identical code is the \
         exact fold GH #197 named (\"opt silently treated as one\")"
    );
}

// ---------------------------------------------------------------------
// Census: does *any* backend actually distinguish :one from :opt?
// ---------------------------------------------------------------------

/// Every backend name [`scythe_codegen::get_backend`] recognises as a
/// primary (non-alias) name, mirroring `scythe_codegen::lib::tests::
/// ALL_BACKEND_NAMES` (kept as an independent copy here rather than an
/// import: that list lives in a private `#[cfg(test)] mod tests` in another
/// agent's owned `lib.rs`, and duplicating ~50 string literals into a test
/// file this crate already trusts is cheaper than reaching into another
/// owner's file).
///
/// `ruby-rbs` is intentionally absent: it is not registered in
/// `get_backend` at all (`ruby_rbs` is a `pub(crate)` companion generator
/// invoked alongside `ruby-pg`/etc., not a standalone backend), so this
/// census cannot reach it through the same code path as everything else.
const ALL_BACKEND_NAMES: &[&str] = &[
    "rust-sqlx",
    "rust-tokio-postgres",
    "rust-tiberius",
    "rust-sibyl",
    "python-psycopg3",
    "python-asyncpg",
    "python-aiomysql",
    "python-aiosqlite",
    "python-duckdb",
    "python-pyodbc",
    "python-oracledb",
    "python-snowflake",
    "typescript-postgres",
    "javascript-postgres",
    "typescript-pg",
    "javascript-pg",
    "typescript-mysql2",
    "javascript-mysql2",
    "typescript-better-sqlite3",
    "javascript-better-sqlite3",
    "typescript-duckdb",
    "javascript-duckdb",
    "typescript-node-sqlite",
    "javascript-node-sqlite",
    "typescript-wasm-sqlite",
    "javascript-wasm-sqlite",
    "typescript-kysely",
    "typescript-mssql",
    "javascript-mssql",
    "typescript-oracledb",
    "javascript-oracledb",
    "typescript-snowflake",
    "javascript-snowflake",
    "go-database-sql",
    "go-pgx",
    "go-godror",
    "go-gosnowflake",
    "java-jdbc",
    "java-r2dbc",
    "kotlin-exposed",
    "kotlin-jdbc",
    "kotlin-r2dbc",
    "csharp-npgsql",
    "csharp-mysqlconnector",
    "csharp-microsoft-sqlite",
    "csharp-sqlclient",
    "csharp-oracle",
    "csharp-snowflake",
    "elixir-postgrex",
    "elixir-ecto",
    "elixir-myxql",
    "elixir-exqlite",
    "elixir-tds",
    "elixir-jamdb",
    "ruby-pg",
    "ruby-mysql2",
    "ruby-sqlite3",
    "ruby-trilogy",
    "ruby-tiny-tds",
    "ruby-oci8",
    "php-pdo",
    "php-amphp",
];

/// Tried in order until one resolves the backend's manifest -- mirrors
/// `scythe_codegen::lib::tests::backend_for_select_star`'s `ENGINES` list.
/// This census cares whether `:one` and `:opt` diverge at all, not which
/// engine it observes that on.
const ENGINES: &[&str] = &[
    "postgresql",
    "mysql",
    "sqlite",
    "mssql",
    "mariadb",
    "duckdb",
    "oracle",
    "snowflake",
    "redshift",
];

fn first_supported_engine(name: &str) -> &'static str {
    for engine in ENGINES {
        if get_backend(name, engine).is_ok() {
            return engine;
        }
    }
    panic!("no known engine works for backend '{name}' -- ALL_BACKEND_NAMES or ENGINES has gone stale");
}

/// One entry in [`KNOWN_UNDIFFERENTIATED_BACKENDS`]: a backend name paired
/// with why its `:one` and `:opt` output is currently identical, so the list
/// doubles as its own documentation -- same shape as
/// `scythe_codegen::lib::tests::KNOWN_DIVERGENT_BACKENDS`.
struct BackendNote {
    backend: &'static str,
    reason: &'static str,
}

/// Ratcheting allowlist, same discipline as `lib.rs`'s
/// `KNOWN_DIVERGENT_BACKENDS`: a backend not listed here whose `:one` and
/// `:opt` output is identical is a regression (or, for a backend nobody has
/// fixed yet, evidence the census undercounted -- investigate either way).
/// A backend listed here whose output has started to differ is a stale
/// entry -- the underlying fold was fixed and the line must be deleted, not
/// left to rot.
///
/// Every entry today folds `:one` and `:opt` into ONE SHARED
/// `QueryCommand::One | QueryCommand::Opt` match arm. Which contract wins
/// varies by backend (see the file-level doc comment): most of these
/// backends actually render `:opt`'s own output correctly and instead leave
/// `:one` wrongly permissive (no error on a missing row) -- a real defect,
/// but not the one #197 tracks, and not touched by this fix. `rust-sqlx`
/// `rust-tiberius`, `rust-sqlx` and `rust-tokio-postgres` are deliberately
/// absent: the first two were fixed, and the third already was correct.
///
/// `rust-sqlx` is worth remembering as what this ratchet is for. It was
/// listed here as folding the way #197 describes -- `:opt` inheriting
/// `:one`'s `.fetch_one` -- and running the census proved that reason wrong.
/// Its `:opt` output declared `{Struct}` as the return type while
/// `has_row_struct` excluded `Opt`, so the body emitted the anonymous-record
/// `sqlx::query!` rather than `sqlx::query_as!`: the declared type and the
/// produced type disagreed, and the generated code did not compile at all.
/// A reason derived by reading match arms missed that, because the defect
/// was in the interaction between three arms, not in any one of them.
const KNOWN_UNDIFFERENTIATED_BACKENDS: &[BackendNote] = &[];

/// Census guard for #197: does any backend other than `rust-tiberius` and
/// `rust-tokio-postgres` actually render different code for `:one` vs.
/// `:opt`? Every backend not in [`KNOWN_UNDIFFERENTIATED_BACKENDS`] must
/// differ; every backend in it must currently still be identical -- a stale
/// entry fails exactly as loudly as a regression, so the allowlist cannot
/// silently rot as backends get fixed (same discipline as
/// `scythe_codegen::lib::tests::test_select_star_declares_and_references_the_same_struct_name_across_all_backends`).
///
/// This intentionally does not assert which *direction* a fold goes (`:opt`
/// inheriting `:one`'s strictness, as `rust-sqlx` and pre-fix
/// `rust-tiberius` do, vs. `:one` inheriting `:opt`'s permissiveness, as
/// every other allowlisted backend does) -- both are silent downgrades of
/// one command's contract into the other's, and both are exactly what
/// "a backend silently downgrades :opt" (or, symmetrically, :one) means.
#[test]
fn one_and_opt_render_different_code_except_on_the_known_allowlist() {
    let mut regressions = Vec::new();
    let mut stale_entries = Vec::new();

    for &name in ALL_BACKEND_NAMES {
        let engine = first_supported_engine(name);
        let one_fn = query_fn_for(name, engine, QueryCommand::One);
        let opt_fn = query_fn_for(name, engine, QueryCommand::Opt);
        let differs = one_fn != opt_fn;

        let listed = KNOWN_UNDIFFERENTIATED_BACKENDS.iter().find(|n| n.backend == name);
        match (differs, listed) {
            (false, None) => regressions.push(name),
            (true, Some(_)) => stale_entries.push(name),
            // Confirmed still undifferentiated for the reason on file --
            // visible with `--nocapture`.
            (false, Some(note)) => eprintln!("expected fold, {name}: {}", note.reason),
            (true, None) => {}
        }
    }

    let mut failures = Vec::new();
    for name in &regressions {
        failures.push(format!(
            "REGRESSION: {name}: :one and :opt render identical code and {name} is not in \
             KNOWN_UNDIFFERENTIATED_BACKENDS -- investigate before adding a line"
        ));
    }
    for name in &stale_entries {
        failures.push(format!(
            "STALE ALLOWLIST: {name}: listed in KNOWN_UNDIFFERENTIATED_BACKENDS but :one and \
             :opt now render different code -- delete its entry"
        ));
    }

    assert!(
        failures.is_empty(),
        "{} of {} backends need attention:\n{}",
        failures.len(),
        ALL_BACKEND_NAMES.len(),
        failures.join("\n")
    );
}
