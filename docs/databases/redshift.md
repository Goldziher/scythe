# Redshift

Amazon Redshift support with PostgreSQL-compatible dialect, columnar storage types, and SUPER semi-structured data type.

## Overview

Amazon Redshift is a cloud data warehouse based on PostgreSQL. It uses the PostgreSQL wire protocol and a compatible SQL dialect, with additions like the `SUPER` type for semi-structured data, `IDENTITY` columns, and columnar storage optimizations. Scythe reuses PostgreSQL backends with Redshift-specific type resolution.

## Engine alias

```toml
# scythe.toml
[[sql]]
engine = "redshift"
```

## Supported backends

Redshift uses PostgreSQL backends:

| Backend | Language | Driver |
|---------|----------|--------|
| `rust-sqlx` | Rust | sqlx (PostgreSQL driver) |
| `rust-tokio-postgres` | Rust | tokio-postgres |
| `python-psycopg3` | Python | psycopg3 |
| `python-asyncpg` | Python | asyncpg |
| `typescript-pg` | TypeScript | pg (node-postgres) |
| `typescript-postgres` | TypeScript | postgres.js |
| `go-pgx` | Go | pgx v5 |
| `java-jdbc` | Java | JDBC (Redshift JDBC driver) |
| `kotlin-jdbc` | Kotlin | JDBC (Redshift JDBC driver) |
| `csharp-npgsql` | C# | Npgsql |
| `elixir-postgrex` | Elixir | Postgrex |
| `ruby-pg` | Ruby | pg gem |
| `php-pdo` | PHP | PDO (pgsql driver) |

## Configuration

```toml
# scythe.toml
[[sql]]
engine = "redshift"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "python-psycopg3"
output = "src/generated"
```

## Differences from PostgreSQL

| Feature | PostgreSQL | Redshift |
|---------|-----------|----------|
| `SERIAL` | Sequence-backed auto-increment | Not supported (use `IDENTITY`) |
| `ENUM` types | `CREATE TYPE ... AS ENUM` | Not supported |
| `ARRAY` types | Native `TEXT[]`, `INT[]` | Not supported |
| Range types | `int4range`, `tstzrange` | Not supported |
| `SUPER` | Not available | Semi-structured data type |
| `IDENTITY` | Standard identity columns | `IDENTITY(seed, step)` |
| `HLLSKETCH` | Not available | HyperLogLog sketch type |
| `GEOMETRY` / `GEOGRAPHY` | PostGIS extension | Native spatial types |

## Type mapping table

| Redshift Type | Neutral Type | Notes |
|--------------|-------------|-------|
| `INTEGER` / `INT` / `INT4` | `int32` | |
| `BIGINT` / `INT8` | `int64` | |
| `SMALLINT` / `INT2` | `int16` | |
| `REAL` / `FLOAT4` | `float32` | |
| `DOUBLE PRECISION` / `FLOAT8` | `float64` | |
| `DECIMAL` / `NUMERIC` | `decimal` | |
| `VARCHAR` / `CHAR` / `TEXT` / `BPCHAR` | `string` | |
| `BOOLEAN` / `BOOL` | `bool` | |
| `VARBYTE` / `BINARY VARYING` | `bytes` | Redshift binary type |
| `DATE` | `date` | |
| `TIME` / `TIMETZ` | `time` | |
| `TIMESTAMP` | `datetime` | |
| `TIMESTAMPTZ` | `datetime_tz` | |
| `SUPER` | `json` | Semi-structured data |
| `HLLSKETCH` | `string` | HyperLogLog sketch |
| `GEOMETRY` | `string` | Spatial type |
| `GEOGRAPHY` | `string` | Spatial type |

## Placeholder syntax

Redshift uses PostgreSQL positional `$N` placeholders:

```sql
SELECT id, name FROM users WHERE id = $1;
```

## Notes

- **No ENUM or ARRAY** -- Redshift does not support PostgreSQL `ENUM` or `ARRAY` types. Use `VARCHAR` with check constraints for enum-like behavior, and normalize arrays into separate tables.
- **IDENTITY columns** -- Use `IDENTITY(1,1)` instead of `SERIAL`. Scythe treats `IDENTITY` columns as NOT NULL.
- **SUPER type** -- Redshift's `SUPER` type stores semi-structured data (JSON-like). Scythe maps it to the `json` neutral type.
- **Cloud-only with local testing** -- Redshift is a cloud service, but you can use PostgreSQL locally for development since they share the same wire protocol. Use `engine = "redshift"` to get Redshift-specific type handling.
- **QUALIFY clause** -- Redshift does not support `QUALIFY`. Use subqueries with window functions instead.
