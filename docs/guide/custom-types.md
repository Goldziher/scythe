# Custom Types

Scythe maps SQL types to language-native types automatically via its [neutral type abstraction](../reference/neutral-types.md). When your database uses types scythe does not recognize -- PostgreSQL extensions, domain types, or vendor-specific types -- use `type_overrides` in your `scythe.toml` to control the mapping.

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

The `column` and `db_type` fields are mutually exclusive -- each override entry must use exactly one.

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
2. **Neutral type** -- an intermediate representation defined by the engine manifest (e.g., `string`, `json`, `decimal`). See the [Neutral Types reference](../reference/neutral-types.md) for the full list.
3. **Language type** -- the concrete type in your target language, defined by the backend manifest (e.g., `String` in Rust, `str` in Python).

Type overrides intercept at step 1: they replace the engine manifest's default SQL-to-neutral mapping with your specified neutral type. The neutral-to-language mapping in step 3 remains unchanged.

```text
SQL DDL type
    |
    v
[type_overrides] -- your overrides intercept here
    |
    v
Neutral type (engine manifest default or override)
    |
    v
Language type (backend manifest)
```

**Note:** As of v0.4.0, type overrides are fully wired into the code generation pipeline. In earlier versions, `type_overrides` were parsed from the configuration but not applied during generation. They are now functional across all backends.

## Per-Language Type Overrides (Planned)

Per-language type overrides -- allowing you to specify custom imports, wrapper types, and conversion expressions for individual backends -- are planned for a future release. Track progress in [GitHub issue #6](https://github.com/Goldziher/scythe/issues/6).

## See Also

- [Configuration](configuration.md) -- full `scythe.toml` reference including `type_overrides` field definitions
- [Type Inference](type-inference.md) -- how scythe infers types and nullability from SQL
- [Neutral Types](../reference/neutral-types.md) -- complete mapping table across all supported languages
