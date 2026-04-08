# Configuration Reference

Scythe is configured via `scythe.toml` in your project root.

## Full Reference

```toml
# Required: scythe metadata
[scythe]
version = "1"

# One or more SQL blocks
[[sql]]
name = "main"                          # Block name
engine = "postgresql"                  # postgresql, mysql, sqlite, duckdb, cockroachdb
schema = ["sql/schema/*.sql"]          # Glob patterns for DDL files
queries = ["sql/queries/*.sql"]        # Glob patterns for query files
output = "src/generated"               # Output directory

# Code generation backends (multiple allowed)
[[sql.gen]]
backend = "rust-sqlx"                  # Full backend name
output = "src/generated/rust"          # Output directory for this backend
row_type = "dataclass"                 # Optional: row type style

# Type overrides
[[sql.type_overrides]]
column = "users.metadata"              # Column-level (table.column)
type = "json"                          # Neutral type

[[sql.type_overrides]]
db_type = "citext"                     # DB type-level
type = "string"

# Lint configuration
[lint.categories]
safety = "error"
naming = "warn"
performance = "warn"
style = "off"

[lint.rules]
"SC-S03" = "off"
"SC-N03" = "error"
```

## Fields

### `[scythe]`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `version` | string | yes | Config version. Currently `"1"`. |

### `[[sql]]`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Name for this SQL block. |
| `engine` | string | yes | Database dialect. |
| `schema` | string[] | yes | Glob patterns for schema DDL files. |
| `queries` | string[] | yes | Glob patterns for annotated query files. |
| `output` | string | yes | Output directory for generated code. |
| `gen` | array | no | Code generation backend configurations. |
| `type_overrides` | array | no | Type mapping overrides. |

### `[[sql.gen]]`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `backend` | string | yes | Full backend name (e.g. `rust-sqlx`, `typescript-pg`). |
| `output` | string | yes | Output directory for this backend. |
| `row_type` | string | no | Row type style. Python: `dataclass`/`pydantic`/`msgspec`. TypeScript: `interface`/`zod`. |

### `[[sql.type_overrides]]`

| Field | Type | Description |
|-------|------|-------------|
| `column` | string | Target a specific column (`table.column`). Mutually exclusive with `db_type`. |
| `db_type` | string | Target all columns with this database type. Mutually exclusive with `column`. |
| `type` | string | Neutral type to use (e.g. `string`, `json`, `int64`). |

### Engine Aliases

| Alias | Engine |
|-------|--------|
| `postgresql`, `postgres`, `pg` | PostgreSQL |
| `mysql`, `mariadb` | MySQL |
| `sqlite`, `sqlite3` | SQLite |
| `duckdb` | DuckDB |
| `cockroachdb`, `crdb` | CockroachDB |

## Multiple SQL Blocks

```toml
[scythe]
version = "1"

[[sql]]
name = "users"
engine = "postgresql"
schema = ["sql/users/schema.sql"]
queries = ["sql/users/queries/*.sql"]

[[sql.gen]]
backend = "rust-sqlx"
output = "src/generated/users"

[[sql]]
name = "analytics"
engine = "postgresql"
schema = ["sql/analytics/schema.sql"]
queries = ["sql/analytics/queries/*.sql"]

[[sql.gen]]
backend = "python-psycopg3"
output = "src/generated/analytics"
```

## Neutral Types

Available neutral types for `type_overrides`:

`bool`, `int16`, `int32`, `int64`, `float32`, `float64`, `string`, `bytes`, `decimal`, `uuid`, `date`, `time`, `datetime`, `datetime_tz`, `interval`, `json`, `inet`, `array`, `nullable`
