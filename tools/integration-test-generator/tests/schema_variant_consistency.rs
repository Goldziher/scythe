//! Closes GH #196 item 2: `schema_file` (`src/main.rs` `SCHEMA_FILE_OVERRIDES`)
//! controls which schema file a backend is *generated from*, but every
//! integration-test harness template hardcodes its own runtime schema
//! filename independently, so the schema a harness *applies* can silently
//! diverge from the one its queries were typed against.
//!
//! Full unification (making every harness read `{{ schema_file }}` from the
//! same override table `scythe.toml.jinja` uses) is not safe here: the
//! `redshift` and `oracle` variant files are not filename aliases of the same
//! content, they are deliberately different SQL — `schema_pg_compat.sql`
//! substitutes Redshift-only syntax (`IDENTITY`, `GETDATE()`) with
//! PostgreSQL-compatible syntax (`SERIAL`, `NOW()`) so the harness can run
//! against a real Postgres server in CI, and `schema_full.sql` adds Oracle
//! sequences/triggers that `sqlparser` cannot parse for type inference. A
//! harness is allowed to apply a different file than codegen used, as long
//! as the two files agree on table and column *shape* — the thing type
//! inference actually depends on. So this is a check, not a merge: it
//! verifies the pairs that are known to diverge by filename still agree by
//! shape, and fails if a future edit adds or removes a column in only one of
//! the pair.
//!
//! Reverting this check is not what should ever be needed to "fix" a
//! failure: a failure here means a table/column list drifted between the
//! codegen-time schema and the runtime-applied schema, which is exactly the
//! defect GH #196 item 2 describes.

use std::fs;
use std::path::Path;

/// Extracts, per `CREATE TABLE`, the ordered list of column names (table-level
/// constraints such as `PRIMARY KEY (...)`, `CONSTRAINT ...`, `FOREIGN KEY
/// (...)`, `UNIQUE (...)`, and `CHECK (...)` are not columns and are
/// skipped). Good enough for the hand-written fixture schemas under
/// `integration_tests/sql/`, which are simple single-line-per-column DDL with
/// no nested parentheses inside a column definition.
fn extract_tables(sql: &str) -> Vec<(String, Vec<String>)> {
    const TABLE_LEVEL_KEYWORDS: &[&str] = &["PRIMARY", "CONSTRAINT", "FOREIGN", "UNIQUE", "CHECK"];

    let mut tables = Vec::new();
    let mut lines = sql.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if !trimmed.starts_with("CREATE TABLE") {
            continue;
        }
        let table_name = trimmed
            .split_whitespace()
            .nth(2)
            .expect("CREATE TABLE line must name a table")
            .to_string();

        let mut columns = Vec::new();
        for body_line in lines.by_ref() {
            let body_trimmed = body_line.trim();
            if body_trimmed == ");" || body_trimmed == ")" {
                break;
            }
            if body_trimmed.is_empty() {
                continue;
            }
            let first_word = body_trimmed
                .split_whitespace()
                .next()
                .expect("non-empty table body line must have a first token");
            let keyword = first_word.trim_end_matches(',').to_uppercase();
            if TABLE_LEVEL_KEYWORDS.contains(&keyword.as_str()) {
                continue;
            }
            columns.push(first_word.to_string());
        }
        tables.push((table_name, columns));
    }
    tables
}

/// Asserts that two schema files, read from `integration_tests/sql/`, agree
/// on table names and per-table column names/order, even though their column
/// *types* and DDL dialect are allowed to differ (that divergence is the
/// entire reason the pair exists).
fn assert_same_shape(base_relative: &str, variant_relative: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../integration_tests/sql");
    let base_path = root.join(base_relative);
    let variant_path = root.join(variant_relative);

    let base_sql =
        fs::read_to_string(&base_path).unwrap_or_else(|error| panic!("reading {}: {error}", base_path.display()));
    let variant_sql =
        fs::read_to_string(&variant_path).unwrap_or_else(|error| panic!("reading {}: {error}", variant_path.display()));

    // ~keep Sorted by table name before comparing: a column's *position* is part of the
    // shape a generated row type is read against, but the order the tables are declared
    // in is not -- the Oracle pair genuinely declares `attachments` and `user_tags` in
    // opposite orders, and that difference cannot reach a query. Comparing the raw Vec
    // would fail on that alone and teach the next reader to relax the whole check.
    let mut base_tables = extract_tables(&base_sql);
    let mut variant_tables = extract_tables(&variant_sql);
    base_tables.sort_by(|a, b| a.0.cmp(&b.0));
    variant_tables.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(
        base_tables, variant_tables,
        "{base_relative} and {variant_relative} must declare the same tables, each with the \
         same columns in the same order (types/dialect may differ; table declaration order \
         may differ) — a harness that applies one at runtime while codegen used the other \
         must not see a different table shape than what queries were typed against"
    );
}

#[test]
fn oracle_schema_full_matches_schema_shape() {
    // csharp-oracle, elixir-jamdb-oracle, kotlin-jdbc-oracle, python-oracledb-oracle,
    // ruby-oci8-oracle, rust-sibyl-oracle and typescript-oracledb-oracle are all
    // generated from oracle/schema.sql (the SCHEMA_FILE_OVERRIDES default) but
    // every harness template applies oracle/schema_full.sql at runtime, because
    // schema_full.sql adds the sequences/triggers Oracle needs for
    // auto-increment and sqlparser cannot parse. That is safe only because the
    // two files declare identical table/column shapes; this test is what keeps
    // that true.
    assert_same_shape("oracle/schema.sql", "oracle/schema_full.sql");
}

#[test]
fn redshift_schema_pg_compat_matches_schema_shape() {
    // java-*-redshift, kotlin-*-redshift and go-pgx-redshift apply
    // redshift/schema_pg_compat.sql at runtime (it substitutes Redshift-only
    // syntax with PostgreSQL-compatible syntax so the harness can run against a
    // real Postgres server), while every redshift project's scythe.toml
    // generates from redshift/schema.sql (genuine Redshift syntax, needed for
    // accurate type inference). Safe only because both declare the same table
    // and column shape.
    assert_same_shape("redshift/schema.sql", "redshift/schema_pg_compat.sql");
}
