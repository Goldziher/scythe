---
title: MariaDB
description: MariaDB support -- native UUID, RETURNING, INET types, and differences from MySQL.
---

MariaDB support with MySQL-compatible dialect plus MariaDB-specific features like native UUID, RETURNING, and INET4/INET6 types.

## Overview

MariaDB is a MySQL-compatible database that has diverged with its own features. While scythe previously treated MariaDB as a MySQL alias, v0.6.0 adds dedicated MariaDB manifests to support MariaDB-specific types and syntax that differ from MySQL. MariaDB backends use the same drivers as MySQL but with MariaDB-aware type resolution.

## Engine alias

```toml
# scythe.toml
[[sql]]
engine = "mariadb"
```

Note: `mariadb` was previously an alias for MySQL. In v0.6.0+, `engine = "mariadb"` selects
MariaDB-specific backend manifests for the four backends that have one (see below). Parsing and type
inference are unaffected: the parser resolves `mariadb` to the same `SqlDialect::MySQL` as `mysql`
(`crates/scythe-core/src/dialect.rs`), so nullability and type analysis are byte-identical to MySQL.

## Supported backends

MariaDB uses the same drivers as MySQL. Only four backends have a dedicated MariaDB manifest; the rest
accept `engine = "mariadb"` but load the same manifest as `engine = "mysql"`:

| Backend | Language | Driver | Manifest |
|---------|----------|--------|----------|
| `rust-sqlx` | Rust | sqlx (MySQL driver) | MariaDB-specific |
| `go-database-sql` | Go | database/sql | MariaDB-specific |
| `java-jdbc` | Java | JDBC (MariaDB Connector/J) | MariaDB-specific |
| `kotlin-jdbc` | Kotlin | JDBC (MariaDB Connector/J) | MariaDB-specific |
| `python-aiomysql` | Python | aiomysql | MySQL (shared) |
| `typescript-mysql2` | TypeScript | mysql2 | MySQL (shared) |
| `csharp-mysqlconnector` | C# | MySqlConnector | MySQL (shared) |
| `elixir-myxql` | Elixir | MyXQL | MySQL (shared) |
| `ruby-mysql2` | Ruby | mysql2 gem | MySQL (shared) |
| `php-pdo` | PHP | PDO (mysql driver) | MySQL (shared) |

`java-r2dbc` and `kotlin-r2dbc` also have dedicated MariaDB manifests but are not listed above because
neither backend supports MSSQL or Oracle either -- see their `supported_engines` list
(`postgresql`, `mysql`, `mariadb`, `sqlite`).

## Configuration

```toml
# scythe.toml
[[sql]]
name = "main"
engine = "mariadb"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "python-aiomysql"
output = "src/generated"
```

## Differences from MySQL

| Feature | MySQL | MariaDB |
|---------|-------|---------|
| `UUID` type | Not native (use `CHAR(36)`) | Native `UUID` type (10.7+) |
| `RETURNING` | Not supported | Supported on INSERT/DELETE (10.5+) |
| `INET4` / `INET6` | Not supported | Native network address types (10.10+) |
| Sequences | Not supported | `CREATE SEQUENCE` (10.3+) |
| Temporal tables | Not supported | System-versioned tables (10.3+) |
| JSON | Native JSON type | Alias for `LONGTEXT` with JSON validation |

## Type mapping table

| MariaDB Type | Neutral Type | Notes |
|-------------|-------------|-------|
| `INT` / `INTEGER` | `int32` | |
| `BIGINT` | `int64` | |
| `SMALLINT` | `int16` | |
| `TINYINT` | `int16` | |
| `MEDIUMINT` | `int32` | |
| `FLOAT` | `float32` | |
| `DOUBLE` | `float64` | |
| `DECIMAL` / `NUMERIC` | `decimal` | |
| `TINYINT UNSIGNED` / `SMALLINT UNSIGNED` | `int16` | Same-width signed neutral type |
| `MEDIUMINT UNSIGNED` / `INT UNSIGNED` | `int32` | Same-width signed neutral type |
| `BIGINT UNSIGNED` | `int64` | Same-width signed neutral type |
| `VARCHAR` / `CHAR` / `TEXT` | `string` | |
| `BOOLEAN` / `BOOL` | `bool` | |
| `BLOB` / `BINARY` / `VARBINARY` | `bytes` | |
| `UUID` | `uuid` | MariaDB 10.7+ native UUID |
| `DATE` | `date` | |
| `TIME` | `time` | |
| `DATETIME` | `datetime` | |
| `TIMESTAMP` | `datetime` | |
| `JSON` | `json` | Alias for LONGTEXT in MariaDB |
| `ENUM(...)` | `enum::{table}_{column}` | Registered as a catalog enum, same as MySQL |

`INET4` and `INET6` have no type mapping. A column declared with either fails code generation.

`UNSIGNED` integer columns map to the same-width signed neutral type -- there is no
dedicated unsigned neutral type, and `BIGINT UNSIGNED` has no wider type to widen
to. Values near the top of the unsigned range (for example, above `i16::MAX` for
`SMALLINT UNSIGNED` or above `i64::MAX` for `BIGINT UNSIGNED`) need
application-level handling in the generated code. This applies to MariaDB because
it resolves to the same `SqlDialect::MySQL` type-resolution path as MySQL.

## Placeholder syntax

MariaDB uses `?` positional placeholders, same as MySQL:

```sql
SELECT id, name FROM users WHERE id = ?;
```

## Notes

- **Native UUID** -- MariaDB 10.7+ has a native `UUID` type stored as 16 bytes internally. Scythe maps this to the `uuid` neutral type, unlike MySQL where `CHAR(36)` maps to `string`.
- **RETURNING support** -- MariaDB supports `RETURNING` on INSERT and DELETE statements (10.5+). Scythe generates `:one` and `:many` return handling for these queries.
- **INET types unsupported** -- `INET4` and `INET6` are native MariaDB types (10.10+), but scythe has no
  type mapping for them yet. A column declared with either fails code generation.
- **JSON handling** -- MariaDB's JSON type is an alias for `LONGTEXT` with JSON check constraint. Type mapping is identical to MySQL.
- **Inline ENUM** -- like MySQL, an inline `ENUM(...)` column is registered as a catalog enum named
  `{table}_{column}` and resolves to `enum::{table}_{column}`, not a bare `string`.
