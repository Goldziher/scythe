# DuckDB

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
[sql]
engine = "duckdb"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "python-duckdb"
out = "src/generated/queries.py"
```

## Type mapping table

| DuckDB Type | Neutral Type | Notes |
|-------------|-------------|-------|
| `INTEGER` / `INT4` | `int32` | |
| `BIGINT` / `INT8` | `int64` | |
| `SMALLINT` / `INT2` | `int16` | |
| `TINYINT` / `INT1` | `int8` | |
| `HUGEINT` | `int64` | 128-bit integer mapped to int64 |
| `UHUGEINT` | `uint64` | Unsigned 128-bit integer |
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
| `LIST` | `array` | Mapped to language-native array/list |
| `STRUCT` | `json` | Mapped to JSON object |
| `MAP` | `json` | Mapped to JSON object |

## Placeholder syntax

DuckDB uses positional `$N` placeholders, same as PostgreSQL:

```sql
SELECT * FROM analytics WHERE user_id = $1 AND event_date > $2;
```

## Notes

- **Embedded architecture** -- no Docker container needed for testing. DuckDB runs in-process, so integration tests execute directly against an in-memory or file-based database.
- **PostgreSQL compatibility** -- most PostgreSQL SQL syntax works unchanged. Scythe reuses PostgreSQL dialect parsing with DuckDB-specific type resolution.
- Standard types (INTEGER, TEXT, BOOLEAN, etc.) follow the same mapping as PostgreSQL.
