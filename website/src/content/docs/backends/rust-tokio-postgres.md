---
title: Rust + tokio-postgres
description: The rust-tokio-postgres backend -- generated code, differences from sqlx, and type mappings.
---

Backend: `rust-tokio-postgres` | Library: [tokio-postgres](https://docs.rs/tokio-postgres) | Engines: PostgreSQL, Redshift

Accepts two undocumented-elsewhere options: `serde` (`true`/`false`, default `false`) adds
`serde::Serialize, serde::Deserialize` to every generated struct and enum derive list, and `derive`
(a comma-separated list, e.g. `derive = "PartialEq, Hash"`) appends arbitrary extra derives
(`crates/scythe-codegen/src/backends/tokio_postgres.rs:22-24,51-74,101-109`).

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

Every generated file starts with a provenance header, then the same
`#![allow(dead_code, unused_imports, ...)]` line as `rust-sqlx`
(`integration_tests/rust-tokio-postgres/src/queries.rs:1-2`):

```rust
// scythe:provenance v=0.14.0 backend=rust-tokio-postgres engine=postgresql schema=sch1:... queries=q1:...
#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]
```

### Row struct with manual `from_row()`

Rows derive `Debug, Clone` (not bare `Debug`), and `from_row` is a **public** associated function, not
private (`crates/scythe-codegen/src/backends/tokio_postgres.rs:51-61,111-120`):

```rust
#[derive(Debug, Clone)]
pub struct GetUserRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl GetUserRow {
    pub fn from_row(row: &tokio_postgres::Row) -> Self {
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

The client parameter is `&(impl tokio_postgres::GenericClient + Sync)`, not `&tokio_postgres::Client`
-- this lets callers pass either a bare `Client` or a `Transaction`
(`crates/scythe-codegen/src/backends/tokio_postgres.rs:77`):

```rust
pub async fn get_user(
    client: &(impl tokio_postgres::GenericClient + Sync),
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
#[derive(Debug, Clone)]
pub struct ListUsersRow {
    pub id: i32,
    pub name: String,
}

impl ListUsersRow {
    pub fn from_row(row: &tokio_postgres::Row) -> Self {
        Self {
            id: row.get("id"),
            name: row.get("name"),
        }
    }
}

pub async fn list_users(
    client: &(impl tokio_postgres::GenericClient + Sync),
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
    client: &(impl tokio_postgres::GenericClient + Sync),
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

### Enum derives: `Display` and `FromStr`

Unlike `rust-sqlx` (a single `#[derive(sqlx::Type)]`), `rust-tokio-postgres` enums also get manual
`std::fmt::Display` and `std::str::FromStr` implementations, alongside the `FromSql`/`ToSql` impls
(`integration_tests/rust-tokio-postgres/src/queries.rs:4-45`):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Inactive,
    Banned,
}

impl std::fmt::Display for UserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserStatus::Active => write!(f, "active"),
            UserStatus::Inactive => write!(f, "inactive"),
            UserStatus::Banned => write!(f, "banned"),
        }
    }
}

impl std::str::FromStr for UserStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(UserStatus::Active),
            "inactive" => Ok(UserStatus::Inactive),
            "banned" => Ok(UserStatus::Banned),
            _ => Err(format!("unknown variant: {}", s)),
        }
    }
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
