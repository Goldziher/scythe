//! Deterministic schema fingerprinting, used for provenance headers (#68)
//! and drift detection (#79).
//!
//! [`Catalog::fingerprint`] reduces a parsed schema to a short, stable tag
//! that changes if and only if the schema *shape* changes. It deliberately
//! does **not** hash the DDL text: two schema files that differ only in
//! whitespace, comments, or top-level statement order must fingerprint
//! identically, while a real change (a column becoming nullable, a column
//! being reordered, a table being added) must not.
//!
//! This module is a child of [`super`] specifically so it can read
//! [`Catalog`]'s private fields directly — no new public accessors were
//! added to `Catalog` for this.

use ahash::AHashMap;

use sha2::{Digest, Sha256};

use crate::dialect::SqlDialect;

use super::Catalog;

/// Version tag for the fingerprint algorithm itself. Bump this if the
/// canonical form or hash truncation ever changes, so old and new
/// fingerprints are never mistaken for one another.
const FINGERPRINT_ALGORITHM_TAG: &str = "sch1";

/// Number of leading hash bytes kept (rendered as `2 * TRUNCATED_BYTES` hex
/// characters).
const TRUNCATED_BYTES: usize = 8;

impl Catalog {
    /// Compute a deterministic fingerprint of this catalog's schema shape.
    ///
    /// The result is a short tag of the form `sch1:<16 hex chars>`. Two
    /// catalogs produce the same fingerprint if and only if they have the
    /// same tables (name, columns, column order, column type, column
    /// nullability, primary-key flags), the same enum types (name, ordered
    /// values), the same composite types (name, ordered fields), and the
    /// same [`SqlDialect`].
    ///
    /// # Stability guarantees
    ///
    /// - **Reformat-invariant**: DDL text is never hashed directly, so
    ///   whitespace, comments, and top-level statement reordering that do
    ///   not change the schema produce the same fingerprint.
    /// - **Process-invariant**: map keys are sorted explicitly before
    ///   hashing. `Catalog`'s maps are [`AHashMap`], whose iteration order
    ///   is seeded once per process — relying on it would make the
    ///   fingerprint of an unchanged schema differ from one run to the
    ///   next.
    ///
    /// # What participates
    ///
    /// - Table names, and each column's name, resolved SQL type,
    ///   nullability, and primary-key flag, **in declared order** (column
    ///   order is positional and semantic, so it is never sorted).
    /// - Enum type names and their values, in declared order.
    /// - Composite type names and their fields (name + type), in declared
    ///   order.
    /// - The dialect this catalog was parsed with, as the 6-variant
    ///   [`SqlDialect`] rather than the 9-way engine alias — so `mysql` and
    ///   `mariadb`, which both resolve to [`SqlDialect::MySQL`], never
    ///   register as drift against each other.
    ///
    /// # What is excluded, deliberately
    ///
    /// - `Column.default`: it is `expr.to_string()` of the `sqlparser` AST,
    ///   which churns across `sqlparser` version bumps and has no downstream
    ///   consumer.
    /// - scythe's own version: it is not part of `Catalog` at all. If it
    ///   participated in this hash, every scythe release would make every
    ///   previously generated artifact report as drifted. The version still
    ///   appears in generated file headers, just as an independent field
    ///   alongside (not folded into) this fingerprint.
    pub fn fingerprint(&self) -> String {
        let canonical = self.canonical_form();
        let digest = Sha256::digest(canonical.as_bytes());
        let hex: String = digest[..TRUNCATED_BYTES].iter().map(|b| format!("{b:02x}")).collect();
        format!("{FINGERPRINT_ALGORITHM_TAG}:{hex}")
    }

