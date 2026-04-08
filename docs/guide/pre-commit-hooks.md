# Pre-commit Hooks

Scythe provides [pre-commit](https://pre-commit.com/) / [prek](https://github.com/Goldziher/prek) hooks for SQL formatting, linting, code generation, and validation.

## Setup

Add scythe to your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.5.0  # use the latest release tag
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
```

Then install the hooks:

```bash
# pre-commit
pre-commit install

# prek
prek install
```

## Available Hooks

| Hook ID | Description | Modifies files | Requires config |
|---------|-------------|:--------------:|:---------------:|
| `scythe-fmt` | Format SQL files in-place | Yes | No |
| `scythe-lint` | Lint SQL files with auto-fix | Yes | No |
| `scythe-generate` | Generate code from SQL schema and queries | Yes | Yes |
| `scythe-check` | Validate SQL without generating code | No | Yes |

### scythe-fmt

Formats SQL files using sqruff integration. Runs on changed `.sql` files and modifies them in-place. Failed formatting (exit code 1) blocks the commit until files are re-staged.

### scythe-lint

Lints SQL files and auto-fixes violations where possible. Runs `scythe lint --fix` by default. When run without a `scythe.toml`, only sqruff rules apply. With a config, both scythe rules (schema-aware) and sqruff rules run.

### scythe-generate

Regenerates code when `.sql` files or `scythe.toml` change. Requires a `scythe.toml` in the repository root. Generated files must be staged and re-committed if they change.

### scythe-check

Validates SQL schema and queries without generating code. Exits with code 1 if any lint errors are found. Useful in CI or as a read-only validation step.

## Customization

Override default arguments in your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.5.0
    hooks:
      # Format with a specific SQL dialect
      - id: scythe-fmt
        args: ["--dialect", "postgres"]

      # Lint without auto-fix (check-only)
      - id: scythe-lint
        args: []

      # Use a custom config path
      - id: scythe-generate
        args: ["--config", "db/scythe.toml"]
```

## Using a Pre-installed Binary

By default, hooks use `language: rust` which compiles scythe from source on first run. If you already have scythe installed (via `cargo install` or `brew`), use `language: system` for faster execution:

```yaml
repos:
  - repo: local
    hooks:
      - id: scythe-fmt
        name: Format SQL (scythe)
        entry: scythe fmt
        language: system
        types: [sql]

      - id: scythe-lint
        name: Lint SQL (scythe)
        entry: scythe lint --fix
        language: system
        types: [sql]
```

## Recommended Combinations

**Most projects** -- format and lint SQL on every commit:

```yaml
hooks:
  - id: scythe-fmt
  - id: scythe-lint
```

**Code generation projects** -- also regenerate code when SQL changes:

```yaml
hooks:
  - id: scythe-fmt
  - id: scythe-lint
  - id: scythe-generate
```

**CI-only validation** -- check without modifying files:

```yaml
hooks:
  - id: scythe-check
```

## Testing Hooks

Verify hooks work in your project:

```bash
# Test a specific hook on all files
prek run scythe-fmt --all-files

# Test with try-repo (no installation needed)
prek try-repo https://github.com/Goldziher/scythe scythe-fmt --all-files

# Dry run to preview what would execute
prek run scythe-lint --dry-run
```

## Notes

- **First run**: `language: rust` compiles scythe from source, which takes a few minutes. Subsequent runs use the cached binary. Use `language: system` to skip compilation if scythe is already installed.
- **Config path**: Hooks that require a config (`scythe-generate`, `scythe-check`) default to `scythe.toml` in the repository root. Override with `args: ["--config", "path/to/scythe.toml"]`.
- **Auto-staging**: When `scythe-fmt` or `scythe-lint --fix` modify files, pre-commit/prek reports the hook as failed. Stage the changes and commit again.
