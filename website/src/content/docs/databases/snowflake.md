---
title: Snowflake
description: Snowflake support -- VARIANT semi-structured type, TIMESTAMP variants, and the QUALIFY clause.
---

Snowflake support with the VARIANT semi-structured type, TIMESTAMP variants, and QUALIFY clause.

## Overview

Snowflake is a cloud-native data warehouse with its own SQL dialect. It features the `VARIANT` semi-structured
data type, multiple timestamp variants, and the `QUALIFY` clause for filtering window function results.
Snowflake is cloud-only -- there is no local Docker container for development.

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
name = "main"
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
| `NUMBER(p,s)` / `DECIMAL(p,s)` / `NUMERIC(p,s)`, `s > 0` | `decimal` | A non-zero scale is a true decimal |
| `NUMBER` / `NUMBER(p)` / `NUMBER(p,0)` | `int64` | Zero scale is an integer, whatever the precision |
| `INT` / `INTEGER` / `BIGINT` / `SMALLINT` / `TINYINT` / `BYTEINT` | `int64` | All integer types are NUMBER(38,0) |
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

- **VARIANT** -- maps to `json` in the neutral type system. Use the `@json` annotation for typed JSON
  deserialization.
- **TIMESTAMP variants** -- Snowflake has three timestamp types: `TIMESTAMP_NTZ` (no time zone, default), `TIMESTAMP_LTZ` (local time zone), and `TIMESTAMP_TZ` (with offset). Scythe maps NTZ to `datetime` and LTZ/TZ to `datetime_tz`.
- **Integer types** -- All Snowflake integer types (`INT`, `BIGINT`, `SMALLINT`, `TINYINT`, `BYTEINT`) are stored as `NUMBER(38,0)`. Scythe maps them to `int64`, and maps an explicit `NUMBER(38,0)` the same way -- that is the spelling `DESCRIBE TABLE` reports, so a schema dumped from a live table types its keys identically to one written with `INT`.
- **No ENUM** -- Snowflake has no `ENUM` type. Use `VARCHAR` with a check constraint for enum-like behavior.
- **`OBJECT` and scalar `ARRAY` are unsupported** -- there is no type mapping for Snowflake's `OBJECT` or
  `ARRAY` column types; a column declared with either fails code generation. Only `VARIANT` is supported
  for semi-structured data.

:::caution
Integration tests run against [fakesnow](https://github.com/tekumara/fakesnow), a Snowflake emulator built on
DuckDB, not a live Snowflake instance (see `.github/workflows/integration.yml`, job
`integration-snowflake`). Behavior that depends on genuine Snowflake server semantics is unverified.

Snowflake is also absent from the nullability conformance suite, which has six legs -- SQLite, PostgreSQL,
MySQL, MariaDB, SQL Server and Oracle. That suite is what validates inference against live engines, so no
live-engine check of Snowflake inference exists at all.
:::
