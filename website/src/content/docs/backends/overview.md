---
title: Backend Architecture
description: How scythe backends are structured -- manifests, templates, and the CodegenBackend trait.
---

Scythe generates type-safe code from SQL queries. Each backend is defined by:

1. **Manifest** (`manifest.toml`) -- declares the language, type mappings, naming conventions, and import rules. Manifests are compiled into the `scythe-codegen` binary from `crates/scythe-codegen/manifests/`; manifest selection is a pure function of `(backend, engine)`, with no filesystem lookup at generation time.
2. **Rust trait** (`CodegenBackend`) -- implements `generate_row_struct`, `generate_query_fn`, `generate_enum_def`, etc., building output strings directly with `std::fmt::Write` rather than rendering templates.
3. **Manifest-driven type resolution** -- the manifest's `[types.scalars]` and `[types.containers]` tables drive how neutral types map to language-native types within the trait implementation.

## Manifest structure

```toml
[backend]
name = "rust-sqlx"
language = "rust"
file_extension = "rs"
engine = "postgresql"

[types.scalars]
int32 = "i32"
string = "String"
datetime_tz = "chrono::DateTime<chrono::Utc>"

[types.containers]
array = "Vec<{T}>"
nullable = "Option<{T}>"
range = "sqlx::postgres::types::PgRange<{T}>"
json_typed = "sqlx::types::Json<{T}>"

[naming]
struct_case = "PascalCase"
fn_case = "snake_case"
enum_variant_case = "PascalCase"
row_suffix = "Row"

[imports.rules]
"chrono::" = "use chrono;"
"uuid::Uuid" = "use uuid::Uuid;"
```

## Type resolution pipeline

```text
SQL type  -->  neutral type  -->  language type
────────       ────────────       ─────────────
SERIAL         int32              i32
TIMESTAMPTZ    datetime_tz        chrono::DateTime<chrono::Utc>
TEXT[]         array<string>      Vec<String>
user_status    enum::user_status  UserStatus
```

Neutral types are the bridge. The analyzer converts SQL types to neutral types; the backend manifest maps neutral types to language types. See [Neutral Types](/scythe/reference/neutral-types/) for the full mapping table.

## Provenance header

