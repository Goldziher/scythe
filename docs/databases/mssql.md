# MSSQL

Microsoft SQL Server support with T-SQL dialect parsing, parameter syntax, and type mappings across all 10 languages.

## Overview

MSSQL (Microsoft SQL Server) is a widely used enterprise relational database. Scythe supports T-SQL dialect parsing with `@pN` named parameter syntax. MSSQL backends generate code that uses the native parameter binding conventions for each language driver.

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
| `go-mssqldb` | Go | go-mssqldb |
| `java-jdbc` | Java | JDBC (Microsoft JDBC Driver) |
| `java-r2dbc` | Java | R2DBC (r2dbc-mssql) |
| `kotlin-jdbc` | Kotlin | JDBC (Microsoft JDBC Driver) |
| `kotlin-r2dbc` | Kotlin | R2DBC (r2dbc-mssql) |
| `csharp-sqlclient` | C# | Microsoft.Data.SqlClient |
| `ruby-tiny-tds` | Ruby | tiny_tds |
| `php-pdo` | PHP | PDO (sqlsrv driver) |
| `elixir-tds` | Elixir | tds |

## Configuration

```toml
# scythe.toml
[[sql]]
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
| `TINYINT` | `int8` | Unsigned 0-255 |
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

MSSQL uses `@pN` named parameter placeholders:

```sql
INSERT INTO users (name, email) VALUES (@p1, @p2);
```

Scythe translates `$N` in your SQL to `@pN` for MSSQL backends:

```sql
-- Written as:
SELECT id, name FROM users WHERE id = $1;

-- Translated to:
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
- **IDENTITY columns** -- `IDENTITY(1,1)` columns are treated as NOT NULL, equivalent to PostgreSQL's `SERIAL`.
- **OUTPUT clause** -- MSSQL uses `OUTPUT INSERTED.*` instead of `RETURNING`. Scythe handles this translation.
- **String types** -- `NVARCHAR` and `VARCHAR` both map to `string`. No distinction is made between Unicode and non-Unicode strings in the neutral type system.
