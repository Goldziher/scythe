//! GH #147: `json_array` is a claim about a driver, so it ratchets both ways.
//!
//! `degrade_unsupported_nested_structs` reads `json_array` straight off the
//! manifest (`backend.manifest().types.scalars`) with no backend code involved,
//! so a single line in a `.toml` decides whether a degraded nested aggregate
//! keeps the fact that it is a *list* or collapses to one opaque scalar. That
//! makes it exactly the shape of declaration #190 had to undo across 84
//! manifests: cheap to add, invisible when wrong, and indistinguishable from
//! real support until someone runs the driver.
//!
//! So every declaration is listed here with the evidence for it, and the list
//! fails in both directions -- an undeclared-but-listed entry fails just as a
//! declared-but-unlisted one does. Adding an entry requires evidence of the
//! named driver's own behaviour: the decode call scythe itself emits, or the
//! driver's documented default parsing. "It compiles" is not evidence; a
//! `String` mapping compiles and hands back raw JSON text.
//!
//! `json_nested` is deliberately *not* manifest-driven and is pinned the same
//! way for the opposite reason -- see the second test.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use scythe_backend::manifest::load_manifest;

/// Manifests allowed to declare `json_array`, each with why it is honest.
const JSON_ARRAY_ALLOWLIST: &[(&str, &str)] = &[
    (
        "elixir-postgrex",
        "Postgrex/Jason decodes a top-level JSON array to a list; verified live against PostgreSQL 16 (board #219).",
    ),
    (
        "elixir-ecto",
        "Ecto.Adapters.SQL.query runs raw SQL through the same Postgrex binary protocol as elixir-postgrex.",
    ),
    (
        "php-pdo",
        "php_pdo.rs emits json_decode($value, true) for a json column; PHP's json_decode yields array for both JSON shapes.",
    ),
    (
        "php-amphp",
        "php_amphp.rs independently emits the identical json_decode($value, true) call.",
    ),
    (
        "python-asyncpg",
        "asyncpg decodes json/jsonb through its own codec into native Python values; a top-level array becomes a list.",
    ),
    (
        "typescript-pg",
        "node-postgres auto-parses json/jsonb via its default OID type parsers (JSON.parse), with no caller configuration.",
    ),
    (
        "typescript-kysely",
        "Kysely's PostgresDialect wraps node-postgres unchanged for this manifest, so it inherits the same auto-parsing.",
    ),
    (
        "typescript-postgres",
        "postgres.js parses json/jsonb by default, giving the same shape guarantee as node-postgres.",
    ),
    (
        "ruby-pg",
        "ruby_pg.rs's ruby_coercion now emits JSON.parse(...) for a json/jsonb column (GH #147); the pg \
         gem itself does no client-side JSON decoding, so JSON.parse is scythe's own call, applied \
         uniformly regardless of the JSON document's top-level shape, and a top-level array becomes a \
         Ruby Array.",
    ),
];

/// The only manifests whose backend implements `generate_nested_struct_def`.
///
/// Unlike `json_array`, a `json_nested` line in a manifest cannot make nesting
/// work: `degrade_unsupported_nested_structs` branches on what the *backend*
/// returns, never on this key. Adding one without the matching backend override
/// would be a declaration that does nothing at all, which is why this list is
/// pinned rather than left to drift.
const JSON_NESTED_BACKENDS: &[&str] = &["go-pgx", "python-psycopg3", "rust-sqlx", "rust-tokio-postgres"];

fn manifest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("manifests")
}

/// Manifest stem -> whether it declares `key` in `[types.scalars]` or `[types.containers]`.
///
/// Reads the same parsed `BackendManifest` production code does (`load_manifest`, the entry
/// point `range_container_consistency.rs` also uses), rather than scanning the raw TOML text
/// for a `"{key} = "` line prefix. The raw-text scan used to diverge from what
/// `degrade_unsupported_nested_structs` actually reads
/// (`backend.manifest().types.scalars.contains_key("json_array")`): a declaration written
/// `json_array="x"` with no space, indented, or moved under a different table would be
/// invisible to this scan while still visible (or not) to production, in either direction --
/// exactly the kind of gate-measures-something-else-than-production gap this suite exists to
/// close (GH #147). `json_array` lives in `[types.scalars]` and `json_nested` in
/// `[types.containers]`; checking both tables keeps this function correct for either key
/// without hardcoding which table each one is in.
fn declarations_of(key: &str) -> BTreeMap<String, bool> {
    let mut found = BTreeMap::new();
    for entry in fs::read_dir(manifest_dir()).expect("read manifests dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("manifest stem")
            .to_string();
        let manifest = load_manifest(&path).unwrap_or_else(|e| panic!("{} must parse: {e}", path.display()));
        let declares = manifest.types.scalars.contains_key(key) || manifest.types.containers.contains_key(key);
        found.insert(stem, declares);
    }
    found
}

#[test]
fn json_array_declarations_match_the_allowlist() {
    let declared = declarations_of("json_array");
    let allowed: BTreeMap<&str, &str> = JSON_ARRAY_ALLOWLIST.iter().copied().collect();

    assert_eq!(
        allowed.len(),
        JSON_ARRAY_ALLOWLIST.len(),
        "JSON_ARRAY_ALLOWLIST has a duplicate manifest name"
    );

    let mut undocumented: Vec<&str> = Vec::new();
    for (stem, declares) in &declared {
        if *declares && !allowed.contains_key(stem.as_str()) {
            undocumented.push(stem);
        }
    }
    assert!(
        undocumented.is_empty(),
        "these manifests declare `json_array` with no entry in JSON_ARRAY_ALLOWLIST: {undocumented:?}\n\
         Add each one with the evidence for the driver's behaviour, or remove the declaration. A \
         `json_array` that the driver does not actually produce is the defect GH #147 exists for."
    );

    let mut stale: Vec<&str> = Vec::new();
    for stem in allowed.keys() {
        match declared.get(*stem) {
            Some(true) => {}
            Some(false) => stale.push(stem),
            None => panic!("JSON_ARRAY_ALLOWLIST names `{stem}`, which is not a manifest on disk"),
        }
    }
    assert!(
        stale.is_empty(),
        "these manifests are allowlisted for `json_array` but no longer declare it: {stale:?}\n\
         Remove the stale entries -- an allowlist that outlives what it describes is how the \
         `range` mappings in #190 drifted to 84 unusable declarations."
    );
}

#[test]
fn json_nested_is_declared_only_where_a_backend_implements_it() {
    let declared = declarations_of("json_nested");

    let mut actual: Vec<&str> = declared
        .iter()
        .filter(|(_, declares)| **declares)
        .map(|(stem, _)| stem.as_str())
        .collect();
    actual.sort_unstable();

    let mut expected = JSON_NESTED_BACKENDS.to_vec();
    expected.sort_unstable();

    assert_eq!(
        actual, expected,
        "`json_nested` must be declared exactly by the manifests whose backend overrides \
         `generate_nested_struct_def`. A manifest cannot grant nesting on its own -- \
         `degrade_unsupported_nested_structs` branches on the backend's return value, not on this \
         key -- so an added declaration here is inert, and a removed one means a backend lost its \
         override. Either way, change the backend and this list together."
    );
}
