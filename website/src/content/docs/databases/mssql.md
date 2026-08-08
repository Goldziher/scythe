---
title: MSSQL
description: Microsoft SQL Server support -- T-SQL dialect, parameter syntax, and type mapping table.
---

Microsoft SQL Server support with T-SQL dialect parsing, parameter syntax, and type mappings across all 10 languages.

## Overview

MSSQL (Microsoft SQL Server) is a widely used enterprise relational database. Scythe supports T-SQL
dialect parsing; you write `$N` placeholders in your SQL and each backend translates them to its
driver's native parameter binding convention (see [Placeholder syntax](#placeholder-syntax)).

## Engine alias

```toml
# scythe.toml
[[sql]]
engine = "mssql"  # or "sqlserver"
```

## Supported backends

| Backend | Language | Driver |
|---------|----------|--------|
| `rust-tiberius` | Rust | tiberius |
| `python-pyodbc` | Python | pyodbc |
| `typescript-mssql` | TypeScript | mssql (tedious) |
| `typescript-kysely` | TypeScript | Kysely |
| `go-database-sql` | Go | database/sql (with `engine = "mssql"`) |
| `java-jdbc` | Java | JDBC (Microsoft JDBC Driver) |
| `kotlin-jdbc` | Kotlin | JDBC (Microsoft JDBC Driver) |
| `csharp-sqlclient` | C# | Microsoft.Data.SqlClient |
| `ruby-tiny-tds` | Ruby | tiny_tds |
| `php-pdo` | PHP | PDO (sqlsrv driver) |
| `elixir-tds` | Elixir | tds |

`java-r2dbc` and `kotlin-r2dbc` do not support MSSQL -- their `supported_engines` list is
`postgresql`, `mysql`, `mariadb`, `sqlite`.

## Configuration

```toml
# scythe.toml
[[sql]]
name = "main"
engine = "mssql"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "csharp-sqlclient"
output = "src/generated"
```

## Type mapping table

| MSSQL Type | Neutral Type | Notes |
|-----------|-------------|-------|
| `INT` | `int32` | |
| `BIGINT` | `int64` | |
| `SMALLINT` | `int16` | |
| `TINYINT` | `int16` | Unsigned 0-255 |
| `BIT` | `bool` | |
| `REAL` | `float32` | |
| `FLOAT` | `float64` | |
| `DECIMAL` / `NUMERIC` | `decimal` | Precision is stripped |
| `MONEY` / `SMALLMONEY` | `decimal` | |
| `VARCHAR` / `NVARCHAR` / `CHAR` / `NCHAR` | `string` | |
| `TEXT` / `NTEXT` | `string` | Deprecated in MSSQL |
| `VARBINARY` / `BINARY` / `IMAGE` | `bytes` | |
| `UNIQUEIDENTIFIER` | `uuid` | |
| `DATE` | `date` | |
| `TIME` | `time` | |
| `DATETIME` / `DATETIME2` / `SMALLDATETIME` | `datetime` | |
| `DATETIMEOFFSET` | `datetime_tz` | |
| `XML` | `string` | |

## Placeholder syntax

Write positional `$N` placeholders in your SQL. Each backend translates them to its driver's native
convention -- placeholder style is per-backend, not uniform across MSSQL:

| Backend | Placeholder style |
|---------|-------------------|
| `rust-tiberius`, `csharp-sqlclient`, `typescript-mssql`, `typescript-kysely` | `@p1`, `@p2`, ... |
| `python-pyodbc`, `java-jdbc`, `kotlin-jdbc`, `go-database-sql` | `?` |
| `ruby-tiny-tds` | `@1`, `@2`, ... |
| `elixir-tds` | `:p1`, `:p2`, ... |
| `php-pdo` | `?` |

```sql
-- Written as:
SELECT id, name FROM users WHERE id = $1;

-- csharp-sqlclient generates:
SELECT id, name FROM users WHERE id = @p1;
```

## Docker setup

```bash
docker run -e 'ACCEPT_EULA=Y' -e 'MSSQL_SA_PASSWORD=YourStrong@Passw0rd' \
  -p 1433:1433 --name mssql \
  mcr.microsoft.com/mssql/server:2022-latest
```

## Notes

- **T-SQL dialect** -- Scythe parses T-SQL syntax including `TOP`, `OUTPUT`, `MERGE`, and `OFFSET FETCH`.
- **IDENTITY columns** -- Scythe strips the `IDENTITY(seed, step)` clause when parsing. Unlike
  PostgreSQL's `SERIAL`, `IDENTITY` alone does not imply NOT NULL: the column is nullable unless it
  also declares `NOT NULL` or `PRIMARY KEY`.
- **OUTPUT clause** -- MSSQL uses `OUTPUT INSERTED.*` instead of `RETURNING`. Scythe handles this translation.
- **String types** -- `NVARCHAR` and `VARCHAR` both map to `string`. No distinction is made between Unicode and non-Unicode strings in the neutral type system.
