<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/Goldziher/scythe/main/logo.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/Goldziher/scythe/main/logo-dark.svg">
    <img width="400" alt="Scythe" src="https://raw.githubusercontent.com/Goldziher/scythe/main/logo-dark.svg" />
  </picture>

  **Write SQL. Generate type-safe code. In any language.**

<div style="display: flex; flex-wrap: wrap; gap: 8px; justify-content: center; margin: 20px 0;">

  <a href="https://crates.io/crates/scythe-cli">
    <img src="https://img.shields.io/crates/v/scythe-cli?label=crates.io&color=007ec6" alt="crates.io">
  </a>
  <a href="https://github.com/Goldziher/homebrew-tap">
    <img src="https://img.shields.io/badge/Homebrew-tap-007ec6" alt="Homebrew">
  </a>
  <a href="https://github.com/Goldziher/scythe/actions/workflows/ci.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/Goldziher/scythe/ci.yml?label=CI&color=007ec6" alt="CI">
  </a>
  <a href="https://github.com/Goldziher/scythe/blob/main/LICENSE">
    <img src="https://img.shields.io/badge/License-MIT-007ec6" alt="License">
  </a>
  <a href="https://goldziher.github.io/scythe">
    <img src="https://img.shields.io/badge/docs-online-blue" alt="Docs">
  </a>
  <a href="https://discord.gg/xt9WY3GnKR">
    <img src="https://img.shields.io/badge/Discord-Join%20our%20community-7289da?logo=discord&logoColor=white" alt="Discord">
  </a>

</div>
</div>

---

Scythe compiles annotated SQL into type-safe database access code. You write SQL queries, scythe generates the boilerplate -- structs, functions, type mappings -- in 10 languages across 10 databases with 56 backends. Built-in linting (58 rules) and formatting catch SQL bugs before they ship.

## Installation

```bash
cargo install scythe-cli
# or, using pre-built binaries for a faster install:
cargo binstall scythe-cli
# or
brew install Goldziher/tap/scythe  # uses pre-built binaries for faster install
```

No Rust toolchain required — both wrappers download the prebuilt binary for your platform and
verify its checksum:

```bash
npm install --save-dev scythe-cli   # Node.js, pins scythe alongside your other dev tools
pip install scythe-sql              # Python
```

