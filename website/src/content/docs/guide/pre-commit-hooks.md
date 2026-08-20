---
title: Pre-commit Hooks
description: pre-commit / prek hooks for SQL formatting, linting, code generation, and validation.
---

Scythe provides [pre-commit](https://pre-commit.com/) / [prek](https://github.com/j178/prek) hooks for SQL formatting, linting, code generation, and validation.

## Setup

Add scythe to your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.16.1  # use the latest release tag
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
      - id: scythe-audit
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
| `scythe-lint` | Lint SQL files with auto-fix (includes audit rules) | Yes | No |
| `scythe-audit` | SC-SEC*/SC-RLS*/SC-MIG*/SC-CHK* security/migration audit | No | No |
| `scythe-inspect` | Live-DB health checks (SC-INS*) — needs `$DATABASE_URL` or `[inspect].database_url` | No | No |
| `scythe-generate` | Generate code from SQL schema and queries | Yes | Yes |
| `scythe-check` | Validate SQL without generating code | No | Yes |

### scythe-fmt

Formats SQL files using sqruff integration. Runs on changed `.sql` files and modifies them in-place
and exits 0 after writing. There is no exit-1 formatting failure to block the commit -- the commit is
blocked by pre-commit/prek's own detection that the hook modified files, the same mechanism that
blocks `scythe-lint`. Re-stage the changed files and commit again.

### scythe-lint

Lints SQL files and auto-fixes violations where possible. Runs `scythe lint --fix` by default. When run without a `scythe.toml`, only sqruff rules apply. With a config, both scythe rules (schema-aware) and sqruff rules run — and the canonical `SC-SEC*`, `SC-RLS*`, `SC-MIG*`, and `SC-CHK*` audit packs run too, dialect-gated by the `[[sql]].engine` field. A `mysql` project will not see postgres-only `SC-MIG*` findings; they're silently skipped.

### scythe-audit

Static SQL audit — runs the canonical security, RLS, migration-safety, and CHECK-integrity rule packs over staged `.sql` files. No `scythe.toml` or database connection required. Defaults to the `postgres` dialect; override via `args: [--dialect, mysql]` (or any other supported engine). Exits 2 when any error-severity rule fires; pass `--exit-zero` for advisory CI integration that publishes findings without blocking the commit.

### scythe-inspect

Connects to a live Postgres database and runs the `SC-INS*` operational health checks (missing FK indexes, RLS misconfig with policies, duplicate indexes). Resolves the connection URL the same way `scythe inspect` does: `$DATABASE_URL`, then `$SCYTHE_DATABASE_URL`, then `[inspect].database_url` in `scythe.toml` (see the [Inspect guide](/scythe/guide/inspect/)). Runs where none of the three is set fail loudly with the same error as the CLI. Designed for CI pre-merge gates and pre-deploy checks, not interactive commit blocking. Exits 2 on error-severity findings; pass `--exit-zero` via `args:` for advisory mode.

### scythe-generate

Regenerates code when `.sql` files or `scythe.toml` change. Requires a `scythe.toml` in the repository root. Generated files must be staged and re-committed if they change.

### scythe-check

Validates SQL schema and queries without generating code. Exits with code 2 if any error-severity
lint finding is present; exit 1 is reserved for operational failures (bad config, unreadable files).
Useful in CI or as a read-only validation step.

## Customization

Override default arguments in your `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.16.1
    hooks:
      # Format with a specific SQL dialect
      - id: scythe-fmt
        args: ["--dialect", "postgres"]

      # Use a custom config path
      - id: scythe-generate
        args: ["--config", "db/scythe.toml"]
```

:::caution[`scythe-lint` has no check-only mode via `args:`]
`.pre-commit-hooks.yaml` bakes `--fix` into the hook's `entry` (`entry: scythe lint --fix`), not into
`args:`. Pre-commit's `args:` only *adds* arguments to `entry` -- it cannot remove `--fix` from it. So
`args: []` on `scythe-lint` still auto-fixes; there is currently no way to make the published
`scythe-lint` hook check-only without modifying files. Use `scythe-check` (which never writes files)
for a check-only step, or invoke `scythe lint` directly (without `--fix`) as a `language: system`
local hook.
:::

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
