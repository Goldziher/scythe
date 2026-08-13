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
//! - `rust-tiberius` (fixed here) implemented the arm as "exactly one row,
//!   `.expect()` if absent" -- so `:opt` panicked at runtime on a missing
//!   row, the exact case `:opt` exists to handle without an error.
//! - Most other backends (csharp-*, python-*, elixir-*, go-*, java-*,
//!   kotlin-*, php-*, ruby-*, typescript-*, rust-sibyl) implemented the arm
//!   as "zero or one row, return null/None" -- so `:opt`'s own output is
//!   actually correct, but `:one` silently inherits `:opt`'s permissiveness
//!   and never errors on a missing row. That is a real, separate defect
//!   (`:one`'s contract, not `:opt`'s), left to the backend owners named in
//!   `KNOWN_UNDIFFERENTIATED_BACKENDS` below.
//!
//! `rust-tokio-postgres` was already correct before this fix and is the
//! reference shape the `rust-tiberius` fix follows: `query_one` for `:one`
//! (errors on a missing row through the driver's own `Result`, never
//! panics) and `query_opt` for `:opt` (returns `Option<Row>`, mapped into
//! `Option<Struct>`).

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
    "typescript-node-sqlite",
    "typescript-wasm-sqlite",
    "typescript-kysely",
    "typescript-mssql",
    "typescript-oracledb",
    "typescript-snowflake",
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
const KNOWN_UNDIFFERENTIATED_BACKENDS: &[BackendNote] = &[
    BackendNote {
        backend: "rust-sibyl",
        reason: "generate_query_fn's QueryCommand::One | QueryCommand::Opt arm always returns \
                  sibyl::Result<Option<Struct>> and returns Ok(None) on a missing row for both \
                  commands -- :opt's own output is correct, but :one silently inherits it and \
                  never errors on a missing row. A real but distinct defect (:one's contract, not \
                  :opt's); not fixed here to stay inside this fix's stated scope.",
    },
    BackendNote {
        backend: "csharp-npgsql",
        reason: "return type `{Struct}?` and `if (!await reader.ReadAsync()) return null;` are \
                  shared by One and Opt -- same :one-inherits-:opt's-nullability shape as rust-sibyl, \
                  reproduced identically across every csharp-*.rs backend in this list.",
    },
    BackendNote {
        backend: "csharp-mysqlconnector",
        reason: "same shared nullable-return-and-null-on-missing-row arm as csharp-npgsql.",
    },
    BackendNote {
        backend: "csharp-microsoft-sqlite",
        reason: "same shared nullable-return-and-null-on-missing-row arm as csharp-npgsql.",
    },
    BackendNote {
        backend: "csharp-sqlclient",
        reason: "same shared nullable-return-and-null-on-missing-row arm as csharp-npgsql.",
    },
    BackendNote {
        backend: "csharp-oracle",
        reason: "same shared nullable-return-and-null-on-missing-row arm as csharp-npgsql.",
    },
    BackendNote {
        backend: "csharp-snowflake",
        reason: "same shared nullable-return-and-null-on-missing-row arm as csharp-npgsql.",
    },
    BackendNote {
        backend: "python-psycopg3",
        reason: "`-> {Struct} | None:` plus `if row is None: return None` are shared by One and \
                  Opt -- :one silently inherits :opt's nullability, reproduced identically across \
                  every python-*.rs backend in this list.",
    },
    BackendNote {
        backend: "python-asyncpg",
        reason: "same shared `{Struct} | None` / `return None` arm as python-psycopg3.",
    },
    BackendNote {
        backend: "python-aiomysql",
        reason: "same shared `{Struct} | None` / `return None` arm as python-psycopg3.",
    },
    BackendNote {
        backend: "python-aiosqlite",
        reason: "same shared `{Struct} | None` / `return None` arm as python-psycopg3.",
    },
    BackendNote {
        backend: "python-duckdb",
        reason: "same shared `{Struct} | None` / `return None` arm as python-psycopg3.",
    },
    BackendNote {
        backend: "python-pyodbc",
        reason: "same shared `{Struct} | None` / `return None` arm as python-psycopg3.",
    },
    BackendNote {
        backend: "python-oracledb",
        reason: "same shared `{Struct} | None` / `return None` arm as python-psycopg3.",
    },
    BackendNote {
        backend: "python-snowflake",
        reason: "same shared `{Struct} | None` / `return None` arm as python-psycopg3.",
    },
    BackendNote {
        backend: "typescript-postgres",
        reason: "One and Opt share one return-type/body arm rendering `{Struct} | null` and a \
                  `null` result on a missing row, reproduced identically across every \
                  typescript-*.rs/javascript-*.rs backend in this list.",
    },
    BackendNote {
        backend: "javascript-postgres",
        reason: "js_mode of typescript-postgres; same shared arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-pg",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "javascript-pg",
        reason: "js_mode of typescript-pg; same shared arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-mysql2",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "javascript-mysql2",
        reason: "js_mode of typescript-mysql2; same shared arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-better-sqlite3",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "javascript-better-sqlite3",
        reason: "js_mode of typescript-better-sqlite3; same shared arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-duckdb",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-node-sqlite",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-wasm-sqlite",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-kysely",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-mssql",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-oracledb",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "typescript-snowflake",
        reason: "same shared nullable arm as typescript-postgres.",
    },
    BackendNote {
        backend: "go-database-sql",
        reason: "One and Opt share one arm; Go's zero-value/error idiom collapses the same way, \
                  reproduced identically across every go-*.rs backend in this list.",
    },
    BackendNote {
        backend: "go-pgx",
        reason: "same shared arm as go-database-sql.",
    },
    BackendNote {
        backend: "go-godror",
        reason: "same shared arm as go-database-sql.",
    },
    BackendNote {
        backend: "go-gosnowflake",
        reason: "same shared arm as go-database-sql.",
    },
    BackendNote {
        backend: "java-jdbc",
        reason: "One and Opt share one `@Nullable {Struct}` arm returning null on a missing row, \
                  reproduced identically across every java-*.rs/kotlin-*.rs backend in this list.",
    },
    BackendNote {
        backend: "java-r2dbc",
        reason: "same shared @Nullable arm as java-jdbc.",
    },
    BackendNote {
        backend: "kotlin-exposed",
        reason: "same shared nullable arm as java-jdbc.",
    },
    BackendNote {
        backend: "kotlin-jdbc",
        reason: "same shared nullable arm as java-jdbc.",
    },
    BackendNote {
        backend: "kotlin-r2dbc",
        reason: "same shared nullable arm as java-jdbc.",
    },
    BackendNote {
        backend: "elixir-postgrex",
        reason: "One and Opt share one arm returning `nil` on a missing row, reproduced \
                  identically across every elixir-*.rs backend in this list.",
    },
    BackendNote {
        backend: "elixir-ecto",
        reason: "same shared nil-on-missing-row arm as elixir-postgrex.",
    },
    BackendNote {
        backend: "elixir-myxql",
        reason: "same shared nil-on-missing-row arm as elixir-postgrex.",
    },
    BackendNote {
        backend: "elixir-exqlite",
        reason: "same shared nil-on-missing-row arm as elixir-postgrex.",
    },
    BackendNote {
        backend: "elixir-tds",
        reason: "same shared nil-on-missing-row arm as elixir-postgrex.",
    },
    BackendNote {
        backend: "elixir-jamdb",
        reason: "same shared nil-on-missing-row arm as elixir-postgrex.",
    },
    BackendNote {
        backend: "ruby-pg",
        reason: "One and Opt share one arm returning `nil` on a missing row, reproduced \
                  identically across every ruby-*.rs backend in this list.",
    },
    BackendNote {
        backend: "ruby-mysql2",
        reason: "same shared nil-on-missing-row arm as ruby-pg.",
    },
    BackendNote {
        backend: "ruby-sqlite3",
        reason: "same shared nil-on-missing-row arm as ruby-pg.",
    },
    BackendNote {
        backend: "ruby-trilogy",
        reason: "same shared nil-on-missing-row arm as ruby-pg.",
    },
    BackendNote {
        backend: "ruby-tiny-tds",
        reason: "same shared nil-on-missing-row arm as ruby-pg.",
    },
    BackendNote {
        backend: "ruby-oci8",
        reason: "same shared nil-on-missing-row arm as ruby-pg.",
    },
    BackendNote {
        backend: "php-pdo",
        reason: "One and Opt share the return-type, docblock, and body arms, all rendering \
                  `?{Struct}` / `{Struct}|null` and `null` on a missing row, reproduced \
                  identically in php-amphp.",
    },
    BackendNote {
        backend: "php-amphp",
        reason: "same shared nullable arm as php-pdo.",
    },
];

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
