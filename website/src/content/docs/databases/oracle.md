---
title: Oracle
description: Oracle Database support -- PL/SQL dialect, bind variables, and type mapping table.
---

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
| `kotlin-jdbc` | Kotlin | JDBC (Oracle JDBC / ojdbc) |
| `csharp-oracle` | C# | ODP.NET (Oracle.ManagedDataAccess) |
| `ruby-oci8` | Ruby | ruby-oci8 |
| `elixir-jamdb` | Elixir | jamdb_oracle |

`java-r2dbc`, `kotlin-r2dbc`, and `php-pdo` do not support Oracle -- their `supported_engines` lists
do not include `oracle`.

## Configuration

```toml
# scythe.toml
[[sql]]
name = "main"
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
| `INTEGER` / `INT` | `int64` | Alias for `NUMBER(38,0)` |
| `NUMBER(p, s)` where `s > 0` | `decimal` | Explicit non-zero scale |
| `NUMBER(p)` / `NUMBER(p, 0)` | `int64` | Explicit precision, zero (or implied zero) scale |
| `NUMBER` (table column, no precision or scale) | `int64` | Pragmatic default. Real schemas overwhelmingly use bare `NUMBER` as an integer/boolean-flag column (e.g. `NUMBER(1)`); a table column's bare `NUMBER` is indistinguishable from `NUMBER(p)` by the time it reaches type resolution |
| `NUMBER` (in `CAST(... AS NUMBER)` or parameter/return type inference, not a column) | `decimal` | Oracle's true "floating" type outside DDL: no precision or scale means it can hold fractional values |
| `BINARY_FLOAT` | `float32` | |
| `BINARY_DOUBLE` | `float64` | |
| `VARCHAR2` / `NVARCHAR2` / `CHAR` / `NCHAR` | `string` | |
| `CLOB` / `NCLOB` | `string` | |
| `RAW` / `BLOB` | `bytes` | |
| `DATE` | `datetime` | Oracle DATE includes time |
| `TIMESTAMP` | `datetime` | |
| `TIMESTAMP WITH TIME ZONE` | `datetime_tz` | |
| `TIMESTAMP WITH LOCAL TIME ZONE` | `datetime_tz` | |
| `INTERVAL YEAR TO MONTH` | `interval` | |
| `INTERVAL DAY TO SECOND` | `interval` | |
| `BOOLEAN` | `bool` | Oracle 23c+ |

`XMLTYPE` has no type mapping (only the generic `XML` spelling does). A column declared `XMLTYPE`
fails code generation.

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

CI runs Oracle integration tests against `gvenzl/oracle-xe:21-slim`:

```bash
docker run -e ORACLE_PASSWORD=oracle -p 1521:1521 --name oracle \
  gvenzl/oracle-xe:21-slim
```

## Notes

- **Oracle DATE** -- Unlike most databases, Oracle's `DATE` type includes time (hour, minute, second). It maps to `datetime`, not `date`.
- **NUMBER type** -- Oracle uses `NUMBER(precision, scale)` for all numeric types. See the type mapping
  table above for the full split between column definitions and direct AST resolution (`CAST`,
  parameters).
- **DUAL table** -- Oracle requires `SELECT ... FROM DUAL` for expressions without a table. Scythe handles this in query parsing.
- **No BOOLEAN before 23c** -- Oracle versions before 23c have no native `BOOLEAN` type. Use `NUMBER(1)` with a type override if targeting older versions.
- **RETURNING INTO** -- Oracle uses `RETURNING ... INTO :var` syntax. Scythe translates `RETURNING` clauses appropriately.
- **Empty string is NULL** -- Oracle stores a zero-length string as NULL: `''` and `N''` literals
  evaluate to NULL, not an empty value. This affects nullability inference --
  `COALESCE(email, '')` is non-nullable everywhere else (the literal fallback proves it), but stays
  **nullable** on Oracle because the fallback itself is NULL there. See
  [Type Inference](/scythe/guide/type-inference/#nullability-from-coalesce) and the live conformance
  fixture
  `testing_data/nullability_live/coalesce_non_null/live_coalesce_with_empty_string_default_is_null_on_oracle.json`.
