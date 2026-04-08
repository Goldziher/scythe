# Snowflake

Snowflake support with VARIANT/OBJECT/ARRAY semi-structured types, TIMESTAMP variants, and QUALIFY clause.

## Overview

Snowflake is a cloud-native data warehouse with its own SQL dialect. It features semi-structured data types (`VARIANT`, `OBJECT`, `ARRAY`), multiple timestamp variants, and the `QUALIFY` clause for filtering window function results. Snowflake is cloud-only -- there is no local Docker container for development.

## Engine alias

```toml
# scythe.toml
[[sql]]
engine = "snowflake"
```

## Supported backends

| Backend | Language | Driver |
|---------|----------|--------|
| `python-snowflake` | Python | snowflake-connector-python |
| `typescript-snowflake` | TypeScript | snowflake-sdk |
| `go-gosnowflake` | Go | gosnowflake |
| `java-jdbc` | Java | JDBC (Snowflake JDBC driver) |
| `kotlin-jdbc` | Kotlin | JDBC (Snowflake JDBC driver) |
| `csharp-snowflake` | C# | Snowflake.Data |
| `php-pdo` | PHP | PDO (Snowflake PDO driver) |

Note: Rust, Ruby, and Elixir backends are not yet available for Snowflake due to limited driver ecosystem.

## Configuration

```toml
# scythe.toml
[[sql]]
engine = "snowflake"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "python-snowflake"
output = "src/generated"
```

## Type mapping table

| Snowflake Type | Neutral Type | Notes |
|---------------|-------------|-------|
| `NUMBER` / `DECIMAL` / `NUMERIC` | `decimal` | Default NUMBER(38,0) |
| `INT` / `INTEGER` / `BIGINT` / `SMALLINT` / `TINYINT` | `int64` | All integer types are NUMBER(38,0) |
| `FLOAT` / `FLOAT4` / `FLOAT8` / `DOUBLE` / `REAL` | `float64` | All float types are DOUBLE |
| `VARCHAR` / `STRING` / `TEXT` / `CHAR` | `string` | |
| `BINARY` / `VARBINARY` | `bytes` | |
| `BOOLEAN` | `bool` | |
| `DATE` | `date` | |
| `TIME` | `time` | |
| `TIMESTAMP_NTZ` / `TIMESTAMP` | `datetime` | No time zone |
| `TIMESTAMP_LTZ` | `datetime_tz` | Local time zone |
| `TIMESTAMP_TZ` | `datetime_tz` | With time zone offset |
| `VARIANT` | `json` | Semi-structured data |
| `OBJECT` | `json` | Key-value semi-structured data |
| `ARRAY` | `json` | Semi-structured array |
| `GEOGRAPHY` | `string` | Spatial type |
| `GEOMETRY` | `string` | Spatial type |

## Placeholder syntax

Snowflake uses `?` positional placeholders:

```sql
SELECT id, name FROM users WHERE id = ?;
```

Scythe translates `$N` in your SQL to `?` for Snowflake backends.

## QUALIFY clause

Snowflake supports `QUALIFY` for filtering window function results without a subquery:

```sql
-- @name GetLatestOrderPerUser
-- @returns :many
SELECT user_id, order_id, total, created_at
FROM orders
QUALIFY ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY created_at DESC) = 1;
```

Scythe parses and supports the `QUALIFY` clause in the Snowflake dialect.

## Notes

- **Cloud-only** -- Snowflake has no local Docker container or embedded mode. For local development, use a Snowflake trial account or mock the database layer. Integration tests must run against a live Snowflake instance.
- **VARIANT/OBJECT/ARRAY** -- All semi-structured types map to `json` in the neutral type system. Use `@json` annotation for typed JSON deserialization.
- **TIMESTAMP variants** -- Snowflake has three timestamp types: `TIMESTAMP_NTZ` (no time zone, default), `TIMESTAMP_LTZ` (local time zone), and `TIMESTAMP_TZ` (with offset). Scythe maps NTZ to `datetime` and LTZ/TZ to `datetime_tz`.
- **Integer types** -- All Snowflake integer types (`INT`, `BIGINT`, `SMALLINT`, `TINYINT`) are stored as `NUMBER(38,0)`. Scythe maps them to `int64`.
- **No ENUM or ARRAY** -- Snowflake has no `ENUM` type and its `ARRAY` is a semi-structured type (not a typed array). Use `VARCHAR` for enum values and `VARIANT` for structured data.
