# Go + pgx

Backend: `go-pgx` | Library: [pgx v5](https://github.com/jackc/pgx) | Engine: PostgreSQL

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

### Struct with json tags and nullable pointers

```go
type GetUserRow struct {
	ID        int32      `json:"id"`
	Name      string     `json:"name"`
	Email     *string    `json:"email"`
	CreatedAt time.Time  `json:"created_at"`
}
```

Nullable columns use `*T` pointers. All field names are `PascalCase`; json tags preserve the SQL column name.

### `:one` -- QueryRow + Scan

```go
func GetUser(ctx context.Context, db *pgx.Conn, id int32) (GetUserRow, error) {
	row := db.QueryRow(ctx,
		"SELECT id, name, email, created_at FROM users WHERE id = $1",
		id,
	)
	var r GetUserRow
	err := row.Scan(&r.ID, &r.Name, &r.Email, &r.CreatedAt)
	return r, err
}
```

`context.Context` is always the first parameter.

### `:many`

```go
type ListUsersRow struct {
	ID   int32  `json:"id"`
	Name string `json:"name"`
}

func ListUsers(ctx context.Context, db *pgx.Conn, limit int64) ([]ListUsersRow, error) {
	rows, err := db.Query(ctx,
		"SELECT id, name FROM users ORDER BY name LIMIT $1",
		limit,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var result []ListUsersRow
	for rows.Next() {
		var r ListUsersRow
		if err := rows.Scan(&r.ID, &r.Name); err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}
```

### `:exec`

```go
func CreateUser(ctx context.Context, db *pgx.Conn, name string, email *string) error {
	_, err := db.Exec(ctx,
		"INSERT INTO users (name, email) VALUES ($1, $2)",
		name, email,
	)
	return err
}
```

## Enum generation

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

```go
type UserStatus string

const (
	UserStatusActive   UserStatus = "active"
	UserStatusInactive UserStatus = "inactive"
	UserStatusBanned   UserStatus = "banned"
)
```

## Type mappings

| SQL Type | Neutral | Go (pgx) |
|----------|---------|----------|
| `SERIAL` / `INTEGER` | `int32` | `int32` |
| `BIGINT` | `int64` | `int64` |
| `TEXT` / `VARCHAR` | `string` | `string` |
| `BOOLEAN` | `bool` | `bool` |
| `BYTEA` | `bytes` | `[]byte` |
| `UUID` | `uuid` | `uuid.UUID` |
| `NUMERIC` | `decimal` | `decimal.Decimal` |
| `DATE` / `TIME` / `TIMESTAMPTZ` | `date` / `time` / `datetime_tz` | `time.Time` |
| `INTERVAL` | `interval` | `time.Duration` |
| `JSON` / `JSONB` | `json` | `json.RawMessage` |
| `INET` | `inet` | `netip.Addr` |
| `TEXT[]` | `array<string>` | `[]string` |
| nullable column | `nullable` | `*T` |
