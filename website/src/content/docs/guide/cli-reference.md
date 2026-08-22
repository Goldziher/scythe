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
scythe generate [--config <path>] [--allow-output-escape] [--validate-output]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--allow-output-escape` | false | Allow a `[[sql.gen]]` `output` directory to resolve outside the project root (via `../` traversal or an absolute path). Without it, such a path is rejected before anything is written |
| `--validate-output` | false | Validate each target's generated output with the real compiler/linter for its language (`poly`, `tsc`, `javac`, `kotlinc`, `gofmt`, `ruby`, ...), reporting per target whether it was validated, skipped (no validator for that language, or the tool it needs is not installed), or failed. Exits 2, not 1, if any target fails. Off by default because it shells out to external toolchains that may not be installed |

Reads the config, parses schema and queries, runs type inference, and writes generated code to the configured output directory. If `scythe.toml` is not found, the command exits with an error.

### check

Validate SQL without generating code. Runs parsing, analysis, and lint rules, plus provenance
verification of already-generated artifacts against the current schema (see below). With
`--database-url`, it also verifies inferred types and checks for schema drift against a live
database.

```bash
scythe check [--config <path>] [--database-url <url>] [--format <format>] [--output <path>] [--exit-zero]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--database-url` | none | Verify inferred types and detect schema drift against a live PostgreSQL database |
| `--format` | `human` | Output format: `human`, `sarif`, or `json` |
| `-o, --output` | stdout | Write findings to a file |
| `--exit-zero` | false | Exit 0 even if error-severity findings are present (advisory CI gate) |

Exits with code 2 when any error-severity finding is present (unless `--exit-zero` is set), and 1
only on operational failure — an unreadable config, unparseable SQL, or an I/O error. Warnings never
affect the exit code. The split lets CI distinguish "your schema drifted" from "your config is
missing"; before 0.14.0 both exited 1. Same convention as `scythe audit` and `scythe inspect`.

#### Provenance verification

Every file `scythe generate` writes carries a first-line header recording the schema, queries,
engine, backend, and scythe version it was generated from:

```text
// scythe:provenance v=0.18.0 backend=rust-sqlx engine=postgresql schema=sch1:ebbab3de0c9715b8 queries=q1:4f39acbf854efd81
```

The comment marker matches the target language (`#` for Python, Ruby, and Elixir; `//` for
everything else). `scythe check` re-derives the same five values from the current `scythe.toml`,
schema and queries, and reports a mismatch as an `SC-PRV*` finding. Unlike [`--database-url`](#verifying-against-a-live-database)
verification below, this needs no database and no toolchain for the target language — it reads the
committed artifact and compares five strings, so it runs on every `scythe check` invocation with no
extra flag.

The `schema=` field is a fingerprint — `sch1:` followed by 16 hex characters (the leading 8 bytes of
a SHA-256 digest over a canonical, sorted rendering of the catalog). It changes if and only if the
schema's *shape* changes: table and column names, column order, types, nullability, primary-key
flags, and enum/composite values and field order. It is reformat-invariant (whitespace, comments,
and statement order in the DDL do not affect it) and deliberately excludes column `DEFAULT`
expressions and scythe's own version.

The `queries=` field is the same idea applied to the query set — `q1:` followed by 16 hex
characters, over a canonical rendering of the *analyzed* queries: each query's name and return kind,
its parameters' names and resolved types in positional order, and its result columns' names, types
and nullability in positional order. Analyzed rather than textual, so reformatting a `.sql` file or
editing its comments produces no drift, while anything that would change generated code does.
Schema drift and query drift are reported as separate findings (`SC-PRV01` and `SC-PRV08`), so a
failure tells you which of the two moved.

:::note[Scope]
Provenance answers "was this file generated from the current schema and the current queries?" —
nothing more. It compares fingerprints, not generated code, so a change that affects output through
neither the catalog nor the analyzed query shape (a backend template change within one scythe
version, say) is outside its reach. Where a full toolchain is available, `scythe generate` followed
by `git status` still answers the stronger question by actually regenerating and diffing; provenance
verification exists for the CI/review path where running the full generator on every check is too
expensive or the toolchain (database drivers, per-language compilers) is unavailable.

An artifact generated by scythe 0.14.0 or earlier carries no `queries=` field. That is not reported
as a malformed header (`SC-PRV06`) or as drift (`SC-PRV08`) — the check is simply skipped until the
file is next regenerated.
:::

| Rule | Default | Fires when |
|------|---------|-----------|
| `SC-PRV01` | Error | The artifact's embedded schema fingerprint differs from the current schema |
| `SC-PRV02` | Warn | The artifact was generated by a different scythe version than the one running |
| `SC-PRV03` | Error | The artifact was generated by a different backend than this target now configures |
| `SC-PRV04` | Error | The artifact was generated for a different engine than this target now configures |
| `SC-PRV05` | Warn | The artifact carries no provenance header at all (predates provenance tracking, or was never scythe-managed) |
| `SC-PRV06` | Warn | A provenance header is present but is missing one or more fields (hand-edited or truncated write) |
| `SC-PRV07` | Warn | The target could not be verified: its backend/engine pair does not construct, or its artifact could not be read for a reason other than not existing |
| `SC-PRV08` | Error | The artifact's embedded query fingerprint differs from the current query set |
| `SC-PRV09` | Error | A `[[sql.gen]]` target could not be constructed -- the same checks `scythe generate` performs before writing anything (unresolvable target, backend/engine that will not build, manifest override or options that fail to apply, output path escaping the project root) |
| `SC-PRV10` | Error | A query file has content but produced zero `-- name:` / `-- @name` query blocks -- nothing in it was checked |
| `SC-PRV11` | Error | The artifact was generated with different `[[sql.gen]]` options (derive list, serde flag, `row_type`, naming case, ...) or manifest overlay contents than this target now configures |

`SC-PRV02` defaults to `Warn`, not `Error`: a scythe release is not a defect in your project, and
defaulting it to `Error` would fail every consumer's CI the day they bump scythe, before they have
had a chance to regenerate. A missing output file (the common case in a fresh checkout, since the
default `.gitignore` excludes `**/generated/`) produces no finding at all — not even `SC-PRV07`.

`SC-PRV*` severities are configurable through the same `[lint]` table as every other rule:

```toml
[lint.rules]
"SC-PRV02" = "error"   # fail CI on any scythe version drift, not just schema/backend/engine drift

[lint.categories]
provenance = "off"      # skip provenance verification entirely
```

The `SC-PRV*` rules are excluded from `scythe lint` and `scythe audit --list-rules`: they compare a
generated file against the current build, which neither command has the context to do. They only
ever appear in `scythe check` output. See the [full rule catalog](/scythe/reference/lint-rules/) for
every provenance and drift rule.

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

##### Schema drift

`--database-url` also compares the committed DDL against the live database's catalog directly —
closing exactly the nullability gap noted above by reading `pg_attribute.attnotnull`, which
preparing a statement can never report. Findings are `SC-DRF*`:

| Rule | Default | Fires when |
|------|---------|-----------|
| `SC-DRF01` | Error | A table declared in the DDL does not exist in the live database |
| `SC-DRF02` | Warn | A table exists in the live database but is not declared in the DDL |
| `SC-DRF03` | Error | A column declared in the DDL does not exist on the live table |
| `SC-DRF04` | Error | A column exists on the live table but is not declared in the DDL |
| `SC-DRF05` | Error | A column's DDL type does not match the type the live database reports |
| `SC-DRF06` | Error | A column's DDL nullability does not match the live database |
| `SC-DRF07` | Error | An enum type's DDL value set does not match the live database |

`SC-DRF02` defaults to `Warn`, not `Error`: every real database carries objects the committed DDL
never declares — a migration ledger (`schema_migrations`, `_sqlx_migrations`), extension
bookkeeping, a colleague's scratch table. Defaulting it to `Error` would fail the first run against
a production database and teach users to never pass `--database-url`. Every other rule describes the
DDL promising something the database does not deliver, which breaks generated code, so it errors.

Configure drift severities the same way as provenance:

```toml
[lint.rules]
"SC-DRF02" = "error"   # fail CI on any undeclared table, not just missing ones

[lint.categories]
drift = "off"           # skip drift checking entirely
```

Like `SC-VER*` and `SC-PRV*`, `SC-DRF*` findings only ever come from `scythe check --database-url` —
they are absent from `scythe lint` and `scythe audit --list-rules`, which have no live connection to
compare against.

The flag is opt-in and the URL is never read from the environment, so
`scythe check` cannot start requiring a database just because `DATABASE_URL`
happens to be set. Generation never needs a database at all. A natural fit is
to generate without one and verify in the CI job that has a real database.

`--database-url` is PostgreSQL only, for both halves: type verification needs the extended query
protocol's describe step, and drift detection reads `pg_catalog` directly through `tokio-postgres`.
A block configured for another engine is skipped with a warning rather than failing the run, so
`--database-url` stays harmless in a mixed-engine config.

### lint

Lint SQL files for correctness, performance, and style.

```bash
scythe lint [--config <path>] [--fix] [--dialect <dialect>] [files...]
```

| Flag | Default | Description |
|------|---------|-------------|
| `-c, --config` | `scythe.toml` | Path to config file |
| `--fix` | false | Auto-fix violations where possible |
| `--dialect` | (none) | SQL dialect for sqruff rules. If omitted: with a config file, each `[[sql]]` block uses its own `engine` (mapped to its sqruff dialect); with explicit files, scythe falls back to the first `[[sql]].engine` in the config. Falls back to `ansi` if no config resolves a dialect |
| `files...` | (from config) | SQL files to lint directly |

**Two modes:**

- **With config:** Runs both scythe rules (schema-aware) and sqruff rules. Each `[[sql]]` block is linted with its own engine's sqruff dialect, unless `--dialect` overrides it for all blocks.
- **With files:** Runs sqruff rules only (no schema context), using `--dialect` or the first `[[sql]].engine` in the config.

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
| `--dialect` | (none) | SQL dialect for formatting rules. If omitted, scythe falls back to the first `[[sql]].engine` in the config (mapped to its sqruff dialect), then to `ansi` if no config resolves one |
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

Connect to a live database and run operational health checks — missing FK indexes, disabled RLS with policies, duplicate indexes. Supports PostgreSQL (13 checks) and MySQL/MariaDB (4 checks). Emits findings in the same human/SARIF/JSON shape as `scythe audit`. See the [Inspect guide](/scythe/guide/inspect/) for the full check catalog and CI integration recipes.

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
| `--explain <CHECK_ID>` | -- | Print full rationale and remediation for a single check ID, then exit 0 |
| `-c, --config <PATH>` | `scythe.toml` | Path to config file. Supplies `[inspect].database_url` when no URL is given elsewhere |
| `--dialect <DIALECT>` | (from URL scheme) | Engine override: `postgres`/`postgresql` (13 checks) or `mysql`/`mariadb` (4 checks) |

Exits with code 2 when any error-severity finding is present (unless `--exit-zero` is set). Same convention as `scythe audit`.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Operational failure (unreadable config, parse error, I/O error), or a `scythe lint` / `scythe fmt --check` failure |
| 2 | Error-severity finding from `scythe audit`, `scythe inspect`, or `scythe check` |

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
