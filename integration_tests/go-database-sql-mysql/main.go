package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"database/sql"

	_ "github.com/go-sql-driver/mysql"

	queries "scythe-integration/go-database-sql-mysql/generated"
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
	databaseURL := os.Getenv("MYSQL_URL")
	if databaseURL == "" {
		fmt.Fprintln(os.Stderr, "MYSQL_URL environment variable is required")
		os.Exit(1)
	}

	ctx := context.Background()

	db, err := sql.Open("mysql", databaseURL)
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to connect to database: %v\n", err)
		os.Exit(1)
	}
	defer db.Close()

	if err := runMigration(ctx, db); err != nil {
		fmt.Fprintf(os.Stderr, "failed to run migration: %v\n", err)
		os.Exit(1)
	}

	testCreateUser(ctx, db)
	testGetUserById(ctx, db)
	testCreateOrder(ctx, db)
	testGetOrdersByUser(ctx, db)
	testListActiveUsers(ctx, db)
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
	schemaPath := filepath.Join(filepath.Dir(thisFile), "..", "sql", "mysql", "schema.sql")

	schema, err := os.ReadFile(schemaPath)
	if err != nil {
		return fmt.Errorf("reading schema file at %s: %w", schemaPath, err)
	}

	dropStatements := []string{
		"DROP TABLE IF EXISTS user_tags",
		"DROP TABLE IF EXISTS tags",
		"DROP TABLE IF EXISTS orders",
		"DROP TABLE IF EXISTS users",
	}
	for _, stmt := range dropStatements {
		if _, err := db.ExecContext(ctx, stmt); err != nil {
			return fmt.Errorf("dropping tables: %w", err)
		}
	}

	if _, err := db.ExecContext(ctx, string(schema)); err != nil {
		return fmt.Errorf("creating schema: %w", err)
	}

	return nil
}

var createdUserID int32

func testCreateUser(ctx context.Context, db *sql.DB) {
	name := "CreateUser"
	email := "alice@example.com"
	if err := queries.CreateUser(ctx, db, "Alice", &email, queries.UsersStatusActive); err != nil {
		fail(name, err)
		return
	}
	user, err := queries.GetLastInsertUser(ctx, db)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, user.Name == "Alice", "expected name Alice, got %s", user.Name) {
		return
	}
	createdUserID = user.Id
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

func testCreateOrder(ctx context.Context, db *sql.DB) {
	name := "CreateOrder"
	notes := "Test order"
	if err := queries.CreateOrder(ctx, db, createdUserID, 99.99, &notes); err != nil {
		fail(name, err)
		return
	}
	order, err := queries.GetLastInsertOrder(ctx, db)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, order.UserId == createdUserID, "expected user_id %d, got %d", createdUserID, order.UserId) {
		return
	}
