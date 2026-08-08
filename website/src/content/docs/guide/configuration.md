---
title: Configuration
description: Full scythe.toml reference -- SQL blocks, code generation options, type overrides, and lint configuration.
---

Scythe is configured via `scythe.toml` in your project root.

Relative paths in `scythe.toml` — `schema`, `queries` glob patterns, and
`output` directories — resolve relative to the directory containing
`scythe.toml`, not the current working directory the CLI is invoked from.
Running `scythe generate --config /path/to/project/scythe.toml` from any
directory behaves identically to running `cd /path/to/project && scythe
generate`. A glob pattern that matches no files is a hard error; if you hit
one unexpectedly after upgrading, check that the pattern is written relative
to the config file, not to your shell's current directory. Absolute paths and
patterns are always used as-is.

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
| `engine` | string | yes | Database dialect: `postgresql`, `mysql`, `sqlite`, `duckdb`, `cockroachdb`, `mssql`, `oracle`, `mariadb`, `redshift`, `snowflake`. |
| `schema` | string[] | yes | Glob patterns for schema DDL files. Relative patterns resolve against the config file's directory. |
| `queries` | string[] | yes | Glob patterns for annotated query files. Relative patterns resolve against the config file's directory. |
| `output` | string | yes | Output directory for generated code. A relative path resolves against the config file's directory. |
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
| `output` | string | yes | Output directory for this backend's generated code. A relative path resolves against the config file's directory. |
| `manifest` | string | no | Path to a partial manifest merged over the backend's built-in one. A relative path resolves against the config file's directory. See below. |
| `row_type` | string | no | Row type style for generated code. See below. |
| `outer_join_unions` | bool | no | Emit outer-join nullability as a discriminated union. TypeScript backends only. See below. |
| `namespace` | string | no | PHP namespace for generated code. PHP backends only. See below. |
| `extension_functions` | bool | no | Generate idiomatic Kotlin extension functions. Kotlin backends only. See below. |

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

### `outer_join_unions`

TypeScript backends only. Off by default.

For an outer join, scythe emits per-column optionality by default. That is
sound but imprecise: it admits rows the query can never produce.

Given `orders.total NOT NULL` and `orders.notes` nullable:

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;
```

the default shape allows `{ total: null, notes: "gift" }`, which is
unreachable — `total` is null exactly when no order matched, and then `notes`
is null too.

```toml
[[sql.gen]]
backend = "typescript-pg"
output = "src/generated"
outer_join_unions = true
```

Every column projected from the outer-joined relation shares one match-bit, so
they are grouped into a union:

```ts
export type GetUserOrdersRow = {
	id: number;
	name: string;
} & (
	| { total: string; notes: string | null }
	| { total: null; notes: null }
);
```

The union is only emitted when the joined relation projects at least one
`NOT NULL` column — that column is the discriminant. Without one, every column
was independently nullable anyway and the flat shape is already exact, so
scythe keeps it. Two independently outer-joined relations each get their own
alternative.

Per-column optionality remains the default and the cross-target shape: Go,
Java, C# and PHP cannot express this cleanly.

### `namespace`

Controls the PHP namespace declaration emitted at the top of every generated file. Applies to `php-pdo` and `php-amphp` backends.

| Value | Description |
|-------|-------------|
| `"App\\Generated"` | (default) |
| any valid PHP namespace | Emits `namespace <value>;` |
| `""` (empty string) | Omits the `namespace` declaration entirely |

```toml
[[sql.gen]]
backend = "php-pdo"
output = "src/generated"
namespace = "App\\Database\\Generated"
```

Set `namespace = ""` for scripts or frameworks that do not use namespaces.

### `extension_functions`

Generates query functions as idiomatic Kotlin [extension functions](https://kotlinlang.org/docs/extensions.html) on the connection receiver, instead of taking the connection as the first parameter. Applies to `kotlin-jdbc` and `kotlin-r2dbc`. Default `false` (non-breaking).

| Value | Description |
|-------|-------------|
| `false` | (default) `fun getUser(conn: Connection, id: Int): UserRow?` |
| `true` | `fun Connection.getUser(id: Int): UserRow?`, called as `connection.getUser(id)` |

```toml
[[sql.gen]]
backend = "kotlin-jdbc"
output = "src/generated"
extension_functions = true
```

When enabled, value-returning functions use expression bodies, and `kotlin-r2dbc` becomes a `suspend` extension on `io.r2dbc.spi.Connection` (the caller owns the connection lifecycle).

### `manifest`

Each backend ships with a built-in manifest holding its type mappings, naming conventions, and import rules. `manifest` points at a **partial** manifest that is merged over it, so you can retarget a few mappings without restating the rest.

```toml
[[sql.gen]]
backend = "rust-sqlx"
output = "src/db"
manifest = "manifests/rust-sqlx-custom.toml"
```

```toml
# manifests/rust-sqlx-custom.toml
[types.scalars]
decimal = "bigdecimal::BigDecimal"

