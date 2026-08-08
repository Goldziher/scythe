---
title: Custom Types
description: Map PostgreSQL extensions, domain types, and vendor-specific types with type_overrides.
---

Scythe maps SQL types to language-native types automatically via its [neutral type abstraction](/scythe/reference/neutral-types/). When your database uses types scythe does not recognize -- PostgreSQL extensions, domain types, or vendor-specific types -- use `type_overrides` in your `scythe.toml` to control the mapping.

## Column-Level Overrides

To map a specific column to a neutral type, specify the fully qualified `table.column` name:

```toml
[[sql.type_overrides]]
column = "users.metadata"
type = "json"
```

This tells scythe to treat `users.metadata` as `json` regardless of its declared database type. Column-level overrides take precedence over database type overrides.

## Database Type Overrides

To map all columns of a given database type, use `db_type`:

```toml
[[sql.type_overrides]]
db_type = "ltree"
type = "string"

[[sql.type_overrides]]
db_type = "citext"
type = "string"
```

Every column declared as `ltree` or `citext` in your schema will be mapped to the `string` neutral type, which each backend then converts to its language-specific string type.

`db_type` matches against the column's already-resolved **neutral** type, not the raw DDL type name
-- see [How Type Resolution Works](#how-type-resolution-works) below. `column` and `db_type` are not
mutually exclusive: nothing rejects setting both on the same entry, and if both are set, `column`
takes priority silently.

## Common Override Examples

The following table shows common PostgreSQL extensions and recommended neutral type mappings, along with the concrete types each backend produces:

| Database Type | Neutral Type | Rust | Python | TypeScript | Go | Java |
|---|---|---|---|---|---|---|
| `ltree` | `string` | `String` | `str` | `string` | `string` | `String` |
| `citext` | `string` | `String` | `str` | `string` | `string` | `String` |
| `hstore` | `json` | `serde_json::Value` | `dict` | `Record<string, unknown>` | `json.RawMessage` | `String` |
| `money` | `decimal` | `rust_decimal::Decimal` | `decimal.Decimal` | `string` | `decimal.Decimal` | `java.math.BigDecimal` |
| `inet` / `cidr` | `string` | `String` | `str` | `string` | `string` | `String` |
| `macaddr` | `string` | `String` | `str` | `string` | `string` | `String` |
| `tsvector` | `string` | `String` | `str` | `string` | `string` | `String` |
| `geometry` (PostGIS) | `string` | `String` | `str` | `string` | `string` | `String` |

Note that `inet` and `cidr` already have built-in mappings in the PostgreSQL engine manifest. Use overrides only when the default mapping does not suit your needs -- for example, mapping `inet` to `string` instead of the default `inet` neutral type when you do not need structured IP address parsing.

## How Type Resolution Works

Scythe resolves types in a three-step pipeline:

1. **SQL type** -- the type declared in your schema DDL (e.g., `CITEXT`, `LTREE`).
2. **Neutral type** -- an intermediate representation defined by the engine manifest (e.g., `string`, `json`, `decimal`). See the [Neutral Types reference](/scythe/reference/neutral-types/) for the full list.
3. **Language type** -- the concrete type in your target language, defined by the backend manifest (e.g., `String` in Rust, `str` in Python).

Type overrides intercept **after** step 1, not before it: the analyzer first resolves each column's
SQL type to a neutral type as normal, and only then does scythe check the override list. A `db_type`
override is matched against that already-resolved neutral type name, not the DDL type as written in
your schema.

```text
SQL DDL type
    |
    v
Neutral type (engine manifest default)
    |
    v
[type_overrides] -- db_type matches HERE, against the neutral type name
    |
    v
Effective neutral type (override, if matched, else unchanged)
    |
    v
Language type (backend manifest)
```

This matters because it means `db_type` only works reliably for types the engine manifest does
**not** already recognize. `ltree` and `citext` are not in scythe's built-in PostgreSQL type table,
so they pass through the SQL-to-neutral step unchanged -- their "neutral type" is literally the
string `"ltree"` or `"citext"` -- which is exactly what `db_type = "ltree"` matches against.

For a type the engine *does* recognize, `db_type` silently does nothing: `db_type = "varchar"` never
matches, because every `varchar` column's neutral type is already `"string"` by the time overrides
run, not `"varchar"`. To override a recognized type, use `db_type` with the **neutral** type name
(e.g. `db_type = "string"`) or use a column-level override instead.

**Note:** As of v0.4.0, type overrides are fully wired into the code generation pipeline. In earlier versions, `type_overrides` were parsed from the configuration but not applied during generation. They are now functional across all backends.

## Per-Language Type Overrides (Planned)

Per-language type overrides -- allowing you to specify custom imports, wrapper types, and conversion expressions for individual backends -- are planned for a future release. Track progress in [GitHub issue #6](https://github.com/Goldziher/scythe/issues/6).

## See Also

- [Configuration](/scythe/guide/configuration/) -- full `scythe.toml` reference including `type_overrides` field definitions
- [Type Inference](/scythe/guide/type-inference/) -- how scythe infers types and nullability from SQL
- [Neutral Types](/scythe/reference/neutral-types/) -- complete mapping table across all supported languages
