package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"database/sql"
	"net/url"
	"strings"

	_ "github.com/godror/godror"

	queries "scythe-integration/go-godror-oracle/generated"
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
	databaseURL := os.Getenv("ORACLE_URL")
	if databaseURL == "" {
		fmt.Fprintln(os.Stderr, "ORACLE_URL environment variable is required")
		os.Exit(1)
	}

	ctx := context.Background()

	// Parse oracle://user:pass@host:port/service into godror DSN: user/pass@host:port/service
	parsedURL, err := url.Parse(databaseURL)
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to parse ORACLE_URL: %v\n", err)
		os.Exit(1)
	}
	password, _ := parsedURL.User.Password()
	godrorDSN := fmt.Sprintf("%s/%s@%s%s",
		parsedURL.User.Username(),
		password,
		parsedURL.Host,
		strings.TrimPrefix(parsedURL.Path, "/"),
	)
	db, err := sql.Open("godror", godrorDSN)
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
	schemaPath := filepath.Join(filepath.Dir(thisFile), "..", "sql", "oracle", "schema_full.sql")

	schema, err := os.ReadFile(schemaPath)
	if err != nil {
		return fmt.Errorf("reading schema file at %s: %w", schemaPath, err)
	}

	// Drop tables and sequences, ignoring ORA errors
	for _, table := range []string{"user_tags", "tags", "orders", "users"} {
		db.ExecContext(ctx, fmt.Sprintf("DROP TABLE %s CASCADE CONSTRAINTS", table)) //nolint:errcheck
	}
	for _, seq := range []string{"tags_seq", "orders_seq", "users_seq"} {
		db.ExecContext(ctx, fmt.Sprintf("DROP SEQUENCE %s", seq)) //nolint:errcheck
	}

	// Execute each PL/SQL block (delimited by /\n)
	for _, block := range strings.Split(string(schema), "/\n") {
		block = strings.TrimSpace(block)
		if block == "" {
			continue
		}
		if _, err := db.ExecContext(ctx, block); err != nil {
			return fmt.Errorf("executing schema block: %w", err)
		}
	}

	return nil
}

var createdUserID int64

func testCreateUser(ctx context.Context, db *sql.DB) {
	name := "CreateUser"
	email := "alice@example.com"
	user, err := queries.CreateUser(ctx, db, "Alice", &email, 1)
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
	order, err := queries.CreateOrder(ctx, db, createdUserID, int64(9999), &notes)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, order.UserId == createdUserID, "expected user_id %d, got %d", createdUserID, order.UserId) {
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

func testListActiveUsers(ctx context.Context, db *sql.DB) {
	name := "ListActiveUsers"
	users, err := queries.ListActiveUsers(ctx, db)
	if err != nil {
		fail(name, err)
		return
	}
	if !assertf(name, len(users) >= 1, "expected at least 1 active user, got %d", len(users)) {
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
	// Verify user is deleted
	_, err = queries.GetUserById(ctx, db, createdUserID)
	if !assertf(name, err != nil, "expected error when fetching deleted user") {
		return
	}
	pass(name)
}
