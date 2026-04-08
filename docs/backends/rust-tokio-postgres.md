# Rust + tokio-postgres

Backend: `rust-tokio-postgres` | Library: [tokio-postgres](https://docs.rs/tokio-postgres) | Engine: PostgreSQL

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

### Row struct with manual `from_row()`

```rust
#[derive(Debug)]
pub struct GetUserRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl GetUserRow {
    fn from_row(row: &tokio_postgres::Row) -> Self {
        Self {
            id: row.get("id"),
            name: row.get("name"),
            email: row.get("email"),
            created_at: row.get("created_at"),
        }
    }
}
```

### `:one` query function

```rust
pub async fn get_user(
    client: &tokio_postgres::Client,
    id: i32,
) -> Result<GetUserRow, tokio_postgres::Error> {
    let row = client
        .query_one(
            "SELECT id, name, email, created_at FROM users WHERE id = $1",
            &[&id],
        )
        .await?;
    Ok(GetUserRow::from_row(&row))
}
```

### `:many` query function

```rust
#[derive(Debug)]
pub struct ListUsersRow {
    pub id: i32,
    pub name: String,
}

impl ListUsersRow {
    fn from_row(row: &tokio_postgres::Row) -> Self {
        Self {
            id: row.get("id"),
            name: row.get("name"),
        }
    }
}

pub async fn list_users(
    client: &tokio_postgres::Client,
    limit: i64,
) -> Result<Vec<ListUsersRow>, tokio_postgres::Error> {
    let rows = client
        .query(
            "SELECT id, name FROM users ORDER BY name LIMIT $1",
            &[&limit],
        )
        .await?;
    Ok(rows.iter().map(ListUsersRow::from_row).collect())
}
```

### `:exec` query function

```rust
pub async fn create_user(
    client: &tokio_postgres::Client,
    name: &str,
    email: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    client
        .execute(
            "INSERT INTO users (name, email) VALUES ($1, $2)",
            &[&name, &email],
        )
        .await
}
```

## Key differences from sqlx

| Feature | sqlx | tokio-postgres |
|---------|------|----------------|
| Row mapping | `#[derive(sqlx::FromRow)]` | Manual `from_row()` |
| Query execution | `sqlx::query_as!()` macro | `client.query_one()` / `client.query()` |
| Compile-time checks | Yes (with `DATABASE_URL`) | No |
| Range types | `PgRange<T>` | `String` (serialized) |
| Enum types | `#[derive(sqlx::Type)]` | Manual `FromSql`/`ToSql` |
| INET | `ipnetwork::IpNetwork` | `std::net::IpAddr` |

## Type mappings

| SQL Type | Neutral | Rust (tokio-postgres) |
|----------|---------|----------------------|
| `SERIAL` / `INTEGER` | `int32` | `i32` |
| `BIGINT` | `int64` | `i64` |
| `TEXT` / `VARCHAR` | `string` | `String` |
| `BOOLEAN` | `bool` | `bool` |
| `UUID` | `uuid` | `uuid::Uuid` |
| `TIMESTAMPTZ` | `datetime_tz` | `chrono::DateTime<chrono::Utc>` |
| `JSON` / `JSONB` | `json` | `serde_json::Value` |
| `INET` | `inet` | `std::net::IpAddr` |
| `INTERVAL` | `interval` | `String` |
| `INT4RANGE` | `range<int32>` | `String` |
| nullable column | `nullable` | `Option<T>` |
