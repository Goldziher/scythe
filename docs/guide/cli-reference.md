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

### audit

Run security rules over SQL schema and queries. Emits findings in human, SARIF, or JSON format. See the [Audit guide](audit.md) for the full rule catalog and CI integration recipes.

```bash
scythe audit [OPTIONS] [files...]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--format` | `human` | Output format: `human`, `sarif`, `json` |
| `--list-rules` | false | Print the rule catalog (id, name, severity, category) and exit 0 |
| `--explain <RULE_ID>` | -- | Print the description and CWE refs for a rule by id, then exit 0 |
| `--severity <LEVEL>` | -- | Drop findings below this severity (`off`, `warn`, `error`) |
| `--exit-zero` | false | Exit 0 even if error-severity findings are present (advisory CI gate) |
| `-o, --output <PATH>` | (stdout) | Write reporter output to a file instead of stdout |
| `--ignore-suppressions` | false | Disable inline `-- scythe-audit: ignore[...]` annotations |
| `--dialect <DIALECT>` | `postgres` | SQL dialect for explicit-file mode (`postgres`, `mysql`, `sqlite`, `mssql`, `oracle`, `snowflake`) |
| `files...` | (from config) | SQL files to audit directly |

Exits with code 2 when any error-severity finding is present (unless `--exit-zero` is set). This is distinct from `scythe lint` exit code 1 so CI can tell apart lint failures from security failures.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (lint failures, parse errors, etc.) |
| 2 | Audit failure (error-severity finding from `scythe audit`) |

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

# Audit a project for security issues
scythe audit

# List every audit rule
scythe audit --list-rules

# Explain a specific rule
scythe audit --explain SC-SEC10

# CI: emit SARIF for GitHub code scanning
scythe audit --format sarif -o audit.sarif

# Advisory mode (don't fail the build)
scythe audit --exit-zero

# Audit explicit files with a non-default dialect
scythe audit --dialect mysql sql/migrations/*.sql
```
