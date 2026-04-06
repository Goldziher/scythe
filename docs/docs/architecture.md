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
    Backend (manifest.toml + CodegenBackend trait)
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
    fn generate_row_struct(&self, ...) -> Result<String, ScytheError>;
    fn generate_model_struct(&self, ...) -> Result<String, ScytheError>;
    fn generate_query_fn(&self, ...) -> Result<String, ScytheError>;
    fn generate_enum_def(&self, ...) -> Result<String, ScytheError>;
    fn generate_composite_def(&self, ...) -> Result<String, ScytheError>;
    fn file_header(&self) -> String;
}
```

Each backend also has a `manifest.toml` that maps neutral types to language-specific types. No Rust code is needed to customize type mappings.

> **Note:** SQL parsing and type inference support PostgreSQL, MySQL, and SQLite. Code generation backends produce driver-specific code for each language, loading type mappings from their respective manifests.

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

| Backend | Language | Database Driver |
|---------|----------|-----------------|
| `rust-sqlx` | Rust | sqlx |
| `rust-tokio-postgres` | Rust | tokio-postgres |
| `python-asyncpg` | Python | asyncpg |
| `python-psycopg3` | Python | psycopg3 |
| `typescript-pg` | TypeScript | pg |
| `typescript-postgres` | TypeScript | postgres |
| `go-pgx` | Go | pgx |
| `java-jdbc` | Java | JDBC |
| `kotlin-jdbc` | Kotlin | JDBC |
| `csharp-npgsql` | C# | Npgsql |
| `elixir-postgrex` | Elixir | Postgrex |
| `ruby-pg` | Ruby | pg |
| `php-pdo` | PHP | PDO |
