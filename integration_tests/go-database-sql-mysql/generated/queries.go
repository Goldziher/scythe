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

func CreateOrder(ctx context.Context, db *sql.DB, UserId int32, Total string, Notes string) error {
	_, err := db.ExecContext(ctx, "INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)", UserId, Total, Notes)
	return err
}

type GetLastInsertOrderRow struct {
	Id int32 `json:"id"`
	UserId int32 `json:"user_id"`
	Total string `json:"total"`
	Notes *string `json:"notes"`
	CreatedAt time.Time `json:"created_at"`
}

func GetLastInsertOrder(ctx context.Context, db *sql.DB) (GetLastInsertOrderRow, error) {
	row := db.QueryRowContext(ctx, "SELECT id, user_id, total, notes, created_at FROM orders WHERE id = LAST_INSERT_ID()")
	var r GetLastInsertOrderRow
	err := row.Scan(&r.Id, &r.UserId, &r.Total, &r.Notes, &r.CreatedAt)
	return r, err
}

type GetOrdersByUserRow struct {
	Id int32 `json:"id"`
	Total string `json:"total"`
	Notes *string `json:"notes"`
	CreatedAt time.Time `json:"created_at"`
}

func GetOrdersByUser(ctx context.Context, db *sql.DB, UserId int32) ([]GetOrdersByUserRow, error) {
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
	TotalSum *string `json:"total_sum"`
}

func GetOrderTotal(ctx context.Context, db *sql.DB, UserId int32) (GetOrderTotalRow, error) {
	row := db.QueryRowContext(ctx, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?", UserId)
	var r GetOrderTotalRow
	err := row.Scan(&r.TotalSum)
	return r, err
}

func DeleteOrdersByUser(ctx context.Context, db *sql.DB, UserId int32) (int64, error) {
	result, err := db.ExecContext(ctx, "DELETE FROM orders WHERE user_id = ?", UserId)
	if err != nil {
		return 0, err
	}
	return result.RowsAffected()
}

type GetUserByIdRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
	Status UsersStatus `json:"status"`
	CreatedAt time.Time `json:"created_at"`
}

func GetUserById(ctx context.Context, db *sql.DB, Id int32) (GetUserByIdRow, error) {
	row := db.QueryRowContext(ctx, "SELECT id, name, email, status, created_at FROM users WHERE id = ?", Id)
	var r GetUserByIdRow
	err := row.Scan(&r.Id, &r.Name, &r.Email, &r.Status, &r.CreatedAt)
	return r, err
}

type ListActiveUsersRow struct {
	Id int32 `json:"id"`
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

func CreateUser(ctx context.Context, db *sql.DB, Name string, Email string, Status UsersStatus) error {
	_, err := db.ExecContext(ctx, "INSERT INTO users (name, email, status) VALUES (?, ?, ?)", Name, Email, Status)
	return err
}

type GetLastInsertUserRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
	Status UsersStatus `json:"status"`
	CreatedAt time.Time `json:"created_at"`
}

func GetLastInsertUser(ctx context.Context, db *sql.DB) (GetLastInsertUserRow, error) {
	row := db.QueryRowContext(ctx, "SELECT id, name, email, status, created_at FROM users WHERE id = LAST_INSERT_ID()")
	var r GetLastInsertUserRow
	err := row.Scan(&r.Id, &r.Name, &r.Email, &r.Status, &r.CreatedAt)
	return r, err
}

func UpdateUserEmail(ctx context.Context, db *sql.DB, Email string, Id int32) error {
	_, err := db.ExecContext(ctx, "UPDATE users SET email = ? WHERE id = ?", Email, Id)
	return err
}

func DeleteUser(ctx context.Context, db *sql.DB, Id int32) error {
	_, err := db.ExecContext(ctx, "DELETE FROM users WHERE id = ?", Id)
	return err
}

type SearchUsersRow struct {
	Id int32 `json:"id"`
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
