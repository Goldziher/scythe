# CLI Reference

```bash
scythe <command> [options]
```

## Commands

### generate

Generate code from SQL schema and queries.

```bash
scythe generate [--config <path>] [--allow-output-escape] [--validate-output]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--allow-output-escape` | false | Allow a `[[sql.gen]]` `output` directory to resolve outside the project root (`../` traversal or an absolute path). Without it, such a path is rejected before anything is written |
| `--validate-output` | false | Validate each target's generated output with the real compiler/linter for its language (`poly`, `tsc`, `javac`, `kotlinc`, `gofmt`, `ruby`, ...). Exits 2, not 1, if any target fails. Off by default -- it shells out to external toolchains that may not be installed |

### check

Validate SQL without generating code. Runs parsing, analysis, lint rules, and
the provenance rules that compare committed artifacts against the current
schema.

```bash
scythe check [--config <path>] [--database-url <url>] [--format <fmt>] [--output <path>] [--exit-zero]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--database-url` | none | Verify inferred types and detect schema drift against a live database. PostgreSQL only. Never read from the environment -- `check` cannot start requiring a database just because `DATABASE_URL` is set |
| `--format` | `human` | `human`, `sarif`, or `json` |
| `-o, --output` | stdout | Write findings to a file |
| `--exit-zero` | false | Exit 0 even when error-severity findings are present |

**Exits 2** -- not 1 -- on error-severity findings. See [Exit Codes](#exit-codes).

### lint

Lint SQL files for correctness, performance, and style.

```bash
scythe lint [--config <path>] [--fix] [--dialect <dialect>] [--database-url <url>] [--format <fmt>] [--output <path>] [--exit-zero] [files...]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--fix` | false | Auto-fix violations where possible |
| `--dialect` | none | SQL dialect for sqruff rules. No default is declared; sqruff resolves it |
| `--database-url` | none | Also run the live-database `inspect` checks (PostgreSQL or MySQL/MariaDB) as part of this `lint` run. Opt-in like `check --database-url` -- never read from `$DATABASE_URL` automatically; falls back to `[inspect].database_url` in `scythe.toml` when omitted |
| `--format` | `human` | `human`, `sarif`, or `json` |
| `-o, --output` | stdout | Write reporter output to a file |
| `--exit-zero` | false | Exit 0 even when error-severity findings are present |

### audit

Scan SQL for security issues: privilege grants, dangerous functions, cartesian
joins, unbounded `LIKE`, `SECURITY DEFINER` misuse, literal passwords, weak
hashes over credential columns, `SELECT *` over PII, session-state mutation.

```bash
scythe audit [--config <path>] [--format <fmt>] [--severity <level>] [--exit-zero] [files...]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--format` | `human` | `human`, `sarif`, or `json` |
| `--list-rules` | false | Print the rule catalog (id, name, severity, category) and exit 0 |
| `--explain <RULE_ID>` | none | Print a rule's description and CWE references, then exit 0 |
| `--severity <LEVEL>` | none | Drop findings below `off`, `warn`, or `error` |
| `--exit-zero` | false | Exit 0 even when error-severity findings are present |
| `-o, --output` | stdout | Write reporter output to a file |
| `--ignore-suppressions` | false | Disable inline `-- scythe-audit: ignore[...]` annotations |
| `--dialect` | from config | Dialect for explicit-file mode |

### inspect

Check a live database for operational issues: foreign keys without covering
indexes, tables carrying policies while RLS is disabled, duplicate indexes.
PostgreSQL (13 checks) and MySQL/MariaDB (4 checks); any other engine has no
`scythe inspect` driver.

```bash
scythe inspect [database_url] [--format <fmt>] [--severity <level>] [--exit-zero]
```

The connection URL resolves in order: positional argument, `$DATABASE_URL`,
`$SCYTHE_DATABASE_URL`, then `[inspect].database_url` in `scythe.toml`.

| Flag | Default | Description |
|------|---------|-------------|
| `--format` | `human` | `human`, `sarif`, or `json` |
| `--list-checks` | false | Print the check catalog and exit 0 |
| `--explain <CHECK_ID>` | none | Print a check's rationale and remediation, then exit 0 |
| `--severity <LEVEL>` | none | Drop findings below `off`, `warn`, or `error` |
| `--exit-zero` | false | Exit 0 even when error-severity findings are present |
| `-o, --output` | stdout | Write reporter output to a file |
| `-c, --config` | `scythe.toml` | Path to config file |
| `--dialect` | from URL scheme | Engine to target: `postgres`/`postgresql` or `mysql`/`mariadb` |

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
| `--dialect` | none | SQL dialect for formatting rules. No default is declared; sqruff resolves it |

### migrate

Convert a sqlc project to scythe format.

```bash
scythe migrate [sqlc_config]
```

Reads sqlc config (v1 or v2), converts annotations, generates `scythe.toml`.

## Exit Codes

`check`, `lint`, `audit` and `inspect` separate "your SQL has problems" from "scythe
could not run", because CI needs to tell them apart: a findings failure is the
tool working, while an operational failure means the check never happened and
must not be mistaken for a clean run.

| Code | Meaning |
|------|---------|
| 0 | Success -- no error-severity findings, or `--exit-zero` was passed |
| 1 | Operational failure: unreadable config, unparseable SQL, unconstructible backend, I/O error |
| 2 | Error-severity findings were reported (`check`, `lint`, `audit`, `inspect`) |

`--exit-zero` collapses 2 to 0. It does not affect 1: an operational failure is
still a failure.

## Examples

```bash
scythe generate                          # Generate with default config
scythe generate --config my-project.toml # Custom config path
scythe check                             # Validate SQL
scythe check --database-url "$DB_URL"    # Also verify types and detect schema drift
scythe lint --fix                        # Lint with auto-fix
scythe lint --dialect postgres sql/*.sql # Lint specific files
scythe audit --format sarif -o audit.sarif  # Security scan for CI
scythe audit --explain SC-SEC01          # Why a rule exists, and its CWE refs
scythe inspect "$DB_URL"                 # Operational health of a live database
scythe fmt --check                       # CI formatting check
scythe fmt --diff                        # Preview formatting changes
scythe migrate sqlc.yaml                 # Migrate from sqlc
```

## Pre-commit Hooks

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.18.0
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
      - id: scythe-audit
      - id: scythe-inspect
      - id: scythe-generate
      - id: scythe-check
```
