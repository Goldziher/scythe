---
title: PostgreSQL
description: Scythe's primary and most complete SQL dialect -- supported features and type mapping table.
---

Scythe's primary and most complete dialect. Every feature below is parsed and compiled; the two
caveats worth knowing before you rely on them are runtime composite decoding and nested aggregates,
both noted where they apply.

## Supported features

- **Enums** -- `CREATE TYPE ... AS ENUM (...)` parsed and mapped to `enum::name`
- **Composite types** -- `CREATE TYPE ... AS (...)` mapped to `composite::name`
- **Arrays** -- `TEXT[]`, `INTEGER[]`, etc. mapped to `array<T>`
- **JSONB / JSON** -- mapped to `json`; typed JSON via `@json` annotation
- **Nested aggregates** -- `json_agg`/`jsonb_agg` and `row_to_json`/`to_json`/`to_jsonb` over
  `alias.*` mapped to `json_nested<T>`
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
| `MONEY` | `decimal` | Fixed-point currency type |
| `TEXT` / `VARCHAR` / `CHAR` | `string` | All character types unify to `string` |
| `XML` | `string` | No driver surfaces it as anything richer |
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
| `TSRANGE` | `range<datetime>` | |
| `TSTZRANGE` | `range<datetime_tz>` | |
| `DATERANGE` | `range<date>` | |
| `NUMRANGE` | `range<decimal>` | |
| User-defined enum | `enum::name` | |
| User-defined composite | `composite::name` | See note below -- most backends declare the type but do not decode it |
| Domain type | resolves to base | NOT NULL propagated |

A composite column is parsed and mapped to `composite::name` on every backend, and every backend
emits a struct/record type for it. Decoding a **nullable composite column at runtime**, however, only
works on four of the fifteen PostgreSQL backends: `rust-sqlx` and `rust-tokio-postgres` (via their
drivers' derive macros) and `java-jdbc` and `kotlin-jdbc` (which parse the composite text form). On
the other eleven -- `csharp-npgsql`, `python-psycopg3`, `python-asyncpg`, the `typescript-pg` family,
`php-pdo`, `php-amphp`, `ruby-pg`, `elixir-postgrex`, `elixir-ecto` and `go-pgx` -- the generated row
type declares the composite struct, but the driver's raw value is assigned straight through without
parsing it, so the type annotation does not match what the driver returns at runtime.

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

## Nested aggregates

`json_agg(alias.*)`, `jsonb_agg(alias.*)`, `row_to_json(alias.*)`, `to_json(alias.*)` and
`to_jsonb(alias.*)` over a relation resolve to a struct scythe synthesizes from that relation's
columns, rather than to an opaque `json` scalar:

```sql
-- @name GetUsersWithOrders
-- @returns :many
SELECT u.id, u.name, json_agg(o.*) AS orders
FROM users u
JOIN orders o ON o.user_id = u.id
GROUP BY u.id, u.name;
```

`orders` becomes `Option<sqlx::types::Json<Vec<GetUsersWithOrdersRowOrders>>>` on `rust-sqlx`, with
the struct declared alongside the row struct.

`json_agg(json_build_object(...))`/`jsonb_agg(jsonb_build_object(...))` also infer a struct, with
fields taken from the call's own string-literal keys instead of the relation's schema -- see
[the inline form](/scythe/guide/type-inference/#the-json_build_object-inline-form) for the
nullability rules, which differ from the whole-row form above.

This is PostgreSQL-only, and among PostgreSQL-compatible engines it applies to PostgreSQL and
CockroachDB. Redshift is excluded: it uses the PostgreSQL dialect but has no `json_agg`. Six backends
decode the result -- `rust-sqlx`, `rust-tokio-postgres`, `go-pgx`, `python-psycopg3`, `php-pdo` and
`php-amphp`. On every other backend the column keeps the plain `json` mapping.

See [Type Inference](/scythe/guide/type-inference/#nested-aggregates) for naming, nullability and JSON
key handling.

## Placeholder syntax

PostgreSQL uses positional `$N` placeholders:

```sql
INSERT INTO users (name, email) VALUES ($1, $2);
```
