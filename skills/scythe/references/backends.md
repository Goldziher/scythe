# Backends Reference

Scythe provides 70+ backends across 10 languages and 10 database engines.

## Language Coverage

| Language | PostgreSQL | MySQL | SQLite | DuckDB | CockroachDB | MSSQL | Oracle | MariaDB | Redshift | Snowflake |
|----------|-----------|-------|--------|--------|-------------|-------|--------|---------|----------|-----------|
| Rust | sqlx, tokio-postgres | sqlx | sqlx | duckdb-rs | sqlx | tiberius | sibyl | sqlx | sqlx | -- |
| Python | psycopg3, asyncpg | aiomysql | aiosqlite | duckdb | psycopg3 | pyodbc | oracledb | aiomysql | psycopg3 | snowflake-connector |
| TypeScript | postgres.js, pg | mysql2 | better-sqlite3 | duckdb-node | -- | mssql | oracledb | mysql2 | pg | snowflake-sdk |
| Go | pgx | database/sql | database/sql | -- | pgx | go-mssqldb | godror | database/sql | pgx | gosnowflake |
| Java | JDBC, R2DBC | JDBC | JDBC | -- | JDBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC | JDBC |
| Kotlin | JDBC, R2DBC, Exposed | JDBC | JDBC | -- | JDBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC | JDBC |
| C# | Npgsql | MySqlConnector | Microsoft.Data.Sqlite | -- | -- | Microsoft.Data.SqlClient | ODP.NET | MySqlConnector | Npgsql | Snowflake.Data |
| Elixir | Postgrex | MyXQL | Exqlite | -- | -- | tds | jamdb_oracle | MyXQL | Postgrex | -- |
| Ruby | pg | mysql2 | sqlite3 | -- | -- | tiny_tds | ruby-oci8 | mysql2 | pg | -- |
| PHP | PDO | PDO | PDO | -- | -- | PDO | PDO | PDO | PDO | PDO |

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

### MSSQL

`rust-tiberius`, `python-pyodbc`, `typescript-mssql`, `go-mssqldb`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `csharp-sqlclient`, `ruby-tiny-tds`, `php-pdo`, `elixir-tds`

### Oracle

`rust-sibyl`, `python-oracledb`, `typescript-oracledb`, `go-godror`, `java-jdbc`, `java-r2dbc`, `kotlin-jdbc`, `kotlin-r2dbc`, `csharp-odpnet`, `ruby-oci8`, `php-pdo`, `elixir-jamdb-oracle`

### MariaDB

`rust-sqlx`, `python-aiomysql`, `typescript-mysql2`, `go-database-sql`, `java-jdbc`, `kotlin-jdbc`, `csharp-mysqlconnector`, `elixir-myxql`, `ruby-mysql2`, `php-pdo`

### Redshift

`rust-sqlx`, `rust-tokio-postgres`, `python-psycopg3`, `python-asyncpg`, `typescript-pg`, `typescript-postgres`, `go-pgx`, `java-jdbc`, `kotlin-jdbc`, `csharp-npgsql`, `elixir-postgrex`, `ruby-pg`, `php-pdo`

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

## Type Resolution

```text
SQL type  -->  neutral type  -->  language type
SERIAL         int32              i32 (Rust) / int (Python) / number (TS)
TIMESTAMPTZ    datetime_tz        chrono::DateTime<Utc> / datetime / Date
TEXT[]         array<string>      Vec<String> / list[str] / string[]
user_status    enum::user_status  UserStatus (all languages)
```