Every generated file carries a one-line provenance comment, emitted right after
[`file_preamble`](#adding-a-new-backend) and before `file_header`
(`crates/scythe-codegen/src/provenance.rs`):

```text
// scythe:provenance v=0.17.0 backend=csharp-npgsql engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
```

The comment token is derived from the backend's `language`: `#` for Python, Ruby, and Elixir; `//` for
everything else. Python's header line also carries a trailing `# noqa: E501` suffix, preceded by the
two spaces ruff requires before an inline comment, since the line routinely exceeds ruff's default
88-character limit. For PHP, the header follows the `<?php` preamble
rather than preceding it, since nothing may come before that tag.

## Supported backends

Scythe provides 62 selectable backend names across 10 languages (plus plain JavaScript, emitted by the TypeScript backends' JSDoc mode) and 10 database engines, implemented by 52 `CodegenBackend` types: the ten `javascript-*` names are a JSDoc emit mode on the matching TypeScript backend rather than separate implementations (see [JavaScript output](/scythe/backends/typescript/#javascript-output-jsdoc)). Some backends (like `java-jdbc`) support multiple engines via engine-specific manifests loaded at runtime.

### PostgreSQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `rust-tokio-postgres` | Rust | tokio-postgres |
| `python-psycopg3` | Python | psycopg3 |
| `python-asyncpg` | Python | asyncpg |
| `typescript-postgres` | TypeScript | postgres.js |
| `typescript-pg` | TypeScript | pg (node-postgres) |
| `typescript-kysely` | TypeScript | Kysely (any dialect) |
| `javascript-postgres` | JavaScript | postgres.js (JSDoc types) |
| `javascript-pg` | JavaScript | pg (node-postgres, JSDoc types) |
| `go-pgx` | Go | pgx v5 |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-npgsql` | C# | Npgsql |
| `elixir-postgrex` | Elixir | Postgrex |
| `ruby-pg` | Ruby | pg gem |
| `php-pdo` | PHP | PDO |
| `php-amphp` | PHP | AMPHP SQL |
| `java-r2dbc` | Java | R2DBC (Project Reactor) |
| `kotlin-r2dbc` | Kotlin | R2DBC (coroutines) |
| `kotlin-exposed` | Kotlin | Exposed |
| `elixir-ecto` | Elixir | Ecto (via `Postgrex.query/3`, not `Ecto.Repo`) |

### MySQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-aiomysql` | Python | aiomysql |
| `typescript-mysql2` | TypeScript | mysql2 |
| `typescript-kysely` | TypeScript | Kysely (any dialect) |
| `javascript-mysql2` | JavaScript | mysql2 (JSDoc types) |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `java-r2dbc` | Java | R2DBC (Project Reactor) |
| `kotlin-jdbc` | Kotlin | JDBC |
| `kotlin-r2dbc` | Kotlin | R2DBC (coroutines) |
| `csharp-mysqlconnector` | C# | MySqlConnector |
| `elixir-myxql` | Elixir | MyXQL |
| `ruby-mysql2` | Ruby | mysql2 gem |
| `ruby-trilogy` | Ruby | Trilogy |
| `php-pdo` | PHP | PDO |
| `php-amphp` | PHP | AMPHP SQL |

### SQLite

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-aiosqlite` | Python | aiosqlite |
| `typescript-better-sqlite3` | TypeScript | better-sqlite3 |
| `typescript-kysely` | TypeScript | Kysely (any SQLite dialect, incl. third-party ones) |
| `typescript-node-sqlite` | TypeScript | node:sqlite (synchronous) |
| `typescript-wasm-sqlite` | TypeScript | @sqlite.org/sqlite-wasm (synchronous) |
| `javascript-better-sqlite3` | JavaScript | better-sqlite3 (JSDoc types, synchronous) |
| `javascript-node-sqlite` | JavaScript | node:sqlite (JSDoc types, synchronous) |
| `javascript-wasm-sqlite` | JavaScript | @sqlite.org/sqlite-wasm (JSDoc types, synchronous) |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `java-r2dbc` | Java | R2DBC (Project Reactor) |
| `kotlin-jdbc` | Kotlin | JDBC |
| `kotlin-r2dbc` | Kotlin | R2DBC (coroutines) |
| `csharp-microsoft-sqlite` | C# | Microsoft.Data.Sqlite |
| `elixir-exqlite` | Elixir | Exqlite |
| `ruby-sqlite3` | Ruby | sqlite3 gem |
| `php-pdo` | PHP | PDO |

### DuckDB

| Backend | Language | Library |
|---------|----------|---------|
| `python-duckdb` | Python | duckdb |
| `typescript-duckdb` | TypeScript | duckdb-node |
| `javascript-duckdb` | JavaScript | duckdb-node (JSDoc types) |
| `go-database-sql` | Go | database/sql (DuckDB driver) |
| `java-jdbc` | Java | JDBC (DuckDB JDBC driver) |
| `kotlin-jdbc` | Kotlin | JDBC (DuckDB JDBC driver) |

There is no `rust-duckdb` backend.

### CockroachDB

CockroachDB is wire-compatible with PostgreSQL. Every backend listed in the [PostgreSQL](#postgresql)
table above supports CockroachDB: `normalize_engine` folds `engine = "cockroachdb"` (and `crdb`) into
`postgresql` before a backend's engine support is ever consulted
(`crates/scythe-codegen/src/backends/mod.rs`), so backend construction and manifest resolution are
identical to the PostgreSQL path. The CockroachDB column is therefore identical to the PostgreSQL
column by construction, not by coincidence.

Redshift does not fold that way. It stays its own engine and each backend that supports it needs a
per-backend `*.redshift.toml` manifest, which is why its column is narrower — see
[Redshift](/scythe/databases/redshift/).

### MSSQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-tiberius` | Rust | tiberius |
| `python-pyodbc` | Python | pyodbc |
| `typescript-mssql` | TypeScript | mssql (tedious) |
| `javascript-mssql` | JavaScript | mssql (tedious, JSDoc types) |
| `typescript-kysely` | TypeScript | Kysely (any dialect) |
| `go-database-sql` | Go | database/sql (MSSQL driver) |
| `java-jdbc` | Java | JDBC (Microsoft JDBC Driver) |
| `kotlin-jdbc` | Kotlin | JDBC (Microsoft JDBC Driver) |
| `csharp-sqlclient` | C# | Microsoft.Data.SqlClient |
| `ruby-tiny-tds` | Ruby | tiny_tds |
| `php-pdo` | PHP | PDO (sqlsrv driver) |
| `elixir-tds` | Elixir | tds |

`java-r2dbc` and `kotlin-r2dbc` do not support MSSQL — their `new()` only accepts `postgresql`,
`mysql`, `mariadb`, and `sqlite` (`crates/scythe-codegen/src/backends/java_r2dbc.rs:28-40`,
`kotlin_r2dbc.rs:28-40`). No MSSQL manifest ships for either backend.

### Oracle

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sibyl` | Rust | sibyl |
| `python-oracledb` | Python | oracledb |
| `typescript-oracledb` | TypeScript | oracledb (node-oracledb) |
| `javascript-oracledb` | JavaScript | oracledb (node-oracledb, JSDoc types) |
| `go-godror` | Go | godror |
| `java-jdbc` | Java | JDBC (Oracle JDBC / ojdbc) |
| `kotlin-jdbc` | Kotlin | JDBC (Oracle JDBC / ojdbc) |
| `csharp-oracle` | C# | ODP.NET |
| `ruby-oci8` | Ruby | ruby-oci8 |
| `elixir-jamdb` | Elixir | jamdb_oracle (alias: `jamdb`) |

`java-r2dbc` and `kotlin-r2dbc` do not support Oracle, for the same reason as MSSQL above. No Oracle
manifest ships for either backend.
`php-pdo`'s `supported_engines` has no `oracle` entry either — it hard-errors for this engine, despite
a `php-pdo.oracle.toml` manifest shipping in the tree.

### MariaDB

MariaDB uses MySQL drivers with MariaDB-specific type resolution:

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx (MySQL driver) |
| `python-aiomysql` | Python | aiomysql |
| `typescript-mysql2` | TypeScript | mysql2 |
| `typescript-kysely` | TypeScript | Kysely (any dialect) |
| `javascript-mysql2` | JavaScript | mysql2 (JSDoc types) |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC (MariaDB Connector/J) |
| `java-r2dbc` | Java | R2DBC (Project Reactor) |
| `kotlin-jdbc` | Kotlin | JDBC (MariaDB Connector/J) |
| `kotlin-r2dbc` | Kotlin | R2DBC (coroutines) |
| `csharp-mysqlconnector` | C# | MySqlConnector |
| `elixir-myxql` | Elixir | MyXQL |
| `ruby-mysql2` | Ruby | mysql2 |
| `ruby-trilogy` | Ruby | Trilogy |
| `php-pdo` | PHP | PDO |
| `php-amphp` | PHP | AMPHP SQL |

### Redshift

Redshift uses PostgreSQL backends with Redshift-specific type resolution:

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx (PostgreSQL driver) |
| `rust-tokio-postgres` | Rust | tokio-postgres |
| `python-psycopg3` | Python | psycopg3 |
| `python-asyncpg` | Python | asyncpg |
| `typescript-pg` | TypeScript | pg |
| `typescript-postgres` | TypeScript | postgres.js |
| `typescript-kysely` | TypeScript | Kysely (any dialect) |
| `javascript-pg` | JavaScript | pg (JSDoc types) |
| `javascript-postgres` | JavaScript | postgres.js (JSDoc types) |
| `go-pgx` | Go | pgx v5 |
| `java-jdbc` | Java | JDBC (PostgreSQL driver) |
| `kotlin-jdbc` | Kotlin | JDBC (PostgreSQL driver) |
| `csharp-npgsql` | C# | Npgsql |
| `elixir-postgrex` | Elixir | Postgrex |
| `ruby-pg` | Ruby | pg |
| `php-pdo` | PHP | PDO |

### Snowflake

| Backend | Language | Library |
|---------|----------|---------|
| `python-snowflake` | Python | snowflake-connector-python |
| `typescript-snowflake` | TypeScript | snowflake-sdk |
| `javascript-snowflake` | JavaScript | snowflake-sdk (JSDoc types) |
| `go-gosnowflake` | Go | gosnowflake |
| `java-jdbc` | Java | JDBC (Snowflake JDBC driver) |
| `kotlin-jdbc` | Kotlin | JDBC (Snowflake JDBC driver) |
| `csharp-snowflake` | C# | Snowflake.Data |
| `php-pdo` | PHP | PDO (Snowflake PDO driver) |

### Language coverage summary

| Language | PostgreSQL | MySQL | SQLite | DuckDB | CockroachDB | MSSQL | Oracle | MariaDB | Redshift | Snowflake |
|----------|-----------|-------|--------|--------|-------------|-------|--------|---------|----------|-----------|
| Rust | sqlx, tokio-postgres | sqlx | sqlx | -- | sqlx, tokio-postgres | tiberius | sibyl | sqlx | sqlx, tokio-postgres | -- |
| Python | psycopg3, asyncpg | aiomysql | aiosqlite | duckdb | psycopg3, asyncpg | pyodbc | oracledb | aiomysql | psycopg3, asyncpg | snowflake-connector |
| TypeScript | postgres.js, pg, Kysely | mysql2, Kysely | better-sqlite3, Kysely, node:sqlite, wasm-sqlite | duckdb-node | postgres.js, pg, Kysely | mssql, Kysely | oracledb | mysql2, Kysely | postgres.js, pg, Kysely | snowflake-sdk |
| JavaScript | postgres.js, pg | mysql2 | better-sqlite3, node:sqlite, wasm-sqlite | duckdb-node | postgres.js, pg | mssql | oracledb | mysql2 | postgres.js, pg | snowflake-sdk |
| Go | pgx | database/sql | database/sql | database/sql | pgx | database/sql | godror | database/sql | pgx | gosnowflake |
| Java | JDBC, R2DBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC, R2DBC | JDBC | JDBC | JDBC, R2DBC | JDBC | JDBC |
| Kotlin | JDBC, R2DBC, Exposed | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC, R2DBC, Exposed | JDBC | JDBC | JDBC, R2DBC | JDBC | JDBC |
| C# | Npgsql | MySqlConnector | Microsoft.Data.Sqlite | -- | Npgsql | Microsoft.Data.SqlClient | ODP.NET | MySqlConnector | Npgsql | Snowflake.Data |
| Elixir | Postgrex, Ecto | MyXQL | Exqlite | -- | Postgrex, Ecto | tds | jamdb_oracle | MyXQL | Postgrex | -- |
| Ruby | pg | mysql2, trilogy | sqlite3 | -- | pg | tiny_tds | ruby-oci8 | mysql2, trilogy | pg | -- |
| PHP | PDO, AMPHP | PDO, AMPHP | PDO | -- | PDO, AMPHP | PDO | -- | PDO, AMPHP | PDO | PDO |

`java-r2dbc` and `kotlin-r2dbc` only cover PostgreSQL/CockroachDB, MySQL, MariaDB, and SQLite — not
MSSQL, Oracle, Redshift, or Snowflake. `elixir-ecto` and `kotlin-exposed` cover PostgreSQL/CockroachDB
only. `php-amphp` covers PostgreSQL/CockroachDB, MySQL, and MariaDB only. `php-pdo` has no Oracle
support despite a manifest existing for it. The JavaScript row is the `javascript-*` JSDoc emit mode
of the ten TypeScript backends it names -- `typescript-kysely` is the only TypeScript backend with no
JavaScript counterpart.

## Adding a new backend

1. Create a manifest TOML with scalar/container type mappings.
2. Implement the `CodegenBackend` trait, building output with `std::fmt::Write` (there is no template
   engine in the workspace).
3. Register the backend in `get_backend` (`crates/scythe-codegen/src/backends/mod.rs`).

The `CodegenBackend` trait (`crates/scythe-codegen/src/backend_trait.rs`):

```rust
pub trait CodegenBackend: Send + Sync {
    fn name(&self) -> &str;
    fn manifest(&self) -> &BackendManifest;
    fn manifest_mut(&mut self) -> &mut BackendManifest;
    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError>;
    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError>;
    fn generate_query_fn(&self, analyzed: &AnalyzedQuery, struct_name: &str, columns: &[ResolvedColumn], params: &[ResolvedParam]) -> Result<String, ScytheError>;
    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError>;
    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError>;
    fn file_preamble(&self) -> String { String::new() }
    fn file_header(&self) -> String { String::new() }
    fn file_header_for_results(&self, generated: &[GeneratedCode]) -> String { self.file_header() }
    fn file_footer(&self) -> String { String::new() }
    fn query_class_header(&self) -> String { String::new() }
    fn post_footer(&self) -> String { String::new() }
    fn generate_rbs_file(&self, context: &RbsGenerationContext) -> Option<String> { None }
    fn generate_grouped_structs(&self, parent_struct_name: &str, child_struct_name: &str, parent_columns: &[ResolvedColumn], child_columns: &[ResolvedColumn], key_column: &str) -> Result<String, ScytheError>;
    fn generate_grouped_query_fn(&self, request: &GroupedQueryFn<'_>) -> Result<String, ScytheError>;
    fn apply_options(&mut self, options: &std::collections::HashMap<String, String>) -> Result<(), ScytheError> {
        crate::backend_options::reject_unknown_options(&[], options)
    }
    fn supported_engines(&self) -> &[&str] { &["postgresql"] }
}
```

The default `apply_options` rejects every key (an empty known-key list) — the correct behavior for
backends that take no options, and what a new backend gets for free if its author forgets to override
it. `apply_options` is the entire `[[sql.gen]]` option surface (`row_type`, `field_case`, `namespace`,
`structs_only`, `serde`, `derive`, `extension_functions`, and so on) — every backend that accepts
options overrides it. Methods shown with a default body above are optional to override.
`generate_grouped_structs` / `generate_grouped_query_fn` default to an error
("grouped queries are not yet supported by the '\<name\>' backend") rather than an empty string;
backends opt into `:grouped` query support by overriding both.
