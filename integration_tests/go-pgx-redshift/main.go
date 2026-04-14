package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/shopspring/decimal"

	queries "scythe-integration/go-pgx-redshift/generated"
)

var passed int
var failed int

func pass(name string) {
	fmt.Printf("PASS: %s\n", name)
	passed++
}

func fail(name string, err error) {
	fmt.Printf("FAIL: %s - %v\n", name, err)
	failed++
}

func assertf(name string, condition bool, format string, args ...interface{}) bool {
	if !condition {
		fail(name, fmt.Errorf(format, args...))
		return false
	}
	return true
}

func main() {
	databaseURL := os.Getenv("REDSHIFT_URL")
	if databaseURL == "" {
		fmt.Fprintln(os.Stderr, "REDSHIFT_URL environment variable is required")
		os.Exit(1)
	}

	ctx := context.Background()

	pool, err := pgxpool.New(ctx, databaseURL)
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to connect to database: %v\n", err)
		os.Exit(1)
	}
	defer pool.Close()

	if err := runMigration(ctx, pool); err != nil {
		fmt.Fprintf(os.Stderr, "failed to run migration: %v\n", err)
		os.Exit(1)
	}

	testCreateUser(ctx, pool)
	testGetUserById(ctx, pool)
	testUpdateUserEmail(ctx, pool)
	testCreateOrder(ctx, pool)
	testGetOrdersByUser(ctx, pool)
	testGetOrderTotal(ctx, pool)
	testListActiveUsers(ctx, pool)
	testGetUserOrders(ctx, pool)
	testCountUsersByStatus(ctx, pool)
	testSearchUsers(ctx, pool)
	testDeleteOrdersByUser(ctx, pool)
	testDeleteUser(ctx, pool)

	fmt.Printf("\nResults: %d passed, %d failed\n", passed, failed)
	if failed > 0 {
		os.Exit(1)
	}
	fmt.Println("ALL TESTS PASSED")
}

func runMigration(ctx context.Context, pool *pgxpool.Pool) error {
	_, thisFile, _, _ := runtime.Caller(0)
	schemaPath := filepath.Join(filepath.Dir(thisFile), "..", "sql", "pg", "schema.sql")

	schema, err := os.ReadFile(schemaPath)
	if err != nil {
		return fmt.Errorf("reading schema file at %s: %w", schemaPath, err)
	}

	dropSQL := `
		DROP TABLE IF EXISTS user_tags CASCADE;
		DROP TABLE IF EXISTS tags CASCADE;
		DROP TABLE IF EXISTS orders CASCADE;
		DROP TABLE IF EXISTS users CASCADE;
		DROP TYPE IF EXISTS user_status CASCADE;
	`
	if _, err := pool.Exec(ctx, dropSQL); err != nil {
		return fmt.Errorf("dropping tables: %w", err)
	}

	if _, err := pool.Exec(ctx, string(schema)); err != nil {
		return fmt.Errorf("creating schema: %w", err)
	}

	return nil
}

// Store user IDs for cross-test use
var createdUserID int32

func testCreateUser(ctx context.Context, pool *pgxpool.Pool) {
	name := "CreateUser"
	user, err := queries.CreateUser(ctx, pool, "Alice", "alice@example.com", queries.UserStatusActive)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, user.Name == "Alice", "expected name Alice, got %s", user.Name) {
		return
	}
	if !assertf(name, user.Email != nil && *user.Email == "alice@example.com", "expected email alice@example.com") {
		return
	}
	if !assertf(name, user.Status == queries.UserStatusActive, "expected status active, got %s", user.Status) {
		return
	}
	createdUserID = user.Id
	pass(name)
}

func testGetUserById(ctx context.Context, pool *pgxpool.Pool) {
	name := "GetUserById"
	user, err := queries.GetUserById(ctx, pool, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, user.Name == "Alice", "expected name Alice, got %s", user.Name) {
		return
	}
	if !assertf(name, user.Id == createdUserID, "expected id %d, got %d", createdUserID, user.Id) {
		return
	}
	pass(name)
}

func testUpdateUserEmail(ctx context.Context, pool *pgxpool.Pool) {
	name := "UpdateUserEmail"
	err := queries.UpdateUserEmail(ctx, pool, "alice-updated@example.com", createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	user, err := queries.GetUserById(ctx, pool, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, user.Email != nil && *user.Email == "alice-updated@example.com", "expected updated email") {
		return
	}
	pass(name)
}

func testCreateOrder(ctx context.Context, pool *pgxpool.Pool) {
	name := "CreateOrder"
	total := decimal.NewFromFloat(99.99)
	order, err := queries.CreateOrder(ctx, pool, createdUserID, total, "Test order")
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, order.UserId == createdUserID, "expected user_id %d, got %d", createdUserID, order.UserId) {
		return
	}
	if !assertf(name, order.Total.Equal(total), "expected total %s, got %s", total, order.Total) {
		return
	}
	pass(name)
}

func testGetOrdersByUser(ctx context.Context, pool *pgxpool.Pool) {
	name := "GetOrdersByUser"
	orders, err := queries.GetOrdersByUser(ctx, pool, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, len(orders) == 1, "expected 1 order, got %d", len(orders)) {
		return
	}
	pass(name)
}

func testGetOrderTotal(ctx context.Context, pool *pgxpool.Pool) {
	name := "GetOrderTotal"
	result, err := queries.GetOrderTotal(ctx, pool, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	expected := decimal.NewFromFloat(99.99)
	if !assertf(name, result.TotalSum != nil && result.TotalSum.Equal(expected), "expected total_sum %s", expected) {
		return
	}
	pass(name)
}

func testListActiveUsers(ctx context.Context, pool *pgxpool.Pool) {
	name := "ListActiveUsers"
	users, err := queries.ListActiveUsers(ctx, pool, queries.UserStatusActive)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, len(users) >= 1, "expected at least 1 active user, got %d", len(users)) {
		return
	}
	pass(name)
}

func testGetUserOrders(ctx context.Context, pool *pgxpool.Pool) {
	name := "GetUserOrders"
	results, err := queries.GetUserOrders(ctx, pool, queries.UserStatusActive)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, len(results) >= 1, "expected at least 1 result, got %d", len(results)) {
		return
	}
	pass(name)
}

func testCountUsersByStatus(ctx context.Context, pool *pgxpool.Pool) {
	name := "CountUsersByStatus"
	result, err := queries.CountUsersByStatus(ctx, pool, queries.UserStatusActive)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, result.UserCount >= 1, "expected count >= 1, got %d", result.UserCount) {
		return
	}
	pass(name)
}

func testSearchUsers(ctx context.Context, pool *pgxpool.Pool) {
	name := "SearchUsers"
	users, err := queries.SearchUsers(ctx, pool, "%Alice%")
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, len(users) >= 1, "expected at least 1 user matching Alice, got %d", len(users)) {
		return
	}
	pass(name)
}

func testDeleteOrdersByUser(ctx context.Context, pool *pgxpool.Pool) {
	name := "DeleteOrdersByUser"
	count, err := queries.DeleteOrdersByUser(ctx, pool, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, count == 1, "expected 1 deleted order, got %d", count) {
		return
	}
	pass(name)
}

func testDeleteUser(ctx context.Context, pool *pgxpool.Pool) {
	name := "DeleteUser"
	err := queries.DeleteUser(ctx, pool, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	// Verify user is deleted
	_, err = queries.GetUserById(ctx, pool, createdUserID)
	if !assertf(name, err != nil, "expected error when fetching deleted user") {
		return
	}
	pass(name)
}
