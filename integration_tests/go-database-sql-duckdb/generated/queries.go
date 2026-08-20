// scythe:provenance v=0.16.1 backend=go-database-sql engine=duckdb schema=sch1:a58e9693abcdb5e7 queries=q1:3fcd9a387f9d569e options=opt1:cbf29ce484222325
package queries

import (
	"context"
	"database/sql"
	"time"
)

// go-duckdb cannot bind a typed pointer, so a nullable
// parameter is passed as its value or as an untyped nil.
func duckdbBindValue[T any](v *T) any {
	if v == nil {
		return nil
	}
	return *v
}

func CreateOrder(ctx context.Context, db *sql.DB, UserId int32, Total float64, Notes *string) error {
	_, err := db.ExecContext(ctx, "INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3)", UserId, Total, duckdbBindValue(Notes))
	return err
}

type GetOrdersByUserRow struct {
	Id int32 `json:"id"`
	Total float64 `json:"total"`
	Notes *string `json:"notes"`
	CreatedAt time.Time `json:"created_at"`
}

func GetOrdersByUser(ctx context.Context, db *sql.DB, UserId int32) ([]GetOrdersByUserRow, error) {
	rows, err := db.QueryContext(ctx, "SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC", UserId)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []GetOrdersByUserRow
	for rows.Next() {
		var r GetOrdersByUserRow
		if err := rows.Scan(&r.Id, &r.Total, &r.Notes, &r.CreatedAt); err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}

type GetOrderTotalRow struct {
	TotalSum *float64 `json:"total_sum"`
}

func GetOrderTotal(ctx context.Context, db *sql.DB, UserId int32) (GetOrderTotalRow, error) {
	row := db.QueryRowContext(ctx, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1", UserId)
	var r GetOrderTotalRow
	err := row.Scan(&r.TotalSum)
	return r, err
}

func DeleteOrdersByUser(ctx context.Context, db *sql.DB, UserId int32) (int64, error) {
	result, err := db.ExecContext(ctx, "DELETE FROM orders WHERE user_id = $1", UserId)
	if err != nil {
		return 0, err
	}
	return result.RowsAffected()
}

type GetUserByIdRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
	Status string `json:"status"`
	CreatedAt time.Time `json:"created_at"`
}

func GetUserById(ctx context.Context, db *sql.DB, Id int32) (GetUserByIdRow, error) {
	row := db.QueryRowContext(ctx, "SELECT id, name, email, status, created_at FROM users WHERE id = $1", Id)
	var r GetUserByIdRow
	err := row.Scan(&r.Id, &r.Name, &r.Email, &r.Status, &r.CreatedAt)
	return r, err
}

type ListActiveUsersRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
}

func ListActiveUsers(ctx context.Context, db *sql.DB, Status string) ([]ListActiveUsersRow, error) {
	rows, err := db.QueryContext(ctx, "SELECT id, name, email FROM users WHERE status = $1", Status)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []ListActiveUsersRow
	for rows.Next() {
		var r ListActiveUsersRow
		if err := rows.Scan(&r.Id, &r.Name, &r.Email); err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}

func CreateUser(ctx context.Context, db *sql.DB, Name string, Email *string, Status string) error {
	_, err := db.ExecContext(ctx, "INSERT INTO users (name, email, status) VALUES ($1, $2, $3)", Name, duckdbBindValue(Email), Status)
	return err
}

func UpdateUserEmail(ctx context.Context, db *sql.DB, Email string, Id int32) error {
	_, err := db.ExecContext(ctx, "UPDATE users SET email = $1 WHERE id = $2", Email, Id)
	return err
}

func DeleteUser(ctx context.Context, db *sql.DB, Id int32) error {
	_, err := db.ExecContext(ctx, "DELETE FROM users WHERE id = $1", Id)
	return err
}

type SearchUsersRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
}

func SearchUsers(ctx context.Context, db *sql.DB, Name string) ([]SearchUsersRow, error) {
	rows, err := db.QueryContext(ctx, "SELECT id, name, email FROM users WHERE name LIKE $1", Name)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []SearchUsersRow
	for rows.Next() {
		var r SearchUsersRow
		if err := rows.Scan(&r.Id, &r.Name, &r.Email); err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}
