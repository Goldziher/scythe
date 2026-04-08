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

Reads the config, parses schema and queries, runs type inference, and writes generated code to the configured output directory. If `scythe.toml` is not found, the command exits with an error.

### check

Validate SQL without generating code. Runs parsing, analysis, and lint rules.

```bash
scythe check [--config <path>]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |

Exits with code 1 if any lint errors are found. Warnings are reported but do not cause failure.

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
| `files...` | (from config) | SQL files to lint directly |

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
| `-c, --config` | `scythe.toml` | Path to config file |
| `--check` | false | Report files needing formatting; exit 1 if any |
| `--diff` | false | Show unified diff of changes |
| `--dialect` | `ansi` | SQL dialect for formatting rules |
| `files...` | (from config) | SQL files to format directly |

### migrate

Convert a sqlc project to scythe format.

```bash
scythe migrate [sqlc_config]
```

| Argument | Default | Description |
|----------|---------|-------------|
| `sqlc_config` | `sqlc.yaml` | Path to sqlc config file (v1 or v2) |

Reads the sqlc config, converts query annotations from sqlc format to scythe format, and generates a `scythe.toml`.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (lint failures, parse errors, etc.) |

## Examples

```bash
# Generate code with default config
scythe generate

# Generate code with custom config path
scythe generate --config my-project.toml

# Check SQL validity
scythe check

# Lint with auto-fix
scythe lint --fix

# Lint specific files without a config
scythe lint --dialect postgres sql/*.sql

# Format check in CI
scythe fmt --check

# Preview formatting changes
scythe fmt --diff

# Migrate from sqlc
scythe migrate sqlc.yaml
```
