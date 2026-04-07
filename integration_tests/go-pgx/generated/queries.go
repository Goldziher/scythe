package queries

import (
	"context"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/shopspring/decimal"
)


type UserStatus string

const (
	UserStatusActive UserStatus = "active"
	UserStatusInactive UserStatus = "inactive"
	UserStatusBanned UserStatus = "banned"
)

type CreateOrderRow struct {
	Id int32 `json:"id"`
	UserId int32 `json:"user_id"`
	Total decimal.Decimal `json:"total"`
	Notes *string `json:"notes"`
	CreatedAt time.Time `json:"created_at"`
}

func CreateOrder(ctx context.Context, db *pgxpool.Pool, UserId int32, Total decimal.Decimal, Notes string) (CreateOrderRow, error) {
	row := db.QueryRow(ctx, "INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at", UserId, Total, Notes)
	var r CreateOrderRow
	err := row.Scan(&r.Id, &r.UserId, &r.Total, &r.Notes, &r.CreatedAt)
	return r, err
}

type GetOrdersByUserRow struct {
	Id int32 `json:"id"`
	Total decimal.Decimal `json:"total"`
	Notes *string `json:"notes"`
	CreatedAt time.Time `json:"created_at"`
}

func GetOrdersByUser(ctx context.Context, db *pgxpool.Pool, UserId int32) ([]GetOrdersByUserRow, error) {
	rows, err := db.Query(ctx, "SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC", UserId)
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
	TotalSum *decimal.Decimal `json:"total_sum"`
}

func GetOrderTotal(ctx context.Context, db *pgxpool.Pool, UserId int32) (GetOrderTotalRow, error) {
	row := db.QueryRow(ctx, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1", UserId)
	var r GetOrderTotalRow
	err := row.Scan(&r.TotalSum)
	return r, err
}

func DeleteOrdersByUser(ctx context.Context, db *pgxpool.Pool, UserId int32) (int64, error) {
	result, err := db.Exec(ctx, "DELETE FROM orders WHERE user_id = $1", UserId)
	if err != nil {
		return 0, err
	}
	return result.RowsAffected(), nil
}

type GetUserByIdRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
	Status UserStatus `json:"status"`
	CreatedAt time.Time `json:"created_at"`
}

func GetUserById(ctx context.Context, db *pgxpool.Pool, Id int32) (GetUserByIdRow, error) {
	row := db.QueryRow(ctx, "SELECT id, name, email, status, created_at FROM users WHERE id = $1", Id)
	var r GetUserByIdRow
	err := row.Scan(&r.Id, &r.Name, &r.Email, &r.Status, &r.CreatedAt)
	return r, err
}

type ListActiveUsersRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
}

func ListActiveUsers(ctx context.Context, db *pgxpool.Pool, Status UserStatus) ([]ListActiveUsersRow, error) {
	rows, err := db.Query(ctx, "SELECT id, name, email FROM users WHERE status = $1", Status)
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
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
	Status UserStatus `json:"status"`
	CreatedAt time.Time `json:"created_at"`
}

func CreateUser(ctx context.Context, db *pgxpool.Pool, Name string, Email string, Status UserStatus) (CreateUserRow, error) {
	row := db.QueryRow(ctx, "INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at", Name, Email, Status)
	var r CreateUserRow
	err := row.Scan(&r.Id, &r.Name, &r.Email, &r.Status, &r.CreatedAt)
	return r, err
}

func UpdateUserEmail(ctx context.Context, db *pgxpool.Pool, Email string, Id int32) error {
	_, err := db.Exec(ctx, "UPDATE users SET email = $1 WHERE id = $2", Email, Id)
	return err
}

func DeleteUser(ctx context.Context, db *pgxpool.Pool, Id int32) error {
	_, err := db.Exec(ctx, "DELETE FROM users WHERE id = $1", Id)
	return err
}

type GetUserOrdersRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Total *decimal.Decimal `json:"total"`
	Notes *string `json:"notes"`
}

func GetUserOrders(ctx context.Context, db *pgxpool.Pool, Status UserStatus) ([]GetUserOrdersRow, error) {
	rows, err := db.Query(ctx, "SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = $1", Status)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []GetUserOrdersRow
	for rows.Next() {
		var r GetUserOrdersRow
		if err := rows.Scan(&r.Id, &r.Name, &r.Total, &r.Notes); err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}

type CountUsersByStatusRow struct {
	Status UserStatus `json:"status"`
	UserCount int64 `json:"user_count"`
}

func CountUsersByStatus(ctx context.Context, db *pgxpool.Pool, Status UserStatus) (CountUsersByStatusRow, error) {
	row := db.QueryRow(ctx, "SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1", Status)
	var r CountUsersByStatusRow
	err := row.Scan(&r.Status, &r.UserCount)
	return r, err
}

type GetUserWithTagsRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	TagName string `json:"tag_name"`
}

func GetUserWithTags(ctx context.Context, db *pgxpool.Pool, Id int32) ([]GetUserWithTagsRow, error) {
	rows, err := db.Query(ctx, "SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = $1", Id)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var result []GetUserWithTagsRow
	for rows.Next() {
		var r GetUserWithTagsRow
		if err := rows.Scan(&r.Id, &r.Name, &r.TagName); err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}

type SearchUsersRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Email *string `json:"email"`
}

func SearchUsers(ctx context.Context, db *pgxpool.Pool, Name string) ([]SearchUsersRow, error) {
	rows, err := db.Query(ctx, "SELECT id, name, email FROM users WHERE name LIKE $1", Name)
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
