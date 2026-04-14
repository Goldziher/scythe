package main

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"


	queries "scythe-integration/go-gosnowflake/generated"
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
	databaseURL := os.Getenv("SNOWFLAKE_URL")
	if databaseURL == "" {
		fmt.Fprintln(os.Stderr, "SNOWFLAKE_URL environment variable is required")
		os.Exit(1)
	}

	ctx := context.Background()


	fmt.Printf("\nResults: %d passed, %d failed\n", passed, failed)
	if failed > 0 {
		os.Exit(1)
	}
	fmt.Println("ALL TESTS PASSED")
}
