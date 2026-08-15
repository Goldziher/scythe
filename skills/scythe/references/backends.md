# Backends Reference

Scythe provides 59 selectable backend names across 10 languages and 10 database engines, implemented by 52 `CodegenBackend` types -- the seven `javascript-*` names are a JSDoc emit mode on the matching TypeScript backend, not separate implementations.

## Language Coverage

| Language | PostgreSQL | MySQL | SQLite | DuckDB | CockroachDB | MSSQL | Oracle | MariaDB | Redshift | Snowflake |
|----------|-----------|-------|--------|--------|-------------|-------|--------|---------|----------|-----------|
| Rust | sqlx, tokio-postgres | sqlx | sqlx | -- | sqlx, tokio-postgres | tiberius | sibyl | sqlx | sqlx, tokio-postgres | -- |
| Python | psycopg3, asyncpg | aiomysql | aiosqlite | duckdb | psycopg3, asyncpg | pyodbc | oracledb | aiomysql | psycopg3, asyncpg | snowflake-connector |
| TypeScript | postgres.js, pg, kysely | mysql2, kysely | better-sqlite3, node:sqlite, sqlite-wasm, kysely | duckdb-node | postgres.js, pg, kysely | mssql, kysely | oracledb | mysql2, kysely | postgres.js, pg, kysely | snowflake-sdk |
| JavaScript | postgres.js, pg | mysql2 | better-sqlite3, node:sqlite, sqlite-wasm | -- | postgres.js, pg | -- | -- | mysql2 | postgres.js, pg | snowflake-sdk |
| Go | pgx | database/sql | database/sql | database/sql | pgx | go-mssqldb | godror | database/sql | pgx | gosnowflake |
| Java | JDBC, R2DBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC, R2DBC | JDBC | JDBC | JDBC, R2DBC | JDBC | JDBC |
| Kotlin | JDBC, R2DBC, Exposed | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC, R2DBC, Exposed | JDBC | JDBC | JDBC, R2DBC | JDBC | JDBC |
| C# | Npgsql | MySqlConnector | Microsoft.Data.Sqlite | -- | Npgsql | Microsoft.Data.SqlClient | ODP.NET | MySqlConnector | Npgsql | Snowflake.Data |
| Elixir | Postgrex, Ecto | MyXQL | Exqlite | -- | Postgrex, Ecto | tds | jamdb_oracle | MyXQL | Postgrex | -- |
| Ruby | pg | mysql2, Trilogy | sqlite3 | -- | pg | tiny_tds | ruby-oci8 | mysql2, Trilogy | pg | -- |
| PHP | PDO, AMPHP | PDO, AMPHP | PDO | -- | PDO, AMPHP | PDO | -- | PDO, AMPHP | PDO | PDO |

The CockroachDB column matches the PostgreSQL column exactly, by construction:
`normalize_engine` folds `cockroachdb` and `crdb` into `postgresql` before a
backend's engine support is consulted, so every PostgreSQL backend is also a
CockroachDB backend. Redshift does not fold that way -- it stays a distinct
engine requiring a per-backend `*.redshift.toml` manifest, which is why fewer
backends offer it.

