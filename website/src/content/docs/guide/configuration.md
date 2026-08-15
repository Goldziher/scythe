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
db_type = "uuid"                       # Override all columns whose *neutral* type is this
type = "string"                        # Neutral type to map to

# Optional: lint configuration
[lint]

# Set severity by category (naming, safety, style, performance, antipattern, codegen,
# security, migration, provenance, drift)
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
| `output` | string | no | **Legacy.** Output directory used only as the default for the legacy `[sql.gen.<lang>]` table form below. Ignored by the `[[sql.gen]]` array form -- each array target needs its own `output` key. Defaults to `"generated"` when `gen` is absent entirely. A relative path resolves against the config file's directory. |
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

:::caution[`[[sql]].output` does not apply to `[[sql.gen]]` targets]
`output` on the enclosing `[[sql]]` block is legacy -- it is only used as the default output
directory for the `[sql.gen.<lang>]` table form documented further down. Every `[[sql.gen]]` array
entry needs its own `output` key, even if you also set `[[sql]].output`. Omitting it fails to parse
and names the specific mistake:

```text
error: failed to parse config 'scythe.toml': TOML parse error at line 4, column 1
[[sql.gen]] entry #1 (backend = "rust-sqlx") is missing `output`. This block sets
`[[sql]].output = "src/generated"`, but that field is only used as a default for the legacy
`[sql.gen.<lang>]` table form -- it is ignored for `[[sql.gen]]` array entries. Add
`output = "..."` to this entry directly.
```

:::

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `backend` | string | yes | Full backend name (e.g. `rust-sqlx`, `typescript-pg`, `python-aiomysql`). |
| `output` | string | yes | Output directory for this backend's generated code. A relative path resolves against the config file's directory. |
| `manifest` | string | no | Path to a partial manifest merged over the backend's built-in one. A relative path resolves against the config file's directory. See below. |
| `row_type` | string | no | Row type style for generated code. See below. |
| `outer_join_unions` | bool | no | Emit outer-join nullability as a discriminated union. TypeScript backends only. See below. |
| `structs_only` | bool | no | Suppress generated query functions, emitting only row/struct types. TypeScript backends and `rust-sqlx`. |
| `field_case` | string | no | Case convention for generated column/param field names. TypeScript and Java/Kotlin backends. See below. |
| `namespace` | string | no | PHP namespace for generated code. PHP backends only. See below. |
| `extension_functions` | bool | no | Generate idiomatic Kotlin extension functions. Kotlin backends only. See below. |
| `serde` | bool | no | Add `serde::Serialize, serde::Deserialize` to every generated struct and enum derive list. `rust-sqlx`, `rust-tokio-postgres`, `rust-tiberius`, `rust-sibyl` only. |
| `derive` | string | no | Comma-separated list of additional derive macros to append (e.g. `derive = "PartialEq, Hash"`). `rust-sqlx`, `rust-tokio-postgres`, `rust-tiberius`, `rust-sibyl` only. |

`row_type`, `outer_join_unions`, `structs_only`, and `field_case` are the only options every
TypeScript backend accepts (`typescript-pg`, `typescript-postgres`, `typescript-kysely`,
`typescript-mysql2`, `typescript-mssql`, `typescript-oracledb`, `typescript-snowflake`,
`typescript-duckdb`, `typescript-better-sqlite3`, `typescript-node-sqlite`,
`typescript-wasm-sqlite`); `namespace` and `extension_functions` are not on that list. Setting either
on a TypeScript target fails with the `unknown option` error shown below — see the caution box.

