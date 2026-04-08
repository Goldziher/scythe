# MariaDB

MariaDB support with MySQL-compatible dialect plus MariaDB-specific features like native UUID, RETURNING, and INET4/INET6 types.

## Overview

MariaDB is a MySQL-compatible database that has diverged with its own features. While scythe previously treated MariaDB as a MySQL alias, v0.6.0 adds dedicated MariaDB manifests to support MariaDB-specific types and syntax that differ from MySQL. MariaDB backends use the same drivers as MySQL but with MariaDB-aware type resolution.

## Engine alias

```toml
# scythe.toml
[[sql]]
engine = "mariadb"
```

Note: `mariadb` was previously an alias for MySQL. In v0.6.0+, it activates MariaDB-specific type handling and feature support.

## Supported backends

MariaDB uses the same drivers as MySQL:

| Backend | Language | Driver |
|---------|----------|--------|
| `rust-sqlx` | Rust | sqlx (MySQL driver) |
| `python-aiomysql` | Python | aiomysql |
| `typescript-mysql2` | TypeScript | mysql2 |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC (MariaDB Connector/J) |
| `kotlin-jdbc` | Kotlin | JDBC (MariaDB Connector/J) |
| `csharp-mysqlconnector` | C# | MySqlConnector |
| `elixir-myxql` | Elixir | MyXQL |
| `ruby-mysql2` | Ruby | mysql2 gem |
| `php-pdo` | PHP | PDO (mysql driver) |

## Configuration

```toml
# scythe.toml
[[sql]]
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
| `VARCHAR` / `CHAR` / `TEXT` | `string` | |
| `BOOLEAN` / `BOOL` | `bool` | |
| `BLOB` / `BINARY` / `VARBINARY` | `bytes` | |
| `UUID` | `uuid` | MariaDB 10.7+ native UUID |
| `DATE` | `date` | |
| `TIME` | `time` | |
| `DATETIME` | `datetime` | |
| `TIMESTAMP` | `datetime` | |
| `JSON` | `json` | Alias for LONGTEXT in MariaDB |
| `INET4` | `inet` | MariaDB 10.10+ |
| `INET6` | `inet` | MariaDB 10.10+ |
| `ENUM(...)` | `string` | |

## Placeholder syntax

MariaDB uses `?` positional placeholders, same as MySQL:

```sql
SELECT id, name FROM users WHERE id = ?;
```

## Notes

- **Native UUID** -- MariaDB 10.7+ has a native `UUID` type stored as 16 bytes internally. Scythe maps this to the `uuid` neutral type, unlike MySQL where `CHAR(36)` maps to `string`.
- **RETURNING support** -- MariaDB supports `RETURNING` on INSERT and DELETE statements (10.5+). Scythe generates `:one` and `:many` return handling for these queries.
- **INET types** -- `INET4` and `INET6` are native MariaDB types (10.10+) mapped to the `inet` neutral type.
- **JSON handling** -- MariaDB's JSON type is an alias for `LONGTEXT` with JSON check constraint. Type mapping is identical to MySQL.
