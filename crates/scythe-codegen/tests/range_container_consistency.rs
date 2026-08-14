//! Regression tests for GH #190: the `range` container was declared in 84 of
//! 102 manifests, in seven mutually incompatible spellings, and nothing
//! asserted anything about it.
//!
//! Two independent inconsistencies had accumulated, and neither could be seen
//! from inside any single manifest:
//!
//! 1. **Presence did not track engine capability, in either direction.**
//!    `range` was declared for engines with no range type at all (mysql,
//!    mariadb, sqlite, mssql, oracle, duckdb, redshift) and omitted from
//!    several manifests of engines alongside siblings that declared it. Only
//!    snowflake was internally consistent, and only by accident: all seven of
//!    its manifests happened to omit the key.
//!
//! 2. **The spelling disagreed within a single language family**, and in one
//!    case within a single file. `python-asyncpg` said `tuple[{T}, {T}]`
//!    while `python-asyncpg.redshift` said `str` -- those are different
//!    types, not different spellings of one. `elixir-postgrex` said
//!    `string()` while `elixir-postgrex.redshift` said `String.t()`, and
//!    `string()` is not even the Elixir typespec for a binary (it is
//!    `[char()]`, a charlist); every Elixir manifest maps its own `string`
//!    scalar to `String.t()`, so those files contradicted themselves.
//!
//! # The direction taken
//!
//! `range` is now declared **only where the engine has a range type**, which
//! by the repository's own database documentation means PostgreSQL alone:
//!
//! - `website/src/content/docs/databases/postgresql.md` lists `INT4RANGE`,
//!   `INT8RANGE`, `TSRANGE`, `TSTZRANGE`, `DATERANGE` and `NUMRANGE` in its
//!   type-mapping table, mapped to `range<T>`.
//! - `redshift.md` -- "Range types | `int4range`, `tstzrange` | Not supported"
//! - `mysql.md` -- "Range types | Native | Not supported"
//! - `sqlite.md` -- "**Range types** -- not available."
//! - `cockroachdb.md` -- "Range types | ... | Not supported"
//!
//! MariaDB is a MySQL fork whose documentation page records only its deltas
//! from MySQL and never adds a range type; MSSQL, Oracle, Snowflake and
//! DuckDB have no PostgreSQL-style range *column* type either (DuckDB's
//! `range` is a table function, not a type). Dropping the key there is not a
//! degradation: `resolve_type` turns an undeclared container into
//! `BackendError::UnknownContainer("range")`, so a `range<T>` reaching a
//! MySQL target now fails generation loudly instead of quietly producing a
//! `string` field for a column that engine cannot have.
//!
//! # Runtime verification (this pass)
//!
//! `6e3b85e4` settled *where* `range` is declared but explicitly punted on
//! *whether the 19 survivors decode at runtime* -- three were flagged as
//! "believed wrong" from reasoning alone, never checked. This pass checked
//! all 19, against a live `postgres:14-alpine` container (`scythe-live-pg`,
//! `postgresql://scythe:scythe@localhost:55432/scythe_inspect_test`) seeded
//! with an `int4range`/`int8range`/`tsrange`/`tstzrange`/`daterange`/
//! `numrange` probe table, using each driver's real client library. Where a
//! client could not be run live (no ecosystem tooling reachable), the
//! decision fell back to the driver's own source, not to inference -- see
//! each line below.
//!
//! **Confirmed correct as a text mapping** (the driver hands back the range's
//! literal form, e.g. `"[1,10)"`, on both read and bind): `java-jdbc` /
//! `kotlin-jdbc` / `kotlin-exposed` (pgjdbc 42.7.4 `ResultSet.getString`,
//! live), `java-r2dbc` / `kotlin-r2dbc` (`org.postgresql:r2dbc-postgresql`
//! 1.0.7.RELEASE `Row.get(col, String.class)`, live), `typescript-pg` (`pg`
//! 8.x, live), `typescript-postgres` (`postgres` 8.x, live),
//! `typescript-kysely` (its PostgreSQL dialect wraps `pg`, so it inherits
//! that result), `php-pdo` (PDO pgsql, live), `php-amphp`
//! (`amphp/postgres` 2.2.1 over the `pgsql` extension, live), `ruby-pg`
//! (`pg` gem, live).
//!
//! **Confirmed correct as a typed mapping**: `rust-sqlx` --
//! `sqlx::postgres::types::PgRange<T>` has `Type`/`Encode`/`Decode` impls in
//! `sqlx-postgres` 0.9.0 for every element type this repo's range scalars
//! produce (`i32`, `i64`, `BigDecimal`/`Decimal`, `NaiveDate`,
//! `NaiveDateTime`, `DateTime<Tz>`), read from the vendored crate source.
//!
//! **Confirmed WRONG and fixed in this pass**:
//!
//! - `csharp-npgsql` said `string`; Npgsql 10.0.3 throws
//!   `InvalidCastException` from `GetString` on a range column (live) and
//!   decodes correctly through `GetFieldValue<NpgsqlTypes.NpgsqlRange<T>>`
//!   (live, both read and bind). Fixed to
//!   `NpgsqlTypes.NpgsqlRange<{T}>` -- fully qualified, matching how `inet`
//!   already spells `System.Net.IPAddress` in this same manifest, so no
//!   `[imports.rules]` entry is needed. `reader_method` in
//!   `csharp_npgsql.rs` has no special case for `range`; it falls through to
//!   `GetFieldValue<{lang_type}>`, so the manifest edit alone changes the
//!   emitted reader call.
//! - `go-pgx` said `string`; pgx v5.10.0 refuses to scan `int4range` into
//!   `*string` at all (`cannot scan int4range (OID 3904) in binary format
//!   into *string`, live) and decodes correctly into
//!   `pgtype.Range[T]` for every element type exercised (`int32`, `int64`,
//!   `time.Time`, `decimal.Decimal`, live). This one was not named as a
//!   suspect by #190 or board #198 -- it surfaced only from running the
//!   driver. Fixed to `pgtype.Range[{T}]`, with a new
//!   `"pgtype." = "\"github.com/jackc/pgx/v5/pgtype\""` import rule.
//! - `elixir-postgrex` and `elixir-ecto` said `String.t()`; Postgrex 0.22.4
//!   decodes every range OID into a `%Postgrex.Range{}` struct, never a
//!   binary, and rejects a plain string bound as a range parameter
//!   (`DBConnection.EncodeError`, live). Ecto's raw-SQL path
//!   (`Ecto.Adapters.SQL.query/4`, which is what this backend generates)
//!   delegates straight to Postgrex with no additional casting, so the same
//!   struct comes back through Ecto too -- confirmed by reading
//!   `elixir_ecto.rs`, not assumed. Both fixed to `Postgrex.Range.t()`
//!   (`Postgrex.Range`'s own `@type t`, from `postgrex/builtins.ex`); no
//!   `{T}` substitution exists on that struct (fields are `term`), and Elixir
//!   needs no import rule for a fully-qualified module reference.
//! - `python-asyncpg` and `python-psycopg3` said `tuple[{T}, {T}]`; asyncpg
//!   0.31.0 returns `asyncpg.Range` and psycopg 3.3.4 returns
//!   `psycopg.types.range.Range` (both live) -- neither is a tuple, neither
//!   supports tuple-unpacking (`asyncpg.Range` raises `TypeError: cannot
//!   unpack non-iterable Range object`), and both are themselves generic
//!   (`Range[int]` subscripts cleanly on both, live). Fixed to
//!   `asyncpg.Range[{T}]` (new `"asyncpg." = "import asyncpg"` rule) and
//!   `psycopg.types.range.Range[{T}]` (new
//!   `"psycopg.types.range." = "import psycopg.types.range"` rule). The two
//!   spellings differ because the two drivers ship genuinely different,
//!   mutually incompatible `Range` classes -- see
//!   `RANGE_SPELLING_EXCEPTIONS`.
//!
//! **Confirmed WRONG in the previous pass, now fixed**:
//!
//! - `rust-tokio-postgres` said `String`, then nothing at all (board #198's
//!   predecessor pass removed the declaration outright: `postgres-types`
//!   0.2.14 -- read from `~/.cargo/registry`, cargo is off-limits in this
//!   pass so this is source inspection, not a live run -- ships no `Range<T>`
//!   with a `FromSql`/`ToSql` impl, so no string could be named that both
//!   compiled and decoded correctly). This pass re-derived the crate surface
//!   from the same vendored source and found the gap closable without a new
//!   dependency: `postgres-protocol` 0.6.12 (a hard, non-optional dependency
//!   of `postgres-types` -- see its `Cargo.toml` and `postgres-types`'
//!   `use postgres_protocol::types::{self, ArrayDimension}` -- the same
//!   crate `postgres-types` itself uses to decode arrays) exports
//!   `types::range_from_sql`/`range_to_sql`/`Range`/`RangeBound`, the exact
//!   binary wire-format primitives a `FromSql`/`ToSql` impl needs. Fixed by
//!   hand-rolling `PgRange<T>` in `tokio_postgres.rs` on top of those
//!   primitives -- the same shape `generate_enum_def` already uses for
//!   enums, a `FromSql`/`ToSql` pair written directly against
//!   `tokio_postgres::types` (which re-exports `postgres_types::*`
//!   wholesale, confirmed from `tokio-postgres`'s own `src/types.rs`).
//!   Emitted once per output file, only when a generated fragment actually
//!   names `PgRange<` (`TokioPostgresBackend::file_header_for_results`),
//!   mirroring how `go_pgx.rs` gates its own conditional import.
//!   `postgres-protocol` is not re-exported under any `tokio_postgres::`
//!   path, so the generated crate needs it as a *direct* dependency to call
//!   it by name -- exactly the situation `postgres-types = { features =
//!   ["derive"] }` was already added to
//!   `tools/integration-test-generator/templates/Cargo.toml.integration.jinja`
//!   to solve, for the same reason. See that file for the matching
//!   `postgres-protocol` addition this pass needs but does not itself make
//!   (out of scope for a manifest/backend-only pass; tracked as a reported
//!   follow-up, not a silent gap).
//!
//! Fixed to `PgRange<{T}>`. This necessarily diverges from `rust-sqlx`'s
//! family-mate spelling (`sqlx::postgres::types::PgRange<{T}>`) because that
//! is a real type in a crate `rust-tokio-postgres` does not depend on --
//! see `RANGE_SPELLING_EXCEPTIONS`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use scythe_backend::manifest::{BackendManifest, load_manifest};
use scythe_backend::types::resolve_type;
use scythe_codegen::GeneratedCode;
use scythe_codegen::backends::get_backend;

