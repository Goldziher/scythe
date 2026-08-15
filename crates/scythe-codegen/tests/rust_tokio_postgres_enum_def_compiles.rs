//! Compile-only coverage for `rust-tokio-postgres`'s `generate_enum_def`, the
//! declaration site the enum-variant-collision guard in
//! `enum_and_query_name_regression.rs` protects (GH #136 point 4).
//!
//! That regression suite proves the *colliding* case is rejected before any
//! code is written -- there is nothing left to compile there, since
//! `generate_enum_defs_via_backend` returns `Err` before calling
//! `backend.generate_enum_def` at all. This file proves the complementary
//! half: a *non*-colliding enum -- the case the new
//! `resolve::check_enum_variant_collisions` guard must let straight through
//! -- still renders code a real `rustc` accepts, not just code that looks
//! plausible under a string match. `validation.rs`'s `validate_with_tools`
//! deliberately has no automated coverage for any `rust-*` backend (see its
//! module doc comment on the `_ => return ToolValidation::Unsupported` arm),
//! so this is the only thing that compiles `generate_enum_def`'s output for
//! `rust-tokio-postgres` with a real toolchain, mirroring
//! `tokio_postgres_range_wrapper_compiles.rs`'s scratch-crate approach for
//! the identical reason: `tokio_postgres::types::{FromSql, ToSql}` are
//! referenced by fully-qualified path with no `use` to stub around.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scythe_codegen::backends::get_backend;
use scythe_core::analyzer::EnumInfo;

/// A fixed, non-repo path so repeat local runs reuse `cargo check`'s
/// incremental cache -- see `tokio_postgres_range_wrapper_compiles.rs` for
/// why this must live outside the workspace.
fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join("scythe-tokio-postgres-enum-def-compile-check")
}

fn write_scratch_crate(enum_source: &str) -> PathBuf {
    let dir = scratch_dir();
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).expect("create scratch crate src directory");

    let cargo_toml = r#"[package]
name = "scythe-tokio-postgres-enum-def-compile-check"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
tokio-postgres = "0.7"
"#;
    fs::write(dir.join("Cargo.toml"), cargo_toml).expect("write scratch Cargo.toml");

    let mut lib_rs = String::from(
        "//! Scratch crate written by rust_tokio_postgres_enum_def_compiles.rs -- not part of \
         the scythe workspace, never committed. See that file for why it exists.\n\
         #![allow(dead_code)]\n\n",
    );
    lib_rs.push_str(enum_source);
    fs::write(src_dir.join("lib.rs"), lib_rs).expect("write scratch src/lib.rs");

    dir
}

/// Drives a real `cargo check` over the scratch crate and asserts it
/// compiles, surfacing the compiler's own output on failure.
#[test]
fn rust_tokio_postgres_enum_def_compiles_with_a_real_toolchain() {
    let backend = get_backend("rust-tokio-postgres", "postgresql").expect("rust-tokio-postgres/postgresql");

    // Two SQL values that are distinct enough not to collide under
    // PascalCase -- the exact shape `check_enum_variant_collisions` must let
    // through unmodified.
    let enum_info = EnumInfo {
        sql_name: "model".to_string(),
        values: vec!["gpt-4-turbo".to_string(), "claude-3-opus".to_string()],
    };
    let enum_def = backend
        .generate_enum_def(&enum_info)
        .expect("a non-colliding enum must render");
    assert!(
        enum_def.contains("pub enum Model {"),
        "expected the enum declaration:\n{enum_def}"
    );
    assert!(
        enum_def.contains("Gpt4Turbo") && enum_def.contains("Claude3Opus"),
        "expected both distinct variants:\n{enum_def}"
    );

    let dir = write_scratch_crate(&enum_def);

    let output = Command::new("cargo")
        .args(["check", "--quiet"])
        .current_dir(&dir)
        // ~keep Explicit, not inherited: this must never share (and possibly
        // corrupt) whatever `CARGO_TARGET_DIR` the outer `cargo test`
        // invocation running this very test is using.
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .expect("spawn `cargo check` for the scratch crate -- cargo must be on PATH to have run this test at all");

    assert!(
        output.status.success(),
        "generate_enum_def's output does not compile:\n\
         --- stdout ---\n{}\n--- stderr ---\n{}\n\
         --- source ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        enum_def
    );
}