    /// Render this catalog into a line-oriented, tab-separated canonical
    /// form suitable for hashing. Never render via `{:?}` (`Debug`) — that
    /// output is not a stable contract and can change on any dependency or
    /// compiler bump.
    fn canonical_form(&self) -> String {
        let mut lines: Vec<String> = Vec::new();

        lines.push(format!("dialect\t{}", dialect_tag(self.dialect)));

        for (key, table) in canonical_entries(self.dialect, &self.tables) {
            lines.push(format!("table\t{key}\t{}", table.columns.len()));
            for (idx, column) in table.columns.iter().enumerate() {
                lines.push(format!(
                    "column\t{key}\t{idx}\t{}\t{}\t{}\t{}",
                    column.name, column.sql_type, column.nullable, column.primary_key
                ));
            }
        }

        for (key, enum_type) in canonical_entries(self.dialect, &self.enums) {
            lines.push(format!("enum\t{key}\t{}", enum_type.values.join("|")));
        }

        for (key, composite) in canonical_entries(self.dialect, &self.composites) {
            let fields = composite
                .fields
                .iter()
                .map(|field| format!("{}:{}", field.name, field.sql_type))
                .collect::<Vec<_>>()
                .join("|");
            lines.push(format!("composite\t{key}\t{fields}"));
        }

        lines.join("\n")
    }
}

/// Stable string tag for a dialect. Deliberately distinct from
/// [`SqlDialect::from_str`]'s engine-alias parsing (`mariadb`, `crdb`,
/// `duckdb`, `redshift`, ... all collapse to one of these 6 variants before
/// this function ever sees them).
fn dialect_tag(dialect: SqlDialect) -> &'static str {
    match dialect {
        SqlDialect::PostgreSQL => "postgresql",
        SqlDialect::MySQL => "mysql",
        SqlDialect::SQLite => "sqlite",
        SqlDialect::MsSql => "mssql",
        SqlDialect::Oracle => "oracle",
        SqlDialect::Snowflake => "snowflake",
    }
}

