# Backends Reference

Scythe provides 34 backends across 10 languages and 5 database engines.

## Language Coverage

| Language | PostgreSQL | MySQL | SQLite | DuckDB | CockroachDB |
|----------|-----------|-------|--------|--------|-------------|
| Rust | sqlx, tokio-postgres | sqlx | sqlx | duckdb-rs | sqlx |
| Python | psycopg3, asyncpg | aiomysql | aiosqlite | duckdb | psycopg3 |
| TypeScript | postgres.js, pg | mysql2 | better-sqlite3 | duckdb-node | -- |
| Go | pgx | database/sql | database/sql | -- | pgx |
| Java | JDBC, R2DBC | JDBC | JDBC | -- | JDBC |
| Kotlin | JDBC, R2DBC, Exposed | JDBC | JDBC | -- | JDBC |
| C# | Npgsql | MySqlConnector | Microsoft.Data.Sqlite | -- | -- |
| Elixir | Postgrex | MyXQL | Exqlite | -- | -- |
| Ruby | pg | mysql2 | sqlite3 | -- | -- |
| PHP | PDO | PDO | PDO | -- | -- |

## Backend Names

Use these exact names in `[[sql.gen]] backend = "..."`:

### PostgreSQL

`rust-sqlx`, `rust-tokio-postgres`, `python-psycopg3`, `python-asyncpg`, `typescript-postgres`, `typescript-pg`, `go-pgx`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `kotlin-exposed`, `csharp-npgsql`, `elixir-postgrex`, `ruby-pg`, `php-pdo`

### MySQL

`rust-sqlx`, `python-aiomysql`, `typescript-mysql2`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-mysqlconnector`, `elixir-myxql`, `ruby-mysql2`, `php-pdo`

### SQLite

`rust-sqlx`, `python-aiosqlite`, `typescript-better-sqlite3`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-microsoft-sqlite`, `elixir-exqlite`, `ruby-sqlite3`, `php-pdo`

### DuckDB

`python-duckdb`, `rust-duckdb`, `typescript-duckdb`

### CockroachDB

`rust-sqlx`, `python-psycopg3`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`

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

## Type Resolution

```text
SQL type  -->  neutral type  -->  language type
SERIAL         int32              i32 (Rust) / int (Python) / number (TS)
TIMESTAMPTZ    datetime_tz        chrono::DateTime<Utc> / datetime / Date
TEXT[]         array<string>      Vec<String> / list[str] / string[]
user_status    enum::user_status  UserStatus (all languages)
```
