# Rust + sqlx

Backend: `rust-sqlx` | Library: [sqlx](https://github.com/launchbadge/sqlx) | Engine: PostgreSQL

## SQL input

```sql
-- @name GetUser
-- @returns :one
SELECT id, name, email, created_at FROM users WHERE id = $1;

-- @name ListUsers
-- @returns :many
SELECT id, name FROM users ORDER BY name LIMIT $1;

-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, email) VALUES ($1, $2);
```

Schema:

```sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

## Generated code

### Row struct (`:one` / `:many`)

```rust
#[derive(Debug, sqlx::FromRow)]
pub struct GetUserRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

`email` is `Option<String>` because the column is nullable. `created_at` maps `TIMESTAMPTZ` to `chrono::DateTime<chrono::Utc>`.

### `:one` query function

```rust
pub async fn get_user(
    pool: &sqlx::PgPool,
    id: i32,
) -> Result<GetUserRow, sqlx::Error> {
    sqlx::query_as!(
        GetUserRow,
        "SELECT id, name, email, created_at FROM users WHERE id = $1",
        id
    )
    .fetch_one(pool)
    .await
}
```

### `:many` query function

```rust
#[derive(Debug, sqlx::FromRow)]
pub struct ListUsersRow {
    pub id: i32,
    pub name: String,
}

pub async fn list_users(
    pool: &sqlx::PgPool,
    limit: i64,
) -> Result<Vec<ListUsersRow>, sqlx::Error> {
    sqlx::query_as!(
        ListUsersRow,
        "SELECT id, name FROM users ORDER BY name LIMIT $1",
        limit
    )
    .fetch_all(pool)
    .await
}
```

### `:exec` query function

```rust
pub async fn create_user(
    pool: &sqlx::PgPool,
    name: &str,
    email: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO users (name, email) VALUES ($1, $2)",
        name,
        email
    )
    .execute(pool)
    .await?;
    Ok(())
}
```

## Enum generation

Given:

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

Generates:

```rust
#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "user_status", rename_all = "lowercase")]
pub enum UserStatus {
    Active,
    Inactive,
    Banned,
}
```

## Type mappings

| SQL Type | Neutral | Rust (sqlx) |
|----------|---------|-------------|
| `SERIAL` / `INTEGER` | `int32` | `i32` |
| `BIGSERIAL` / `BIGINT` | `int64` | `i64` |
| `SMALLINT` | `int16` | `i16` |
| `REAL` | `float32` | `f32` |
| `DOUBLE PRECISION` | `float64` | `f64` |
| `TEXT` / `VARCHAR` | `string` | `String` |
| `BOOLEAN` | `bool` | `bool` |
| `BYTEA` | `bytes` | `Vec<u8>` |
| `UUID` | `uuid` | `uuid::Uuid` |
| `NUMERIC` | `decimal` | `rust_decimal::Decimal` |
| `DATE` | `date` | `chrono::NaiveDate` |
| `TIME` | `time` | `chrono::NaiveTime` |
| `TIMESTAMPTZ` | `datetime_tz` | `chrono::DateTime<chrono::Utc>` |
| `TIMESTAMP` | `datetime` | `chrono::NaiveDateTime` |
| `INTERVAL` | `interval` | `sqlx::postgres::types::PgInterval` |
| `JSON` / `JSONB` | `json` | `serde_json::Value` |
| `INET` | `inet` | `ipnetwork::IpNetwork` |
| `TEXT[]` | `array<string>` | `Vec<String>` |
| `INT4RANGE` | `range<int32>` | `sqlx::postgres::types::PgRange<i32>` |
| nullable column | `nullable` | `Option<T>` |
