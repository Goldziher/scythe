---
title: Simple CRUD
description: A synthetic example with two tables covering all basic operations.
---

A synthetic example with two tables covering all basic operations.

## Schema

```sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    total NUMERIC(10,2) NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

## Queries

```sql
-- @name GetUser
-- @returns :one
SELECT id, name, email, created_at FROM users WHERE id = $1;

-- @name CreateUser
-- @returns :one
INSERT INTO users (name, email) VALUES ($1, $2) RETURNING id, name, email, created_at;

-- @name UpdateUserEmail
-- @returns :exec
UPDATE users SET email = $1 WHERE id = $2;

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = $1;

-- @name ListOrdersByUser
-- @returns :many
SELECT o.id, o.total, o.status, o.created_at, u.name AS user_name
FROM orders o
JOIN users u ON u.id = o.user_id
WHERE o.user_id = $1
ORDER BY o.created_at DESC;
```

## Generated code

### Rust (sqlx)

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn get_user(pool: &sqlx::PgPool, id: i32) -> Result<GetUserRow, sqlx::Error> {
    sqlx::query_as!(GetUserRow,
        "SELECT id, name, email, created_at FROM users WHERE id = $1", id)
        .fetch_one(pool).await
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CreateUserRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn create_user(pool: &sqlx::PgPool, name: &str, email: Option<&str>) -> Result<CreateUserRow, sqlx::Error> {
    sqlx::query_as!(CreateUserRow,
        "INSERT INTO users (name, email) VALUES ($1, $2) RETURNING id, name, email, created_at",
        name, email)
        .fetch_one(pool).await
}

pub async fn update_user_email(pool: &sqlx::PgPool, email: Option<&str>, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE users SET email = $1 WHERE id = $2", email, id)
        .execute(pool).await?;
    Ok(())
}

pub async fn delete_user(pool: &sqlx::PgPool, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM users WHERE id = $1", id)
        .execute(pool).await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ListOrdersByUserRow {
    pub id: i32,
    pub total: rust_decimal::Decimal,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub user_name: String,
}

pub async fn list_orders_by_user(pool: &sqlx::PgPool, user_id: i32) -> Result<Vec<ListOrdersByUserRow>, sqlx::Error> {
    sqlx::query_as!(ListOrdersByUserRow,
        "SELECT o.id, o.total, o.status, o.created_at, u.name AS user_name FROM orders o JOIN users u ON u.id = o.user_id WHERE o.user_id = $1 ORDER BY o.created_at DESC",
        user_id)
        .fetch_all(pool).await
}
```

### Python (asyncpg)

```python
from asyncpg import Connection

@dataclass(frozen=True, slots=True)
class GetUserRow:
    id: int
    name: str
    email: str | None
    created_at: datetime.datetime

async def get_user(conn: Connection, *, id: int) -> GetUserRow | None:
    row = await conn.fetchrow(
        """SELECT id, name, email, created_at FROM users WHERE id = $1""",
        id,
    )
    if row is None:
        return None
    return GetUserRow(id=row["id"], name=row["name"], email=row["email"], created_at=row["created_at"])

@dataclass(frozen=True, slots=True)
class CreateUserRow:
    id: int
    name: str
    email: str | None
    created_at: datetime.datetime

async def create_user(conn: Connection, *, name: str, email: str | None) -> CreateUserRow | None:
    row = await conn.fetchrow(
        """INSERT INTO users (name, email) VALUES ($1, $2) RETURNING id, name, email, created_at""",
        name, email,
    )
    if row is None:
        return None
    return CreateUserRow(id=row["id"], name=row["name"], email=row["email"], created_at=row["created_at"])

async def update_user_email(conn: Connection, *, email: str | None, id: int) -> None:
    await conn.execute(
        """UPDATE users SET email = $1 WHERE id = $2""",
        email, id,
    )

async def delete_user(conn: Connection, *, id: int) -> None:
    await conn.execute(
        """DELETE FROM users WHERE id = $1""",
        id,
    )
```

Parameters are keyword-only (the `*,`), and `:one` queries return `Row | None` -- `fetchrow` returns `None` when no row matches even for an `INSERT ... RETURNING`.

### Go (pgx)

```go
type GetUserRow struct {
    Id int32 `json:"id"`
    Name string `json:"name"`
    Email *string `json:"email"`
    CreatedAt time.Time `json:"created_at"`
}

func GetUser(ctx context.Context, db *pgxpool.Pool, Id int32) (GetUserRow, error) {
    row := db.QueryRow(ctx, "SELECT id, name, email, created_at FROM users WHERE id = $1", Id)
    var r GetUserRow
    err := row.Scan(&r.Id, &r.Name, &r.Email, &r.CreatedAt)
    return r, err
}

func UpdateUserEmail(ctx context.Context, db *pgxpool.Pool, Email *string, Id int32) error {
    _, err := db.Exec(ctx, "UPDATE users SET email = $1 WHERE id = $2", Email, Id)
    return err
}

func DeleteUser(ctx context.Context, db *pgxpool.Pool, Id int32) error {
    _, err := db.Exec(ctx, "DELETE FROM users WHERE id = $1", Id)
    return err
}
```

Fields and parameters are `PascalCase` (`Id`, not `ID`), and the connection type is `*pgxpool.Pool`, not `*pgx.Conn`.

### TypeScript (postgres.js)

```typescript
import type { Sql } from "postgres";

export interface GetUserRow {
    id: number;
    name: string;
    email: string | null;
    created_at: Date;
}

export async function getUser(sql: Sql, id: number): Promise<GetUserRow | null> {
    const rows = await sql<GetUserRow[]>`
    SELECT id, name, email, created_at FROM users WHERE id = ${id}
  `;
    return rows[0] ?? null;
}

export interface CreateUserRow {
    id: number;
    name: string;
    email: string | null;
    created_at: Date;
}

export async function createUser(
    sql: Sql,
    name: string,
    email: string | null,
): Promise<CreateUserRow | null> {
    const rows = await sql<CreateUserRow[]>`
    INSERT INTO users (name, email) VALUES (${name}, ${email}) RETURNING id, name, email, created_at
  `;
    return rows[0] ?? null;
}

export async function updateUserEmail(
    sql: Sql,
    email: string | null,
    id: number,
): Promise<void> {
    await sql`
    UPDATE users SET email = ${email} WHERE id = ${id}
  `;
}

export async function deleteUser(sql: Sql, id: number): Promise<void> {
    await sql`
    DELETE FROM users WHERE id = ${id}
  `;
}
```

Fields are `snake_case` (`created_at`, not `createdAt`), the connection type is `Sql` imported from `postgres`, and `:one` returns `Promise<Row | null>`.
