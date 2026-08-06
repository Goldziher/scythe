# integration_tests

Cross-language integration test harnesses for scythe-generated database access code, one
directory per backend (language + driver + engine combination).

## How these directories are produced

Two generators feed this tree, run via `task generate:all`:

1. `tools/integration-test-generator` renders each backend's scaffolding (`scythe.toml`, the
   test harness, and the dependency manifest) from `tools/integration-test-generator/templates/`.
   The full backend list is defined once, in `build_backends()`
   (`tools/integration-test-generator/src/main.rs`); run
   `cargo run --quiet -p integration-test-generator -- --list` to print it. Do not hand-maintain
   a copy of this list elsewhere — `integration_tests/Taskfile.yaml`'s `generate` task consumes
   it directly so the two can't drift.
2. `scythe generate`, run inside each backend directory, renders the `generated/` (or
   language-equivalent) query code from `sql/*/schema.sql` and `sql/*/queries/*.sql`.

Both are generated output. Never hand-edit files under a backend's `generated/` directory, or
`scythe.toml`/dependency manifests for a backend that `build_backends()` owns — fix the
generator or the jinja templates instead, then run `task generate:all` and commit the result.

## Directories outside the generator

Eight directories are not produced by `tools/integration-test-generator` and are intentionally
absent from `build_backends()`:

### Hand-maintained by design

- `kotlin-jdbc-ext` — exercises the `extension_functions = "true"` codegen option for the
  `kotlin-jdbc` backend. Kept out of the generated matrix because it's a one-off option
  combination, not a distinct language/driver/engine axis.
- `php-pdo-namespace` — exercises the configurable PHP `namespace` codegen option for the
  `php-pdo` backend, for the same reason.

  Both have their own hand-written `scythe.toml` and dependency manifest, but their
  `generated/` output is still produced by `scythe generate` and drifts like any other generated
  output, so `task generate` regenerates them too — via the explicit `HAND_MAINTAINED` list in
  `Taskfile.yaml`, since they are absent from `build_backends()`. Only their scaffolding is
  hand-written; their query code is gated like everything else. If you add another directory of
  this kind, add it to `HAND_MAINTAINED` as well or its output will silently rot.

### Orphaned local artifacts — not part of version control

- `csharp-sqlclient`, `elixir-tds`, `python-oracledb`, `python-pyodbc`, `rust-sibyl`,
  `rust-tiberius` — these directories contain nothing but gitignored build output (`bin/`,
  `obj/`, `_build/`, `deps/`, `Cargo.lock`) and have **no files tracked in git, ever** (verified
  via `git log --all -- integration_tests/<dir>`). They appear to be pre-engine-suffix leftovers
  from local runs, superseded by their generator-owned, engine-suffixed counterparts
  (`csharp-sqlclient-mssql`, `elixir-tds-mssql`, `python-oracledb-oracle`,
  `python-pyodbc-mssql`, `rust-sibyl-oracle`, `rust-tiberius-mssql`). They are harmless (git
  ignores them and CI never sees them) but are not meaningful test coverage — safe to delete
  locally with `rm -rf`.

Losing track of which hand-maintained directories are load-bearing test coverage versus inert
local cruft is exactly how the Oracle LOB regression coverage was destroyed during 0.12.0. Keep
this section accurate: if you add a new hand-maintained directory outside the generator, or
confirm one of the orphaned directories is actually load-bearing, update this file in the same
change.
