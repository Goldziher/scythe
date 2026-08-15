//! Closes GH #196 item 2: `schema_file` (`src/main.rs` `SCHEMA_FILE_OVERRIDES`,
//! now keyed by engine rather than by backend name) controls which schema
//! file a backend is *generated from*, and every integration-test harness
//! template now reads `{{ schema_file }}` for its runtime schema filename
//! too — oracle backends generate from and apply `schema_full.sql`, redshift
//! backends generate from and apply `schema_pg_compat.sql`. Before this fix,
//! only 2 of 9 oracle backends and 0 of 14 redshift backends had a matching
//! override, so the rest generated code from `schema.sql` while their
//! harness applied a different file at runtime.
//!
//! `oracle/schema.sql` and `redshift/schema.sql` (the bare, non-override
//! variants) are no longer read by any generated codegen or harness — every
//! backend for those two engines now uses the same file for both halves.
//! They are kept as fixtures rather than deleted: `oracle/schema.sql` is the
//! plain-DDL reference a human reads to see the table shape without the
//! sequence/trigger noise, and `redshift/schema.sql` documents genuine
//! Redshift syntax (`IDENTITY`, `GETDATE()`) in case a future backend needs
//! to run against a real Redshift cluster instead of a PG-compatible stand-in.
//! Both are proven parseable by `sqlparser` already — `go-godror-oracle` and
//! `java-jdbc-oracle` generated successfully from `schema_full.sql` before
//! this fix, and `schema_pg_compat.sql` is strictly simpler PostgreSQL syntax
//! than what the `postgresql` engine already parses.
//!
//! This test remains valuable as a guard, not a merge: nothing generated
//! reads the bare variants anymore, so nothing else would notice if a future
//! edit added or removed a column in only one half of a pair. A failure here
//! means the shape has drifted between two files documenting what is
//! supposed to be the same table set — the class of defect GH #196 item 2
//! describes.

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
    // All 9 oracle backends now generate from and apply oracle/schema_full.sql
    // (SCHEMA_FILE_OVERRIDES is keyed on engine == "oracle"). oracle/schema.sql
    // is no longer read by anything generated; it survives as a plain-DDL
    // reference. This test is what keeps it from silently drifting out of
    // sync with the file everything actually uses.
    assert_same_shape("oracle/schema.sql", "oracle/schema_full.sql");
}

#[test]
fn redshift_schema_pg_compat_matches_schema_shape() {
    // All 14 redshift backends now generate from and apply
    // redshift/schema_pg_compat.sql (SCHEMA_FILE_OVERRIDES is keyed on
    // engine == "redshift"). redshift/schema.sql, with genuine Redshift
    // syntax (IDENTITY, GETDATE()), is no longer read by anything generated;
    // it survives in case a future backend runs against a real Redshift
    // cluster. This test is what keeps it from silently drifting out of sync
    // with the file everything actually uses.
    assert_same_shape("redshift/schema.sql", "redshift/schema_pg_compat.sql");
}
