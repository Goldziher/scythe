package main

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	_ "modernc.org/sqlite"

	queries "scythe-integration/go-database-sql-sqlite/generated"
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
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to open database: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	ctx := context.Background()

	if err := runMigration(ctx, db); err != nil {
		fmt.Fprintf(os.Stderr, "failed to run migration: %v\n", err)
		os.Exit(1)
	}

	testCreateUser(ctx, db)
	testGetUserById(ctx, db)
	testUpdateUserEmail(ctx, db)
	testCreateOrder(ctx, db)
	testGetOrdersByUser(ctx, db)
	testGetOrderTotal(ctx, db)
	testListActiveUsers(ctx, db)
	testSearchUsers(ctx, db)
	testDeleteOrdersByUser(ctx, db)
	testDeleteUser(ctx, db)

	fmt.Printf("\nResults: %d passed, %d failed\n", passed, failed)
	if failed > 0 {
		os.Exit(1)
	}
	fmt.Println("ALL TESTS PASSED")
}

func runMigration(ctx context.Context, db *sql.DB) error {
	_, thisFile, _, _ := runtime.Caller(0)
	schemaPath := filepath.Join(filepath.Dir(thisFile), "..", "sql", "sqlite", "schema.sql")

	schema, err := os.ReadFile(schemaPath)
	if err != nil {
		return fmt.Errorf("reading schema file at %s: %w", schemaPath, err)
	}

	if _, err := db.ExecContext(ctx, string(schema)); err != nil {
		return fmt.Errorf("creating schema: %w", err)
	}

	return nil
}

var createdUserID int32

func testCreateUser(ctx context.Context, db *sql.DB) {
	name := "CreateUser"
	err := queries.CreateUser(ctx, db, "Alice", "alice@example.com", "active")
	if err != nil {
		fail(name, err)
		return
	}
	createdUserID = 1 // SQLite AUTOINCREMENT starts at 1
	pass(name)
}

func testGetUserById(ctx context.Context, db *sql.DB) {
	name := "GetUserById"
	user, err := queries.GetUserById(ctx, db, createdUserID)
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

func testUpdateUserEmail(ctx context.Context, db *sql.DB) {
	name := "UpdateUserEmail"
	err := queries.UpdateUserEmail(ctx, db, "alice-updated@example.com", createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	user, err := queries.GetUserById(ctx, db, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, user.Email != nil && *user.Email == "alice-updated@example.com", "expected updated email") {
		return
	}
	pass(name)
}

func testCreateOrder(ctx context.Context, db *sql.DB) {
	name := "CreateOrder"
	err := queries.CreateOrder(ctx, db, createdUserID, float32(99.99), "Test order")
	if err != nil {
		fail(name, err)
		return
	}
	pass(name)
}

func testGetOrdersByUser(ctx context.Context, db *sql.DB) {
	name := "GetOrdersByUser"
	orders, err := queries.GetOrdersByUser(ctx, db, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, len(orders) == 1, "expected 1 order, got %d", len(orders)) {
		return
	}
	pass(name)
}

func testGetOrderTotal(ctx context.Context, db *sql.DB) {
	name := "GetOrderTotal"
	result, err := queries.GetOrderTotal(ctx, db, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, result.TotalSum != nil, "expected non-nil total_sum") {
		return
	}
	pass(name)
}

func testListActiveUsers(ctx context.Context, db *sql.DB) {
	name := "ListActiveUsers"
	users, err := queries.ListActiveUsers(ctx, db, "active")
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, len(users) >= 1, "expected at least 1 active user, got %d", len(users)) {
		return
	}
	pass(name)
}

func testSearchUsers(ctx context.Context, db *sql.DB) {
	name := "SearchUsers"
	users, err := queries.SearchUsers(ctx, db, "%Alice%")
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, len(users) >= 1, "expected at least 1 user matching Alice, got %d", len(users)) {
		return
	}
	pass(name)
}

func testDeleteOrdersByUser(ctx context.Context, db *sql.DB) {
	name := "DeleteOrdersByUser"
	count, err := queries.DeleteOrdersByUser(ctx, db, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, count == 1, "expected 1 deleted order, got %d", count) {
		return
	}
	pass(name)
}

func testDeleteUser(ctx context.Context, db *sql.DB) {
	name := "DeleteUser"
	err := queries.DeleteUser(ctx, db, createdUserID)
	if err != nil {
		fail(name, err)
		return
	}
	pass(name)
}