R2DBC covers PostgreSQL, MySQL, MariaDB and SQLite only. It does *not* cover
MSSQL or Oracle -- `supported_engines()` rejects both engines, and no
MSSQL/Oracle manifests ship for `java-r2dbc` or `kotlin-r2dbc` (see
[#105](https://github.com/Goldziher/scythe/issues/105)).

`kysely` (backend `typescript-kysely`) is dialect-parameterised: it compiles to Kysely's `sql` tag, which renders whatever placeholder syntax the connected `Dialect` needs at runtime, so one generated call site runs against any Kysely dialect. Scythe pins and tests five dialects -- PostgreSQL, MySQL, SQLite, MSSQL, MariaDB -- plus a Redshift manifest that reuses the PostgreSQL dialect. Third-party dialects (libsql, PlanetScale, Cloudflare D1, Neon, PGlite, and `node:sqlite` / `@sqlite.org/sqlite-wasm` used as a Kysely dialect) are wire-compatible but not pinned or tested by scythe.

`typescript-node-sqlite` and `typescript-wasm-sqlite` (and their `javascript-node-sqlite` / `javascript-wasm-sqlite` counterparts) generate synchronous code (plain `export function`, no `async`, no `Promise`), unlike every other TypeScript/JavaScript backend. `typescript-node-sqlite` and `javascript-node-sqlite` require `--experimental-sqlite` on Node 22 and are unflagged from Node 23.4 onward.

The JavaScript row is `javascript-postgres`, `javascript-pg`, `javascript-mysql2`, `javascript-better-sqlite3`, `javascript-node-sqlite`, `javascript-wasm-sqlite`, and `javascript-snowflake` -- a JSDoc emit mode on the matching TypeScript backend, not separate backends. Output is `queries.js`: plain ESM, no driver import (handles are typed inline as `import("pg").PoolClient` in `@param`), row types as `@typedef {object}` with nullable columns always `{T | null}`. `row_type = "zod"`, `outer_join_unions`, and `field_case = "camelCase"` are hard errors there, each naming the TypeScript backend to use instead; `structs_only` and `field_case = "snake_case"` work. The other four TypeScript backends -- `typescript-duckdb`, `typescript-kysely`, `typescript-mssql`, `typescript-oracledb` -- have no JavaScript counterpart.

## Backend Names

Use these exact names in `[[sql.gen]] backend = "..."`:

### PostgreSQL

`rust-sqlx`, `rust-tokio-postgres`, `python-psycopg3`, `python-asyncpg`, `typescript-postgres`, `typescript-pg`, `typescript-kysely`, `javascript-postgres`, `javascript-pg`, `go-pgx`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `kotlin-exposed`, `csharp-npgsql`, `elixir-postgrex`, `elixir-ecto`, `ruby-pg`, `php-pdo`, `php-amphp`

### MySQL

`rust-sqlx`, `python-aiomysql`, `typescript-mysql2`, `typescript-kysely`, `javascript-mysql2`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-mysqlconnector`, `elixir-myxql`, `ruby-mysql2`, `ruby-trilogy`, `php-pdo`, `php-amphp`

### SQLite

`rust-sqlx`, `python-aiosqlite`, `typescript-better-sqlite3`, `typescript-node-sqlite`, `typescript-wasm-sqlite`, `typescript-kysely`, `javascript-better-sqlite3`, `javascript-node-sqlite`, `javascript-wasm-sqlite`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-microsoft-sqlite`, `elixir-exqlite`, `ruby-sqlite3`, `php-pdo`

### DuckDB

`python-duckdb`, `typescript-duckdb`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`

### CockroachDB

`rust-sqlx`, `python-psycopg3`, `typescript-pg`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`, `csharp-npgsql`, `ruby-pg`, `php-pdo`, `php-amphp`, `elixir-postgrex`

### MSSQL

`rust-tiberius`, `python-pyodbc`, `typescript-mssql`, `typescript-kysely`, `go-database-sql`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `csharp-sqlclient`, `ruby-tiny-tds`, `php-pdo`, `elixir-tds`

### Oracle

`rust-sibyl`, `python-oracledb`, `typescript-oracledb`, `go-godror`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `csharp-oracle`, `ruby-oci8`, `elixir-jamdb`

`php-pdo` does not support Oracle.

### MariaDB

`rust-sqlx`, `python-aiomysql`, `typescript-mysql2`, `typescript-kysely`, `javascript-mysql2`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-mysqlconnector`, `elixir-myxql`, `ruby-mysql2`, `ruby-trilogy`, `php-pdo`, `php-amphp`

### Redshift

`rust-sqlx`, `rust-tokio-postgres`, `python-psycopg3`, `python-asyncpg`, `typescript-pg`, `typescript-postgres`, `typescript-kysely`, `javascript-pg`, `javascript-postgres`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`, `csharp-npgsql`, `elixir-postgrex`, `ruby-pg`, `php-pdo`

### Snowflake

`python-snowflake`, `typescript-snowflake`, `javascript-snowflake`, `go-gosnowflake`, `java-jdbc`, `kotlin-jdbc`, `csharp-snowflake`, `php-pdo`

## Row Type Options

| Language | Backend | row_type Values |
|----------|---------|----------------|
| Python | all Python backends | `dataclass` (default), `pydantic`, `msgspec` |
| TypeScript | all TS backends | `interface` (default), `zod` |
| JavaScript | all `javascript-*` backends | JSDoc `@typedef` only -- `zod` is rejected |

```toml
[[sql.gen]]
backend = "python-psycopg3"
output = "src/generated"
row_type = "pydantic"
```

## structs_only Option

`structs_only = "true"` emits only row types (interfaces/Zod schemas, enums, composites) and suppresses query functions and the driver import. Supported by `rust-sqlx` and every TypeScript backend. Combine with `row_type = "zod"` for a types-only package with no driver dependency:

```toml
[[sql.gen]]
backend = "typescript-pg"
output = "src/generated/types"
row_type = "zod"
structs_only = "true"
```

## Type Resolution

```text
SQL type  -->  neutral type  -->  language type
SERIAL         int32              i32 (Rust) / int (Python) / number (TS)
TIMESTAMPTZ    datetime_tz        chrono::DateTime<Utc> / datetime / Date
TEXT[]         array<string>      Vec<String> / list[str] / string[]
user_status    enum::user_status  UserStatus (all languages)
```