[imports.rules]
"bigdecimal::" = "use bigdecimal::BigDecimal;"
```

The path resolves against the directory containing `scythe.toml`, not the directory you run `scythe` from — the same rule every other path in the config follows. Generated output is therefore identical no matter where the command is invoked.

The override is **per target**. A backend name alone does not identify a manifest: `rust-sqlx` covers five engines and `java-jdbc` nine, and each engine has its own type mappings. Because `manifest` sits on a `[[sql.gen]]` target, it inherits that target's engine from the enclosing `[[sql]]` block, and two targets naming the same backend under different engines each get their own override.

#### Merge rules

| Section | Granularity | New keys |
|---------|-------------|----------|
| `[types.scalars]` | per key | rejected |
| `[types.containers]` | per key | rejected |
| `[imports.rules]` | per key | allowed |
| `[naming]` | per field, whole value | rejected |

Map-valued tables merge one key at a time: a key you list replaces exactly that entry, and every key you omit keeps its built-in value. `[naming]` fields replace whole values; omitted fields inherit.

`[naming]` accepts four fields — `struct_case`, `fn_case`, `enum_variant_case` and `row_suffix`. The list is an allowlist rather than a mirror of the manifest, so any other naming field is a parse error, not a silent no-op.

`[types.scalars]` and `[types.containers]` are replace-only. Neutral type names (`int32`, `datetime_tz`, `array`, …) are a fixed vocabulary, so a key outside it is a typo — and a silently accepted typo would leave the original mapping in place and generate code you did not ask for. `[imports.rules]` does accept new keys, because its keys are prefixes of the *generated* language types, which necessarily change when you retarget a scalar.

There is no `[backend]` section. `name`, `language`, `file_extension`, and `engine` are identity, not configuration.

`manifest` is read by `scythe generate` and exists only on the `[[sql.gen]]` array form; the legacy `[sql.gen.rust]` syntax below has no equivalent key.

#### Errors

Every problem fails `scythe generate` and names the backend, the resolved absolute path, and the offending key. Nothing falls back to the built-in manifest silently.

```text
error: backend 'rust-sqlx': invalid manifest override '/repo/manifests/rust-sqlx-custom.toml':
  manifest error: unknown [types.scalars] key 'int_64' (did you mean 'int64'?);
  this table may only override mappings the backend already defines
```

A missing file is an error, not a fallback:

```text
error: backend 'rust-sqlx': failed to read manifest override '/repo/manifests/nope.toml':
  No such file or directory (os error 2)
```

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

See [Linting](/scythe/guide/linting/) for the full list of rules and categories.

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
| `mysql` | MySQL |
| `sqlite`, `sqlite3` | SQLite |
| `duckdb` | DuckDB |
| `cockroachdb`, `crdb` | CockroachDB |
| `mssql`, `sqlserver` | MSSQL |
| `oracle` | Oracle |
| `mariadb` | MariaDB |
| `redshift` | Redshift |
| `snowflake` | Snowflake |
