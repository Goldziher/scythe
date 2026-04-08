# CockroachDB

Distributed SQL database with PostgreSQL wire compatibility. All PostgreSQL backends work with CockroachDB without modification.

## Overview

CockroachDB is a distributed SQL database that implements the PostgreSQL wire protocol. Scythe treats CockroachDB as a PostgreSQL-compatible engine -- all existing PostgreSQL backends automatically accept the `cockroachdb` engine. No special backends are needed.

## Engine alias

CockroachDB can be specified with either its full name or its abbreviation:

```toml
# scythe.toml -- either form is accepted
[sql]
engine = "cockroachdb"
# or
engine = "crdb"
```

## Supported backends

All PostgreSQL backends work with CockroachDB:

| Backend | Language | Driver |
|---------|----------|--------|
| `rust-sqlx` | Rust | sqlx with PostgreSQL driver |
| `rust-tokio-postgres` | Rust | tokio-postgres |
| `python-psycopg` | Python | psycopg (v3) |
| `python-asyncpg` | Python | asyncpg |
| `typescript-pg` | TypeScript | node-postgres (pg) |
| `typescript-postgres-js` | TypeScript | postgres.js |
| `go-pgx` | Go | pgx |
| `go-database-sql` | Go | database/sql with pgx driver |
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

## Configuration

```toml
# scythe.toml
[sql]
engine = "cockroachdb"
schema = ["schema.sql"]
queries = ["queries/"]

[[sql.gen]]
backend = "python-asyncpg"
out = "src/generated/queries.py"
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

- All PostgreSQL backends automatically accept the `cockroachdb` (or `crdb`) engine alias. No backend changes are needed when migrating from PostgreSQL to CockroachDB.
- Scythe uses PostgreSQL dialect parsing for CockroachDB. If your schema uses CockroachDB-specific features not present in PostgreSQL, define them in your DDL files and use `type_overrides` in `scythe.toml` for correct mapping.
- Standard type mappings are identical to PostgreSQL. See the [PostgreSQL](postgresql.md) page for the full type mapping table.
