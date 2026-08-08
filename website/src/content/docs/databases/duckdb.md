---
title: DuckDB
description: Embedded analytical database support -- backends, configuration, and type mapping table.
---

Embedded analytical database with PostgreSQL-compatible SQL. DuckDB runs in-process -- no server required.

## Overview

DuckDB is an in-process analytical database designed for OLAP workloads. It speaks a PostgreSQL-compatible SQL dialect, making it straightforward for scythe to support with minimal engine-specific logic. Because DuckDB is embedded, there is no Docker container or external service needed for development or testing.

## Supported backends

| Backend | Language | Driver |
|---------|----------|--------|
| `python-duckdb` | Python | `duckdb` (native Python API) |
| `typescript-duckdb` | TypeScript | `duckdb-node` / `@duckdb/node-api` |
| `go-database-sql` | Go | `github.com/marcboeker/go-duckdb` via `database/sql` |
| `java-jdbc` | Java | DuckDB JDBC driver |
| `kotlin-jdbc` | Kotlin | DuckDB JDBC driver |

## Configuration

```toml
# scythe.toml
[[sql]]
name = "main"
engine = "duckdb"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "python-duckdb"
output = "src/generated"
```

## Type mapping table

| DuckDB Type | Neutral Type | Notes |
|-------------|-------------|-------|
| `INTEGER` / `INT4` | `int32` | |
| `BIGINT` / `INT8` | `int64` | |
| `SMALLINT` / `INT2` | `int16` | |
| `TINYINT` / `INT1` | `int16` | There is no `int8` neutral type |
| `HUGEINT` | `decimal` | 128-bit integer; no neutral type is wide enough to hold it losslessly |
| `UHUGEINT` | `decimal` | Unsigned 128-bit integer; there is no `uint64` neutral type |
| `REAL` / `FLOAT4` | `float32` | |
| `DOUBLE` / `FLOAT8` | `float64` | |
| `DECIMAL` / `NUMERIC` | `decimal` | Precision is stripped |
| `VARCHAR` / `TEXT` | `string` | |
| `BOOLEAN` / `BOOL` | `bool` | |
| `BLOB` | `bytes` | |
| `UUID` | `uuid` | |
| `DATE` | `date` | |
| `TIME` | `time` | |
| `TIMESTAMP` | `datetime` | |
| `TIMESTAMP WITH TIME ZONE` | `datetime_tz` | |
| `INTERVAL` | `interval` | |
| `JSON` | `json` | |

## Placeholder syntax

Write positional `$N` placeholders in your SQL, same as PostgreSQL. DuckDB backends translate
these to `?` in the generated code:

```sql
-- Written as:
SELECT * FROM analytics WHERE user_id = $1 AND event_date > $2;

-- Generated as:
SELECT * FROM analytics WHERE user_id = ? AND event_date > ?;
```

## Notes

- **Embedded architecture** -- no Docker container needed for testing. DuckDB runs in-process, so
  there is no server to provision.
- **PostgreSQL compatibility** -- most PostgreSQL SQL syntax works unchanged. Scythe reuses
  PostgreSQL dialect parsing with DuckDB-specific type resolution.
- Standard types (INTEGER, TEXT, BOOLEAN, etc.) follow the same mapping as PostgreSQL.

:::caution
There is no DuckDB integration test suite and no DuckDB CI job. Type mappings above are verified by
unit tests in `scythe-core`, but end-to-end behavior against a real DuckDB database is unverified.
:::
