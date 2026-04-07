package main

import (
	"context"
	"database/sql"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"runtime"

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
	dsn := os.Getenv("MYSQL_DSN")
	if dsn == "" {
		// Fall back to DATABASE_URL and convert mysql:// URL to Go DSN format
		dbURL := os.Getenv("DATABASE_URL")
		if dbURL == "" {
			fmt.Fprintln(os.Stderr, "MYSQL_DSN or DATABASE_URL environment variable is required")
			os.Exit(1)
		}
		parsed, err := url.Parse(dbURL)
		if err != nil {
			fmt.Fprintf(os.Stderr, "failed to parse DATABASE_URL: %v\n", err)
			os.Exit(1)
		}
		password, _ := parsed.User.Password()
		host := parsed.Hostname()
		port := parsed.Port()
		if port == "" {
			port = "3306"
		}
		dbName := parsed.Path[1:] // strip leading /
		dsn = fmt.Sprintf("%s:%s@tcp(%s:%s)/%s?parseTime=true", parsed.User.Username(), password, host, port, dbName)
	}

	db, err := sql.Open("mysql", dsn)
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
	testGetLastInsertUser(ctx, db)
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
	schemaPath := filepath.Join(filepath.Dir(thisFile), "..", "sql", "mysql", "schema.sql")

	schema, err := os.ReadFile(schemaPath)
	if err != nil {
		return fmt.Errorf("reading schema file at %s: %w", schemaPath, err)
	}

	dropSQL := []string{
		"DROP TABLE IF EXISTS user_tags",
		"DROP TABLE IF EXISTS tags",
		"DROP TABLE IF EXISTS orders",
		"DROP TABLE IF EXISTS users",
	}
	for _, stmt := range dropSQL {
		if _, err := db.ExecContext(ctx, stmt); err != nil {
			return fmt.Errorf("dropping tables: %w", err)
		}
	}

	// MySQL requires executing statements one at a time
	statements := splitStatements(string(schema))
	for _, stmt := range statements {
		if stmt == "" {
			continue
		}
		if _, err := db.ExecContext(ctx, stmt); err != nil {
			return fmt.Errorf("creating schema: %w", err)
		}
	}

	return nil
}

func splitStatements(sql string) []string {
	var result []string
	current := ""
	for _, line := range splitLines(sql) {
		current += line + "\n"
		if len(line) > 0 && line[len(line)-1] == ';' {
			result = append(result, current)
			current = ""
		}
	}
	if current != "" {
		result = append(result, current)
	}
	return result
}

func splitLines(s string) []string {
	var lines []string
	start := 0
	for i := 0; i < len(s); i++ {
		if s[i] == '\n' {
			lines = append(lines, s[start:i])
			start = i + 1
		}
	}
	if start < len(s) {
		lines = append(lines, s[start:])
	}
	return lines
}

var createdUserID int32

func testCreateUser(ctx context.Context, db *sql.DB) {
	name := "CreateUser"
	err := queries.CreateUser(ctx, db, "Alice", "alice@example.com", queries.UsersStatusActive)
	if err != nil {
		fail(name, err)
		return
	}
	pass(name)
}

func testGetLastInsertUser(ctx context.Context, db *sql.DB) {
	name := "GetLastInsertUser"
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
	err := queries.CreateOrder(ctx, db, createdUserID, "99.99", "Test order")
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
	users, err := queries.ListActiveUsers(ctx, db, queries.UsersStatusActive)
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
