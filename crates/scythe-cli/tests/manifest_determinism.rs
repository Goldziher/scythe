//! Regression test for #82: manifest selection must be a pure function of
//! `(backend, engine)`, with no filesystem lookup that could pick up a
//! working-directory-relative `backends/<name>/manifest.toml` override.
//! Generated code must not depend on the process's current working directory.
//!
//! IMPORTANT: this file must contain exactly ONE `#[test]`. It calls
//! `std::env::set_current_dir`, which mutates process-wide state. `cargo test`
//! runs tests within a single test binary on a shared thread pool by default,
//! so a second test in this file could race with this one's CWD change and
//! produce flaky, order-dependent failures. Keep this file to one test; add
//! unrelated tests elsewhere.

use std::env;
use std::path::PathBuf;

/// A deliberately wrong sentinel scalar mapping. If manifest selection ever
/// falls back to reading `backends/<name>/manifest.toml` relative to the
/// process CWD, this sentinel leaks into the backend's manifest and the
/// assertions below fail.
const SENTINEL_SCALAR: &str = "__SENTINEL_WRONG_TYPE__";

/// A structurally valid `rust-sqlx` manifest (same shape as the real,
/// compiled-in one) with `int32` deliberately mapped to a sentinel value that
/// never appears in the real manifest.
const SENTINEL_MANIFEST_TOML: &str = r#"
[backend]
name = "rust-sqlx"
language = "rust"
file_extension = "rs"
engine = "postgresql"
description = "Sentinel manifest -- must never be picked up by get_backend"

[types.scalars]
bool = "bool"
int16 = "i16"
int32 = "__SENTINEL_WRONG_TYPE__"
int64 = "i64"
float32 = "f32"
float64 = "f64"
string = "String"
bytes = "Vec<u8>"
uuid = "uuid::Uuid"
decimal = "rust_decimal::Decimal"
date = "chrono::NaiveDate"
time = "chrono::NaiveTime"
time_tz = "sqlx::postgres::types::PgTimeTz"
datetime = "chrono::NaiveDateTime"
datetime_tz = "chrono::DateTime<chrono::Utc>"
interval = "sqlx::postgres::types::PgInterval"
json = "serde_json::Value"
inet = "ipnetwork::IpNetwork"

[types.containers]
array = "Vec<{T}>"
nullable = "Option<{T}>"
range = "sqlx::postgres::types::PgRange<{T}>"
json_typed = "sqlx::types::Json<{T}>"

[naming]
struct_case = "PascalCase"
fn_case = "snake_case"
enum_variant_case = "PascalCase"
row_suffix = "Row"

[imports.rules]
"chrono::" = "use chrono;"
"uuid::Uuid" = "use uuid::Uuid;"
"rust_decimal::" = "use rust_decimal::Decimal;"
"serde_json::" = "use serde_json;"
"ipnetwork::" = "use ipnetwork::IpNetwork;"
"#;

/// Restores the original working directory on drop, even if the test panics,
/// so a failure here can't leave later tests (or later runs) chdir'd into a
/// directory that has since been deleted.
struct CwdGuard {
    original: PathBuf,
}

impl CwdGuard {
    fn new() -> Self {
        Self {
            original: env::current_dir().expect("failed to read current dir"),
        }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original);
    }
}

#[test]
fn manifest_selection_ignores_cwd_backends_override() {
    let temp = tempfile::tempdir().expect("failed to create temp dir");
    let manifest_dir = temp.path().join("backends").join("rust-sqlx");
    std::fs::create_dir_all(&manifest_dir).expect("failed to create backends/rust-sqlx dir");
    std::fs::write(manifest_dir.join("manifest.toml"), SENTINEL_MANIFEST_TOML)
        .expect("failed to write sentinel manifest.toml");

    let guard = CwdGuard::new();
    env::set_current_dir(temp.path()).expect("failed to chdir into temp dir");

    let backend = scythe_codegen::get_backend("rust-sqlx", "postgresql")
        .expect("get_backend should succeed regardless of CWD contents");

    let scalars = &backend.manifest().types.scalars;
    assert_eq!(
        scalars.get("int32").map(String::as_str),
        Some("i32"),
        "manifest must be the compiled-in one, not the CWD-relative sentinel file"
    );
    assert!(
        !scalars.values().any(|v| v == SENTINEL_SCALAR),
        "sentinel scalar leaked into the manifest -- manifest selection is reading the filesystem"
    );

    drop(guard);
}