/// Engines with a range column type, and therefore the only engines whose
/// manifests may declare the `range` container. See the module doc comment
/// for the per-engine evidence.
const RANGE_CAPABLE_ENGINES: &[&str] = &["postgresql"];

/// Total manifests expected on disk. A floor, not the count: a glob that has
/// stopped matching must fail rather than sweep nothing and pass.
const MANIFEST_FLOOR: usize = 102;

/// Manifests expected to declare `range` -- the PostgreSQL targets. Also a
/// floor.
const DECLARATION_FLOOR: usize = 19;

/// One entry in an exception list: the manifest file it exempts, and why.
/// Same shape as `opt_command_regression.rs`'s `BackendNote`, and for the
/// same reason -- the list has to document itself, since a ratchet can only
/// check pass/fail and never *why*.
struct ManifestNote {
    manifest: &'static str,
    reason: &'static str,
}

/// Manifests allowed to disagree with their engine's range capability --
/// either declaring `range` for an engine that has none, or omitting it for
/// an engine that has one.
///
/// Empty, and kept rather than deleted with its last entry: an empty list is
/// what makes the next mismatched declaration show up as a regression instead
/// of as the status quo. `rust-tokio-postgres` no longer needs an entry here
/// -- see the module doc comment's "now fixed" section -- it declares `range`
/// like every other PostgreSQL manifest.
const RANGE_CAPABILITY_EXCEPTIONS: &[ManifestNote] = &[];

