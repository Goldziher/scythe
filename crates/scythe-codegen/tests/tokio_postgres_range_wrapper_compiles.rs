//! Compile-only coverage for the `PgRange<T>` wrapper `TokioPostgresBackend`
//! hand-rolls (board #203, following up on `range_container_consistency.rs`'s
//! "now fixed" entry for `rust-tokio-postgres`).
//!
//! Nothing else in this repository feeds that wrapper's source to a real Rust
//! compiler. `range_container_consistency.rs`'s
//! `rust_tokio_postgres_range_emits_a_working_pgrange_wrapper` asserts the
//! emitted *text* contains the right substrings, never that it compiles.
//! Neither `integration_tests/sql/pg/schema.sql` (compiled into every
//! committed live postgresql-engine integration project) nor
//! `integration_tests/sql/torture/schema.sql` (regenerated against those same
//! projects and real-build-checked by `scripts/check-generated-backends.py`,
//! which runs `cargo check` for `rust-tokio-postgres` specifically) declares a
//! range column, so no committed generated file anywhere names `PgRange<` and
//! no CI job has ever handed it to `rustc`.
//!
//! # Why not add a range column to one of those shared schemas instead
//!
//! Both schemas are swept as a single file across every postgresql-engine
//! project by their respective gates: `sql/pg/schema.sql` is compiled into
//! every live project's committed output, and `sql/torture/schema.sql` is
//! regenerated and real-build-checked (`dotnet build`, `mvn compile`,
//! `mix compile`, `pnpm typecheck`, ...) across all of them by
//! `check-generated-backends.py`, whose `find_pg_projects` sweep is not
//! scoped per backend. `range_container_consistency.rs`'s module doc comment
//! has already verified all 19 `range`-capable manifests' *type mapping* is
//! correct, but `rust-tokio-postgres` is the odd one out: every other
//! backend's mapping just names a type a third-party driver crate already
//! ships (`sqlx::postgres::types::PgRange<T>`, `NpgsqlTypes.NpgsqlRange<T>`,
//! `pgtype.Range[T]`, ...), so there is nothing of scythe's *own* authorship
//! to compile there. `rust-tokio-postgres` alone hand-rolls the impl, so it
//! alone needs a real compiler in the loop. Adding a range column to either
//! shared schema would make all 18 other backends' first real-build encounter
//! with `range` a side effect of a change scoped to this one wrapper, and any
//! new red on that gate would say nothing about the wrapper this file is
//! about.
//!
//! # What this does instead
//!
//! Takes the exact string `TokioPostgresBackend::file_header_for_results`
//! emits -- the same call `range_container_consistency.rs` makes, not a
//! hand-copied duplicate that could drift from it -- and compiles it for
//! real, in a throwaway single-file crate depending on nothing this workspace
//! does not already fetch from crates.io for `scythe-inspect`
//! (`tokio-postgres`, a direct dependency there) and for
//! `rust-tokio-postgres`'s own committed integration project
//! (`postgres-protocol`, a direct dependency of that project's `Cargo.toml`
//! since `6ecbd529`, for this exact wrapper). `cargo check` there catches
//! what `syn::parse_file` cannot: an unresolved path, a missing trait bound,
//! a wrong method name on `postgres_protocol::types`.
//!
//! # Ratchet
//!
//! When `sql/torture/schema.sql` gains a real range column exercised by every
//! postgresql-engine project's real build tool (or a dedicated
//! `rust-tokio-postgres`-only integration project starts compiling one),
//! that is strictly broader coverage than this file provides and this test
//! should be retired in its favor -- see `check-generated-backends.py`'s
//! `BUILD_COMMANDS["rust-tokio-postgres"]`. Until then, this is the only
//! thing that compiles `PG_RANGE_SUPPORT`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use scythe_backend::types::resolve_type;
use scythe_codegen::GeneratedCode;
use scythe_codegen::backends::get_backend;

/// A fixed, non-repo path so repeat local runs reuse `cargo check`'s
/// incremental cache instead of recompiling `tokio-postgres` and its
/// dependency tree from scratch every time. Deliberately outside the
/// workspace: a scratch crate placed under this repo's own `target/` would
/// still hit `cargo`'s ancestor search, land on the workspace root's
/// `Cargo.toml`, and be rejected as a path not listed in `[workspace]
/// members`.
fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join("scythe-pg-range-wrapper-compile-check")
}

/// Writes a throwaway single-file crate containing `wrapper_source` and
/// returns its directory.
fn write_scratch_crate(wrapper_source: &str) -> PathBuf {
    let dir = scratch_dir();
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).expect("create scratch crate src directory");

    // ~keep `[workspace]` with no members, exactly like
    // `integration_tests/rust-tokio-postgres/Cargo.toml`'s own empty table:
    // stops `cargo` from trying to fold this scratch crate into a workspace
    // it does not belong to.
    let cargo_toml = r#"[package]
name = "scythe-pg-range-wrapper-compile-check"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
tokio-postgres = "0.7"
postgres-protocol = "0.6"
"#;
    fs::write(dir.join("Cargo.toml"), cargo_toml).expect("write scratch Cargo.toml");

    let mut lib_rs = String::from(
        "//! Scratch crate written by tokio_postgres_range_wrapper_compiles.rs -- not part of \
         the scythe workspace, never committed. See that file for why it exists.\n\
         #![allow(dead_code)]\n\n",
    );
    lib_rs.push_str(wrapper_source);
    lib_rs.push_str(
        "\n\n// Forces monomorphization of the FromSql/ToSql impls above for a concrete element \
         type -- `i32`, the same element type `resolve_type(\"range<int32>\", ...)` maps to in \
         production.\nfn _pg_range_i32_round_trips(v: PgRange<i32>) -> PgRange<i32> {\n    v\n}\n",
    );
    fs::write(src_dir.join("lib.rs"), lib_rs).expect("write scratch src/lib.rs");

    dir
}

/// Drives a real `cargo check` over the scratch crate and asserts it
/// compiles, surfacing the compiler's own output on failure.
#[test]
fn tokio_postgres_range_wrapper_compiles_with_a_real_toolchain() {
    let backend = get_backend("rust-tokio-postgres", "postgresql").expect("rust-tokio-postgres/postgresql");
    let resolved = resolve_type("range<int32>", backend.manifest(), false).expect("range<int32> must resolve");

    let with_range = [GeneratedCode::build(|c| {
        c.row_struct = Some(format!("pub struct GetSpanRow {{\n    pub span: {resolved},\n}}\n"));
    })];
    let header = backend.file_header_for_results(&with_range);
    assert!(
        header.contains("pub enum PgRange<T>"),
        "file_header_for_results did not emit the PgRange wrapper for a fragment naming it:\n{header}"
    );

    let dir = write_scratch_crate(&header);

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
        "the PgRange<T> wrapper rust-tokio-postgres emits does not compile:\n\
         --- stdout ---\n{}\n--- stderr ---\n{}\n\
         --- source (TokioPostgresBackend::PG_RANGE_SUPPORT, via file_header_for_results) ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        header
    );
}
