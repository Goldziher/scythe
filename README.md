<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/Goldziher/scythe/main/logo.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/Goldziher/scythe/main/logo-dark.svg">
    <img width="400" alt="Scythe" src="https://raw.githubusercontent.com/Goldziher/scythe/main/logo-dark.svg" />
  </picture>

  **SQL Compiler and Linter.**

<div style="display: flex; flex-wrap: wrap; gap: 8px; justify-content: center; margin: 20px 0;">

  <!-- Package -->
  <a href="https://crates.io/crates/scythe-cli">
    <img src="https://img.shields.io/crates/v/scythe-cli?label=crates.io&color=007ec6" alt="crates.io">
  </a>
  <a href="https://github.com/Goldziher/homebrew-tap">
    <img src="https://img.shields.io/badge/Homebrew-tap-007ec6" alt="Homebrew">
  </a>

  <!-- Project Info -->
  <a href="https://github.com/Goldziher/scythe/actions/workflows/ci.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/Goldziher/scythe/ci.yml?label=CI&color=007ec6" alt="CI">
  </a>
  <a href="https://github.com/Goldziher/scythe/blob/main/LICENSE">
    <img src="https://img.shields.io/badge/License-MIT-007ec6" alt="License">
  </a>

  <!-- Community -->
  <a href="https://discord.gg/xt9WY3GnKR">
    <img src="https://img.shields.io/badge/Discord-Join%20our%20community-7289da?logo=discord&logoColor=white" alt="Discord">
  </a>

</div>
</div>

---

## What is Scythe

Scythe is an SQL compiler and linter. It is inspired by [sqlc](https://github.com/sqlc-dev/sqlc) and [sqlfluff](https://github.com/sqlfluff/sqlfluff). It builds on [sqruff](https://github.com/quarylabs/sqruff) with additional linting capabilties. Why? ORMs add unnecessary complexity, bloat and hard to debug errors. Making SQL the source of truth makes life easier and simpler. You also gain:

1. zero bloat
2. max performance
3. safer code
4. better control


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
## License

[MIT](LICENSE)
