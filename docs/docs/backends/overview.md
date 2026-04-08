# Backend Architecture

Scythe generates type-safe code from SQL queries. Each backend is defined by:

1. **Manifest** (`manifest.toml`) -- declares the language, type mappings, naming conventions, and import rules.
2. **Templates** (Jinja2) -- render row structs, query functions, enums, and composites.
3. **Rust trait** (`CodegenBackend`) -- implements `generate_row_struct`, `generate_query_fn`, `generate_enum_def`, etc.

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
field_case = "snake_case"
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

Neutral types are the bridge. The analyzer converts SQL types to neutral types; the backend manifest maps neutral types to language types. See [Neutral Types](../reference/neutral-types.md) for the full mapping table.

## Supported backends

Scythe provides 31 backends across 10 languages and 5 database engines. Some backends (like `java-jdbc`) support multiple engines via engine-specific manifests loaded at runtime.

### PostgreSQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `rust-tokio-postgres` | Rust | tokio-postgres |
| `python-psycopg3` | Python | psycopg3 |
| `python-asyncpg` | Python | asyncpg |
| `typescript-postgres` | TypeScript | postgres.js |
| `typescript-pg` | TypeScript | pg (node-postgres) |
| `go-pgx` | Go | pgx v5 |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-npgsql` | C# | Npgsql |
| `elixir-postgrex` | Elixir | Postgrex |
| `ruby-pg` | Ruby | pg gem |
| `php-pdo` | PHP | PDO |
| `java-r2dbc` | Java | R2DBC (Project Reactor) |
| `kotlin-r2dbc` | Kotlin | R2DBC (coroutines) |
| `kotlin-exposed` | Kotlin | Exposed |

### MySQL

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-aiomysql` | Python | aiomysql |
| `typescript-mysql2` | TypeScript | mysql2 |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-mysqlconnector` | C# | MySqlConnector |
| `elixir-myxql` | Elixir | MyXQL |
| `ruby-mysql2` | Ruby | mysql2 gem |
| `php-pdo` | PHP | PDO |

### SQLite

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-aiosqlite` | Python | aiosqlite |
| `typescript-better-sqlite3` | TypeScript | better-sqlite3 |
| `go-database-sql` | Go | database/sql |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-microsoft-sqlite` | C# | Microsoft.Data.Sqlite |
| `elixir-exqlite` | Elixir | Exqlite |
| `ruby-sqlite3` | Ruby | sqlite3 gem |
| `php-pdo` | PHP | PDO |

### DuckDB

| Backend | Language | Library |
|---------|----------|---------|
| `python-duckdb` | Python | duckdb |
| `rust-duckdb` | Rust | duckdb-rs |
| `typescript-duckdb` | TypeScript | duckdb-node |

### CockroachDB

CockroachDB is wire-compatible with PostgreSQL. The following PostgreSQL backends support CockroachDB when the engine is set to `cockroachdb`:

| Backend | Language | Library |
|---------|----------|---------|
| `rust-sqlx` | Rust | sqlx |
| `python-psycopg3` | Python | psycopg3 |
| `go-pgx` | Go | pgx v5 |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |

### Language coverage summary

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

## Adding a new backend

1. Create a manifest TOML with scalar/container type mappings.
2. Add Jinja2 templates for row structs, query functions, enums, and composites.
3. Implement the `CodegenBackend` trait.
4. Register the backend in the codegen module.

The `CodegenBackend` trait:

```rust
pub trait CodegenBackend: Send + Sync {
    fn name(&self) -> &str;
    fn manifest(&self) -> &BackendManifest;
    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError>;
    fn generate_model_struct(&self, table_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError>;
    fn generate_query_fn(&self, analyzed: &AnalyzedQuery, struct_name: &str, columns: &[ResolvedColumn], params: &[ResolvedParam]) -> Result<String, ScytheError>;
    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError>;
    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError>;
    fn file_header(&self) -> String;
    fn file_footer(&self) -> String;
    fn supported_engines(&self) -> &[&str];
}
```
