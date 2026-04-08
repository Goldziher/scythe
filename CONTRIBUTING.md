# Contributing to Scythe

## Quick Start

Prerequisites:

- Rust (latest stable)
- [Task](https://taskfile.dev/) runner
- Docker (for integration tests only)
- [prek](https://github.com/Goldziher/prek) (pre-commit hook manager)

```bash
git clone https://github.com/Goldziher/scythe.git
cd scythe
task setup    # install pre-commit hooks, fetch dependencies
task check    # run lint + test — verify everything passes
```

This should take under 5 minutes from clone to green tests.

## Architecture

Crate dependency graph:

```text
scythe-cli
  |-- scythe-codegen
  |     |-- scythe-backend  (manifests, type resolution, naming)
  |     +-- scythe-core     (SQL parsing, catalog, type analysis)
  +-- scythe-lint            (SQL linting rules + sqruff integration)
```

What each crate does:

- **scythe-core** -- SQL parsing, schema catalog construction, type inference
- **scythe-backend** -- manifest-driven type resolution, naming conventions, template rendering
- **scythe-codegen** -- code generation backends implementing `CodegenBackend` trait
- **scythe-lint** -- lint engine, rule registry, and built-in SQL lint rules
- **scythe-cli** -- CLI binary, config loading, orchestration

Key directories:

| Directory | Purpose |
|---|---|
| `crates/` | All workspace crates |
| `crates/scythe-codegen/manifests/` | Backend manifest TOML files (type mappings per backend/engine) |
| `crates/scythe-codegen/src/backends/` | Backend implementations |
| `crates/scythe-lint/src/rules/` | Lint rule implementations (performance, safety, style, naming, antipattern, codegen) |
| `testing_data/` | 275+ JSON test fixtures for snapshot testing |
| `integration_tests/` | Per-backend integration tests with Docker database containers |
| `tools/test-generator/` | Generates Rust tests from JSON fixtures |
| `tools/snippet-runner/` | Validates documentation code snippets |
| `tools/migrate-fixtures/` | Fixture migration utility |
| `docs/` | Documentation site |

## Adding a Language Backend

1. Create a manifest TOML at `crates/scythe-codegen/manifests/{name}.toml`. For engine-specific variants, use `{name}.{engine}.toml` (e.g., `java-jdbc.mysql.toml`). Existing manifests serve as reference.

2. Implement the `CodegenBackend` trait in `crates/scythe-codegen/src/backends/{name}.rs`. The trait is defined in `crates/scythe-codegen/src/backend_trait.rs`.

3. Register the backend in `crates/scythe-codegen/src/backends/mod.rs`.

4. Add any backend-specific validation logic in `crates/scythe-codegen/src/lib.rs`.

5. Create an integration test directory at `integration_tests/{name}/` with a working project that imports the generated code and executes queries against a real database.

6. Add test fixture expectations in `testing_data/` if the backend requires new fixture coverage.

7. Add integration test tasks in `integration_tests/Taskfile.yaml`.

All 275+ existing fixtures will automatically test the new backend through snapshot tests.

## Adding a Database Engine

1. Add the engine variant to the SQL parser and catalog in `scythe-core`. Engine-specific SQL syntax and types must be handled here.

2. Create engine-specific manifests at `crates/scythe-codegen/manifests/{backend}.{engine}.toml` for each backend that supports the engine.

3. Add type mappings for the engine's SQL types in the manifest TOML. Map each SQL type to the target language's native type.

4. Create integration test SQL schemas in `integration_tests/` with engine-appropriate DDL.

5. Add a Docker service in `integration_tests/docker-compose.yml` if the engine requires a running server.

## Adding a Lint Rule

1. Add the rule implementation in the appropriate file under `crates/scythe-lint/src/rules/` (e.g., `performance.rs`, `safety.rs`, `style.rs`, `naming.rs`, `antipattern.rs`, `codegen.rs`). Implement the `LintRule` trait.

2. Register the rule in `crates/scythe-lint/src/registry.rs`.

3. Add test fixtures in `testing_data/lint/` with SQL that triggers and does not trigger the rule.

4. Add tests in the rule file itself.

5. Document the rule in `docs/reference/lint-rules.md`.

## Testing

| Command | What it runs |
|---|---|
| `task test` | Unit tests + snapshot tests + fixture-generated tests |
| `task test:verbose` | Same, with `--nocapture` output |
| `task lint` | All linters via prek (fmt, clippy, cargo-deny, cargo-machete, etc.) |
| `task check` | Lint + test combined |
| `task snippets:validate` | Validate documentation code snippets at syntax level |
| `task snippets:validate:compile` | Validate documentation code snippets at compile level |

Fixture-generated tests: tests are generated from JSON fixtures in `testing_data/` by the `test-generator` tool. CI runs this automatically. To regenerate locally:

```bash
cargo run -p test-generator -- --fixtures testing_data --output crates/scythe-cli/tests/generated
cargo fmt --all
```

Integration tests require Docker. From `integration_tests/`:

```bash
task db:up        # start PostgreSQL/MySQL/SQLite containers
task db:migrate   # apply schema
task all          # run all integration tests
task db:down      # stop containers
```

Individual integration tests: `task test:rust-sqlx`, `task test:python-psycopg3`, etc.

## Code Style

- Rust formatting: `cargo fmt --all`
- Rust linting: `cargo clippy --workspace -- -D warnings`
- Rust edition: 2024
- Use `ahash` instead of `std::collections::HashMap`
- Max 120 character line width
- Generated code must pass real language tooling (ruff, biome, gofmt, etc.)
- Pre-commit hooks enforce all of the above plus:
  - Trailing whitespace and EOF newlines
  - TOML/YAML/JSON validation
  - Markdown linting
  - Shell script formatting (shfmt)
  - Unused dependency detection (cargo-machete)
  - License and advisory checks (cargo-deny)
  - Conventional commit message format (gitfluff)

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

| Prefix | Use for |
|---|---|
| `feat:` | New features |
| `fix:` | Bug fixes |
| `docs:` | Documentation changes |
| `refactor:` | Code restructuring without behavior change |
| `test:` | Test additions or changes |
| `chore:` | Maintenance, dependency updates, CI changes |

First line under 72 characters, imperative mood. Add scope when useful: `feat(codegen): add Ruby MySQL2 backend`.

Use `feat!:` or `fix!:` prefix for breaking changes.

## PR Requirements

- All CI checks must pass (lint, test, build, snippet validation)
- New features need tests
- Backend changes need integration tests
- One logical change per PR
- PR title follows conventional commit format

## Breaking Changes

- Bump the minor version (0.x.0)
- Document in changelog with migration notes
- Use `feat!:` or `fix!:` commit prefix

## Security

- No secrets in code or test files
- Use environment variables for database credentials
- Report security issues privately -- do not open public issues

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
