# Oracle

Oracle Database support with PL/SQL dialect parsing, bind variable syntax, and type mappings across all 10 languages.

## Overview

Oracle Database is an enterprise relational database with its own SQL dialect and type system. Scythe supports Oracle-specific types like `NUMBER`, `VARCHAR2`, and `DATE` (which includes time), and uses `:N` bind variable syntax for parameter placeholders.

## Engine alias

```toml
# scythe.toml
[[sql]]
engine = "oracle"
```

## Supported backends

| Backend | Language | Driver |
|---------|----------|--------|
| `rust-sibyl` | Rust | sibyl |
| `python-oracledb` | Python | oracledb (python-oracledb) |
| `typescript-oracledb` | TypeScript | oracledb (node-oracledb) |
| `go-godror` | Go | godror |
| `java-jdbc` | Java | JDBC (Oracle JDBC / ojdbc) |
| `java-r2dbc` | Java | R2DBC (oracle-r2dbc) |
| `kotlin-jdbc` | Kotlin | JDBC (Oracle JDBC / ojdbc) |
| `kotlin-r2dbc` | Kotlin | R2DBC (oracle-r2dbc) |
| `csharp-odpnet` | C# | ODP.NET (Oracle.ManagedDataAccess) |
| `ruby-oci8` | Ruby | ruby-oci8 |
| `php-pdo` | PHP | PDO (oci driver) |
| `elixir-jamdb-oracle` | Elixir | jamdb_oracle |

## Configuration

```toml
# scythe.toml
[[sql]]
engine = "oracle"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "java-jdbc"
output = "src/generated"
```

## Type mapping table

| Oracle Type | Neutral Type | Notes |
|------------|-------------|-------|
| `NUMBER(*, 0)` / `INTEGER` / `INT` | `int64` | Oracle INTEGER is NUMBER(38,0) |
| `NUMBER(p, s)` where s > 0 | `decimal` | |
| `NUMBER` (no precision) | `decimal` | |
| `BINARY_FLOAT` | `float32` | |
| `BINARY_DOUBLE` | `float64` | |
| `VARCHAR2` / `NVARCHAR2` / `CHAR` / `NCHAR` | `string` | |
| `CLOB` / `NCLOB` | `string` | |
| `RAW` / `BLOB` | `bytes` | |
| `DATE` | `datetime` | Oracle DATE includes time |
| `TIMESTAMP` | `datetime` | |
| `TIMESTAMP WITH TIME ZONE` | `datetime_tz` | |
| `TIMESTAMP WITH LOCAL TIME ZONE` | `datetime_tz` | |
| `INTERVAL YEAR TO MONTH` | `string` | |
| `INTERVAL DAY TO SECOND` | `interval` | |
| `XMLTYPE` | `string` | |
| `BOOLEAN` | `bool` | Oracle 23c+ |

## Placeholder syntax

Oracle uses `:N` bind variable placeholders:

```sql
INSERT INTO users (name, email) VALUES (:1, :2);
```

Scythe translates `$N` in your SQL to `:N` for Oracle backends:

```sql
-- Written as:
SELECT id, name FROM users WHERE id = $1;

-- Translated to:
SELECT id, name FROM users WHERE id = :1;
```

## Docker setup

```bash
docker run -e ORACLE_PASSWORD=oracle -p 1521:1521 --name oracle \
  gvenzl/oracle-free:latest
```

## Notes

- **Oracle DATE** -- Unlike most databases, Oracle's `DATE` type includes time (hour, minute, second). It maps to `datetime`, not `date`.
- **NUMBER type** -- Oracle uses `NUMBER(precision, scale)` for all numeric types. Scythe infers the neutral type based on the scale: scale 0 maps to integer types, scale > 0 maps to `decimal`.
- **DUAL table** -- Oracle requires `SELECT ... FROM DUAL` for expressions without a table. Scythe handles this in query parsing.
- **No BOOLEAN before 23c** -- Oracle versions before 23c have no native `BOOLEAN` type. Use `NUMBER(1)` with a type override if targeting older versions.
- **RETURNING INTO** -- Oracle uses `RETURNING ... INTO :var` syntax. Scythe translates `RETURNING` clauses appropriately.
