---
name: scythe
description: >-
  Generate type-safe database access code from annotated SQL queries in 10
  languages across 10 databases. Use when writing SQL with scythe annotations,
  configuring scythe.toml, choosing backends, linting/formatting SQL, or
  integrating scythe into a project.
license: MIT
metadata:
  author: Goldziher
  version: "0.6.0"
  repository: https://github.com/Goldziher/scythe
---

# Scythe SQL-to-Code Generator

Scythe compiles annotated SQL into type-safe database access code. You write SQL queries with annotations, scythe generates the boilerplate -- structs, functions, type mappings -- in 10 languages across 10 databases with 56 backends. Built-in linting (59 rules) and formatting catch SQL bugs before they ship.

Use this skill when:

- Writing SQL queries with scythe annotations (`@name`, `@returns`, `@optional`, etc.)
- Configuring `scythe.toml` for code generation
- Choosing which backend driver to use for a language/database combination
- Linting or formatting SQL files
- Setting up pre-commit hooks for SQL quality
- Migrating from sqlc to scythe

## Installation

```bash
cargo install scythe-cli
# or
brew install Goldziher/tap/scythe
```

## Quick Start

**1. Write annotated SQL:**

```sql
-- @name GetUserById
-- @returns :one
SELECT id, name, email FROM users WHERE id = $1;
```

