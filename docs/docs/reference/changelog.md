# Changelog

Scythe follows [Keep a Changelog](https://keepachangelog.com/) and [Semantic Versioning](https://semver.org/).

For the latest changes, see the [CHANGELOG.md](https://github.com/basemind-ai/scythe/blob/main/CHANGELOG.md) in the repository root.

## [0.5.0] - 2026-04-08

### Added

- **DuckDB engine support** -- 5 backends (python-duckdb, typescript-duckdb, go-database-sql, java-jdbc, kotlin-jdbc) for embedded analytical database
- **CockroachDB engine support** -- PostgreSQL-compatible alias, all PostgreSQL backends automatically work
- **Java R2DBC backend** -- reactive database access with Project Reactor (Mono/Flux)
- **Kotlin R2DBC backend** -- reactive database access with coroutines (suspend/Flow)
- **Kotlin Exposed backend** -- generates Exposed Table objects with transaction{}/exec() query pattern
- **@returns :grouped** -- new return type for nested result structs with `@group_by` annotation
- **Homebrew bottles** -- pre-built binaries via goreleaser brews section (faster brew install)
- **Integration test generator** -- generates all 39+ backend test suites from minijinja templates

## [0.4.0] - 2026-04-08

### Added

- **Pydantic v2 row types** -- `row_type = "pydantic"` in `[[sql.gen]]` generates `BaseModel` classes instead of `@dataclass` for all 4 Python backends
- **msgspec row types** -- `row_type = "msgspec"` generates `msgspec.Struct` classes for all 4 Python backends
- **Zod v4 schemas** -- `row_type = "zod"` in `[[sql.gen]]` generates `z.object()` schemas with `z.infer<>` types for all 4 TypeScript backends
- **`@optional` annotation** -- dynamic WHERE clauses via `@optional param_name`, rewrites `col = $1` to `($1 IS NULL OR col = $1)`. Supports =, <>, !=, >, <, >=, <=, LIKE, ILIKE operators. Validates param names at analysis time.
- **Real `:batch` operations** -- `:batch` now generates dedicated batch functions with transaction wrapping and driver-specific batch APIs (executemany, addBatch, etc.) across all backends
- **Custom type overrides** -- `type_overrides` in scythe.toml now functional (previously parsed but unused). Column-level and db_type-level overrides intercept type resolution.
- **Ruby Trilogy backend** -- new `ruby-trilogy` backend for GitHub's Trilogy MySQL client
- **Elixir Ecto backend** -- new `elixir-ecto` backend generating Ecto-compatible query modules with `Ecto.Adapters.SQL.query`
- **PHP AMPHP backend** -- new `php-amphp` backend for async PHP with `Amp\Sql\SqlConnectionPool`
- **GenOptions infrastructure** -- per-backend configuration via `[[sql.gen]]` extra keys (enables row_type, future options)
- **11 unit tests** for `rewrite_optional_params` SQL rewriting
- **7 snapshot tests** for new backends and row type variants

### Fixed

- **Ruby Trilogy batch** -- no longer generates double function signature
- **Elixir Ecto @optional** -- properly uses SQL rewriting for optional params
- **PHP AMPHP batch** -- generates transaction-wrapped batch function instead of generator
- **Python ExecResult/ExecRows** -- now returns `int` (row count) instead of `None` for all 4 Python backends
- **@optional qualified columns** -- handles `t.col = $1` patterns (table-qualified names)
- **@optional param validation** -- errors on unknown param names instead of silent ignore
- **Elixir Ecto Multi.run** -- callback uses transaction's repo parameter
- **PHP AMPHP MySQL manifest** -- added missing `range` container
- **Python import ordering** -- pydantic/msgspec imports correctly separated from stdlib

## [0.3.0] - 2026-04-07

### Added

- **Snippet runner tool** -- Rust CLI that validates documentation code snippets across 13 languages (syntax, compile, run levels). Adapted from kreuzberg, supports HTML comment annotations (`<!-- snippet:skip -->`, etc.)
- **SCREAMING_SNAKE_CASE** support in `apply_case` for enum variant naming conventions

### Changed

- **PHP (breaking)**: generated code now uses `namespace App\Generated`, wraps query functions in `final class Queries` with `public static` methods, `:many` queries return `\Generator` instead of `array`
- **Ruby (breaking)**: all generated code wrapped in `module Queries ... end`; call via `Queries.method_name` instead of bare functions
- **C# SQLite (breaking)**: generated code now uses async API (`ExecuteReaderAsync`, `ReadAsync`, etc.) matching the Npgsql and MySqlConnector backends

### Fixed

- **PHP**: functions no longer in global namespace; result sets use lazy generators instead of eager `fetchAll()`
- **Ruby**: enum constants now correctly use SCREAMING_SNAKE_CASE (`ACTIVE` instead of `active`)
- **Ruby mysql2**: `affected_rows` now called on `client` instead of `stmt`
- **Rust tokio-postgres**: enum parsing uses descriptive panic message instead of bare `.unwrap()`
- **C# all backends**: `Enum.Parse` replaced with `Enum.TryParse` + `InvalidOperationException` for clearer error messages
- **Python aiosqlite**: `decimal` type now maps to `decimal.Decimal` instead of `float`
- **Go database-sql MySQL**: `decimal` type now maps to `float64` instead of `string`
- **Go database-sql MySQL**: added missing `time` import rule for `time.Time` types

## [0.2.0] - 2026-04-07

### Added

- Engine-aware backend architecture -- `get_backend(name, engine)` loads engine-specific manifests
- 12 new language backends for MySQL and SQLite: go-database-sql, python-aiomysql, python-aiosqlite, typescript-mysql2, typescript-better-sqlite3, ruby-mysql2, ruby-sqlite3, csharp-mysqlconnector, csharp-microsoft-sqlite, elixir-myxql, elixir-exqlite
- Multi-backend CLI config via `[[sql.gen]]` array syntax in scythe.toml
- Full MySQL and SQLite support across all 10 languages
- 33 real integration tests against PostgreSQL, MySQL, and SQLite
- `supported_engines()`, `manifest()`, and `file_footer()` methods on CodegenBackend trait

### Changed

- `get_backend()` now requires engine parameter for database-aware code generation
- Backend constructors accept engine parameter and load appropriate manifests
- PG-only backends reject non-PostgreSQL engines with clear error messages

### Fixed

- Python codegen: multiline SQL now uses triple-quoted strings, added missing `import decimal`
- TypeScript pg codegen: multiline SQL now uses backtick template literals
- C# codegen: generated code wrapped in `public static class Queries { }`, enum handling improvements
- PHP codegen: MySQL `?` placeholders use positional arrays, enum and DateTimeImmutable handling
- Go codegen: added missing `time` and `decimal` imports
- Java codegen: added import statements to file header
- Ruby mysql2 codegen: `affected_rows` called on statement instead of client

## [0.1.0] - 2026-04-06

### Added

- **SQL analysis**: Full PostgreSQL, MySQL, and SQLite dialect support
- **Code generation**: 13 backends across Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby, and PHP
- **Linting**: 22 built-in rules + 71 sqruff rule categories
- **Formatting**: SQL formatting via sqruff integration
- **Type system**: Neutral type abstraction with automatic SQL-to-language type mapping
- **Catalog**: DDL parsing for tables, enums, composites, domains, and views
