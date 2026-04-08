# CLI Reference

```bash
scythe <command> [options]
```

## Commands

### generate

Generate code from SQL schema and queries.

```bash
scythe generate [--config <path>]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |

### check

Validate SQL without generating code. Runs parsing, analysis, and lint rules.

```bash
scythe check [--config <path>]
```

Exits with code 1 if any lint errors are found.

### lint

Lint SQL files for correctness, performance, and style.

```bash
scythe lint [--config <path>] [--fix] [--dialect <dialect>] [files...]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--fix` | false | Auto-fix violations where possible |
| `--dialect` | `ansi` | SQL dialect for sqruff rules |

**Two modes:**

- **With config:** Runs both scythe rules (schema-aware) and sqruff rules.
- **With files:** Runs sqruff rules only (no schema context).

### fmt

Format SQL files using sqruff.

```bash
scythe fmt [--config <path>] [--check] [--diff] [--dialect <dialect>] [files...]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--check` | false | Report unformatted files; exit 1 |
| `--diff` | false | Show unified diff of changes |
| `--dialect` | `ansi` | SQL dialect for formatting rules |

### migrate

Convert a sqlc project to scythe format.

```bash
scythe migrate [sqlc_config]
```

Reads sqlc config (v1 or v2), converts annotations, generates `scythe.toml`.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (lint failures, parse errors, etc.) |

## Examples

```bash
scythe generate                          # Generate with default config
scythe generate --config my-project.toml # Custom config path
scythe check                             # Validate SQL
scythe lint --fix                        # Lint with auto-fix
scythe lint --dialect postgres sql/*.sql # Lint specific files
scythe fmt --check                       # CI formatting check
scythe fmt --diff                        # Preview formatting changes
scythe migrate sqlc.yaml                 # Migrate from sqlc
```

## Pre-commit Hooks

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.5.0
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
      - id: scythe-generate
      - id: scythe-check
```
