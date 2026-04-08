---
name: scythe
description: >-
  Generate type-safe database access code from annotated SQL queries in 10
  languages across 5 databases. Use when writing SQL with scythe annotations,
  configuring scythe.toml, choosing backends, linting/formatting SQL, or
  integrating scythe into a project.
license: MIT
metadata:
  author: Goldziher
  version: "0.5.0"
  repository: https://github.com/Goldziher/scythe
---

# Scythe SQL-to-Code Generator

Scythe compiles annotated SQL into type-safe database access code. You write SQL queries with annotations, scythe generates the boilerplate -- structs, functions, type mappings -- in 10 languages across 5 databases with 34 backend drivers. Built-in linting (93 rules) and formatting catch SQL bugs before they ship.

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
scythe fmt [--config <path>] [--check] [files] # Format SQL files
scythe migrate [sqlc_config]                   # Convert sqlc project
```

| Flag | Commands | Description |
|------|----------|-------------|
| `-c, --config` | all | Path to config file (default: `scythe.toml`) |
| `--fix` | lint | Auto-fix violations |
| `--check` | fmt | Check without modifying (exit 1 if changes needed) |
| `--diff` | fmt | Show unified diff of changes |
| `--dialect` | lint, fmt | SQL dialect: `ansi`, `postgres`, `mysql` |
| `files...` | lint, fmt | Specific SQL files (if empty, uses config) |

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
engine = "postgresql"           # postgresql, mysql, sqlite, duckdb, cockroachdb
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
| `mysql`, `mariadb` | MySQL |
| `sqlite`, `sqlite3` | SQLite |
| `duckdb` | DuckDB |
| `cockroachdb`, `crdb` | CockroachDB |

### row_type options

| Language | Values |
|----------|--------|
| Python | `dataclass` (default), `pydantic`, `msgspec` |
| TypeScript | `interface` (default), `zod` |

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
| `go-pgx` | Go | pgx v5 |
| `java-jdbc` | Java | JDBC |
| `java-r2dbc` | Java | R2DBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `kotlin-r2dbc` | Kotlin | R2DBC |
| `kotlin-exposed` | Kotlin | Exposed |
| `csharp-npgsql` | C# | Npgsql |
| `elixir-postgrex` | Elixir | Postgrex |
| `ruby-pg` | Ruby | pg |
| `php-pdo` | PHP | PDO |

### MySQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-aiomysql` | Python | aiomysql |
| `typescript-mysql2` | TypeScript | mysql2 |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-mysqlconnector` | C# | MySqlConnector |
| `elixir-myxql` | Elixir | MyXQL |
| `ruby-mysql2` | Ruby | mysql2 |
| `php-pdo` | PHP | PDO |

### SQLite

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-aiosqlite` | Python | aiosqlite |
| `typescript-better-sqlite3` | TypeScript | better-sqlite3 |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-microsoft-sqlite` | C# | Microsoft.Data.Sqlite |
| `elixir-exqlite` | Elixir | Exqlite |
| `ruby-sqlite3` | Ruby | sqlite3 |
| `php-pdo` | PHP | PDO |

### DuckDB

| Backend | Language | Library |
|---------|----------|---------|
| `python-duckdb` | Python | duckdb |
| `rust-duckdb` | Rust | duckdb-rs |
| `typescript-duckdb` | TypeScript | duckdb-node |

### CockroachDB

CockroachDB uses PostgreSQL backends with `engine = "cockroachdb"`:
`rust-sqlx`, `python-psycopg3`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`.

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
- **Subqueries**: Scalar subqueries are nullable

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

22 built-in scythe rules + 71 sqruff rules = 93 total.

### Rule categories

| Category | Prefix | Examples |
|----------|--------|---------|
| Safety | `SC-S` | UPDATE/DELETE without WHERE, SELECT *, unused params |
| Naming | `SC-N` | snake_case tables/columns, verb prefixes on queries |
| Style | `SC-T` | Prefer explicit JOINs, COALESCE, COUNT(*) |
| Performance | `SC-P` | Missing ORDER BY with LIMIT, leading wildcard LIKE |
| Antipattern | `SC-A` | NULL comparisons with =, implicit type coercion |
| Codegen | `SC-C` | Missing @returns, duplicate @name |

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

## Pre-commit Hooks

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.5.0
    hooks:
      - id: scythe-fmt       # Format SQL files
      - id: scythe-lint      # Lint SQL with auto-fix
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
- **[Backends Reference](references/backends.md)** -- All 34 backends with engine support
- **[Lint Rules Reference](references/lint-rules.md)** -- All 93 rules with codes and examples
- **[CLI Reference](references/cli-reference.md)** -- All commands, flags, exit codes

Full documentation: <https://goldziher.github.io/scythe>
GitHub: <https://github.com/Goldziher/scythe>
