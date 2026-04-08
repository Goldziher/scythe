# Architecture

## Pipeline

```text
SQL Schema + Annotated Queries
        |
        v
    Parse (sqlparser-rs)
        |
        v
    Build Catalog (tables, types, constraints)
        |
        v
    Analyze (type inference, nullability, parameters)
        |
        v
    Lint (22 custom rules + sqruff) + Format (sqruff)
        |
        v
    Backend (manifest.toml per (backend, engine) + CodegenBackend trait)
        |
        v
    Generated Code (Rust, Python, TypeScript, Go, ...)
```

## Workspace Crates

| Crate | Purpose |
|-------|---------|
| `scythe-core` | SQL parsing, catalog building, type inference, nullability analysis |
| `scythe-codegen` | Code generation via trait-based backends |
| `scythe-lint` | 22 custom rules + sqruff integration + engine |
| `scythe-backend` | Type resolution, naming conventions, MiniJinja rendering |
| `scythe-cli` | CLI binary with generate, check, lint, fmt, migrate commands |

## Language-Neutral Type System

The analyzer outputs a neutral type vocabulary. Each backend maps these to concrete language types via a manifest:

| Neutral Type | Rust | Python | TypeScript | Go |
|---|---|---|---|---|
| `int32` | `i32` | `int` | `number` | `int32` |
| `string` | `String` | `str` | `string` | `string` |
| `datetime_tz` | `chrono::DateTime<Utc>` | `datetime.datetime` | `Date` | `time.Time` |
| `uuid` | `uuid::Uuid` | `uuid.UUID` | `string` | `uuid.UUID` |
| nullable | `Option<T>` | `T \| None` | `T \| null` | `*T` |

## Backend Trait

Adding a new language requires implementing the `CodegenBackend` trait:

```rust
pub trait CodegenBackend: Send + Sync {
    fn name(&self) -> &str;
    fn manifest(&self) -> &BackendManifest;
    fn generate_row_struct(&self, ...) -> Result<String, ScytheError>;
    fn generate_model_struct(&self, ...) -> Result<String, ScytheError>;
    fn generate_query_fn(&self, ...) -> Result<String, ScytheError>;
    fn generate_enum_def(&self, ...) -> Result<String, ScytheError>;
    fn generate_composite_def(&self, ...) -> Result<String, ScytheError>;
    fn file_header(&self) -> String;
    fn file_footer(&self) -> String;
    fn supported_engines(&self) -> &[&str];
}
```

Each backend also has a `manifest.toml` that maps neutral types to language-specific types. No Rust code is needed to customize type mappings.

### Engine-aware manifests

Backends that support multiple database engines use a manifest-per-(backend, engine) strategy. For example, `java-jdbc` has three manifests:

- `java-jdbc.toml` (PostgreSQL, the default)
- `java-jdbc.mysql.toml` (MySQL-specific type mappings)
- `java-jdbc.sqlite.toml` (SQLite-specific type mappings)

When `get_backend("java-jdbc", "mysql")` is called, the engine-specific manifest is loaded automatically. This allows a single backend implementation to generate correct type mappings for each database engine without code duplication.

Backends that only support one engine (e.g. `rust-tokio-postgres` for PostgreSQL, `elixir-myxql` for MySQL) reject mismatched engines with a clear error via the `supported_engines()` method on the trait.

> **Note:** SQL parsing and type inference support PostgreSQL, MySQL, and SQLite. All 10 languages have backend coverage for every engine. Code generation backends produce driver-specific code for each language, loading type mappings from their respective engine-aware manifests.

### Example manifest.toml (rust-sqlx)

```toml
[backend]
name = "rust-sqlx"
language = "rust"
file_extension = "rs"
engine = "postgresql"

[types.scalars]
bool = "bool"
int32 = "i32"
int64 = "i64"
string = "String"
uuid = "uuid::Uuid"
datetime_tz = "chrono::DateTime<chrono::Utc>"
json = "serde_json::Value"

[types.containers]
array = "Vec<{T}>"
nullable = "Option<{T}>"

[naming]
struct_case = "PascalCase"
field_case = "snake_case"
fn_case = "snake_case"
row_suffix = "Row"
```

## Available Backends

25 backends across 10 languages and 3 database engines. See [Backend Overview](backends/overview.md) for the full list organized by engine.

| Language | PostgreSQL | MySQL | SQLite |
|----------|-----------|-------|--------|
| Rust | sqlx, tokio-postgres | sqlx | sqlx |
| Python | psycopg3, asyncpg | aiomysql | aiosqlite |
| TypeScript | postgres.js, pg | mysql2 | better-sqlite3 |
| Go | pgx | database/sql | database/sql |
| Java | JDBC | JDBC | JDBC |
| Kotlin | JDBC | JDBC | JDBC |
| C# | Npgsql | MySqlConnector | Microsoft.Data.Sqlite |
| Elixir | Postgrex | MyXQL | Exqlite |
| Ruby | pg | mysql2 | sqlite3 |
| PHP | PDO | PDO | PDO |
