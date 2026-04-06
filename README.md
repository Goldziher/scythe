<div align="center">
  <img width="3384" height="573" alt="Scythe" src="https://raw.githubusercontent.com/Goldziher/scythe/main/logo.svg" />

  **SQL-to-code generator with built-in linting and formatting. Write SQL, get type-safe code.**

  [![CI](https://github.com/Goldziher/scythe/actions/workflows/ci.yml/badge.svg)](https://github.com/Goldziher/scythe/actions/workflows/ci.yml)
  [![crates.io](https://img.shields.io/crates/v/scythe-cli.svg)](https://crates.io/crates/scythe-cli)
  [![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
</div>

---

## What is Scythe

Scythe parses your SQL schema and annotated queries, infers types with precision (including nullability from JOINs, COALESCE, and aggregates), and generates idiomatic, type-safe code for 13 language backends. It also lints your SQL with 93 rules and formats it via [sqruff](https://github.com/quarylabs/sqruff) integration.

Inspired by [sqlc](https://github.com/sqlc-dev/sqlc), Scythe improves on it with standard SQL parameters (`$1`, `$2` instead of `sqlc.arg()`), smart nullability inference, a polyglot template-based backend architecture, and a comprehensive built-in linter.

## Installation

```bash
# Cargo
cargo install scythe-cli

# Homebrew
brew install Goldziher/tap/scythe
```

## Quick Start

### 1. Write SQL schema and annotated queries

```sql
-- sql/schema.sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    status TEXT NOT NULL
);

CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INT NOT NULL REFERENCES users(id),
    total INT NOT NULL
);
```

```sql
-- sql/queries/users.sql

-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;
```

### 2. Create `scythe.toml`

```toml
[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries/*.sql"]
output = "src/db/generated"

[sql.gen.rust]
target = "sqlx"
```

### 3. Run `scythe generate`

```bash
scythe generate
```

Scythe infers that `o.total` is nullable (right side of LEFT JOIN) and generates:

```rust
pub struct GetUserOrdersRow {
    pub id: i32,
    pub name: String,
    pub total: Option<i32>,
}

pub async fn get_user_orders(
    pool: &sqlx::PgPool,
    status: &str,
) -> Result<Vec<GetUserOrdersRow>, sqlx::Error> {
    // ...
}
```

## Supported Databases

| Database   | Status      |
|------------|-------------|
| PostgreSQL | Supported   |
| MySQL      | Supported   |
| SQLite     | Supported   |

## Supported Languages (13 Backends)

| Language   | Driver          | DTO Pattern                   |
|------------|-----------------|-------------------------------|
| Rust       | sqlx            | structs with derives          |
| Rust       | tokio-postgres  | structs with derives          |
| Python     | psycopg3        | dataclasses                   |
| Python     | asyncpg         | dataclasses                   |
| TypeScript | postgres.js     | interfaces                    |
| TypeScript | pg              | interfaces                    |
| Go         | pgx v5          | structs with json tags        |
| Java       | JDBC            | records                       |
| Kotlin     | JDBC            | data classes                  |
| C#         | Npgsql          | records                       |
| Elixir     | Postgrex        | defstruct                     |
| Ruby       | pg gem          | Data.define                   |
| PHP        | PDO             | readonly classes              |

Each backend is defined by a `manifest.toml` (type mappings, naming rules) and MiniJinja templates. Adding a new language requires no Rust code.

## SQL Annotations

Annotate your queries with comments to control code generation:

```sql
-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, status) VALUES ($1, $2);

-- @name GetUserById
-- @returns :one
SELECT id, name, status FROM users WHERE id = $1;

-- @name ListActiveUsers
-- @returns :many
SELECT id, name FROM users WHERE status = $1;

-- @name GetUserPayload
-- @returns :one
-- @json payload
SELECT id, payload FROM user_data WHERE id = $1;

-- @name FindUser
-- @returns :one
-- @param $1 username
-- @nullable name
-- @nonnull status
SELECT id, name, status FROM users WHERE username = $1;
```

| Annotation     | Purpose                                                |
|----------------|--------------------------------------------------------|
| `@name`        | Query function/struct name                             |
| `@returns`     | `:one`, `:many`, `:exec`, `:exec_result`, `:exec_rows` |
| `@param`       | Name a positional parameter                            |
| `@nullable`    | Force a column to be nullable                          |
| `@nonnull`     | Force a column to be non-nullable                      |
| `@json`        | Treat a column as JSON                                 |
| `@deprecated`  | Mark a query as deprecated                             |

## Configuration

```toml
# scythe.toml

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries/*.sql"]
output = "src/db/generated"

[sql.gen.rust]
target = "sqlx"

[sql.gen.python]
target = "asyncpg"

[lint.rules]
"SC-S03" = "off"      # allow SELECT *
"SC-P01" = "error"    # enforce ORDER BY + LIMIT

[lint.sqruff]
"LT01" = "warn"       # spacing
"CP01" = "off"        # keyword capitalization
```

## CLI Commands

| Command             | Description                                            |
|---------------------|--------------------------------------------------------|
| `scythe generate`   | Parse SQL and generate type-safe code                  |
| `scythe check`      | Parse, analyze, and lint without generating code       |
| `scythe lint`       | Standalone linting with `--fix` auto-repair            |
| `scythe fmt`        | Format SQL files with `--check` and `--diff` modes     |
| `scythe migrate`    | Convert from sqlc to scythe format                     |

## Linting (93 Rules)

### Scythe rules (22) — codegen-aware, uses type inference and catalog

| Code | Rule | Description |
|------|------|-------------|
| SC-S01 | update-without-where | UPDATE without WHERE affects all rows |
| SC-S02 | delete-without-where | DELETE without WHERE affects all rows |
| SC-S03 | no-select-star | SELECT * is fragile for codegen |
| SC-S04 | unused-params | Declared parameters not used in query |
| SC-S05 | missing-returning | DML with :one/:many but no RETURNING |
| SC-S06 | ambiguous-column | Unqualified column in multi-table query |
| SC-C01 | missing-returns | SELECT without @returns annotation |
| SC-C02 | exec-with-returning | :exec discards RETURNING data |
| SC-C03 | duplicate-query-names | Same query name used twice |
| SC-N01 | snake-case-columns | Column aliases should be snake_case |
| SC-N02 | snake-case-tables | Table names should be snake_case |
| SC-N03 | query-name-convention | Query names should start with a verb |
| SC-N04 | alias-casing | Table aliases should be lowercase |
| SC-A01 | not-equal-null | `= NULL` should be `IS NULL` |
| SC-A02 | implicit-coercion | Comparing columns of different types |
| SC-A03 | or-in-join | OR in JOIN ON is usually a mistake |
| SC-P01 | order-without-limit | ORDER BY without LIMIT |
| SC-P02 | like-wildcard | LIKE with leading % can't use indexes |
| SC-P03 | not-in-subquery | NOT IN (subquery) has NULL gotcha |
| SC-T01 | implicit-join | Comma joins should be explicit JOINs |
| SC-T02 | coalesce-over-case | CASE WHEN IS NULL → COALESCE |
| SC-T03 | count-star | COUNT(1) should be COUNT(*) |

### sqruff rules (71) -- formatting, capitalization, structure

Aliasing, ambiguity, capitalization, conventions, layout (spacing/indentation), references, and structure rules. All configurable via `scythe.toml` under `[lint.sqruff]`.

## Formatting

Scythe integrates [sqruff](https://github.com/quarylabs/sqruff) for SQL formatting with multi-dialect support (PostgreSQL, MySQL, BigQuery, Snowflake, and more).

```bash
# Format SQL files in place
scythe fmt

# Check formatting without modifying files
scythe fmt --check

# Show formatting diff
scythe fmt --diff
```

## Migration from sqlc

Scythe includes a migration command that converts `sqlc.yaml` configuration and query annotations to scythe format:

```bash
scythe migrate sqlc.yaml
```

This generates a `scythe.toml` and rewrites `sqlc.arg()` calls to standard `$N` parameters.

## Architecture

```text
SQL Schema + Annotated Queries
        |
        v
    Parse (sqlparser-rs)
        |
        v
    Analyze (type inference, nullability)
        |
        v
    Lint (93 rules) + Format (sqruff)
        |
        v
    Backend (manifest.toml + MiniJinja templates)
        |
        v
    Generated Code (Rust, Python, TypeScript, Go, ...)
```

The analyzer outputs a language-neutral type vocabulary. Each backend maps these to concrete language types via `manifest.toml`:

```toml
# backends/rust-sqlx/manifest.toml
[types.scalars]
int32 = "i32"
string = "String"
datetime_tz = "chrono::DateTime<chrono::Utc>"
json = "serde_json::Value"

[types.containers]
array = "Vec<{T}>"
nullable = "Option<{T}>"
```

## Project Structure

```text
crates/
  scythe-core/        # catalog, parser, analyzer, errors
  scythe-codegen/     # code generation via backend templates
  scythe-lint/        # 22 custom rules + 71 sqruff rules + engine
  scythe-backend/     # type resolution, naming, MiniJinja rendering
  scythe-cli/         # CLI binary (generate, check, lint, fmt, migrate)

backends/
  rust-sqlx/          # Rust + sqlx
  rust-tokio-postgres/# Rust + tokio-postgres
  python-psycopg3/    # Python + psycopg3
  python-asyncpg/     # Python + asyncpg
  typescript-postgres/# TypeScript + postgres.js
  typescript-pg/      # TypeScript + pg
  go-pgx/             # Go + pgx v5
  java-jdbc/          # Java + JDBC
  kotlin-jdbc/        # Kotlin + JDBC
  csharp-npgsql/      # C# + Npgsql
  elixir-postgrex/    # Elixir + Postgrex
  ruby-pg/            # Ruby + pg gem
  php-pdo/            # PHP + PDO

tools/
  migrate-fixtures/   # fixture migration utility
  test-generator/     # generates Rust tests from JSON fixtures
```

## License

[MIT](LICENSE)
