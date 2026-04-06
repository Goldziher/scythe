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

| Backend | Language | Library | Manifest |
|---------|----------|---------|----------|
| `rust-sqlx` | Rust | sqlx | `rust-sqlx.toml` |
| `rust-tokio-postgres` | Rust | tokio-postgres | `rust-tokio-postgres.toml` |
| `python-psycopg3` | Python | psycopg3 | `python-psycopg3.toml` |
| `python-asyncpg` | Python | asyncpg | `python-asyncpg.toml` |
| `typescript-postgres` | TypeScript | postgres.js | `typescript-postgres.toml` |
| `typescript-pg` | TypeScript | pg (node-postgres) | `typescript-pg.toml` |
| `go-pgx` | Go | pgx v5 | `go-pgx.toml` |
| `java-jdbc` | Java | JDBC | `java-jdbc.toml` |
| `kotlin-jdbc` | Kotlin | JDBC | `kotlin-jdbc.toml` |
| `csharp-npgsql` | C# | Npgsql | `csharp-npgsql.toml` |
| `elixir-postgrex` | Elixir | Postgrex | `elixir-postgrex.toml` |
| `ruby-pg` | Ruby | pg gem | `ruby-pg.toml` |
| `php-pdo` | PHP | PDO | `php-pdo.toml` |

## Adding a new backend

1. Create a manifest TOML with scalar/container type mappings.
2. Add Jinja2 templates for row structs, query functions, enums, and composites.
3. Implement the `CodegenBackend` trait.
4. Register the backend in the codegen module.

The `CodegenBackend` trait:

```rust
pub trait CodegenBackend: Send + Sync {
    fn name(&self) -> &str;
    fn generate_row_struct(&self, query_name: &str, columns: &[ResolvedColumn]) -> Result<String, ScytheError>;
    fn generate_query_fn(&self, analyzed: &AnalyzedQuery, struct_name: &str, columns: &[ResolvedColumn], params: &[ResolvedParam]) -> Result<String, ScytheError>;
    fn generate_enum_def(&self, enum_info: &EnumInfo) -> Result<String, ScytheError>;
    fn generate_composite_def(&self, composite: &CompositeInfo) -> Result<String, ScytheError>;
    fn file_header(&self) -> String;
}
```
