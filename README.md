<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/Goldziher/scythe/main/logo.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/Goldziher/scythe/main/logo-dark.svg">
    <img width="400" alt="Scythe" src="https://raw.githubusercontent.com/Goldziher/scythe/main/logo-dark.svg" />
  </picture>

**Write SQL. Get type-safe code. In ten languages.**

Scythe compiles annotated `.sql` files into database access code — row types, query functions and
type mappings — that stays in sync with your schema. It reads your SQL statically, so generation
needs no database connection and no network: it runs in a pre-commit hook.

10 languages · 10 databases · 56 backends · 58 lint and audit rules · nullability inferred through
JOINs, CTEs and window functions

[![crates.io](https://img.shields.io/crates/v/scythe-cli?label=crates.io&color=007ec6)](https://crates.io/crates/scythe-cli)
[![Homebrew](https://img.shields.io/badge/Homebrew-tap-007ec6)](https://github.com/Goldziher/homebrew-tap)
[![CI](https://img.shields.io/github/actions/workflow/status/Goldziher/scythe/ci.yml?label=CI&color=007ec6)](https://github.com/Goldziher/scythe/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-007ec6)](https://github.com/Goldziher/scythe/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-online-blue)](https://goldziher.github.io/scythe)
[![Discord](https://img.shields.io/badge/Discord-Join%20our%20community-7289da?logo=discord&logoColor=white)](https://discord.gg/xt9WY3GnKR)

[Docs](https://goldziher.github.io/scythe) · [Install](#installation) · [What you get](#what-you-get) · [Quick start](#quick-start) · [Commands](#commands) · [Compare](https://goldziher.github.io/scythe/comparisons/alternatives/)

</div>

---

## Why scythe

Every application that talks to a database needs glue: code that maps parameters in, maps result
rows out, and keeps the types aligned. It is tedious, easy to get subtly wrong, and it changes every
time the schema does.

Scythe deletes that layer. You keep writing SQL — the language your database already optimizes and
your team already knows — and the mapping code is compiled from it. The generated code is readable,
has no runtime dependency beyond your driver, and carries a provenance header so `scythe check` can
tell you when it has drifted from the schema it came from.

**Where an ORM still wins:** if your users choose the database, an ORM abstracts dialect differences
at runtime and scythe does not. Scythe targets one engine per configuration block, which is what
buys you engine-specific features and a real query planner. If you control the database, that trade
is worth making.

## What you get

| | What it does | Docs |
|---|---|---|
| **Type inference that reads the query** | Nullability propagated through `LEFT`/`RIGHT`/`FULL` joins, `COALESCE`, `CASE WHEN`, aggregates and window functions — not just column constraints. CTEs (including recursive), `RETURNING`, enums, composites, arrays and JSON map to language-native types. | [Type inference](https://goldziher.github.io/scythe/guide/type-inference/) |
| **58 built-in rules** | 23 lint rules (`UPDATE` without `WHERE`, `NULL` compared with `=`, leading-wildcard `LIKE`, `SELECT *`) and 35 audit rules, plus sqruff's style rules through `scythe fmt`. | [Lint rules](https://goldziher.github.io/scythe/reference/lint-rules/) · [Linting](https://goldziher.github.io/scythe/guide/linting/) |
| **`scythe audit`** | Security and migration-safety scanning: dangerous functions, `GRANT` to `PUBLIC`, literal passwords, `SELECT *` over PII, plus 19 migration rules for locking `ALTER`s and destructive DDL. Human, SARIF or JSON. | [Audit](https://goldziher.github.io/scythe/guide/audit/) |
| **`scythe check`** | Verifies committed code still matches the SQL it was generated from, via 8 provenance rules. Given `--database-url` it adds 7 schema-drift rules against a live PostgreSQL catalog. | [CLI reference](https://goldziher.github.io/scythe/guide/cli-reference/) |
| **`scythe inspect`** | Live-database health checks: foreign keys without covering indexes, tables with policies but RLS disabled, duplicate indexes. PostgreSQL (13 checks) and MySQL/MariaDB (4 checks). | [Inspect](https://goldziher.github.io/scythe/guide/inspect/) |
| **Output you can shape** | Row types as Pydantic, msgspec, dataclasses, Zod schemas or plain interfaces, depending on backend; `structs_only` (TypeScript and `rust-sqlx`) for a types-only package; type overrides for `ltree`, `citext` or PostGIS. | [Configuration](https://goldziher.github.io/scythe/guide/configuration/) · [Custom types](https://goldziher.github.io/scythe/guide/custom-types/) |
| **Annotations beyond `:one` / `:many`** | `@optional` compiles a parameter into a conditional filter, `:batch` for bulk operations, `@returns :grouped` with `@group_by` for nested results. | [Annotations](https://goldziher.github.io/scythe/guide/annotations/) |
| **Coming from sqlc?** | `scythe migrate` converts an existing `sqlc.yaml` into a `scythe.toml`. | [Migration from sqlc](https://goldziher.github.io/scythe/getting-started/migration-from-sqlc/) |

## Installation

```bash
cargo install scythe-cli
cargo binstall scythe-cli              # prebuilt binary, no compile
brew install Goldziher/tap/scythe
```

No Rust toolchain required — both wrappers download the prebuilt binary for your platform and verify
its checksum:

```bash
npm install --save-dev scythe-cli
pip install scythe-sql
```

See [Installation](https://goldziher.github.io/scythe/getting-started/installation/) for supported
platforms, proxy configuration and cache control.

<details>
<summary><strong>Pre-commit / prek hooks</strong></summary>

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.15.0
    hooks:
      - id: scythe-fmt        # format SQL
      - id: scythe-lint       # lint with auto-fix
      - id: scythe-audit      # SC-SEC*/SC-RLS*/SC-MIG*/SC-CHK*
      - id: scythe-inspect    # live-DB checks, needs $DATABASE_URL
      - id: scythe-generate   # regenerate on SQL changes
      - id: scythe-check      # validate without generating
```

The hooks declare `language: rust`, so pre-commit compiles scythe from source on first use — a few
minutes. If you already have the binary on `PATH` (via `brew`, `npm` or `pip` above), add
`language: system` to a hook to use it directly instead.

See [Pre-commit hooks](https://goldziher.github.io/scythe/guide/pre-commit-hooks/) for the full
table and per-hook configuration.

</details>

## Quick start

**1. Annotate a query.** `status` is an enum column; `o.total` and `o.notes` sit on the right side of
a `LEFT JOIN`.

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;
```

**2. Point `scythe.toml` at it.**

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "python-psycopg3"
output = "src/generated"
```

**3. Generate.**

```bash
scythe generate
```

**4. Use it.** `total` and `notes` are `| None` because the join can fail to match; `id` and `name`
are not. The enum became a real `UserStatus`, and the parameter is typed with it.

```python
@dataclass(frozen=True, slots=True)
class GetUserOrdersRow:
    """Row type for GetUserOrders query."""

    id: int
    name: str
    total: decimal.Decimal | None
    notes: str | None


async def get_user_orders(conn: AsyncConnection, *, status: UserStatus) -> list[GetUserOrdersRow]:
    """Execute GetUserOrders query."""
```

<details>
<summary><strong>The same row type in all ten languages</strong></summary>

Every block below is real output for the query above, taken from the projects scythe's CI compiles
on every run. Note where each language expresses "nullable" differently, and how `NUMERIC` lands on
the right native decimal type in each.

**Rust** (`rust-sqlx`)

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserOrdersRow {
    pub id: i32,
    pub name: String,
    pub total: Option<rust_decimal::Decimal>,
    pub notes: Option<String>,
}
```

**TypeScript** (`typescript-pg`)

```typescript
export interface GetUserOrdersRow {
	id: number;
	name: string;
	total: string | null;
	notes: string | null;
}
```

**Go** (`go-pgx`)

```go
type GetUserOrdersRow struct {
	Id int32 `json:"id"`
	Name string `json:"name"`
	Total *decimal.Decimal `json:"total"`
	Notes *string `json:"notes"`
}
```

**Java** (`java-jdbc`)

```java
public record GetUserOrdersRow(
    int id,
    String name,
    @Nullable java.math.BigDecimal total,
    @Nullable String notes
) {}
```

**Kotlin** (`kotlin-jdbc`)

```kotlin
data class GetUserOrdersRow(
    val id: Int,
    val name: String,
    val total: java.math.BigDecimal?,
    val notes: String?,
)
```

**C#** (`csharp-npgsql`)

```csharp
public record GetUserOrdersRow(
    int Id,
    string Name,
    decimal? Total,
    string? Notes
);
```

**Elixir** (`elixir-postgrex`)

```elixir
defmodule GetUserOrdersRow do
  @moduledoc "Row type for GetUserOrders queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    total: Decimal.t() | nil,
    notes: String.t() | nil
  }
  defstruct [:id, :name, :total, :notes]
end
```

**Ruby** (`ruby-pg`)

```ruby
GetUserOrdersRow = Data.define(:id, :name, :total, :notes)
```

**PHP** (`php-pdo`)

```php
readonly class GetUserOrdersRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $total,
        public ?string $notes,
    ) {}
}
```

Plain JavaScript is available too: the four `javascript-*` backends emit ESM `.js` with the types
carried in JSDoc, checkable under `tsc --checkJs --strict` with no build step.

The [quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) walks the whole
flow with full function bodies for every language.

</details>

## Commands

```bash
scythe generate                       # compile SQL to code
scythe check                          # is the committed code still in sync?
scythe lint sql/                      # 23 correctness and performance rules
scythe audit sql/ --format sarif      # 35 security and migration-safety rules
scythe fmt sql/                       # format via sqruff
scythe inspect $DATABASE_URL          # live-database health checks
scythe migrate sqlc.yaml              # convert an sqlc config
```

`check` and `audit` exit `2` on error-severity findings and `1` on operational failure, so CI can
tell the two apart. Full flags and exit codes are in the
[CLI reference](https://goldziher.github.io/scythe/guide/cli-reference/).

## Language and database support

Ten languages — Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby and PHP, plus plain
JavaScript via the four `javascript-*` backends — across PostgreSQL, MySQL, MariaDB, SQLite, DuckDB,
CockroachDB, MSSQL, Oracle, Redshift and Snowflake. **Coverage is not uniform**: not every language
has a driver for every engine.

<details>
<summary><strong>Driver matrix</strong></summary>

| Language | PostgreSQL | MySQL | SQLite | DuckDB | CockroachDB | MSSQL | Oracle | MariaDB | Redshift | Snowflake |
|----------|-----------|-------|--------|--------|-------------|-------|--------|---------|----------|-----------|
| Rust | sqlx, tokio-postgres | sqlx | sqlx | -- | sqlx, tokio-postgres | tiberius | sibyl | sqlx | sqlx, tokio-postgres | -- |
| Python | psycopg3, asyncpg | aiomysql | aiosqlite | duckdb | psycopg3, asyncpg | pyodbc | oracledb | aiomysql | psycopg3, asyncpg | snowflake-connector |
| TypeScript | postgres.js, pg, Kysely | mysql2, Kysely | better-sqlite3, Kysely, node:sqlite, wasm-sqlite | duckdb-node | postgres.js, pg, Kysely | mssql, Kysely | oracledb | mysql2, Kysely | postgres.js, pg, Kysely | snowflake-sdk |
| JavaScript | postgres.js, pg | mysql2 | better-sqlite3 | -- | postgres.js, pg | -- | -- | mysql2 | postgres.js, pg | -- |
| Go | pgx | database/sql | database/sql | database/sql | pgx | database/sql | godror | database/sql | pgx | gosnowflake |
| Java | JDBC, R2DBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC, R2DBC | JDBC | JDBC | JDBC, R2DBC | JDBC | JDBC |
| Kotlin | JDBC, R2DBC, Exposed | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC, R2DBC, Exposed | JDBC | JDBC | JDBC, R2DBC | JDBC | JDBC |
| C# | Npgsql | MySqlConnector | Microsoft.Data.Sqlite | -- | Npgsql | Microsoft.Data.SqlClient | ODP.NET | MySqlConnector | Npgsql | Snowflake.Data |
| Elixir | Postgrex, Ecto | MyXQL | Exqlite | -- | Postgrex, Ecto | tds | jamdb_oracle | MyXQL | Postgrex | -- |
| Ruby | pg | mysql2, trilogy | sqlite3 | -- | pg | tiny_tds | ruby-oci8 | mysql2, trilogy | pg | -- |
| PHP | PDO, AMPHP | PDO, AMPHP | PDO | -- | PDO, AMPHP | PDO | -- | PDO, AMPHP | PDO | PDO |

The CockroachDB column matches PostgreSQL by construction: `normalize_engine` folds `cockroachdb`
into `postgresql` before a backend's engine support is consulted. Redshift does not fold that way —
it needs a per-backend `*.redshift.toml` manifest, which is why its column is narrower.

See the [backend overview](https://goldziher.github.io/scythe/backends/overview/) for per-backend
options and emitted-code notes.

</details>

## Documentation

Full documentation at [goldziher.github.io/scythe](https://goldziher.github.io/scythe).

- [Quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) — zero to generated code, in every language
- [Philosophy](https://goldziher.github.io/scythe/philosophy/) — why compile SQL instead of using an ORM
- [Alternatives](https://goldziher.github.io/scythe/comparisons/alternatives/) — scythe against sqlc, SQLDelight, jOOQ and ORMs
- [Configuration](https://goldziher.github.io/scythe/guide/configuration/) — the full `scythe.toml` reference
- [Annotations](https://goldziher.github.io/scythe/guide/annotations/) — `@name`, `@returns`, `@optional`, `@nullable`, `@json`
- [Type inference](https://goldziher.github.io/scythe/guide/type-inference/) — how nullability and types are derived
- [Custom types](https://goldziher.github.io/scythe/guide/custom-types/) — overrides for extension and domain types
- [CLI reference](https://goldziher.github.io/scythe/guide/cli-reference/) — every command, flag and exit code

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, architecture, and how to add backends, engines or
lint rules.

## License

[MIT](LICENSE)
