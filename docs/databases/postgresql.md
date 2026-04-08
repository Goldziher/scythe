# PostgreSQL

Scythe's primary and most complete dialect. All features are supported.

## Supported features

- **Enums** -- `CREATE TYPE ... AS ENUM (...)` parsed and mapped to `enum::name`
- **Composite types** -- `CREATE TYPE ... AS (...)` mapped to `composite::name`
- **Arrays** -- `TEXT[]`, `INTEGER[]`, etc. mapped to `array<T>`
- **JSONB / JSON** -- mapped to `json`; typed JSON via `@json_typed` annotation
- **Views** -- resolved through underlying table definitions
- **Domains** -- `CREATE DOMAIN` resolved to base type with NOT NULL propagation
- **Range types** -- `int4range`, `tstzrange`, etc. mapped to `range<T>`
- **Network types** -- `INET`, `CIDR`, `MACADDR` mapped to `inet`

## Type mapping table

| PostgreSQL Type | Neutral Type | Notes |
|----------------|-------------|-------|
| `SERIAL` / `INTEGER` / `INT4` | `int32` | SERIAL implies NOT NULL |
| `BIGSERIAL` / `BIGINT` / `INT8` | `int64` | |
| `SMALLSERIAL` / `SMALLINT` / `INT2` | `int16` | |
| `REAL` / `FLOAT4` | `float32` | |
| `DOUBLE PRECISION` / `FLOAT8` | `float64` | |
| `NUMERIC` / `DECIMAL` | `decimal` | Precision is stripped |
| `TEXT` / `VARCHAR` / `CHAR` | `string` | All character types unify to `string` |
| `BOOLEAN` / `BOOL` | `bool` | |
| `BYTEA` | `bytes` | |
| `UUID` | `uuid` | |
| `DATE` | `date` | |
| `TIME` / `TIME WITHOUT TIME ZONE` | `time` | |
| `TIMETZ` / `TIME WITH TIME ZONE` | `time_tz` | |
| `TIMESTAMP` / `TIMESTAMP WITHOUT TIME ZONE` | `datetime` | |
| `TIMESTAMPTZ` / `TIMESTAMP WITH TIME ZONE` | `datetime_tz` | |
| `INTERVAL` | `interval` | |
| `JSON` / `JSONB` | `json` | |
| `INET` / `CIDR` / `MACADDR` | `inet` | |
| `INTEGER[]` | `array<int32>` | Recursive resolution |
| `TEXT[]` | `array<string>` | |
| `INT4RANGE` | `range<int32>` | |
| `INT8RANGE` | `range<int64>` | |
| `TSTZRANGE` | `range<datetime_tz>` | |
| `DATERANGE` | `range<date>` | |
| `NUMRANGE` | `range<decimal>` | |
| User-defined enum | `enum::name` | |
| User-defined composite | `composite::name` | |
| Domain type | resolves to base | NOT NULL propagated |

## PostgreSQL-specific annotations

```sql
-- @name GetUser
-- @returns :one
SELECT id, name, email FROM users WHERE id = $1;
```

- Parameter placeholders use `$N` syntax (`$1`, `$2`, ...)
- `RETURNING` clause support for `:one` and `:many` on INSERT/UPDATE/DELETE
- `ON CONFLICT` (UPSERT) is fully supported
- `SERIAL` / `BIGSERIAL` columns are automatically marked NOT NULL

## Placeholder syntax

PostgreSQL uses positional `$N` placeholders:

```sql
INSERT INTO users (name, email) VALUES ($1, $2);
```