/// Manifests allowed to spell `range` differently from the rest of their
/// language family.
const RANGE_SPELLING_EXCEPTIONS: &[ManifestNote] = &[
    ManifestNote {
        manifest: "python-psycopg3.toml",
        reason: "asyncpg and psycopg3 each ship their own Range class -- asyncpg.Range and \
                 psycopg.types.range.Range -- confirmed live to be two distinct classes (asyncpg.Range \
                 is not psycopg.types.range.Range, and each driver decodes a range column into its own \
                 class, never the other's). Normalizing python-psycopg3 onto asyncpg.Range[{T}] would \
                 produce a type hint the psycopg driver never actually returns.",
    },
    ManifestNote {
        manifest: "rust-tokio-postgres.toml",
        reason: "rust-sqlx spells range as sqlx::postgres::types::PgRange<{T}>, a real type sqlx-postgres \
                 ships with Type/Encode/Decode impls. tokio-postgres's own type crate (postgres-types \
                 0.2.14) has no equivalent, so rust-tokio-postgres.rs hand-rolls its own PgRange<T> -- a \
                 FromSql/ToSql pair built on postgres_protocol::types::range_from_sql/range_to_sql, \
                 emitted directly into the generated file rather than imported from a crate. Normalizing \
                 onto sqlx's fully-qualified spelling would name a type this backend's dependency tree \
                 does not have.",
    },
];

