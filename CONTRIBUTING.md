# Contributing to Scythe

Contributions are welcome. This document covers the development setup and guidelines.

## Development Setup

### Prerequisites

- Rust (latest stable)
- Task runner: [Task](https://taskfile.dev/)

### Clone and build

```bash
git clone https://github.com/Goldziher/scythe.git
cd scythe
task setup
```

### Run tests

```bash
task test
```

### Run linters

```bash
task lint
```

### Format code

```bash
task format
```

## Project Structure

```text
crates/
  scythe-core/        # SQL parsing, catalog, type inference
  scythe-codegen/     # Code generation backends
  scythe-lint/        # Lint engine and rules
  scythe-backend/     # Type resolution, naming, templates
  scythe-cli/         # CLI binary
tools/
  test-generator/     # Generates tests from JSON fixtures
  migrate-fixtures/   # Fixture migration utility
testing_data/         # 275 JSON test fixtures
backends/             # Backend manifests (type mappings)
docs/                 # Documentation site (zensical)
```

## Adding a Language Backend

1. Create `backends/<name>/manifest.toml` with type mappings
2. Create `crates/scythe-codegen/src/backends/<name>.rs` implementing `CodegenBackend`
3. Register in `crates/scythe-codegen/src/backends/mod.rs`
4. Add manifest copy to `crates/scythe-codegen/manifests/`
5. All 275 fixtures will automatically test the new backend

## Adding a Lint Rule

1. Add the rule struct to the appropriate file in `crates/scythe-lint/src/rules/`
2. Implement `LintRule` trait
3. Register in `crates/scythe-lint/src/registry.rs`
4. Add test fixtures in `testing_data/lint/`
5. Add tests in the rule file

## Test Infrastructure

Tests are generated from JSON fixtures in `testing_data/`. To regenerate after modifying fixtures:

```bash
cargo run -p test-generator -- --fixtures testing_data --output crates/scythe-cli/tests/generated
cargo fmt --all
```

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` new features
- `fix:` bug fixes
- `docs:` documentation changes
- `refactor:` code restructuring
- `test:` test additions/changes
- `chore:` maintenance tasks

## Pull Requests

- PRs must pass CI (lint, test, build)
- PR titles must follow conventional commit format
- Keep changes focused — one concern per PR

## Code Style

- `cargo fmt` for Rust formatting
- `cargo clippy -- -D warnings` must pass
- Use `ahash` instead of `std::collections::HashMap`
- Generated code must pass real language tools (ruff, biome, gofmt, etc.)

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
