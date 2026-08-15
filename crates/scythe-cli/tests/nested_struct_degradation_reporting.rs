//! GH #147: `scythe generate` silently narrowed a nested aggregate.
//!
//! When a backend cannot build a struct for a `json_agg`/`row_to_json` column,
//! `degrade_unsupported_nested_structs` rewrites that column to an opaque
//! `json`/`json_array` scalar. The file was written, the command exited 0, and
//! nothing anywhere said the structured row the user asked for had collapsed
//! into a string they now have to parse themselves.
//!
//! Scope, since the issue's own numbers overstate it: `catalog_has_nested_aggregates`
//! only infers a nested aggregate for the PostgreSQL dialect on a postgresql-family
//! engine, so only the 19 postgresql-engine manifests reach this path at all. Of
//! those, 4 build a real struct, 8 keep the array shape via `json_array`, and 7
//! collapse to plain `json` -- `java-jdbc` among them, which is what this test drives.
//!
//! Reported, not fatal: failing every degrading backend would break working setups.
//! The point is that the narrowing is visible, not that it stops the build.

use std::process::{Command, Output};

use tempfile::TempDir;

fn scythe_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scythe"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const SCHEMA_SQL: &str = "\
CREATE TABLE users (id bigint PRIMARY KEY, name text NOT NULL);
CREATE TABLE orders (id bigint PRIMARY KEY, user_id bigint NOT NULL REFERENCES users (id), total numeric NOT NULL);
";

const QUERY_SQL: &str = "\
-- @name GetUserOrders
-- @returns :many
SELECT u.id, json_agg(o.*) AS orders FROM users u JOIN orders o ON o.user_id = u.id GROUP BY u.id;
";

fn write_project(dir: &std::path::Path, backend: &str) -> String {
    std::fs::write(dir.join("schema.sql"), SCHEMA_SQL).unwrap();
    std::fs::write(dir.join("queries.sql"), QUERY_SQL).unwrap();
    let config = format!(
        "[scythe]\nversion = \"1\"\n\n\
         [[sql]]\nname = \"main\"\nengine = \"postgresql\"\n\
         schema = [\"schema.sql\"]\nqueries = [\"queries.sql\"]\n\n\
         [[sql.gen]]\nbackend = \"{backend}\"\noutput = \"out\"\n"
    );
    let config_path = dir.join("scythe.toml");
    std::fs::write(&config_path, config).unwrap();
    config_path.to_string_lossy().into_owned()
}

/// `java-jdbc` maps `json` to a bare `String` and implements no
/// `generate_nested_struct_def`, so the `orders` column degrades. Reverting the
/// reporting loop in `generate_for_backend` makes this assertion fail: the run
/// still succeeds and still writes the file, and stderr goes quiet.
#[test]
fn generate_reports_a_nested_aggregate_it_degraded_to_opaque_json() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(dir.path(), "java-jdbc");

    let output = scythe_bin()
        .args(["generate", "--config", &config_path])
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(
        output.status.success(),
        "degradation is reported, not fatal -- generate must still succeed; stderr: {err}"
    );
    assert!(
        err.contains("degraded to"),
        "stderr must say the column was degraded; got:\n{err}"
    );
    assert!(
        err.contains("orders"),
        "stderr must name the column that lost its shape; got:\n{err}"
    );
    assert!(
        err.contains("java-jdbc"),
        "stderr must name the backend responsible; got:\n{err}"
    );
}

/// The counterpart that keeps the assertion above honest: `rust-sqlx` is one of
/// the four backends that genuinely builds the nested struct, so nothing is
/// degraded and nothing must be reported. Without this, a reporting loop that
/// fired unconditionally would still pass the test above.
#[test]
fn generate_reports_nothing_for_a_backend_that_builds_the_nested_struct() {
    let dir = TempDir::new().unwrap();
    let config_path = write_project(dir.path(), "rust-sqlx");

    let output = scythe_bin()
        .args(["generate", "--config", &config_path])
        .output()
        .expect("run scythe generate");

    let err = stderr(&output);
    assert!(output.status.success(), "generate must succeed; stderr: {err}");
    assert!(
        !err.contains("degraded to"),
        "rust-sqlx builds the struct, so nothing was narrowed and nothing may be reported; got:\n{err}"
    );
}