/// A manifest's `range` declaration as it exists on disk.
struct RangeDeclaration {
    file: String,
    language: String,
    engine: String,
    /// The `[types.containers] range` pattern, or `None` when the key is absent.
    pattern: Option<String>,
}

fn manifests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("manifests")
}

/// Every `*.toml` under `manifests/`, parsed through the same `load_manifest`
/// the product uses -- so this checks the files as scythe reads them, not as
/// a second parser in a test file happens to read them.
fn all_declarations() -> Vec<RangeDeclaration> {
    let dir = manifests_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("manifests dir {} must be readable: {error}", dir.display()))
        .map(|entry| entry.expect("readable dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();

    assert!(
        files.len() >= MANIFEST_FLOOR,
        "expected at least {MANIFEST_FLOOR} manifests under {}, found {} -- the sweep has gone \
         stale and this file is no longer checking anything",
        dir.display(),
        files.len()
    );

    files.iter().map(|path| declaration_for(path)).collect()
}

fn declaration_for(path: &Path) -> RangeDeclaration {
    let manifest: BackendManifest =
        load_manifest(path).unwrap_or_else(|error| panic!("{} must parse: {error}", path.display()));
    RangeDeclaration {
        file: path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("manifest file name must be UTF-8")
            .to_string(),
        language: manifest.backend.language.clone(),
        engine: manifest.backend.engine.clone(),
        pattern: manifest.types.containers.get("range").cloned(),
    }
}

/// Exception entries naming a manifest outside `eligible` are stale in the
/// most basic way: they exempt something the sweep never even looks at, so
/// the entry can never fire and no longer documents anything real.
/// `eligible` is every swept manifest for the capability list, and only the
/// manifests that still declare `range` for the spelling list.
fn unmatched_exceptions(
    list: &[ManifestNote],
    eligible: &BTreeSet<&str>,
    kind: &str,
    eligibility: &str,
) -> Vec<String> {
    list.iter()
        .filter(|note| !eligible.contains(note.manifest))
        .map(|note| {
            format!(
                "STALE {kind} EXCEPTION: {}: listed, but it {eligibility} -- delete its entry \
                 (reason on file: {})",
                note.manifest, note.reason
            )
        })
        .collect()
}

/// Invariant one: `range` is declared exactly where the engine has a range
/// type. A manifest that declares it for an engine without one is a
/// regression; a manifest that omits it for an engine with one is equally a
/// regression, because generation then hard-errors on a column that engine
/// genuinely supports. An exception listed here that has stopped diverging is
/// a stale entry and fails just as loudly, so the list cannot rot.
#[test]
fn range_is_declared_exactly_for_engines_that_have_a_range_type() {
    let declarations = all_declarations();
    let mut failures = Vec::new();

    for declaration in &declarations {
        let capable = RANGE_CAPABLE_ENGINES.contains(&declaration.engine.as_str());
        let declared = declaration.pattern.is_some();
        let listed = RANGE_CAPABILITY_EXCEPTIONS
            .iter()
            .find(|note| note.manifest == declaration.file.as_str());

        match (declared == capable, listed) {
            (true, Some(note)) => failures.push(format!(
                "STALE CAPABILITY EXCEPTION: {}: listed, but its range declaration now agrees \
                 with engine `{}` -- delete its entry (reason on file: {})",
                declaration.file, declaration.engine, note.reason
            )),
            (false, None) if declared => failures.push(format!(
                "UNSUPPORTED ENGINE: {} declares `range = {:?}` but engine `{}` has no range \
                 type -- drop the key, or add it to RANGE_CAPABILITY_EXCEPTIONS with evidence \
                 that the engine gained one",
                declaration.file,
                declaration.pattern.as_deref().unwrap_or_default(),
                declaration.engine
            )),
            (false, None) => failures.push(format!(
                "MISSING: {} omits `range` but engine `{}` has a range type -- resolve_type will \
                 fail with UnknownContainer(\"range\") on any range column",
                declaration.file, declaration.engine
            )),
            _ => {}
        }
    }

    let all_files: BTreeSet<&str> = declarations
        .iter()
        .map(|declaration| declaration.file.as_str())
        .collect();
    failures.extend(unmatched_exceptions(
        RANGE_CAPABILITY_EXCEPTIONS,
        &all_files,
        "CAPABILITY",
        "names no manifest under manifests/",
    ));

    // ~keep Asserted before the floor so a single dropped key reports as the
    // MISSING line naming the file, not as an off-by-one on a count.
    assert!(
        failures.is_empty(),
        "{} of {} manifests need attention:\n{}",
        failures.len(),
        declarations.len(),
        failures.join("\n")
    );

    let declaring = declarations
        .iter()
        .filter(|declaration| declaration.pattern.is_some())
        .count();
    assert!(
        declaring >= DECLARATION_FLOOR,
        "expected at least {DECLARATION_FLOOR} manifests to declare `range`, found {declaring} -- \
         either RANGE_CAPABLE_ENGINES was narrowed without updating this floor, or the sweep \
         stopped seeing the PostgreSQL manifests"
    );
}

/// Invariant two: within one language, every manifest that declares `range`
/// spells it identically. This is what seven spellings of one idea could not
/// survive, and what stops an eighth being added silently: a language family
/// is allowed exactly one mapping, and any second one has to be argued for in
/// `RANGE_SPELLING_EXCEPTIONS` with a reason.
///
/// The stale direction matters as much: an exempt manifest that has drifted
/// back onto its family's spelling no longer needs the exemption, and leaving
/// the line would let a future genuine divergence hide behind it.
#[test]
fn each_language_declares_one_range_spelling() {
    let declarations = all_declarations();
    let mut failures = Vec::new();

    let mut by_language: BTreeMap<&str, Vec<&RangeDeclaration>> = BTreeMap::new();
    for declaration in &declarations {
        if declaration.pattern.is_some() {
            by_language
                .entry(declaration.language.as_str())
                .or_default()
                .push(declaration);
        }
    }

    for (language, group) in &by_language {
        let (exempt, plain): (Vec<&RangeDeclaration>, Vec<&RangeDeclaration>) =
            group.iter().copied().partition(|declaration| {
                RANGE_SPELLING_EXCEPTIONS
                    .iter()
                    .any(|note| note.manifest == declaration.file.as_str())
            });

        let spellings: BTreeSet<&str> = plain
            .iter()
            .filter_map(|declaration| declaration.pattern.as_deref())
            .collect();

        if spellings.len() > 1 {
            let detail: Vec<String> = plain
                .iter()
                .map(|declaration| {
                    format!(
                        "  {} = {:?}",
                        declaration.file,
                        declaration.pattern.as_deref().unwrap_or_default()
                    )
                })
                .collect();
            failures.push(format!(
                "DIVERGENT SPELLING: {language} declares {} different `range` mappings; a \
                 language family gets one:\n{}",
                spellings.len(),
                detail.join("\n")
            ));
        }

        // ~keep With every manifest of a language exempt there is no family
        // spelling left to compare against, so staleness is unknowable rather
        // than false -- skipping is the only sound answer, not a pass.
        let Some(baseline) = spellings.iter().next().copied() else {
            continue;
        };

        for declaration in &exempt {
            if declaration.pattern.as_deref() == Some(baseline) {
                let note = RANGE_SPELLING_EXCEPTIONS
                    .iter()
                    .find(|note| note.manifest == declaration.file.as_str())
                    .expect("partitioned as exempt, so it is listed");
                failures.push(format!(
                    "STALE SPELLING EXCEPTION: {}: listed, but it now spells `range` as {baseline:?}, \
                     the same as the rest of {language} -- delete its entry (reason on file: {})",
                    declaration.file, note.reason
                ));
            }
        }
    }

    let declaring: BTreeSet<&str> = declarations
        .iter()
        .filter(|declaration| declaration.pattern.is_some())
        .map(|declaration| declaration.file.as_str())
        .collect();
    failures.extend(unmatched_exceptions(
        RANGE_SPELLING_EXCEPTIONS,
        &declaring,
        "SPELLING",
        "no longer declares `range` at all, so it can never diverge",
    ));

    assert!(
        !by_language.is_empty(),
        "no manifest declares `range` at all -- the sweep matched nothing and this test would \
         pass vacuously"
    );

    assert!(
        failures.is_empty(),
        "{} language families need attention:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Invariant three, `rust-tokio-postgres` only: a manifest string changing is
/// not evidence the mapping decodes. This drives the real backend through
/// `resolve_type` and `file_header_for_results` and checks the code it
/// actually emits, the same standard `e850c2b0` demanded of the six mappings
/// it fixed.
#[test]
fn rust_tokio_postgres_range_emits_a_working_pgrange_wrapper() {
    let backend = get_backend("rust-tokio-postgres", "postgresql").expect("rust-tokio-postgres/postgresql");

    let resolved = resolve_type("range<int32>", backend.manifest(), false).expect("range<int32> must resolve");
    assert_eq!(
        resolved.as_ref(),
        "PgRange<i32>",
        "the manifest's range mapping must substitute {{T}} the same way every other container does"
    );

    // A file with no range column must not carry the wrapper: nothing would
    // reach it, and shipping it anyway is exactly the kind of unreachable
    // "mapping" #198 exists to catch even when it happens to compile.
    let no_range = [GeneratedCode::build(|c| {
        c.row_struct = Some("pub struct GetUserRow {\n    pub id: i32,\n}\n".to_string());
    })];
    let header_without = backend.file_header_for_results(&no_range);
    assert!(
        !header_without.contains("PgRange"),
        "a file with no range column must not carry the PgRange wrapper:\n{header_without}"
    );

    // A file that does name PgRange<...> -- exactly what row/model struct
    // generation does for a `range<T>` column -- must carry a complete,
    // correctly-shaped wrapper: an Empty/Range enum (not the empty-collapses-
    // into-unbounded shape some other clients use), FromSql and ToSql impls
    // keyed on tokio_postgres::types::Kind::Range for the element type, and
    // decode/encode routed through postgres_protocol's binary range
    // primitives rather than a text/string round-trip.
    let with_range = [GeneratedCode::build(|c| {
        c.row_struct = Some(format!("pub struct GetSpanRow {{\n    pub span: {resolved},\n}}\n"));
    })];
    let header_with = backend.file_header_for_results(&with_range);

    for needle in [
        "pub enum PgRange<T>",
        "Empty,",
        "Range {",
        "std::ops::Bound<T>",
        "impl<'a, T> tokio_postgres::types::FromSql<'a> for PgRange<T>",
        "tokio_postgres::types::Kind::Range(element)",
        "postgres_protocol::types::range_from_sql(raw)?",
        "postgres_protocol::types::Range::Empty => Ok(PgRange::Empty)",
        "impl<T> tokio_postgres::types::ToSql for PgRange<T>",
        "postgres_protocol::types::empty_range_to_sql(out)",
        "postgres_protocol::types::range_to_sql(",
        "tokio_postgres::types::to_sql_checked!();",
    ] {
        assert!(
            header_with.contains(needle),
            "PgRange support text must contain {needle:?}, got:\n{header_with}"
        );
    }

    // The wrapper is emitted exactly once even if several fragments in the
    // same file name it -- `file_header_for_results` is a file-level hook,
    // not a per-fragment one, so a second range column must not double the
    // definition and hand `rustc` an E0428 duplicate-type error.
    let two_range_fragments = [
        GeneratedCode::build(|c| {
            c.row_struct = Some(format!("pub struct GetSpanRow {{\n    pub span: {resolved},\n}}\n"));
        }),
        GeneratedCode::build(|c| {
            c.model_struct = Some(format!("pub struct Booking {{\n    pub during: {resolved},\n}}\n"));
        }),
    ];
    let header_twice = backend.file_header_for_results(&two_range_fragments);
    assert_eq!(
        header_twice.matches("pub enum PgRange<T>").count(),
        1,
        "PgRange must be defined exactly once per file regardless of how many fragments use it:\n{header_twice}"
    );
}
