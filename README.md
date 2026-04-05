# Scythe

A polyglot SQL-to-code generator. Write SQL, get type-safe code.

Scythe parses your SQL schema and annotated queries, infers types with precision, and generates idiomatic code for your language and database driver of choice. Starting with Rust/sqlx and PostgreSQL, with more languages coming.

## Why Scythe

SQL is the best language for talking to databases. ORMs obscure it. Query builders leak abstractions. Scythe takes a different approach: you write real SQL, and it generates the type-safe glue code.

Inspired by [sqlc](https://github.com/sqlc-dev/sqlc), scythe improves on it in several ways:

- **Standard SQL parameters** -- uses `$1`, `$2` (valid PostgreSQL), not custom `sqlc.arg()` syntax
- **Smart nullability inference** -- LEFT JOIN right-side columns become nullable; COALESCE with a literal default is non-nullable; COUNT is non-nullable while SUM/AVG are nullable on empty sets
- **Polyglot from the ground up** -- template-based backends mean adding a new language is writing a manifest + templates, not forking the project
- **Better type system** -- first-class JSON/JSONB mapping, correct enum nullable handling, shared model structs with deduplication

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

Scythe knows `o.total` is nullable (RIGHT side of LEFT JOIN, even though `orders.total` is `NOT NULL` in the schema) and generates:

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

### Language-Neutral Type System

Fixtures and the analyzer work with a neutral type vocabulary (`int32`, `string`, `datetime_tz`, `array<int32>`, `enum::user_status`, etc.). Each backend maps these to language-specific types via a manifest:

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

Code generation uses MiniJinja templates. A backend is a directory with a `manifest.toml` and `templates/`:

```text
backends/rust-sqlx/
  manifest.toml           # type mappings, naming rules
  templates/
    row_struct.jinja      # struct generation
    query_fn.jinja        # function generation
    enum_def.jinja        # enum generation
    model_struct.jinja    # shared model generation
```

Adding a new language means writing a manifest and templates. No Rust code required.

### Test-Driven Development

235 JSON test fixtures define expected behavior across:

- Catalog DDL (tables, enums, composites, views, domains, multi-schema)
- SELECT queries (basic, joins, CTEs, subqueries, star expansion, set operations)
- Nullability inference (JOIN propagation, COALESCE, aggregates, CASE, overrides)
- Type system (enums, arrays, JSON/JSONB, composites, ranges, temporals)
- Parameters (WHERE, INSERT, CAST, BETWEEN, complex positions)
- Expressions (string/math/date functions, operators, pattern matching, window functions)
- Code generation (sqlx output, model sharing, naming conventions)
- Error cases (unknown columns/tables, type mismatches, invalid annotations)

A test generator tool reads these fixtures and produces Rust test code.

## Project Structure

```text
scythe/
  src/                          # main binary (CLI)
  crates/scythe-backend/        # shared backend infrastructure
    src/
      manifest.rs               # backend manifest parsing
      types.rs                  # neutral type -> language type resolution
      naming.rs                 # case conversion utilities
      renderer.rs               # MiniJinja template rendering
  backends/rust-sqlx/           # built-in Rust/sqlx backend
  tools/test-generator/         # generates tests from fixtures
  tools/migrate-fixtures/       # fixture migration utility
  testing_data/                 # 235 JSON test fixtures
  tests/generated/              # auto-generated test files
```

## Status

Early development. The test infrastructure and backend architecture are in place. Core implementation (SQL parsing, catalog building, query analysis, code generation) is next.

## License

[MIT](LICENSE)
