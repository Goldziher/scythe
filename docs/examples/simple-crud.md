# Simple CRUD

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
#[derive(Debug, sqlx::FromRow)]
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

pub async fn create_user(pool: &sqlx::PgPool, name: &str, email: Option<&str>) -> Result<GetUserRow, sqlx::Error> {
    sqlx::query_as!(GetUserRow,
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

#[derive(Debug, sqlx::FromRow)]
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
@dataclass
class GetUserRow:
    id: int
    name: str
    email: str | None
    created_at: datetime.datetime

async def get_user(conn: asyncpg.Connection, id: int) -> GetUserRow:
    row = await conn.fetchrow(
        "SELECT id, name, email, created_at FROM users WHERE id = $1", id)
    return GetUserRow(id=row["id"], name=row["name"], email=row["email"], created_at=row["created_at"])

async def create_user(conn: asyncpg.Connection, name: str, email: str | None) -> GetUserRow:
    row = await conn.fetchrow(
        "INSERT INTO users (name, email) VALUES ($1, $2) RETURNING id, name, email, created_at",
        name, email)
    return GetUserRow(id=row["id"], name=row["name"], email=row["email"], created_at=row["created_at"])

async def update_user_email(conn: asyncpg.Connection, email: str | None, id: int) -> None:
    await conn.execute("UPDATE users SET email = $1 WHERE id = $2", email, id)

async def delete_user(conn: asyncpg.Connection, id: int) -> None:
    await conn.execute("DELETE FROM users WHERE id = $1", id)
```

### Go (pgx)

```go
type GetUserRow struct {
	ID        int32              `json:"id"`
	Name      string             `json:"name"`
	Email     *string            `json:"email"`
	CreatedAt time.Time          `json:"created_at"`
}

func GetUser(ctx context.Context, db *pgx.Conn, id int32) (GetUserRow, error) {
	row := db.QueryRow(ctx,
		"SELECT id, name, email, created_at FROM users WHERE id = $1", id)
	var r GetUserRow
	err := row.Scan(&r.ID, &r.Name, &r.Email, &r.CreatedAt)
	return r, err
}

func UpdateUserEmail(ctx context.Context, db *pgx.Conn, email *string, id int32) error {
	_, err := db.Exec(ctx, "UPDATE users SET email = $1 WHERE id = $2", email, id)
	return err
}

func DeleteUser(ctx context.Context, db *pgx.Conn, id int32) error {
	_, err := db.Exec(ctx, "DELETE FROM users WHERE id = $1", id)
	return err
}
```

### TypeScript (postgres.js)

```typescript
export interface GetUserRow {
  id: number;
  name: string;
  email: string | null;
  createdAt: Date;
}

export async function getUser(sql: postgres.Sql, id: number): Promise<GetUserRow> {
  const [row] = await sql<GetUserRow[]>`
    SELECT id, name, email, created_at FROM users WHERE id = ${id}`;
  return row;
}

export async function createUser(sql: postgres.Sql, name: string, email: string | null): Promise<GetUserRow> {
  const [row] = await sql<GetUserRow[]>`
    INSERT INTO users (name, email) VALUES (${name}, ${email}) RETURNING id, name, email, created_at`;
  return row;
}

export async function updateUserEmail(sql: postgres.Sql, email: string | null, id: number): Promise<void> {
  await sql`UPDATE users SET email = ${email} WHERE id = ${id}`;
}

export async function deleteUser(sql: postgres.Sql, id: number): Promise<void> {
  await sql`DELETE FROM users WHERE id = ${id}`;
}
```
