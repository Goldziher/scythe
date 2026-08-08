---
title: CockroachDB
description: Distributed SQL database support -- engine alias, backends, and differences from PostgreSQL.
---

Distributed SQL database with PostgreSQL wire compatibility. All backends that support the
`postgresql` engine work with CockroachDB without modification.

## Overview

CockroachDB is a distributed SQL database that implements the PostgreSQL wire protocol. Scythe treats
CockroachDB as a PostgreSQL-compatible engine: `cockroachdb`/`crdb` normalize to the same
`postgresql` engine string used for backend selection and to `SqlDialect::PostgreSQL` for parsing and
type inference (`crates/scythe-core/src/dialect.rs`). No special backends or manifests exist for
CockroachDB -- it is indistinguishable from PostgreSQL past the engine-alias normalization step.

## Engine alias

CockroachDB can be specified with either its full name or its abbreviation:

```toml
# scythe.toml
[[sql]]
engine = "cockroachdb"  # or "crdb"
```

## Supported backends

Every backend that accepts the `postgresql` engine works with CockroachDB:

| Backend | Language | Driver |
|---------|----------|--------|
| `rust-sqlx` | Rust | sqlx with PostgreSQL driver |
| `rust-tokio-postgres` | Rust | tokio-postgres |
| `python-psycopg3` | Python | psycopg (v3) |
| `python-asyncpg` | Python | asyncpg |
| `typescript-pg` | TypeScript | node-postgres (pg) |
| `typescript-postgres` | TypeScript | postgres.js |
| `go-pgx` | Go | pgx |
| `java-jdbc` | Java | PostgreSQL JDBC driver |
| `kotlin-jdbc` | Kotlin | PostgreSQL JDBC driver |
| `java-r2dbc` | Java | r2dbc-postgresql |
| `kotlin-r2dbc` | Kotlin | r2dbc-postgresql |
| `kotlin-exposed` | Kotlin | Exposed with PostgreSQL driver |
| `csharp-npgsql` | C# | Npgsql |
| `elixir-postgrex` | Elixir | Postgrex |
| `ruby-pg` | Ruby | pg gem |
| `php-pdo` | PHP | PDO with pgsql driver |
| `php-amphp` | PHP | AMPHP PostgreSQL |

`go-database-sql` does **not** support `postgresql`/`cockroachdb` -- its `supported_engines` list is
`mysql`, `mariadb`, `mssql`, `sqlite`, `duckdb`. Use `go-pgx` for CockroachDB on Go.

## Configuration

```toml
# scythe.toml
[[sql]]
name = "main"
engine = "cockroachdb"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "python-asyncpg"
output = "src/generated"
```

## Type differences from PostgreSQL

While CockroachDB is PostgreSQL-compatible, there are some type and feature differences to be aware of:

| Feature | PostgreSQL | CockroachDB |
|---------|-----------|-------------|
| `SERIAL` | Creates sequence-backed auto-increment | Creates `INT8` with `unique_rowid()` |
| `tsvector` / `tsquery` | Full-text search types | Not supported |
| Advisory locks | `pg_advisory_lock()` | Not supported |
| `MONEY` | Currency type | Not supported |
| Range types | `int4range`, `tstzrange`, etc. | Not supported |

## Placeholder syntax

CockroachDB uses PostgreSQL positional `$N` placeholders:

```sql
INSERT INTO accounts (owner, balance) VALUES ($1, $2);
```

## Notes

- Every backend that supports the `postgresql` engine automatically accepts the `cockroachdb` (or
  `crdb`) engine alias -- no backend changes are needed when migrating from PostgreSQL to CockroachDB.
- Scythe uses PostgreSQL dialect parsing for CockroachDB. If your schema uses CockroachDB-specific features not present in PostgreSQL, define them in your DDL files and use `type_overrides` in `scythe.toml` for correct mapping.
- Type mappings are identical to PostgreSQL -- there is no CockroachDB-specific type resolution path.
  See the [PostgreSQL](/scythe/databases/postgresql/) page for the full type mapping table.

:::caution
There is no CockroachDB CI job. PostgreSQL wire and dialect compatibility is CockroachDB's own claim,
not something scythe's test suite verifies against a live CockroachDB instance.
:::
