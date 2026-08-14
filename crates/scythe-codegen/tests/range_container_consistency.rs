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
//! # What is deliberately still a text mapping
//!
//! Of the 19 surviving declarations only `rust-sqlx` names a decodable type
//! (`sqlx::postgres::types::PgRange<T>`). The rest render the range's text
//! form, which for several drivers is exactly what comes back: node-postgres
//! and postgres.js have no type parser registered for range OIDs and hand
//! back the raw `"[1,4)"` string, JDBC's `getString` returns the same, and
//! PDO's pgsql driver stringifies everything. For those, `string`/`String` is
//! lossy but true.
//!
//! ~keep It is NOT true everywhere, and a green run of this file must not be
//! read as saying otherwise. This file checks that each language family
//! declares ONE spelling; it does not and cannot check that the spelling is
//! what the driver returns at runtime. Three surviving mappings are believed
//! wrong and are tracked separately rather than papered over here, because
//! correcting them needs reader changes under `src/backends/` plus
//! `[imports.rules]` entries, and none of it was verified against a live
//! database:
//!
//! - `csharp-npgsql` maps to `string`, but Npgsql materializes `int4range`
//!   as `NpgsqlRange<int>`, so a `GetString` would throw.
//! - `rust-tokio-postgres` maps to `String`, but `postgres-types`'
//!   `FromSql for String` accepts only TEXT/VARCHAR/BPCHAR/NAME/UNKNOWN and
//!   would reject a range OID.
//! - `python-asyncpg` / `python-psycopg3` map to `tuple[{T}, {T}]`, but both
//!   drivers return a `Range` object, never a tuple.
//!
//! Those are now confined to PostgreSQL, where a range can actually occur and
//! the mapping is reviewable, instead of being sprayed across eight engines
//! where they were pure noise.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use scythe_backend::manifest::{BackendManifest, load_manifest};

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
/// of as the status quo.
const RANGE_CAPABILITY_EXCEPTIONS: &[ManifestNote] = &[];

/// Manifests allowed to spell `range` differently from the rest of their
/// language family.
const RANGE_SPELLING_EXCEPTIONS: &[ManifestNote] = &[ManifestNote {
    manifest: "rust-sqlx.toml",
    reason: "sqlx is the only driver in the tree that ships a range type with a real Decode impl \
             (sqlx::postgres::types::PgRange<T>), so it is the one backend that can hand back a \
             typed range instead of its text form. Normalizing it onto rust-tokio-postgres' \
             String would throw away the only correct mapping in the file to buy uniformity.",
}];

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
