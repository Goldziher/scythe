# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.4] - 2026-04-10

### Added

- Integration tests now run all generated code against real databases (PostgreSQL, MySQL, SQLite) across all 39 backends and 10 languages
- CI split into 3 parallel jobs (PostgreSQL, MySQL, SQLite) covering all backends
- New MySQL/SQLite SQL queries: GetUserOrders, CountUsersByStatus, GetUserWithTags

### Fixed

- tokio-postgres: enum parameters now use `::text::enum_name` casts for proper PostgreSQL enum handling
- tokio-postgres: enum columns in SELECT/RETURNING use `::text` cast for correct deserialization
- sqlx: RETURNING clauses now include enum type annotations (`"status: UserStatus"`)
- sqlx: aggregate functions (COUNT, SUM) get non-null override annotations (`"column_name!"`)
- C# Npgsql: enum extension methods moved to top-level static classes (fixes CS1109)
- C# Microsoft.Data.Sqlite: fixed type mappings (int32->long, float32->double for SQLite)
- Elixir exqlite: updated to Exqlite 0.36 prepare/bind/step API
- Elixir myxql/exqlite/ecto: generated code now properly wrapped in `defmodule`
- Python aiomysql: `?` placeholders correctly rewritten to `%s`
- Go pgx: added missing `time` and `decimal` imports in generated code
- Ruby trilogy: parameterized queries use string interpolation (trilogy lacks prepared statement support)
- TypeScript pg-zod: enum columns use correct Zod schema references

## [0.6.3] - 2026-04-10

### Added

- `fmt` and `lint` commands now auto-detect the SQL dialect from the config `engine` field (CLI `--dialect` flag still takes precedence)

### Fixed

- Sqruff rule `LT01` excluded by default — it incorrectly splits compound operators (`>=`, `<=`, `<@`)
- Compound operators inside CHECK constraints no longer get split by the formatter (e.g., `>=` becoming `> =`)

## [0.6.2] - 2026-04-10

### Changed

- tokio-postgres: `from_row` is now infallible (returns `Self` instead of `Result`) matching tokio-postgres `row.get()` conventions
- tokio-postgres: all query functions uniformly return `Result<T, tokio_postgres::Error>` instead of mixed error types
- tokio-postgres: extracted `ERROR_TYPE` constant to reduce string duplication in signatures

### Fixed

- `:opt` command now correctly generates row structs (was missing from struct generation match)

## [0.6.1] - 2026-04-10

### Added

- `:opt` query command across all backends — returns optional/nullable single row (distinct from `:one` which expects exactly one row)
- Serde and custom derive support for tokio-postgres backend via `serde` and `derive` options
- `apply_options()` method on tokio-postgres backend for runtime configuration
- `is_column_nullable()` helper on analyzer scope for nullable column lookups
- `collect_param_from_expr_with_type_nullable()` for nullable-aware parameter collection
- `version:sync` task in Taskfile for updating all crate versions at once

### Changed

- tokio-postgres: `client` parameter now accepts `&(impl GenericClient + Sync)` instead of concrete `&Client`
- tokio-postgres: batch functions no longer wrap operations in implicit transactions

### Fixed

- INSERT parameter analysis now propagates column nullability to parameters
- Changelog retroactively aligned with Cargo.toml version history (0.1.0–0.6.0)

## [0.6.0] - 2026-04-08

### Added

- Microsoft SQL Server engine (6 backends: tiberius, pyodbc, mssql, sqlclient, tiny_tds, tds)
- Oracle Database engine (6 backends: sibyl, oracledb, godror, oracle, oci8, jamdb)
- MariaDB engine with native UUID support, RETURNING clause, and dedicated manifests
- Amazon Redshift engine (PostgreSQL-based with SUPER type support)
- Snowflake engine with VARIANT/OBJECT/ARRAY types
- 17 new database backends and 51 type mapping manifests
- Pre-commit/prek hooks for scythe users

### Changed

- Flattened docs structure for better organization
- Expanded to 10 total databases with 70+ backend drivers across 10 languages

### Fixed

- Extracted shared `rewrite_pg_placeholders` function (eliminated 26+ duplicated functions)
- Extracted shared `load_or_default_manifest` function (eliminated 49 duplicated code blocks)
- CockroachDB documentation TOML snippet duplicate key issue
- Python DuckDB missing datetime import
- TypeScript DuckDB import type issue
- Go godror PascalCase conversion issue
- Go unconditional imports problem
- SQLx hardcoded PgPool issue
- Tiberius unwrap error handling
- Kotlin wasNull null handling
- Ruby batch operation fix
- Sibyl error swallowing issue
- Go `interface{}` updated to `any` keyword

## [0.5.0] - 2026-04-08

### Added

- CockroachDB engine support
- DuckDB engine support
- `:grouped` operation support
- Kotlin Exposed backend
- R2DBC backend support
- Homebrew bottles for distribution
- Integration test generator for all 39 backend test suites

