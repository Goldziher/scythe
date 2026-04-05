# Scythe

<div align="center">
  <img width="3384" height="573" alt="Banner" src="https://raw.githubusercontent.com/Goldziher/scythe/main/logo.svg" />
</div>

A polyglot SQL-to-code generator with built-in linting and formatting. Write SQL, get type-safe code.

> **Status**: Under active development. Core pipeline is functional and tested against production workloads.

## What It Does

Scythe parses your SQL schema and annotated queries, infers types with precision, and generates idiomatic code for your language and database driver. It also lints your SQL for correctness, style, and performance — combining 22 scythe-specific rules with 71 rules from [sqruff](https://github.com/quarylabs/sqruff).

### CLI

```bash
scythe generate   # SQL → type-safe code (Rust/sqlx, tokio-postgres)
scythe check      # parse + analyze + lint (93 rules)
scythe lint       # standalone lint with --fix auto-repair
scythe fmt        # SQL formatting with --check and --diff modes
scythe migrate    # convert from sqlc to scythe format
```

## Why Scythe

Inspired by [sqlc](https://github.com/sqlc-dev/sqlc), scythe improves on it in several ways:

- **Standard SQL parameters** — `$1`, `$2` (valid PostgreSQL), not `sqlc.arg()` custom syntax
- **Smart nullability** — LEFT JOIN right-side columns become nullable; COALESCE with literal is non-nullable; COUNT is non-nullable while SUM/AVG are nullable on empty sets
- **93 lint rules** — safety, performance, antipatterns, naming, style, formatting (via sqruff integration)
- **SQL formatting** — `scythe fmt` formats SQL files with configurable style
- **Polyglot architecture** — template-based backends; adding a language is a manifest + templates
- **Multi-dialect** — PostgreSQL, MySQL, BigQuery, Snowflake, and more via sqruff

## How It Works

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
    Generated Code (Rust, Python, Go, ...)
```

### Write SQL

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;
```

### Get Type-Safe Code

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

## Architecture

### Workspace Crates

```text
crates/
  scythe-core/        # catalog, parser, analyzer, errors
  scythe-codegen/     # code generation via backend templates
  scythe-lint/        # 22 custom rules + 71 sqruff rules + engine
  scythe-backend/     # type resolution, naming, MiniJinja rendering
  scythe-cli/         # CLI binary (generate, check, lint, fmt, migrate)
```

### Language-Neutral Type System

The analyzer outputs a neutral type vocabulary. Each backend maps these to language types:

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

### Template-Based Backends

```text
backends/rust-sqlx/
  manifest.toml           # type mappings, naming rules
  templates/
    row_struct.jinja
    query_fn.jinja
    enum_def.jinja
    model_struct.jinja
```

Adding a new language means writing a manifest and templates. No Rust code required.

### Lint Rules (93 total)

**Scythe rules (22)** — codegen-aware, uses type inference and catalog:

| Category | Rules | Examples |
|----------|-------|---------|
| Safety | 6 | UPDATE without WHERE, SELECT *, ambiguous columns |
| Codegen | 3 | exec with RETURNING, duplicate query names |
| Naming | 4 | snake_case columns, PascalCase query names |
| Antipattern | 3 | `= NULL` instead of `IS NULL`, OR in JOIN |
| Performance | 3 | ORDER BY without LIMIT, leading wildcard LIKE |
| Style | 3 | implicit joins, COUNT(1) vs COUNT(*) |

**sqruff rules (71)** — formatting, capitalization, structure:

Aliasing, ambiguity, capitalization, conventions, layout (spacing/indentation), references, structure rules — all configurable via `scythe.toml`.

### Configuration

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

[lint.rules]
"SC-S03" = "off"      # allow SELECT *
"SC-P01" = "error"    # enforce ORDER BY + LIMIT

[lint.sqruff]
"LT01" = "warn"       # spacing
"CP01" = "off"        # keyword caps
```

### Test-Driven Development

275 JSON test fixtures define expected behavior across:

- Catalog DDL, SELECT queries, JOINs, CTEs, subqueries
- Nullability inference (JOIN propagation, COALESCE, aggregates, CASE)
- Type system (enums, arrays, JSON/JSONB, composites, ranges)
- Parameters, expressions, code generation, error cases
- Lint rules (40 fixtures covering all 22 scythe rules)

A test generator reads fixtures and produces Rust test code.

## Getting Started

```bash
# Generate code
scythe generate --config scythe.toml

# Lint SQL
scythe lint --config scythe.toml

# Format SQL
scythe fmt --config scythe.toml

# Migrate from sqlc
scythe migrate sqlc.yaml
```

## License

[MIT](LICENSE)
