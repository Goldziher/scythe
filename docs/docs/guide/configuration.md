# Configuration

Scythe is configured via `scythe.toml` in your project root.

## Full Reference

```toml
# Required: scythe metadata
[scythe]
version = "1"

# One or more SQL blocks. Each block defines a schema + queries + output target.
[[sql]]
name = "main"                          # Block name (used in CLI output)
engine = "postgresql"                  # Database engine: postgresql, mysql, sqlite
schema = ["sql/schema/*.sql"]          # Glob patterns for DDL files
queries = ["sql/queries/*.sql"]        # Glob patterns for annotated query files
output = "src/generated"               # Output directory for generated code

# Optional: code generation settings
[sql.gen.rust]
target = "sqlx"                        # Backend target (e.g. sqlx, tokio-postgres)
derive = ["Debug", "Clone", "serde::Serialize"]  # Extra derive macros on structs
serde = true                           # Add serde derives

# Optional: type overrides
[[sql.type_overrides]]
column = "users.metadata"              # Specific column to override
type = "json"                          # Neutral type to use

[[sql.type_overrides]]
db_type = "citext"                     # Override all columns of this DB type
type = "string"                        # Neutral type to map to

# Optional: lint configuration
[lint]

# Set severity by category (naming, safety, style, performance, antipattern, codegen)
[lint.categories]
safety = "error"
naming = "warn"
performance = "warn"

# Override severity for individual rules
[lint.rules]
"SC-S03" = "off"       # Disable SELECT * warning
"SC-N03" = "error"     # Promote query naming to error
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
| `engine` | string | yes | Database dialect: `postgresql`, `mysql`, `sqlite`, `duckdb`, `cockroachdb`. |
| `schema` | string[] | yes | Glob patterns for schema DDL files. |
| `queries` | string[] | yes | Glob patterns for annotated query files. |
| `output` | string | yes | Output directory for generated code. |
| `gen` | table | no | Code generation options per language. |
| `type_overrides` | array | no | Type mapping overrides. |

### `[[sql.gen]]` (recommended for v0.2.0+)

The new array syntax allows generating code for multiple backends from a single SQL block:

```toml
[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema/*.sql"]
queries = ["sql/queries/*.sql"]

[[sql.gen]]
backend = "rust-sqlx"
output = "src/generated/rust"

[[sql.gen]]
backend = "typescript-pg"
output = "src/generated/ts"

[[sql.gen]]
backend = "python-duckdb"
output = "src/generated/duckdb"

[[sql.gen]]
backend = "java-r2dbc"
output = "src/generated/java-r2dbc"

[[sql.gen]]
backend = "kotlin-exposed"
output = "src/generated/kotlin-exposed"
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `backend` | string | yes | Full backend name (e.g. `rust-sqlx`, `typescript-pg`, `python-aiomysql`). |
| `output` | string | yes | Output directory for this backend's generated code. |
| `row_type` | string | no | Row type style for generated code. See below. |

### `row_type`

Controls what data structure is used for generated row types. Available options depend on the backend language:

**Python backends:**

| Value | Description |
|-------|-------------|
| `"dataclass"` | (default) Standard library `@dataclass` |
| `"pydantic"` | Pydantic `BaseModel` with validation |
| `"msgspec"` | msgspec `Struct` for high-performance serialization |

```toml
[[sql.gen]]
backend = "python-psycopg3"
output = "src/generated"
row_type = "pydantic"
```

**TypeScript backends:**

| Value | Description |
|-------|-------------|
| `"interface"` | (default) TypeScript `interface` |
| `"zod"` | Zod schema with inferred types |

```toml
[[sql.gen]]
backend = "typescript-pg"
output = "src/generated"
row_type = "zod"
```

Other languages use their standard row type and do not currently support `row_type` configuration.

### `[sql.gen.rust]` (legacy)

The legacy syntax is still supported but limited to a single backend per language:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `target` | string | yes | Backend name (e.g. `sqlx`, `tokio-postgres`). |
| `derive` | string[] | no | Additional derive macros for generated structs. |
| `serde` | bool | no | Add serde Serialize/Deserialize derives. |

### `[[sql.type_overrides]]`

| Field | Type | Description |
|-------|------|-------------|
| `column` | string | Target a specific column (`table.column`). Mutually exclusive with `db_type`. |
| `db_type` | string | Target all columns with this database type. Mutually exclusive with `column`. |
| `type` | string | Neutral type to use (e.g. `string`, `json`, `int64`). |

### `[lint]`

See [Linting](linting.md) for the full list of rules and categories.

## Multiple SQL Blocks

You can define multiple `[[sql]]` blocks for different databases or schemas:

```toml
[scythe]
version = "1"

[[sql]]
name = "users"
engine = "postgresql"
schema = ["sql/users/schema.sql"]
queries = ["sql/users/queries/*.sql"]
output = "src/generated/users"

[[sql]]
name = "analytics"
engine = "postgresql"
schema = ["sql/analytics/schema.sql"]
queries = ["sql/analytics/queries/*.sql"]
output = "src/generated/analytics"
```

## Engine Aliases

| Alias | Engine |
|-------|--------|
| `postgresql`, `postgres`, `pg` | PostgreSQL |
| `mysql`, `mariadb` | MySQL |
| `sqlite`, `sqlite3` | SQLite |
| `duckdb` | DuckDB |
| `cockroachdb`, `crdb` | CockroachDB |