## [0.4.0] - 2026-04-08

### Added

- Real `:batch` operations across all backends
- PHP AMPHP backend
- Custom type overrides feature
- `@optional` annotation support
- Elixir Ecto backend
- Ruby Trilogy backend
- Pydantic/msgspec row types for Python
- Zod v4 schemas for TypeScript
- GenOptions infrastructure for per-backend configuration

### Changed

- Extended Quick Start documentation with all 10 languages

## [0.3.0] - 2026-04-07

### Added

- Snippet-runner tool for validating documentation code snippets across 13 languages
- PHP namespace support and Generator for `:many` queries
- C# SQLite async API
- Ruby module `Queries` encapsulation across all 3 backends

### Changed

- C# all backends: Enum.TryParse with descriptive InvalidOperationException
- Python aiosqlite: Decimal maps to `decimal.Decimal` instead of float
- Go database-sql MySQL: Decimal maps to float64
- Ruby: SCREAMING_SNAKE_CASE enum variants
- PHP: Final class `Queries` wrapper

### Fixed

- 8 backend-specific fixes across PHP, Ruby, C#, Rust, Python, and Go

## [0.2.0] - 2026-04-07

### Added

- Engine-aware backend architecture — `get_backend(name, engine)` loads engine-specific manifests
- 12 new language backends for MySQL and SQLite: go-database-sql, python-aiomysql, python-aiosqlite, typescript-mysql2, typescript-better-sqlite3, ruby-mysql2, ruby-sqlite3, csharp-mysqlconnector, csharp-microsoft-sqlite, elixir-myxql, elixir-exqlite
- Multi-backend CLI config via `[[sql.gen]]` array syntax in scythe.toml
- Full MySQL support across all 10 languages (Rust, Go, Python, TypeScript, Java, Kotlin, C#, Elixir, Ruby, PHP)
- Full SQLite support across all 10 languages
- 33 real integration tests against PostgreSQL, MySQL, and SQLite
- `supported_engines()` method on CodegenBackend trait for engine validation
- `manifest()` method on CodegenBackend trait for direct manifest access
- `file_footer()` method on CodegenBackend trait for class wrappers (C#)
- Engine-specific manifest files for multi-DB backends (java-jdbc, kotlin-jdbc, php-pdo, rust-sqlx)
- Docker Compose setup for integration testing (PostgreSQL + MySQL)

### Changed

- `get_backend()` now requires engine parameter for database-aware code generation
- Backend constructors accept engine parameter and load appropriate manifests
- PG-only backends reject non-PostgreSQL engines with clear error messages

### Fixed

- Python codegen: multiline SQL now uses triple-quoted strings
- Python codegen: added missing `import decimal` to file headers
- TypeScript pg codegen: multiline SQL now uses backtick template literals
- C# codegen: generated code now wrapped in `public static class Queries { }`
- C# codegen: enum parameters use `.ToString().ToLower()` with `::enum_type` SQL cast
- C# codegen: enum columns deserialized via `Enum.Parse<T>(reader.GetString(i), true)`
- PHP codegen: MySQL `?` placeholders use positional arrays instead of named params
- PHP codegen: enum params use `->value`, enum columns use `::from()`, DateTimeImmutable for timestamps
- Go codegen: added missing `time` and `decimal` imports to file header
- Java codegen: added import statements to file header
- Ruby mysql2 codegen: `affected_rows` called on statement instead of client

## [0.1.0] - 2026-04-06

### Added

- SQL-to-code generation for 13 language backends:
  - Rust (sqlx, tokio-postgres)
  - Python (psycopg3, asyncpg)
  - TypeScript (postgres.js, pg)
  - Go (pgx v5)
  - Java (JDBC with records)
  - Kotlin (JDBC with data classes)
  - C# (Npgsql with records)
  - Elixir (Postgrex with defstruct)
  - Ruby (pg gem with Data.define)
  - PHP (PDO with readonly classes)
- Database dialect support: PostgreSQL, MySQL, SQLite
- SQL annotation system (@name, @returns, @param, @nullable, @nonnull, @json, @deprecated)
- Smart type inference with nullability propagation (JOIN, COALESCE, aggregates, CASE)
- Language-neutral type vocabulary with per-backend type mapping via manifest.toml
- 93 SQL lint rules (22 scythe-specific + 71 via sqruff integration)
- SQL formatting via sqruff integration
- CLI commands: generate, check, lint, fmt, migrate
- sqlc migration tool (convert sqlc.yaml to scythe.toml, migrate query annotations)
- 275 JSON test fixtures with auto-generated test code
- Real language tool validation (ruff, biome, gofmt, ktlint, ruby -c, php -l)
- Template-based backend architecture (manifest.toml + MiniJinja templates)
- Trait-based CodegenBackend for extensible language support