The ten `javascript-*` names (`javascript-pg`, `javascript-postgres`, `javascript-mysql2`,
`javascript-better-sqlite3`, `javascript-duckdb`, `javascript-node-sqlite`, `javascript-wasm-sqlite`,
`javascript-mssql`, `javascript-oracledb`, `javascript-snowflake`) accept the same four keys, but
reject three of their values outright:
`row_type = "zod"`, `outer_join_unions = true`, and `field_case = "camelCase"` each need
TypeScript-only syntax a plain `.js` file cannot carry, so generation aborts with an error naming the
TypeScript backend to use instead. See
[JavaScript output (JSDoc)](/scythe/backends/typescript/#javascript-output-jsdoc).

:::caution[Unrecognized options are a hard error on every backend]
`[[sql.gen]]` uses `#[serde(flatten)]`, so every key besides `backend`, `output`, and `manifest` is
handed to the target backend's `apply_options`. Every backend rejects any key it doesn't recognize
instead of silently ignoring it — the `CodegenBackend::apply_options` trait default rejects all
options unless a backend explicitly declares a known-key list. A typo like `row_typ = "zod"` — or any
forward-compat key you were carrying for a future scythe version — aborts `scythe generate` instead
of doing nothing:

```text
error: backend 'typescript-pg' apply_options failed: unknown option 'row_typ' (did you mean 'row_type'?):
  valid options are row_type, outer_join_unions, structs_only, field_case
```

If you hit this after upgrading, remove the offending key or fix the typo the error suggests. This
applies to every backend, not just TypeScript — for example, `field_casing = "camelCase"` (meant to be
`field_case`) on a `java-jdbc` target is also a hard error, not a silent no-op.
:::

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

### `field_case`

Accepted by 16 backends: the 11 TypeScript backends listed above, plus `java-jdbc`, `java-r2dbc`,
`kotlin-jdbc`, `kotlin-r2dbc`, and `kotlin-exposed`. Controls the case of generated
row/interface/record/data-class field names and function parameter names — the SQL side (column
names, bound parameter order) is unaffected.

| Value | Description |
|-------|-------------|
| `"snake_case"` | (default) Field names mirror SQL column/param names as-is |
| `"camelCase"` | Field names are converted to camelCase |

Those two values are the whole vocabulary; anything else fails `scythe generate` with an error naming
the backend and the rejected value.

```toml
[[sql.gen]]
backend = "typescript-pg"
output = "src/generated"
field_case = "camelCase"
```

```sql
-- @name GetUser
-- @returns :one
SELECT id, user_name, created_at FROM users WHERE id = $1;
```

```ts
export interface GetUserRow {
  id: number;
  userName: string;
  createdAt: Date;
}
```

On the ten driver-backed TypeScript backends, renaming a field is not just a label change: the driver
still returns rows keyed by the original SQL column name, so `field_case = "camelCase"` also switches
the function body from a blind cast of the driver's row to a field-by-field reconstruction
(`row['user_name']` read into a `userName` property). Without that remap the generated code would
still type-check under `tsc` but every field would read back `undefined` at runtime. The Java/Kotlin
backends need no such remap — each column is read by an explicit getter keyed by the raw SQL column
name, so only the declared field name changes. See
[Java](/scythe/backends/java/#field-naming-field_case) and
[Kotlin](/scythe/backends/kotlin/#field-naming-field_case).

:::caution[`typescript-kysely` requires `CamelCasePlugin`]
`typescript-kysely` is the exception, and it is the one case where the option alone is not enough.
It hands back Kysely's own result rows and deliberately does **not** remap, because Kysely's
`CamelCasePlugin` already renames result keys — including for `sql` template results, not just
builder queries. Two renamers disagreeing is worse than one.

So `field_case = "camelCase"` on `typescript-kysely` means "my Kysely instance already has
`CamelCasePlugin` installed". Scythe cannot verify that at generation time. If you set the option and
omit the plugin, the declared type says `userId` while the row carries `user_id`, and every field
reads back `undefined` at runtime with `tsc` still green.

```typescript
import { CamelCasePlugin, Kysely, PostgresDialect } from "kysely";
import { Pool } from "pg";

const db = new Kysely<any>({
  dialect: new PostgresDialect({ pool: new Pool() }),
  plugins: [new CamelCasePlugin()],
});
```

:::

The ten `javascript-*` backends accept `field_case = "snake_case"` but reject `"camelCase"`: the
remap needs a TypeScript `as T` assertion that a plain `.js` file cannot carry. The error points at
the matching `typescript-*` backend.

Two SQL identifiers that collapse onto the same generated name under the active `field_case` are a
hard error, not a silent last-write-wins — this also applies under the default `snake_case`, since
quoted SQL (`SELECT "USER_ID", user_id FROM t`) can produce two distinct column names that resolve
to one field:

```text
error: columns 'user_id' and 'userId' both resolve to field name 'userId' under field_case = "camelCase"
  -- alias one of them in SQL, or set field_case = "snake_case"
```

`field_case` is a backend option, not a manifest field: a manifest's `[naming]` table cannot set it
(see `manifest` below), so a partial manifest override cannot silently disable it.

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
| `[types.docblock_containers]` | per key | rejected |
| `[imports.rules]` | per key | allowed |
| `[naming]` | per field, whole value | rejected |

Map-valued tables merge one key at a time: a key you list replaces exactly that entry, and every key you omit keeps its built-in value. `[naming]` fields replace whole values; omitted fields inherit.

`[naming]` accepts four fields — `struct_case`, `fn_case`, `enum_variant_case` and `row_suffix`. The list is an allowlist rather than a mirror of the manifest, so any other naming field is a parse error, not a silent no-op.

:::caution[Breaking: acronym handling changed for `PascalCase` and `camelCase`]
Every case conversion now normalizes a run of consecutive capitals (an acronym, or an
`ID`/`URL`-style suffix) the same way: `to_pascal_case` — which backs `struct_case = "PascalCase"`,
set by every one of scythe's 102 shipped manifests — previously returned a mixed-case input with no
underscore unchanged, so `CreateAPIKey` produced the row type `CreateAPIKeyRow`, preserving the
`APIKey` run. It now routes through the same snake_case normalization `to_camel_case` (`fn_case =
"camelCase"`) uses, producing `CreateApiKeyRow`:

| Query name | Old `struct_case = "PascalCase"` row type | New |
|---|---|---|
| `CreateAPIKey` | `CreateAPIKeyRow` | `CreateApiKeyRow` |
| `RetrieveUserAccountByID` | `RetrieveUserAccountByIDRow` | `RetrieveUserAccountByIdRow` |

This is not limited to TypeScript: `struct_case = "PascalCase"` is universal across every backend,
so every language's generated struct, row-type, and enum-type name changes for any query, table, or
enum name with an acronym run. `to_camel_case` (`fn_case = "camelCase"`, set by 53 of the 102
manifests) is now defined in terms of the fixed `to_pascal_case`, guaranteeing struct and function
names derived from the same identifier agree — its own output for some inputs may shift too where
the two previously disagreed. `fn_case = "snake_case"` is unaffected. Update call sites that
reference an old identifier after regenerating.
:::

`[types.scalars]`, `[types.containers]` and `[types.docblock_containers]` are replace-only. Neutral type names (`int32`, `datetime_tz`, `array`, …) are a fixed vocabulary, so a key outside it is a typo — and a silently accepted typo would leave the original mapping in place and generate code you did not ask for. `[imports.rules]` does accept new keys, because its keys are prefixes of the *generated* language types, which necessarily change when you retarget a scalar.

`[types.docblock_containers]` maps the same container names as `[types.containers]`, but is used only where the target language accepts a type in a comment rather than in a native type position — a PHPStan docblock today. A container it omits falls back to `[types.containers]`, so only the PHP backends declare it: `array` maps to a bare `array` natively, because `public array<string> $tags` is a PHP parse error, and to `array<{T}>` in the docblock, because a bare `array` is `array<mixed, mixed>` at PHPStan level 9. Its accepted keys are the container vocabulary, not just the ones the backend already overrides for docblocks.

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

### `[sql.gen.rust]`, `[sql.gen.python]`, `[sql.gen.typescript]`, `[sql.gen.go]`, `[sql.gen.kotlin]` (legacy)

The legacy syntax is still supported but limited to a single backend per language. Each table takes a
`target` string that resolves to a full backend name; `[sql.gen.rust]` additionally accepts `derive`
and `serde`.

```toml
[sql.gen.rust]
target = "sqlx"                        # -> rust-sqlx
derive = ["Debug", "Clone", "serde::Serialize"]
serde = true

[sql.gen.python]
target = "psycopg3"                    # -> python-psycopg3

[sql.gen.typescript]
target = "pg"                          # -> typescript-pg

[sql.gen.go]
target = "pgx"                         # -> go-pgx

[sql.gen.kotlin]
target = "jdbc"                        # -> kotlin-jdbc
```

| Table | Field | Type | Required | Accepted `target` values |
|-------|-------|------|----------|---------------------------|
| `[sql.gen.rust]` | `target` | string | yes | `sqlx`, `tokio-postgres`, `tiberius`, `sibyl` |
| `[sql.gen.rust]` | `derive` | string[] | no | Additional derive macros for generated structs. |
| `[sql.gen.rust]` | `serde` | bool | no | Add serde Serialize/Deserialize derives. |
| `[sql.gen.python]` | `target` | string | yes | `psycopg3`, `asyncpg`, `aiomysql`, `aiosqlite`, `duckdb`, `pyodbc`, `oracledb`, `snowflake` |
| `[sql.gen.typescript]` | `target` | string | yes | `pg`, `postgres`, `mysql2`, `better-sqlite3`, `node-sqlite`, `duckdb`, `wasm-sqlite`, `kysely`, `mssql`, `oracledb`, `snowflake` |
| `[sql.gen.go]` | `target` | string | yes | `pgx`, `database-sql`, `godror`, `gosnowflake` |
| `[sql.gen.kotlin]` | `target` | string | yes | `jdbc`, `exposed`, `r2dbc` |

`target` resolves to a backend name by prefixing the language (e.g. `[sql.gen.python] target =
"psycopg3"` resolves to the `python-psycopg3` backend). `derive` and `serde` are Rust-only; the other
four tables accept only `target`. Anything beyond a single backend per language, or backend options
like `row_type` and `field_case`, requires the `[[sql.gen]]` array form above.

### `[[sql.type_overrides]]`

| Field | Type | Description |
|-------|------|-------------|
| `column` | string | Target a specific column (`table.column`). If both `column` and `db_type` are set on the same entry, `column` wins silently — nothing rejects setting both. |
| `db_type` | string | Target all columns whose *neutral* type matches this value (see [Custom Types](/scythe/guide/custom-types/) for what "neutral type" means here). |
| `type` | string | Neutral type to use (e.g. `string`, `json`, `int64`). |

### `[lint]`

See [Linting](/scythe/guide/linting/) for the full list of rules and categories.

### `[lint.sqruff]`

Controls the sqruff style-rule integration used by `scythe lint` (not `scythe fmt`, which always runs
sqruff's default rule set and ignores this table entirely -- except for `LT01`, which is excluded
under `scythe fmt` too; see [Formatting](/scythe/guide/formatting/)).

```toml
[lint.sqruff]
enabled = true      # set to false to skip sqruff entirely under `scythe lint`

[lint.sqruff.rules]
"LT01" = "off"      # bare sqruff codes, not the SQ- prefix scythe uses in output
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `enabled` | bool | no | When `false`, `scythe lint` skips sqruff entirely (no sqruff findings). Does not affect `scythe fmt`, which never reads `[lint.sqruff]`. |
| `rules` | table | no | Per-rule status keyed by bare sqruff code (e.g. `"LT01"`). Only `"off"` is supported -- sqruff has no per-rule severity, so any other value is rejected as a config error, as is an unrecognized rule code. A rejected table fails the entire `scythe lint` run, scythe's own rules included. Inert when `enabled = false`. See below. |

See [Linting](/scythe/guide/linting/#sqruff-configuration) for details.

### `[inspect]`

Configures `scythe inspect`, the live-database health-check command. See the
[Inspect guide](/scythe/guide/inspect/#project-configuration-inspect-in-scythetoml) for the full
field reference, including `[inspect.severity_overrides]`, `[[inspect.suppression]]`, and
`[[inspect.check]]` for user-defined checks (IDs must be prefixed `USER-INS-`).

```toml
[inspect]
database_url = "postgres://localhost/dev"
api_schemas  = ["public", "api"]
extra_rules  = ["./inspect-rules.toml"]
```

### `[audit]`

Configures `scythe audit`'s user-defined rules. See the [Audit guide](/scythe/guide/audit/#user-defined-rules)
for the full field reference and available matchers. Custom rule IDs must be prefixed `USER-`.

```toml
[audit]
extra_rules = ["./security_rules.toml"]

[[audit.rule]]
id = "USER-001"
name = "no-debug-functions"
severity = "error"
description = "calls to debug-only functions should not ship"
message = "call to debug function `{func}` — remove before merging"
matcher = "function_name_in_set"

[audit.rule.matcher_args]
functions = ["dump_internal_state", "debug_print"]
```

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

Aliases resolve to one of six `SqlDialect` variants (`crates/scythe-core/src/dialect.rs`).
`duckdb`, `redshift`, `cockroachdb`, and `crdb` are not separate dialects — they alias directly to
the PostgreSQL dialect and get identical parsing and type resolution.

| Alias | Resolves to |
|-------|--------|
| `postgresql`, `postgres`, `pg`, `cockroachdb`, `crdb`, `duckdb`, `redshift` | PostgreSQL |
| `mysql`, `mariadb` | MySQL |
| `sqlite`, `sqlite3` | SQLite |
| `mssql`, `sqlserver`, `tsql` | MsSql |
| `oracle` | Oracle |
| `snowflake` | Snowflake |
