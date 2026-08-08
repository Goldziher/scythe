---
title: Inspect (live database)
description: Connect to a running database and run operational health checks with scythe inspect.
---

`scythe inspect` connects to a running database and runs a set of catalog
checks for operational issues that **only emerge in a live system** — foreign
keys without covering indexes, tables with policies but Row Level Security
disabled, duplicate indexes, and (in later phases) schema drift, unused
indexes, slow queries.

It is the live counterpart to `scythe audit` (static rules) and `scythe lint`
(schema-aware static rules + sqruff). All three share the same `Finding`
shape, severity model, and reporter dispatch — output is human-readable text,
SARIF 2.1.0, or JSON.

## Quick start

```bash
# Default reporter (human-readable, grouped by file)
scythe inspect postgres://user:pass@localhost/mydb

# SARIF 2.1.0 for GitHub Actions code-scanning
scythe inspect "$DATABASE_URL" --format sarif --output report.sarif

# Print the check catalog and exit
scythe inspect --list-checks
```

The connection URL is resolved in order: positional argument →
`$DATABASE_URL` → `$SCYTHE_DATABASE_URL` → `[inspect].database_url` in
`scythe.toml`. If none is set, `scythe inspect` exits with a clear error.

## Phase 0 check catalog

Phase 0 (v0.10.0) ships three Postgres checks. The catalog grows in later
phases — see the roadmap below.

| ID | Name | Severity | Detection |
|---|---|---|---|
| SC-INS01 | missing-fk-index | warn | Foreign-key columns with no covering index — every join through the constraint forces a sequential scan. |
| SC-INS02 | policy-exists-rls-disabled | error | Table has `CREATE POLICY` definitions but `ROW LEVEL SECURITY` is disabled — policies never apply. |
| SC-INS03 | duplicate-index | warn | Two or more indexes on the same table have identical definitions modulo name — wasted writes and storage. |

Detection patterns are clean-room reimplementations of the equivalent
supabase/splinter lints (0001, 0006, 0009). See `ATTRIBUTIONS.md`.

## Severity and exit codes

`scythe inspect` follows the same exit-code convention as `scythe audit`:

- **0** — no findings, or no error-severity findings.
- **2** — at least one error-severity finding.
- **1** — runtime error (couldn't connect, query failed, bad config).

The `--exit-zero` flag forces exit 0 even when error-severity findings are
present, for advisory CI integration that publishes findings without
blocking a merge.

`--severity warn|error` drops findings below the given level before
emission. The default keeps everything.

## Engine support

Phase 0 supports PostgreSQL (and PostgreSQL-compatible engines like
CockroachDB; see the [`SqlDialect::from_str`](https://docs.rs/scythe-core/)
mapping for the full list of accepted scheme aliases). MySQL is recognised
but stubbed — `scythe inspect --dialect mysql --list-checks` prints
"no checks available for engine `mysql` at Phase 0". A real MySQL driver
lands in Phase 3.

Other engines (MSSQL, Snowflake, Oracle) are not yet wired.

## Project configuration (`[inspect]` in `scythe.toml`)

`scythe.toml` accepts an `[inspect]` section, mirroring the shape of
`[audit]`:

```toml
[inspect]
database_url = "postgres://localhost/dev"
api_schemas  = ["public", "api"]
extra_rules  = ["./inspect-rules.toml"]

[inspect.severity_overrides]
"SC-INS10" = "error"
"SC-INS13" = "off"

[[inspect.suppression]]
rule   = "SC-INS09"
schema = "public"
object = "pgtap"

[[inspect.check]]
id          = "USER-INS-001"
name        = "no-comments-on-tables"
category    = "schema"
severity    = "warn"
engines     = ["postgres"]
description = "tables must have COMMENT ON TABLE"
message     = "table `{schema_name}.{table_name}` has no COMMENT"
sql         = """
SELECT n.nspname AS schema_name, c.relname AS table_name
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind = 'r' AND n.nspname = 'app'
  AND obj_description(c.oid, 'pg_class') IS NULL
"""
```

- `database_url` — lowest-precedence URL source; see the resolution order
  above.
- `api_schemas` — schemas treated as the "API surface" for SC-INS10 (tables
  without RLS). Defaults to `["public"]` when empty.
- `extra_rules` — paths to additional check TOML files, resolved relative to
  `scythe.toml`.
- `[inspect.severity_overrides]` — per-check severity override keyed by
  check ID. Value is `"warn"`, `"error"`, or `"off"`; `"off"` removes the
  check from the active set.
- `[[inspect.suppression]]` — suppresses matching findings. A finding is
  suppressed when every `Some` field matches: `rule` (required), `schema`
  (matches a row binding whose name contains `"schema"`), `object` (matches
  a row binding whose name contains `"name"`).
- `[[inspect.check]]` — inline user-defined checks. IDs must be prefixed
  `USER-INS-`; every `{binding}` placeholder in `message` must exist as a
  column in the check's `sql`. Both are validated when `scythe.toml` loads.

`scythe inspect --explain <CHECK_ID>` prints a check's full rationale
without connecting to a database, for both canonical `SC-INS*` and
user-defined `USER-INS-*` checks.

## CI integration

### GitHub Actions

```yaml
name: scythe inspect

on:
  pull_request:
    branches: [main]

jobs:
  inspect:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:16-alpine
        env:
          POSTGRES_USER: scythe
          POSTGRES_PASSWORD: scythe
          POSTGRES_DB: scythe_ci
        ports: ["5432:5432"]
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
    steps:
      - uses: actions/checkout@v6
      - name: Apply schema
        run: PGPASSWORD=scythe psql -h localhost -U scythe -d scythe_ci -f schema.sql
      - name: Install scythe
        run: cargo install scythe-cli --locked
      - name: Inspect
        run: scythe inspect postgres://scythe:scythe@localhost/scythe_ci --format sarif --output inspect.sarif
      - uses: github/codeql-action/upload-sarif@v3
        with: { sarif_file: inspect.sarif }
```

### Pre-commit hook (CI mode)

The published `scythe-inspect` pre-commit hook runs `scythe inspect` with no
positional URL argument, so it resolves the connection URL the same way the
CLI does: `$DATABASE_URL` → `$SCYTHE_DATABASE_URL` →
`[inspect].database_url` in `scythe.toml`. Runs where none of the three is
set fail loudly with the same error as the CLI.

```yaml
- repo: https://github.com/Goldziher/scythe
  rev: v0.13.0
  hooks:
    - id: scythe-inspect
      # args: [--exit-zero]    # uncomment for advisory CI integration
```

Set `[inspect].database_url` in `scythe.toml` (see above) to make the hook
usable in ordinary local pre-commit runs, not just CI jobs that export
`$DATABASE_URL`.

## What `scythe inspect` does **not** do (yet)

`scythe inspect` is a per-invocation CLI command, not a daemon. It connects,
runs a fixed set of queries, prints findings, and exits. There is no
continuous monitoring, no historical state, no anomaly detection. For
those, point a real observability stack (pganalyze, Datadog, Prometheus +
postgres_exporter) at your database — `scythe inspect` is for the things you
can catch with a single catalog snapshot.

Also not implemented (by `scythe inspect` specifically):

- **Stats-based checks** (unused indexes via `pg_stat_user_indexes`, slow
  queries via `pg_stat_statements`, bloat via `pgstattuple`) — Phase 4.

:::note[Schema drift shipped, but in `scythe check`, not here]
Schema drift (declared `scythe.toml` catalog vs. live database) is implemented as of v0.14.0, as
`SC-DRF01`–`SC-DRF07` under [`scythe check --database-url`](/scythe/guide/cli-reference/#schema-drift),
not as a `scythe inspect` check. It reads `pg_catalog` the same way `scythe inspect`'s checks do, but
answers a different question ("does my committed schema still match the database?") than `scythe
inspect` does ("does my live database have an operational problem?").
:::

## Phased roadmap

| Phase | Release | Theme | Engines | Checks |
|---|---|---|---|---|
| **0** | v0.10.0 (shipped) | MVP — three Postgres checks | PG (MySQL stub) | SC-INS01..03 |
| **1** | v0.11.0 (shipped) | Full PG check pack + TOML rule registry + `--explain` + `[inspect]` config | PG | SC-INS04..13 |
| **2** | v0.14.0 (shipped, as `scythe check --database-url`) | Schema drift — declared catalog vs live | PG | SC-DRF01..07 |
| **3** | v0.13.0 | MySQL driver + initial MySQL check pack | PG + MySQL | SC-INS-MY01..06 |
| **4** | v0.14.0 | Stats-based — unused indexes, slow queries via `pg_stat_*` | PG | SC-INS-STAT01..04 |