See [Installation](https://goldziher.github.io/scythe/getting-started/installation/) for supported
platforms, proxy configuration and cache control.

## Pre-commit / prek

Scythe provides [pre-commit](https://pre-commit.com/) / [prek](https://github.com/j178/prek) hooks for SQL formatting and linting:

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.14.0
    hooks:
      - id: scythe-fmt       # Format SQL files
      - id: scythe-lint      # Lint SQL with auto-fix (includes audit rules when scythe.toml is present)
      - id: scythe-audit     # SC-SEC*/SC-RLS*/SC-MIG*/SC-CHK* on every staged .sql file
      - id: scythe-inspect   # SC-INS* live-DB health checks (CI mode, needs $DATABASE_URL)
      - id: scythe-generate  # Regenerate code on SQL changes
      - id: scythe-check     # Validate SQL without generating
```

See [Pre-commit Hooks](https://goldziher.github.io/scythe/guide/pre-commit-hooks/) for all available hooks and configuration options.

## Quick Start

**1. Write annotated SQL queries:**

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;
```

**2. Configure `scythe.toml`:**

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]
output = "src/generated"

[[sql.gen]]
backend = "rust-sqlx"
```

**3. Generate code:**

```bash
scythe generate
```

**4. Use the generated code:**

Scythe knows `o.total` and `o.notes` are nullable (right side of LEFT JOIN) and generates precise types:

<details>
<summary><strong>Rust (sqlx)</strong></summary>

```rust
#[derive(Debug, sqlx::FromRow)]
pub struct GetUserOrdersRow {
    pub id: i32,
    pub name: String,
    pub total: Option<rust_decimal::Decimal>,
    pub notes: Option<String>,
}

pub async fn get_user_orders(
    pool: &sqlx::PgPool, status: &str,
) -> Result<Vec<GetUserOrdersRow>, sqlx::Error> {
    sqlx::query_as!(GetUserOrdersRow,
        "SELECT u.id, u.name, o.total, o.notes
         FROM users u LEFT JOIN orders o ON u.id = o.user_id
         WHERE u.status = $1", status)
        .fetch_all(pool).await
}
```

</details>

<details>
<summary><strong>Python (psycopg3)</strong></summary>

```python
@dataclass
class GetUserOrdersRow:
    id: int
    name: str
    total: decimal.Decimal | None
    notes: str | None

async def get_user_orders(
    conn: AsyncConnection, *, status: str,
) -> list[GetUserOrdersRow]:
    cur = await conn.execute(
        "SELECT u.id, u.name, o.total, o.notes "
        "FROM users u LEFT JOIN orders o ON u.id = o.user_id "
        "WHERE u.status = %(status)s",
        {"status": status},
    )
    rows = await cur.fetchall()
    return [GetUserOrdersRow(id=r[0], name=r[1], total=r[2], notes=r[3]) for r in rows]
```

</details>

<details>
<summary><strong>TypeScript (pg)</strong></summary>

```typescript
interface GetUserOrdersRow {
    id: number;
    name: string;
    total: string | null;
    notes: string | null;
}

export async function getUserOrders(
    client: PoolClient, status: string,
): Promise<GetUserOrdersRow[]> {
    const { rows } = await client.query<GetUserOrdersRow>(
        `SELECT u.id, u.name, o.total, o.notes
         FROM users u LEFT JOIN orders o ON u.id = o.user_id
         WHERE u.status = $1`, [status]);
    return rows;
}
```

</details>

<details>
<summary><strong>Go (pgx)</strong></summary>

```go
type GetUserOrdersRow struct {
    ID    int32   `json:"id"`
    Name  string  `json:"name"`
    Total *string `json:"total"`
    Notes *string `json:"notes"`
}

func GetUserOrders(ctx context.Context, pool *pgxpool.Pool, status string) ([]GetUserOrdersRow, error) {
    rows, err := pool.Query(ctx,
        "SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = $1",
        status)
    // ... scan rows into []GetUserOrdersRow
}
```

</details>

<details>
<summary><strong>Java (JDBC)</strong></summary>

```java
public record GetUserOrdersRow(
    int id,
    String name,
    @Nullable java.math.BigDecimal total,
    @Nullable String notes
) {}

public static List<GetUserOrdersRow> getUserOrders(
    Connection conn, String status
) throws SQLException {
    // PreparedStatement + ResultSet scanning
}
```

</details>

<details>
<summary><strong>Kotlin (JDBC)</strong></summary>

```kotlin
data class GetUserOrdersRow(
    val id: Int,
    val name: String,
    val total: java.math.BigDecimal?,
    val notes: String?,
)

fun getUserOrders(conn: Connection, status: String): List<GetUserOrdersRow> {
    conn.prepareStatement("...").use { ps ->
        ps.setObject(1, status)
        ps.executeQuery().use { rs -> /* scan rows */ }
    }
}
```

</details>

<details>
<summary><strong>C# (Npgsql)</strong></summary>

```csharp
public record GetUserOrdersRow(
    int Id, string Name, decimal? Total, string? Notes
);

public static async Task<List<GetUserOrdersRow>> GetUserOrders(
    NpgsqlConnection conn, string status
) {
    await using var cmd = new NpgsqlCommand("...", conn);
    cmd.Parameters.AddWithValue("p1", status);
    await using var reader = await cmd.ExecuteReaderAsync();
    // read rows
}
```

</details>

<details>
<summary><strong>Elixir (Postgrex)</strong></summary>

```elixir
defmodule GetUserOrdersRow do
  @type t :: %__MODULE__{
    id: integer(), name: String.t(),
    total: Decimal.t() | nil, notes: String.t() | nil
  }
  defstruct [:id, :name, :total, :notes]
end

@spec get_user_orders(pid(), String.t()) :: {:ok, [%GetUserOrdersRow{}]} | {:error, term()}
def get_user_orders(conn, status) do
  case Postgrex.query(conn, "...", [status]) do
    {:ok, %{rows: rows}} -> {:ok, Enum.map(rows, &to_struct/1)}
    {:error, err} -> {:error, err}
  end
end
```

</details>

<details>
<summary><strong>Ruby (pg)</strong></summary>

```ruby
module Queries
  GetUserOrdersRow = Data.define(:id, :name, :total, :notes)

  def self.get_user_orders(conn, status)
    result = conn.exec_params(
      "SELECT u.id, u.name, o.total, o.notes ...", [status])
    result.map do |row|
      GetUserOrdersRow.new(
        id: row["id"].to_i, name: row["name"],
        total: row["total"], notes: row["notes"])
    end
  end
end
```

</details>

<details>
<summary><strong>PHP (PDO)</strong></summary>

```php
readonly class GetUserOrdersRow {
    public function __construct(
        public int $id, public string $name,
        public ?string $total, public ?string $notes,
    ) {}
}

final class Queries {
    public static function getUserOrders(
        \PDO $pdo, string $status
    ): \Generator {
        $stmt = $pdo->prepare("SELECT ...");
        $stmt->execute(["p1" => $status]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield GetUserOrdersRow::fromRow($row);
        }
    }
}
```

</details>

See the [full quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) for complete examples with imports and full function bodies.

## Features

- **10 languages** -- Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby, PHP
- **10 databases** -- PostgreSQL, MySQL, SQLite, DuckDB, CockroachDB, MSSQL, Oracle, MariaDB, Redshift, Snowflake
- **56 backends** -- sqlx, tokio-postgres, psycopg3, asyncpg, pg, postgres.js, Kysely, pgx, JDBC, R2DBC, Exposed, Npgsql, PDO, tiberius, oracledb, pyodbc, and more (52 implementations; the four `javascript-*` names are a JSDoc emit mode on the matching TypeScript backend)
- **Dialect-agnostic TypeScript with Kysely** -- `typescript-kysely` compiles to Kysely's `sql` tag, which works unmodified against any Kysely dialect: five pinned dialects (PostgreSQL, MySQL, SQLite, MSSQL, MariaDB) plus a Redshift manifest, and wire-compatible (but unpinned/untested by scythe) third-party dialects such as libsql, PlanetScale, Cloudflare D1, Neon, and PGlite
- **Synchronous TypeScript SQLite backends** -- `typescript-node-sqlite` (Node's built-in `node:sqlite`, zero npm dependencies, needs Node 23.4+ or `--experimental-sqlite` on Node 22) and `typescript-wasm-sqlite` (`@sqlite.org/sqlite-wasm`) emit plain `export function` calls with no `async`/`Promise`
- **JavaScript output via JSDoc** -- `javascript-pg`, `javascript-postgres`, `javascript-mysql2`, and `javascript-better-sqlite3` emit plain ESM `.js` with every type carried in JSDoc comments and no driver import, checkable with `tsc --checkJs --strict` and runnable with no build step
- **58 built-in rules** -- 23 lint rules (UPDATE without WHERE, SELECT *, NULL comparisons, leading wildcard LIKE) and 35 audit rules, plus sqruff's 69 style rules via integration
- **14 check-time rules** -- 7 `SC-PRV*` provenance rules (generated artifact vs. current schema, engine, backend, scythe version) and 7 `SC-DRF*` schema drift rules (committed DDL vs. a live PostgreSQL catalog). Reported only by `scythe check`, counted separately from the 58, and configured from the same `[lint]` table as every other rule
- **`scythe audit`** -- security scanner for SQL: dangerous functions, GRANT ALL, GRANT to PUBLIC, cartesian joins, unbounded LIKE, SECURITY DEFINER without pinned `search_path`, role privilege escalation, literal passwords, weak hashes over credential columns, SELECT * over PII, session-state mutation. Emits human / SARIF / JSON for CI integration
- **`scythe inspect`** -- live-database operational health checks: foreign keys without covering indexes, tables with policies but RLS disabled, duplicate indexes. Connects via `tokio-postgres`, emits the same human / SARIF / JSON reports as audit. (Postgres only at v0.10; MySQL in Phase 3.)
- **SQL formatting** -- consistent style via sqruff integration
- **Smart type inference** -- nullability from JOINs, COALESCE, window functions, CASE WHEN, aggregates
- **`@optional` parameters** -- SQL rewriting for conditional filters (`WHERE ($1 IS NULL OR col = $1)`)
- **`:batch` execution** -- bulk inserts and batch operations
- **`@returns :grouped`** -- result grouping with `@group_by` for grouped query results
- **R2DBC reactive backends** -- non-blocking database access for Java and Kotlin
- **Kotlin Exposed** -- first-class Exposed ORM backend for Kotlin
- **Configurable row types** -- Pydantic, msgspec, Zod, dataclass, interface per backend
- **`structs_only` codegen option** -- emit only row types (interfaces/Zod schemas/enums/composites), no query functions and no driver import; supported by `rust-sqlx` and every TypeScript backend, combinable with `row_type = "zod"` for a types-only package
- **CTEs and window functions** -- ROW_NUMBER, RANK, LAG, LEAD, recursive CTEs with correct type inference
- **Enums, composites, arrays** -- PostgreSQL types mapped to language-native equivalents
- **Custom type overrides** -- map ltree, citext, PostGIS geometry to any target type

## Supported Languages

| Language   | PostgreSQL | MySQL | SQLite | DuckDB | CockroachDB | MSSQL | Oracle | MariaDB | Redshift | Snowflake |
|------------|:----------:|:-----:|:------:|:------:|:-----------:|:-----:|:------:|:-------:|:--------:|:---------:|
| Rust       | sqlx, tokio-postgres | sqlx | sqlx | -- | sqlx | tiberius | sibyl | sqlx | sqlx | -- |
| Python     | psycopg3, asyncpg | aiomysql | aiosqlite | python-duckdb | psycopg3 | pyodbc | oracledb | aiomysql | psycopg3 | snowflake-connector |
| TypeScript | pg, postgres.js, kysely | mysql2, kysely | better-sqlite3, node:sqlite, sqlite-wasm, kysely | typescript-duckdb | pg, kysely | mssql, kysely | oracledb | mysql2, kysely | pg, kysely | snowflake-sdk |
| Go         | pgx | database/sql | database/sql | database/sql | pgx | go-mssqldb | godror | database/sql | pgx | gosnowflake |
| Java       | JDBC, R2DBC | JDBC | JDBC | JDBC | JDBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC | JDBC |
| Kotlin     | JDBC, R2DBC, Exposed | JDBC | JDBC | JDBC | JDBC | JDBC, R2DBC | JDBC, R2DBC | JDBC | JDBC | JDBC |
| C#         | Npgsql | MySqlConnector | Microsoft.Data.Sqlite | -- | Npgsql | Microsoft.Data.SqlClient | ODP.NET | MySqlConnector | Npgsql | Snowflake.Data |
| Ruby       | pg, Trilogy | mysql2, Trilogy | sqlite3 | -- | pg | tiny_tds | ruby-oci8 | mysql2 | pg | -- |
| PHP        | PDO, AMPHP | PDO | PDO | -- | PDO | PDO | -- | PDO | PDO | PDO |
| Elixir     | Postgrex, Ecto | MyXQL | Exqlite | -- | Postgrex | tds | jamdb_oracle | MyXQL | Postgrex | -- |

`kysely` (backend `typescript-kysely`) is dialect-parameterised rather than driver-parameterised: the same generated call site runs against any Kysely `Dialect`. Scythe pins and tests five dialects (PostgreSQL, MySQL, SQLite, MSSQL, MariaDB) plus a Redshift manifest that reuses the PostgreSQL dialect. Third-party dialects -- libsql, PlanetScale, Cloudflare D1, Neon, PGlite, and Node's `node:sqlite` / `@sqlite.org/sqlite-wasm` used as a Kysely dialect -- are wire-compatible but not pinned or tested by scythe.

## Documentation

Full documentation at [goldziher.github.io/scythe](https://goldziher.github.io/scythe):

- [Quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) -- zero to generated code in 5 minutes
- [Philosophy](https://goldziher.github.io/scythe/philosophy/) -- why compile SQL instead of using an ORM
- [Alternatives](https://goldziher.github.io/scythe/comparisons/alternatives/) -- how scythe compares to sqlc, SQLDelight, jOOQ, and ORMs
- [Custom Types](https://goldziher.github.io/scythe/guide/custom-types/) -- type overrides for PostgreSQL extensions
- [Configuration](https://goldziher.github.io/scythe/guide/configuration/) -- full scythe.toml reference
- [Annotations](https://goldziher.github.io/scythe/guide/annotations/) -- @name, @returns, @optional, @nullable, @json, and more
- [Lint Rules](https://goldziher.github.io/scythe/reference/lint-rules/) -- all rules with codes and examples
- [Audit (security)](https://goldziher.github.io/scythe/guide/audit/) -- the `scythe audit` subcommand, suppressions, user-defined rules, and CI integration
- [Inspect (live database)](https://goldziher.github.io/scythe/guide/inspect/) -- the `scythe inspect` subcommand, check catalog, CI integration, phased roadmap

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, architecture, and how to add backends/engines/lint rules.

## License

[MIT](LICENSE)