/// Return `(key, value)` pairs from `map`, sorted by key, with a PostgreSQL
/// leading `public.` schema qualifier stripped — mirroring
/// [`Catalog::get_table`], which already treats `public.users` and `users`
/// as the same table.
///
/// If stripping would make two distinct raw keys collide (e.g. both
/// `public.users` and a literal bare `users` exist in the same catalog),
/// stripping is abandoned for the whole map and raw keys are used instead.
/// Merging colliding entries would silently drop one of them from the
/// fingerprint; falling back to raw keys keeps both distinguishable.
fn canonical_entries<T>(dialect: SqlDialect, map: &AHashMap<String, T>) -> Vec<(String, &T)> {
    let mut entries: Vec<(String, &T)> = if dialect == SqlDialect::PostgreSQL {
        let stripped: Vec<(String, &T)> = map
            .iter()
            .map(|(key, value)| (strip_public_schema(key), value))
            .collect();

        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::with_capacity(stripped.len());
        let collides = stripped.iter().any(|(key, _)| !seen.insert(key.as_str()));

        if collides {
            map.iter().map(|(key, value)| (key.clone(), value)).collect()
        } else {
            stripped
        }
    } else {
        map.iter().map(|(key, value)| (key.clone(), value)).collect()
    };

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// Strip a single leading `public.` schema qualifier, if present.
fn strip_public_schema(key: &str) -> String {
    key.strip_prefix("public.").unwrap_or(key).to_string()
}

#[cfg(test)]
mod tests {
    use crate::dialect::SqlDialect;

    use super::Catalog;

    #[test]
    fn test_reformatted_ddl_produces_same_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL);"])
        .unwrap();

        let b = Catalog::from_ddl(
            &["-- orders first this time, extra whitespace and comments throughout\n\
             CREATE   TABLE   orders (\n  id INTEGER PRIMARY KEY, -- pk\n  user_id INTEGER NOT NULL\n);\n\n\
             /* users table */\n\
             CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL);"],
        )
        .unwrap();

        assert_eq!(
            a.fingerprint(),
            b.fingerprint(),
            "whitespace, comments, and top-level statement order must not affect the fingerprint"
        );
    }

    #[test]
    fn test_nullability_change_produces_different_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER, name TEXT);"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER, name TEXT NOT NULL);"]).unwrap();

        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn test_column_reorder_produces_different_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE t (a INTEGER, b TEXT);"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TABLE t (b TEXT, a INTEGER);"]).unwrap();

        assert_ne!(
            a.fingerprint(),
            b.fingerprint(),
            "column order is positional and must be part of the hash"
        );
    }

    #[test]
    fn test_column_default_change_produces_same_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER, age INTEGER DEFAULT 0);"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER, age INTEGER DEFAULT 1);"]).unwrap();

        assert_eq!(
            a.fingerprint(),
            b.fingerprint(),
            "Column.default must be excluded from the fingerprint"
        );
    }

    #[test]
    fn test_table_added_produces_different_hash() {
        let a = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER);"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TABLE t (id INTEGER); CREATE TABLE u (id INTEGER);"]).unwrap();

        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn test_enum_and_composite_participate() {
        let a = Catalog::from_ddl(&["CREATE TYPE mood AS ENUM ('sad', 'happy');"]).unwrap();
        let b = Catalog::from_ddl(&["CREATE TYPE mood AS ENUM ('sad', 'happy', 'ok');"]).unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());

        let c = Catalog::from_ddl(&["CREATE TYPE address AS (street TEXT, city TEXT);"]).unwrap();
        let d = Catalog::from_ddl(&["CREATE TYPE address AS (street TEXT, city TEXT, zip INTEGER);"]).unwrap();
        assert_ne!(c.fingerprint(), d.fingerprint());
    }

    #[test]
    fn test_dialect_participates() {
        let pg = Catalog::from_ddl_with_dialect(&["CREATE TABLE t (id INTEGER);"], &SqlDialect::PostgreSQL).unwrap();
        let mysql = Catalog::from_ddl_with_dialect(&["CREATE TABLE t (id INT);"], &SqlDialect::MySQL).unwrap();

        assert_ne!(pg.fingerprint(), mysql.fingerprint());
    }

    #[test]
    fn test_mysql_and_mariadb_alias_share_one_dialect_variant() {
        // `mariadb` is not a distinct `SqlDialect` variant; `from_str` folds
        // it into `SqlDialect::MySQL`, so there is nothing further for the
        // fingerprint to distinguish -- this test documents that guarantee
        // at the alias-resolution boundary the fingerprint depends on.
        assert_eq!(SqlDialect::from_str("mysql"), SqlDialect::from_str("mariadb"));
    }

    #[test]
    fn test_public_prefix_stripped_when_no_collision() {
        let a = Catalog::from_ddl_with_dialect(&["CREATE TABLE public.users (id INTEGER);"], &SqlDialect::PostgreSQL)
            .unwrap();
        let b = Catalog::from_ddl_with_dialect(&["CREATE TABLE users (id INTEGER);"], &SqlDialect::PostgreSQL).unwrap();

        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn test_public_schema_collision_falls_back_to_raw_keys() {
        let base = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE public.users (id INTEGER); CREATE TABLE users (name TEXT);"],
            &SqlDialect::PostgreSQL,
        )
        .unwrap();
        let fp_base = base.fingerprint();

        let changed_bare = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE public.users (id INTEGER); CREATE TABLE users (name TEXT, extra BOOLEAN);"],
            &SqlDialect::PostgreSQL,
        )
        .unwrap();
        assert_ne!(
            fp_base,
            changed_bare.fingerprint(),
            "changing the bare `users` table must change the hash even though `public.users` also exists"
        );

        let changed_qualified = Catalog::from_ddl_with_dialect(
            &["CREATE TABLE public.users (id INTEGER, extra BOOLEAN); CREATE TABLE users (name TEXT);"],
            &SqlDialect::PostgreSQL,
        )
        .unwrap();
        assert_ne!(
            fp_base,
            changed_qualified.fingerprint(),
            "changing `public.users` must change the hash even though bare `users` also exists"
        );
    }

    /// Line prefix the child half prints its fingerprint behind. Shared by
    /// both halves so the writer and the reader cannot drift.
    const CHILD_FINGERPRINT_PREFIX: &str = "SCH1_CHILD_FINGERPRINT=";

    /// Fully qualified name of [`cross_process_child_prints_fingerprint`],
    /// as libtest's `--exact` filter spells it.
    const CHILD_TEST_NAME: &str = "catalog::fingerprint::tests::cross_process_child_prints_fingerprint";

    /// The child half of [`test_cross_process_ordering_independence`]:
    /// prints this process's fingerprint of the shared sample catalog.
    ///
    /// Split out into its own `#[test]` rather than living behind an
    /// environment-variable branch inside the parent. With an env-gated
    /// branch, a `SCYTHE_FINGERPRINT_CHILD` that happened to already be set
    /// in the ambient environment sent the *parent* run down the child path:
    /// it printed a fingerprint, returned, and asserted nothing at all,
    /// while still reporting as a pass. Which process is the child is now a
    /// property of which test libtest was asked to run — something only the
    /// parent decides, via `--exact`, and something no inherited environment
    /// can forge.
    ///
    /// Running this on its own (as a normal `cargo test` sweep does) is
    /// harmless and asserts nothing; it exists to be spawned.
    #[test]
    fn cross_process_child_prints_fingerprint() {
        println!(
            "{CHILD_FINGERPRINT_PREFIX}{}",
            cross_process_sample_catalog().fingerprint()
        );
    }

    /// Cross-process ordering independence.
    ///
    /// The failure this guards against is a `canonical_form` that leaked
    /// `AHashMap` iteration order into the hash, making the same unchanged
    /// schema fingerprint differently from one run to the next — which would
    /// report as permanent, unfixable SC-PRV01 drift in CI.
    ///
    /// A second process is required to catch it. `ahash`'s `RandomState`
    /// draws on two sources, and both differ across processes: a set of
    /// fixed seeds initialized once per process, and a per-instance value
    /// from `DefaultRandomSource::gen_hasher_seed`, which is a static
    /// counter seeded from a static's (ASLR-dependent) address. Note the
    /// per-instance half means the claim that a single process's maps share
    /// one iteration order is false — two `AHashMap`s built in the same
    /// process already disagree, so an in-process comparison would be
    /// *probabilistic*: it could catch an order-dependent `canonical_form`,
    /// but only by luck, and it would say nothing about the per-process
    /// fixed seeds that are the actual run-to-run variable. Spawning a real
    /// second process exercises both sources at once, which is why the
    /// design is right regardless.
    ///
    /// This re-invokes the current test binary (`current_exe()`), asking it
    /// to run [`cross_process_child_prints_fingerprint`] and nothing else,
    /// then compares what the child printed against its own in-process
    /// fingerprint.
    #[test]
    fn test_cross_process_ordering_independence() {
        let parent_fingerprint = cross_process_sample_catalog().fingerprint();

        let exe = std::env::current_exe().expect("current test binary path");
        let output = std::process::Command::new(exe)
            .arg("--exact")
            .arg(CHILD_TEST_NAME)
            .arg("--nocapture")
            .output()
            .expect("failed to spawn child test process");

        assert!(
            output.status.success(),
            "child process failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let child_fingerprint = stdout
            .lines()
            .find_map(|line| line.strip_prefix(CHILD_FINGERPRINT_PREFIX))
            .unwrap_or_else(|| {
                panic!(
                    "child process did not print a fingerprint -- if `{CHILD_TEST_NAME}` was renamed, \
                     update CHILD_TEST_NAME. stdout was:\n{stdout}"
                )
            });

        assert_eq!(
            parent_fingerprint, child_fingerprint,
            "fingerprint must be identical across independently seeded processes"
        );
    }

    /// A catalog with enough distinct map keys (tables, enums, composites)
    /// that two independently seeded `AHashMap` random states are very
    /// likely to disagree on raw iteration order -- giving the
    /// cross-process test something real to catch.
    fn cross_process_sample_catalog() -> Catalog {
        Catalog::from_ddl(&["CREATE TABLE alpha (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE bravo (id INTEGER PRIMARY KEY, alpha_id INTEGER NOT NULL);
             CREATE TABLE charlie (id INTEGER PRIMARY KEY, note TEXT);
             CREATE TABLE delta (id INTEGER PRIMARY KEY, amount NUMERIC(10,2));
             CREATE TABLE echo (id INTEGER PRIMARY KEY, active BOOLEAN NOT NULL);
             CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
             CREATE TYPE status AS ENUM ('pending', 'done');
             CREATE TYPE address AS (street TEXT, city TEXT, zip INTEGER);
             CREATE TYPE point AS (x INTEGER, y INTEGER);"])
        .unwrap()
    }
}
