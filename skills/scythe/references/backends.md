# Backends Reference

Scythe provides 52 backends across 10 languages and 10 database engines.

## Language Coverage

| Language | PostgreSQL | MySQL | SQLite | DuckDB | CockroachDB | MSSQL | Oracle | MariaDB | Redshift | Snowflake |
|----------|-----------|-------|--------|--------|-------------|-------|--------|---------|----------|-----------|
| Rust | sqlx, tokio-postgres | sqlx | sqlx | -- | sqlx | tiberius | sibyl | sqlx | sqlx | -- |
| Python | psycopg3, asyncpg | aiomysql | aiosqlite | duckdb | psycopg3 | pyodbc | oracledb | aiomysql | psycopg3 | snowflake-connector |
| TypeScript | postgres.js, pg, kysely | mysql2, kysely | better-sqlite3, node:sqlite, sqlite-wasm, kysely | duckdb-node | pg, kysely | mssql, kysely | oracledb | mysql2, kysely | pg, kysely | snowflake-sdk |
| Go | pgx | database/sql | database/sql | database/sql | pgx | go-mssqldb | godror | database/sql | pgx | gosnowflake |
| Java | JDBC, R2DBC | JDBC | JDBC | JDBC | JDBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC | JDBC |
| Kotlin | JDBC, R2DBC, Exposed | JDBC | JDBC | JDBC | JDBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC | JDBC |
| C# | Npgsql | MySqlConnector | Microsoft.Data.Sqlite | -- | Npgsql | Microsoft.Data.SqlClient | ODP.NET | MySqlConnector | Npgsql | Snowflake.Data |
| Elixir | Postgrex, Ecto | MyXQL | Exqlite | -- | Postgrex | tds | jamdb_oracle | MyXQL | Postgrex | -- |
| Ruby | pg, Trilogy | mysql2, Trilogy | sqlite3 | -- | pg | tiny_tds | ruby-oci8 | mysql2 | pg | -- |
| PHP | PDO, AMPHP | PDO | PDO | -- | PDO | PDO | -- | PDO | PDO | PDO |

`kysely` (backend `typescript-kysely`) is dialect-parameterised: it compiles to Kysely's `sql` tag, which renders whatever placeholder syntax the connected `Dialect` needs at runtime, so one generated call site runs against any Kysely dialect. Scythe pins and tests five dialects -- PostgreSQL, MySQL, SQLite, MSSQL, MariaDB -- plus a Redshift manifest that reuses the PostgreSQL dialect. Third-party dialects (libsql, PlanetScale, Cloudflare D1, Neon, PGlite, and `node:sqlite` / `@sqlite.org/sqlite-wasm` used as a Kysely dialect) are wire-compatible but not pinned or tested by scythe.

`typescript-node-sqlite` and `typescript-wasm-sqlite` generate synchronous code (plain `export function`, no `async`, no `Promise`), unlike every other TypeScript backend. `typescript-node-sqlite` requires `--experimental-sqlite` on Node 22 and is unflagged from Node 23.4 onward.

## Backend Names

Use these exact names in `[[sql.gen]] backend = "..."`:

### PostgreSQL

`rust-sqlx`, `rust-tokio-postgres`, `python-psycopg3`, `python-asyncpg`, `typescript-postgres`, `typescript-pg`, `typescript-kysely`, `go-pgx`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `kotlin-exposed`, `csharp-npgsql`, `elixir-postgrex`, `elixir-ecto`, `ruby-pg`, `php-pdo`

### MySQL

`rust-sqlx`, `python-aiomysql`, `typescript-mysql2`, `typescript-kysely`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-mysqlconnector`, `elixir-myxql`, `ruby-mysql2`, `php-pdo`

### SQLite

`rust-sqlx`, `python-aiosqlite`, `typescript-better-sqlite3`, `typescript-node-sqlite`, `typescript-wasm-sqlite`, `typescript-kysely`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-microsoft-sqlite`, `elixir-exqlite`, `ruby-sqlite3`, `php-pdo`

### DuckDB

`python-duckdb`, `typescript-duckdb`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`

### CockroachDB

`rust-sqlx`, `python-psycopg3`, `typescript-pg`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`, `csharp-npgsql`, `ruby-pg`, `php-pdo`, `elixir-postgrex`

### MSSQL

`rust-tiberius`, `python-pyodbc`, `typescript-mssql`, `typescript-kysely`, `go-database-sql`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `csharp-sqlclient`, `ruby-tiny-tds`, `php-pdo`, `elixir-tds`

### Oracle

`rust-sibyl`, `python-oracledb`, `typescript-oracledb`, `go-godror`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `csharp-oracle`, `ruby-oci8`, `elixir-jamdb`

`php-pdo` does not support Oracle.

### MariaDB

`rust-sqlx`, `python-aiomysql`, `typescript-mysql2`, `typescript-kysely`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-mysqlconnector`, `elixir-myxql`, `ruby-mysql2`, `php-pdo`

### Redshift

`rust-sqlx`, `rust-tokio-postgres`, `python-psycopg3`, `python-asyncpg`, `typescript-pg`, `typescript-postgres`, `typescript-kysely`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`, `csharp-npgsql`, `elixir-postgrex`, `ruby-pg`, `php-pdo`

### Snowflake

`python-snowflake`, `typescript-snowflake`, `go-gosnowflake`, `java-jdbc`, `kotlin-jdbc`, `csharp-snowflake`, `php-pdo`

## Row Type Options

| Language | Backend | row_type Values |
|----------|---------|----------------|
| Python | all Python backends | `dataclass` (default), `pydantic`, `msgspec` |
| TypeScript | all TS backends | `interface` (default), `zod` |

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
