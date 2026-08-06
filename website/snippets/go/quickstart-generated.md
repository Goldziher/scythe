```go title="Go (pgx)"
package queries

import (
    "context"
    "time"

    "github.com/jackc/pgx/v5/pgxpool"
)

type UserStatus string

const (
    UserStatusActive   UserStatus = "active"
    UserStatusInactive UserStatus = "inactive"
    UserStatusBanned   UserStatus = "banned"
)

type GetUserByIdRow struct {
    Id        int32      `json:"id"`
    Name      string     `json:"name"`
    Email     *string    `json:"email"`
    Status    UserStatus `json:"status"`
    CreatedAt time.Time  `json:"created_at"`
}

func GetUserById(
    ctx context.Context, db *pgxpool.Pool, Id int32,
) (GetUserByIdRow, error) {
    row := db.QueryRow(ctx,
        "SELECT id, name, email, status, created_at "+
            "FROM users WHERE id = $1", Id)
    var r GetUserByIdRow
    err := row.Scan(
        &r.Id, &r.Name, &r.Email, &r.Status, &r.CreatedAt,
    )
    return r, err
}

type ListActiveUsersRow struct {
    Id    int32   `json:"id"`
    Name  string  `json:"name"`
    Email *string `json:"email"`
}

func ListActiveUsers(
    ctx context.Context, db *pgxpool.Pool,
    Status UserStatus,
) ([]ListActiveUsersRow, error) {
    rows, err := db.Query(ctx,
        "SELECT id, name, email FROM users "+
            "WHERE status = $1", Status)
    if err != nil {
        return nil, err
    }
    defer rows.Close()
    var result []ListActiveUsersRow
    for rows.Next() {
        var r ListActiveUsersRow
        if err := rows.Scan(
            &r.Id, &r.Name, &r.Email,
        ); err != nil {
            return nil, err
        }
        result = append(result, r)
    }
    return result, rows.Err()
}

func UpdateUserEmail(
    ctx context.Context, db *pgxpool.Pool,
    Email string, Id int32,
) error {
    _, err := db.Exec(ctx,
        "UPDATE users SET email = $1 WHERE id = $2",
        Email, Id)
    return err
}
```
