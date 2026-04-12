package queries

import (
	"context"
	"database/sql"
	"time"
)


type UsersStatus string

const (
	UsersStatusActive UsersStatus = "active"
	UsersStatusInactive UsersStatus = "inactive"
	UsersStatusBanned UsersStatus = "banned"
)

type CreateOrderRow struct {
	Id int32 `json:"id"`
	UserId string `json:"user_id"`
	Total float64 `json:"total"`
	Notes *string `json:"notes"`
	CreatedAt time.Time `json:"created_at"`
}

func CreateOrder(ctx context.Context, db *sql.DB, UserId string, Total float64, Notes *string) (CreateOrderRow, error) {
	row := db.QueryRowContext(ctx, "INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at", UserId, Total, Notes)
	var r CreateOrderRow
	err := row.Scan(&r.Id, &r.UserId, &r.Total, &r.Notes, &r.CreatedAt)
	return r, err
}

type GetOrdersByUserRow struct {
	Id int32 `json:"id"`
	Total float64 `json:"total"`
	Notes *string `json:"notes"`
	CreatedAt time.Time `json:"created_at"`
}

func GetOrdersByUser(ctx context.Context, db *sql.DB, UserId string) ([]GetOrdersByUserRow, error) {
	rows, err := db.QueryContext(ctx, "SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC", UserId)
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

func GetOrderTotal(ctx context.Context, db *sql.DB, UserId string) (GetOrderTotalRow, error) {
	row := db.QueryRowContext(ctx, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?", UserId)
	var r GetOrderTotalRow
	err := row.Scan(&r.TotalSum)
	return r, err
}

func DeleteOrdersByUser(ctx context.Context, db *sql.DB, UserId string) (int64, error) {
	result, err := db.ExecContext(ctx, "DELETE FROM orders WHERE user_id = ?", UserId)
	if err != nil {
		return 0, err
	}
	return result.RowsAffected()
}

type GetUserByIdRow struct {
	Id string `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
	Status UsersStatus `json:"status"`
	CreatedAt time.Time `json:"created_at"`
}

func GetUserById(ctx context.Context, db *sql.DB, Id string) (GetUserByIdRow, error) {
	row := db.QueryRowContext(ctx, "SELECT id, name, email, status, created_at FROM users WHERE id = ?", Id)
	var r GetUserByIdRow
	err := row.Scan(&r.Id, &r.Name, &r.Email, &r.Status, &r.CreatedAt)
	return r, err
}

type ListActiveUsersRow struct {
	Id string `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
}

func ListActiveUsers(ctx context.Context, db *sql.DB, Status UsersStatus) ([]ListActiveUsersRow, error) {
	rows, err := db.QueryContext(ctx, "SELECT id, name, email FROM users WHERE status = ?", Status)
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

type CreateUserRow struct {
	Id string `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
}

func CreateUser(ctx context.Context, db *sql.DB, Name string, Email *string, Status UsersStatus) (CreateUserRow, error) {
	row := db.QueryRowContext(ctx, "INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email", Name, Email, Status)
	var r CreateUserRow
	err := row.Scan(&r.Id, &r.Name, &r.Email)
	return r, err
}

func UpdateUserEmail(ctx context.Context, db *sql.DB, Email string, Id string) error {
	_, err := db.ExecContext(ctx, "UPDATE users SET email = ? WHERE id = ?", Email, Id)
	return err
}

func DeleteUser(ctx context.Context, db *sql.DB, Id string) error {
	_, err := db.ExecContext(ctx, "DELETE FROM users WHERE id = ? RETURNING id", Id)
	return err
}

type SearchUsersRow struct {
	Id string `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
}

func SearchUsers(ctx context.Context, db *sql.DB, Name string) ([]SearchUsersRow, error) {
	rows, err := db.QueryContext(ctx, "SELECT id, name, email FROM users WHERE name LIKE ?", Name)
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
