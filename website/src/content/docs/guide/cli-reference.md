---
title: CLI Reference
description: All scythe commands, flags, and exit codes.
---

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
scythe check [--config <path>] [--database-url <url>] [--format <format>] [--output <path>]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--database-url` | none | Verify inferred types against a live PostgreSQL database |
| `--format` | `human` | Output format: `human`, `sarif`, or `json` |
| `-o, --output` | stdout | Write findings to a file |

Exits with code 1 if any errors are found. Warnings are reported but do not cause failure.

#### Verifying against a live database

Type inference is static: scythe derives the row type from the schema DDL and
the query, without ever asking a database. `--database-url` adds a second
opinion. Each query is prepared server-side — prepared, never executed, so it
is safe against production — and the result columns and parameters the server
reports are compared against what was inferred.

```bash
scythe check --database-url postgres://user:pass@localhost/mydb
```

This catches a misparsed projection, a wrongly mapped catalog type, a
parameter count or type mismatch, and a query the parser accepted but the
server rejects — including schema drift, where the DDL declares a table that
was never migrated.

| Rule | Fires when |
|------|-----------|
| `SC-VER01` | The database rejected the query |
| `SC-VER02` | Result column count differs from inference |
| `SC-VER03` | A result column's type differs from inference |
| `SC-VER04` | Parameter count differs from inference |
| `SC-VER05` | A parameter's type differs from inference |

Type comparison is deliberately permissive within a family — integer widths
against each other, float widths against each other, enum against string,
and an inferred `string` against a more specific type the server reports
(`uuid`, `json`, `inet`). Static inference cannot always recover the exact
width the server picks, and flagging those would bury the real mismatches.
`decimal` is held to exact equality against the float widths, and `uuid`,
`json`, and `inet` are held to exact equality against each other — these are
different types, not width variants, so a mismatch there is exactly the
wrongly-mapped catalog type this check exists to catch.

:::note
This cannot verify **nullability**. The server's describe response carries
type OIDs but no nullability information, and nullability is the part that
matters most — outer joins and aggregates over empty sets are where inference
is hardest. Verification covers the half the server knows about; the other
half remains scythe's own analysis.
:::

The flag is opt-in and the URL is never read from the environment, so
`scythe check` cannot start requiring a database just because `DATABASE_URL`
happens to be set. Generation never needs a database at all. A natural fit is
to generate without one and verify in the CI job that has a real database.

PostgreSQL only — it is the engine whose extended query protocol lets a
statement be described without executing it.

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

Run security rules over SQL schema and queries. Emits findings in human, SARIF, or JSON format. See the [Audit guide](/scythe/guide/audit/) for the full rule catalog and CI integration recipes.

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

### inspect

Connect to a live database and run operational health checks — missing FK indexes, disabled RLS with policies, duplicate indexes. Emits findings in the same human/SARIF/JSON shape as `scythe audit`. See the [Inspect guide](/scythe/guide/inspect/) for the full check catalog and CI integration recipes.

```bash
scythe inspect [OPTIONS] [DATABASE_URL]
```

| Flag | Default | Description |
|------|---------|-------------|
| `DATABASE_URL` | (from env) | Positional connection URL. Falls back to `$DATABASE_URL`, then `$SCYTHE_DATABASE_URL` |
| `--format` | `human` | Output format: `human`, `sarif`, `json` |
| `--list-checks` | false | Print the check catalog (id, name, severity, description) and exit 0 |
| `--severity <LEVEL>` | -- | Drop findings below this severity (`off`, `warn`, `error`) |
| `--exit-zero` | false | Exit 0 even if error-severity findings are present (advisory CI gate) |
| `-o, --output <PATH>` | (stdout) | Write reporter output to a file instead of stdout |
| `--dialect <DIALECT>` | (from URL scheme) | Engine override: `postgres` (full), `mysql` (stub) |

Exits with code 2 when any error-severity finding is present (unless `--exit-zero` is set). Same convention as `scythe audit`.

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