**2. Configure `scythe.toml`:**

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema/*.sql"]
queries = ["sql/queries/*.sql"]

[[sql.gen]]
backend = "rust-sqlx"
output = "src/generated"
```

**3. Generate code:**

```bash
scythe generate
```

## CLI Commands

```bash
scythe generate [--config <path>]              # Generate code (default: scythe.toml)
scythe check [--config <path>]                 # Validate SQL without generating
scythe lint [--config <path>] [--fix] [files]  # Lint SQL files
scythe audit [--config <path>] [files]         # Security scan (SC-SEC*)
scythe inspect [database_url]                  # Operational checks on a live database
scythe fmt [--config <path>] [--check] [files] # Format SQL files
scythe migrate [sqlc_config]                   # Convert sqlc project
```

| Flag | Commands | Description |
|------|----------|-------------|
| `-c, --config` | all | Path to config file (default: `scythe.toml`) |
| `--fix` | lint | Auto-fix violations |
| `--check` | fmt | Check without modifying (exit 1 if changes needed) |
| `--diff` | fmt | Show unified diff of changes |
| `--dialect` | lint, fmt, audit, inspect | SQL dialect: `postgres`, `mysql`, `sqlite`, `mssql`, `oracle`, `snowflake` |
| `--database-url` | check | Verify types and detect schema drift against a live database (PostgreSQL only) |
| `--database-url` | lint | Also run the live-database `inspect` checks as part of `lint` (PostgreSQL or MySQL/MariaDB) |
| `--format` | check, lint, audit, inspect | `human` (default), `sarif`, `json` |
| `-o, --output` | check, lint, audit, inspect | Write findings to a file instead of stdout |
| `--severity` | audit, inspect | Drop findings below `off`, `warn`, or `error` |
| `--exit-zero` | check, lint, audit, inspect | Exit 0 even when error-severity findings are present |
| `files...` | lint, fmt, audit | Specific SQL files (if empty, uses config) |

**Exit codes:** 0 clean, **2** on error-severity findings from `check`, `lint`,
`audit` or `inspect`, 1 on operational failure (unreadable config, unparseable SQL, I/O
error). `--exit-zero` collapses 2 to 0 and leaves 1 alone. Findings failures and
"scythe could not run" are deliberately distinguishable, so CI never mistakes a
crashed run for a clean one.

See [references/cli-reference.md](references/cli-reference.md) for the full flag
tables.

## Annotations

All annotations use `-- @` prefix in SQL comments.

| Annotation | Required | Description |
|------------|----------|-------------|
| `@name QueryName` | Yes | Names the generated function and struct |
| `@returns :type` | Yes | Return type: `:one`, `:many`, `:exec`, `:exec_result`, `:batch`, `:grouped` |
| `@group_by table.column` | With `:grouped` | Specifies parent table for grouped results |
| `@optional param` | No | Makes a parameter optional (SQL rewritten to skip filter when NULL) |
| `@param name: desc` | No | Documents a parameter |
| `@nullable col1, col2` | No | Forces columns to be nullable |
| `@nonnull col1, col2` | No | Forces columns to be non-nullable |
| `@json col = TypeName` | No | Maps column to typed JSON struct |
| `@deprecated message` | No | Marks query as deprecated |

### @returns values

| Value | Description |
|-------|-------------|
| `:one` | Single row (SELECT ... WHERE id = $1) |
| `:many` | Multiple rows (SELECT ... WHERE status = $1) |
| `:exec` | No return (INSERT, UPDATE, DELETE) |
| `:exec_result` | Returns affected row count |
| `:batch` | Bulk execution |
| `:grouped` | Rows grouped by `@group_by` key |

### @optional rewriting

`@optional` rewrites `WHERE col = $1` into `WHERE ($1 IS NULL OR col = $1)`. Works with: `=`, `<>`, `!=`, `>`, `<`, `>=`, `<=`, `LIKE`, `ILIKE`.

```sql
-- @name SearchUsers
-- @returns :many
-- @optional status
-- @optional name_pattern
SELECT id, name FROM users
WHERE status = $1 AND name ILIKE $2;
```

### @json typed mapping

```sql
-- @name GetEvent
-- @returns :one
-- @json data = EventData
SELECT id, data FROM events WHERE id = $1;
```

Generates `Json<EventData>` (Rust), `EventData` (TypeScript), etc.

### @returns :grouped

```sql
-- @name GetUsersWithOrders
-- @returns :grouped
-- @group_by users.id
SELECT u.id, u.name, o.id AS order_id, o.total
FROM users u JOIN orders o ON o.user_id = u.id;
```

Generates a parent struct with nested child collection.

## Configuration

### scythe.toml structure

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"           # postgresql, mysql, sqlite, duckdb, cockroachdb, mssql, oracle, mariadb, redshift, snowflake
schema = ["sql/schema/*.sql"]
queries = ["sql/queries/*.sql"]

# Multiple backends from one SQL block
[[sql.gen]]
backend = "rust-sqlx"
output = "src/generated/rust"

[[sql.gen]]
backend = "typescript-pg"
output = "src/generated/ts"
row_type = "zod"                # optional: interface (default) or zod

[[sql.gen]]
backend = "python-psycopg3"
output = "src/generated/python"
row_type = "pydantic"           # optional: dataclass (default), pydantic, or msgspec

# Type overrides
[[sql.type_overrides]]
column = "users.metadata"      # specific column
type = "json"

[[sql.type_overrides]]
db_type = "citext"             # all columns of this type
type = "string"

# Lint configuration
[lint.categories]
safety = "error"
naming = "warn"

[lint.rules]
"SC-S03" = "off"               # disable SELECT * warning
```

### Engine aliases

| Alias | Engine |
|-------|--------|
| `postgresql`, `postgres`, `pg` | PostgreSQL |
| `mysql` | MySQL |
| `sqlite`, `sqlite3` | SQLite |
| `duckdb` | DuckDB |
| `cockroachdb`, `crdb` | CockroachDB |
| `mssql`, `sqlserver` | MSSQL |
| `oracle` | Oracle |
| `mariadb` | MariaDB |
| `redshift` | Redshift |
| `snowflake` | Snowflake |

### row_type options

| Language | Values |
|----------|--------|
| Python | `dataclass` (default), `pydantic`, `msgspec` |
| TypeScript | `interface` (default), `zod` |

### structs_only option

`structs_only = "true"` emits only row types (interfaces/Zod schemas, enums, composites) and suppresses query functions and the driver import. Supported by `rust-sqlx` and every TypeScript backend (`typescript-postgres`, `typescript-pg`, `typescript-mysql2`, `typescript-better-sqlite3`, `typescript-node-sqlite`, `typescript-wasm-sqlite`, `typescript-duckdb`, `typescript-mssql`, `typescript-oracledb`, `typescript-snowflake`, `typescript-kysely`). Combine with `row_type = "zod"` for a types-only package with no driver dependency:

```toml
[[sql.gen]]
backend = "typescript-pg"
output = "src/generated/types"
row_type = "zod"
structs_only = "true"
```

## Supported Backends

### PostgreSQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `rust-tokio-postgres` | Rust | tokio-postgres |
| `python-psycopg3` | Python | psycopg3 |
| `python-asyncpg` | Python | asyncpg |
| `typescript-postgres` | TypeScript | postgres.js |
| `typescript-pg` | TypeScript | pg |
| `typescript-kysely` | TypeScript | Kysely (`PostgresDialect`) |
| `javascript-postgres` | JavaScript | postgres.js (JSDoc types) |
| `javascript-pg` | JavaScript | pg (JSDoc types) |
| `go-pgx` | Go | pgx v5 |
| `java-jdbc` | Java | JDBC |
| `java-r2dbc` | Java | R2DBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `kotlin-r2dbc` | Kotlin | R2DBC |
| `kotlin-exposed` | Kotlin | Exposed |
| `csharp-npgsql` | C# | Npgsql |
| `elixir-postgrex` | Elixir | Postgrex |
| `elixir-ecto` | Elixir | Ecto |
| `ruby-pg` | Ruby | pg |
| `php-pdo` | PHP | PDO |
| `php-amphp` | PHP | AMPHP |

### MySQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-aiomysql` | Python | aiomysql |
| `typescript-mysql2` | TypeScript | mysql2 |
| `typescript-kysely` | TypeScript | Kysely (`MysqlDialect`) |
| `javascript-mysql2` | JavaScript | mysql2 (JSDoc types) |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-mysqlconnector` | C# | MySqlConnector |
| `elixir-myxql` | Elixir | MyXQL |
| `ruby-mysql2` | Ruby | mysql2 |
| `ruby-trilogy` | Ruby | Trilogy |
| `php-pdo` | PHP | PDO |
| `php-amphp` | PHP | AMPHP |

### SQLite

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-aiosqlite` | Python | aiosqlite |
| `typescript-better-sqlite3` | TypeScript | better-sqlite3 |
| `typescript-node-sqlite` | TypeScript | node:sqlite (sync, zero deps) |
| `typescript-wasm-sqlite` | TypeScript | @sqlite.org/sqlite-wasm (sync) |
| `typescript-kysely` | TypeScript | Kysely (`SqliteDialect`) |
| `javascript-better-sqlite3` | JavaScript | better-sqlite3 (JSDoc types, sync) |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-microsoft-sqlite` | C# | Microsoft.Data.Sqlite |
| `elixir-exqlite` | Elixir | Exqlite |
| `ruby-sqlite3` | Ruby | sqlite3 |
| `php-pdo` | PHP | PDO |

`typescript-node-sqlite` and `typescript-wasm-sqlite` generate synchronous code -- plain `export function`, no `async`, no `Promise` -- unlike every other TypeScript backend ([#66](https://github.com/Goldziher/scythe/issues/66)). `typescript-node-sqlite` requires `--experimental-sqlite` on Node 22 and is unflagged from Node 23.4 onward; generated code needs Node 23.4+ to run without the flag.

`javascript-postgres`, `javascript-pg`, `javascript-mysql2`, and `javascript-better-sqlite3` are a JSDoc emit mode on the matching TypeScript backend, not separate backends ([#81](https://github.com/Goldziher/scythe/issues/81)). Output is `queries.js`: plain ESM, no driver import (handles are typed inline as `import("pg").PoolClient` in `@param`), row types as `@typedef {object}` with nullable columns always `{T | null}`. `row_type = "zod"`, `outer_join_unions`, and `field_case = "camelCase"` are hard errors there -- each needs TypeScript-only syntax; `structs_only` and `field_case = "snake_case"` work. No other TypeScript backend has a JavaScript counterpart.

### DuckDB

| Backend | Language | Library |
|---------|----------|---------|
| `python-duckdb` | Python | duckdb |
| `typescript-duckdb` | TypeScript | duckdb-node |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |

### CockroachDB

CockroachDB uses PostgreSQL backends with `engine = "cockroachdb"`:
`rust-sqlx`, `python-psycopg3`, `typescript-pg`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`, `csharp-npgsql`, `ruby-pg`, `php-pdo`, `php-amphp`, `elixir-postgrex`.

### MSSQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-tiberius` | Rust | tiberius |
| `python-pyodbc` | Python | pyodbc |
| `typescript-mssql` | TypeScript | mssql (tedious) |
| `typescript-kysely` | TypeScript | Kysely (`MssqlDialect`, tedious+tarn) |
| `go-database-sql` | Go | go-mssqldb |
| `java-jdbc` | Java | JDBC |
| `java-r2dbc` | Java | R2DBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `kotlin-r2dbc` | Kotlin | R2DBC |
| `csharp-sqlclient` | C# | Microsoft.Data.SqlClient |
| `ruby-tiny-tds` | Ruby | tiny_tds |
| `php-pdo` | PHP | PDO |
| `elixir-tds` | Elixir | tds |

### Oracle

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sibyl` | Rust | sibyl |
| `python-oracledb` | Python | oracledb |
| `typescript-oracledb` | TypeScript | oracledb |
| `go-godror` | Go | godror |
| `java-jdbc` | Java | JDBC |
| `java-r2dbc` | Java | R2DBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `kotlin-r2dbc` | Kotlin | R2DBC |
| `csharp-oracle` | C# | ODP.NET |
| `ruby-oci8` | Ruby | ruby-oci8 |
| `elixir-jamdb` | Elixir | jamdb_oracle |

`php-pdo` does not support Oracle (no `oci` engine mapping) despite PDO's own OCI driver; there is no `php-pdo` Oracle backend.

### MariaDB

MariaDB uses MySQL drivers with MariaDB-specific type resolution:
`rust-sqlx`, `python-aiomysql`, `typescript-mysql2`, `typescript-kysely`, `javascript-mysql2`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-mysqlconnector`, `elixir-myxql`, `ruby-mysql2`, `ruby-trilogy`, `php-pdo`, `php-amphp`.

### Redshift

Redshift uses PostgreSQL backends with `engine = "redshift"`:
`rust-sqlx`, `rust-tokio-postgres`, `python-psycopg3`, `python-asyncpg`, `typescript-pg`, `typescript-postgres`, `typescript-kysely`, `javascript-pg`, `javascript-postgres`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`, `csharp-npgsql`, `elixir-postgrex`, `ruby-pg`, `php-pdo`.

### Snowflake

| Backend | Language | Library |
|---------|----------|---------|
| `python-snowflake` | Python | snowflake-connector-python |
| `typescript-snowflake` | TypeScript | snowflake-sdk |
| `go-gosnowflake` | Go | gosnowflake |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-snowflake` | C# | Snowflake.Data |
| `php-pdo` | PHP | PDO |

## Type System

### Type resolution pipeline

```text
SQL type  -->  neutral type  -->  language type
SERIAL         int32              i32 (Rust) / int (Python) / number (TS)
TIMESTAMPTZ    datetime_tz        chrono::DateTime<Utc> / datetime / Date
TEXT[]         array<string>      Vec<String> / list[str] / string[]
```

### Type inference

Scythe infers nullability from SQL context:

- **LEFT JOIN**: Right-side columns become nullable
- **RIGHT JOIN**: Left-side columns become nullable
- **COALESCE**: Result is non-nullable
- **Aggregates**: COUNT is non-nullable; SUM/AVG/MIN/MAX are nullable
- **CASE WHEN**: Nullable unless all branches and ELSE are non-nullable
- **Subqueries**: Scalar subqueries are nullable unless the subquery is a single ungrouped, non-windowed aggregate

Override with `@nullable` and `@nonnull` annotations.

### Custom type overrides

```toml
# Column-level (takes precedence)
[[sql.type_overrides]]
column = "users.metadata"
type = "json"

# Database type-level
[[sql.type_overrides]]
db_type = "ltree"
type = "string"
```

Common PostgreSQL extension mappings:

| DB Type | Neutral Type |
|---------|-------------|
| `ltree`, `citext`, `tsvector`, `macaddr` | `string` |
| `hstore` | `json` |
| `money` | `decimal` |
| `geometry` (PostGIS) | `string` |

## Linting

23 built-in scythe lint rules + 35 audit rules = 58 built-in, plus sqruff's 69 style rules via integration.

The 11 `SC-PRV*` provenance rules and the 7 `SC-DRF*` schema drift rules are not part of the 58. They
run only from `scythe check` and never appear in `scythe lint` or `scythe audit --list-rules` output.

### Rule categories

| Category | Prefix | Examples |
|----------|--------|---------|
| Safety | `SC-S` | UPDATE/DELETE without WHERE, SELECT *, unused params |
| Naming | `SC-N` | snake_case tables/columns, verb prefixes on queries |
| Style | `SC-T` | Prefer explicit JOINs, COALESCE, COUNT(*) |
| Performance | `SC-P` | Missing ORDER BY with LIMIT, leading wildcard LIKE |
| Antipattern | `SC-A` | NULL comparisons with =, implicit type coercion |
| Codegen | `SC-C` | Missing @returns, duplicate @name |
| Security | `SC-SEC`, `SC-RLS` | Dangerous functions, GRANT ALL, RLS misconfiguration |
| Migration | `SC-MIG` | Irreversible or lock-prone DDL |
| Provenance | `SC-PRV` | `scythe check` only: generated artifact vs. current schema/engine/backend/version |
| Drift | `SC-DRF` | `scythe check --database-url` only: committed DDL vs. a live database |

### Configuration

```toml
[lint.categories]
safety = "error"
naming = "warn"
style = "off"

[lint.rules]
"SC-S03" = "off"        # allow SELECT *
"SC-N03" = "error"      # enforce query naming
```

`SC-PRV*` and `SC-DRF*` are configured from the same table -- `scythe check` applies `[lint]` to their
registries before resolving severities:

```toml
[lint.rules]
"SC-PRV02" = "error"    # fail CI on scythe version drift
"SC-DRF02" = "error"    # fail on tables the DDL never declares

[lint.categories]
provenance = "off"      # skip provenance verification
drift = "off"           # skip schema drift checking
```

## Pre-commit Hooks

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.16.0
    hooks:
      - id: scythe-fmt       # Format SQL files
      - id: scythe-lint      # Lint SQL with auto-fix
      - id: scythe-audit     # Security scan
      - id: scythe-inspect   # Live-database operational checks
      - id: scythe-generate  # Regenerate code on SQL changes
      - id: scythe-check     # Validate SQL without generating
```

## Common Pitfalls

1. **Missing `@returns`**: Every query needs both `@name` and `@returns` annotations.
2. **`:one` vs `:many`**: Use `:one` only for queries guaranteed to return 0-1 rows (WHERE id = $1). `:one` returns `Option<T>` / `T | null`.
3. **LEFT JOIN nullability**: Columns from the right side of LEFT JOIN are always nullable. Use `@nonnull` to override if you know better.
4. **`@optional` parameter names**: Must match a parameter in the query. Typos produce errors.
5. **Engine mismatch**: Backend must support the configured engine (e.g., `python-asyncpg` only works with `postgresql`).
6. **Multiple `[[sql.gen]]` blocks**: Each needs its own `output` directory.
7. **Type overrides**: `column` and `db_type` are mutually exclusive in each override entry.

## Additional Resources

Detailed reference files for specific topics:

- **[Configuration Reference](references/configuration.md)** -- Full scythe.toml reference
- **[Annotations Reference](references/annotations.md)** -- All annotations with examples
- **[Backends Reference](references/backends.md)** -- All 56 backend names with engine support
- **[Lint Rules Reference](references/lint-rules.md)** -- All rules with codes and examples
- **[CLI Reference](references/cli-reference.md)** -- All commands, flags, exit codes

Full documentation: <https://goldziher.github.io/scythe>
GitHub: <https://github.com/Goldziher/scythe>
