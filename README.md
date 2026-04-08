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

Scythe compiles annotated SQL into type-safe database access code. You write SQL queries, scythe generates the boilerplate -- structs, functions, type mappings -- in 10 languages across 5 databases with 34 backend drivers. Built-in linting (93 rules) and formatting catch SQL bugs before they ship.

## Installation

```bash
cargo install scythe-cli
# or
brew install Goldziher/tap/scythe  # uses pre-built binaries for faster install
```

## Pre-commit / prek

Scythe provides [pre-commit](https://pre-commit.com/) / [prek](https://github.com/Goldziher/prek) hooks for SQL formatting and linting:

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.5.0
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
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
- **5 databases** -- PostgreSQL, MySQL, SQLite, DuckDB, CockroachDB
- **34 backend drivers** -- sqlx, tokio-postgres, psycopg3, asyncpg, pg, postgres.js, pgx, JDBC, R2DBC, Exposed, Npgsql, PDO, Trilogy, Ecto, AMPHP, python-duckdb, and more
- **93 lint rules** -- UPDATE without WHERE, SELECT *, NULL comparisons, leading wildcard LIKE, plus 71 sqruff rules
- **SQL formatting** -- consistent style via sqruff integration
- **Smart type inference** -- nullability from JOINs, COALESCE, window functions, CASE WHEN, aggregates
- **`@optional` parameters** -- SQL rewriting for conditional filters (`WHERE ($1 IS NULL OR col = $1)`)
- **`:batch` execution** -- bulk inserts and batch operations
- **`@returns :grouped`** -- result grouping with `@group_by` for grouped query results
- **R2DBC reactive backends** -- non-blocking database access for Java and Kotlin
- **Kotlin Exposed** -- first-class Exposed ORM backend for Kotlin
- **Configurable row types** -- Pydantic, msgspec, Zod, dataclass, interface per backend
- **CTEs and window functions** -- ROW_NUMBER, RANK, LAG, LEAD, recursive CTEs with correct type inference
- **Enums, composites, arrays** -- PostgreSQL types mapped to language-native equivalents
- **Custom type overrides** -- map ltree, citext, PostGIS geometry to any target type

## Supported Languages

| Language   | PostgreSQL | MySQL | SQLite | DuckDB | CockroachDB |
|------------|:----------:|:-----:|:------:|:------:|:-----------:|
| Rust       | sqlx, tokio-postgres | sqlx | sqlx | -- | sqlx |
| Python     | psycopg3, asyncpg | aiomysql | aiosqlite | python-duckdb | psycopg3 |
| TypeScript | pg, postgres.js | mysql2 | better-sqlite3 | typescript-duckdb | pg |
| Go         | pgx | database/sql | database/sql | database/sql | pgx |
| Java       | JDBC, R2DBC | JDBC | JDBC | JDBC | JDBC |
| Kotlin     | JDBC, R2DBC, Exposed | JDBC | JDBC | JDBC | JDBC |
| C#         | Npgsql | MySqlConnector | Microsoft.Data.Sqlite | -- | Npgsql |
| Ruby       | pg, Trilogy | mysql2, Trilogy | sqlite3 | -- | pg |
| PHP        | PDO, AMPHP | PDO | PDO | -- | PDO |
| Elixir     | Postgrex, Ecto | MyXQL | Exqlite | -- | Postgrex |

## Documentation

Full documentation at [goldziher.github.io/scythe](https://goldziher.github.io/scythe):

- [Quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) -- zero to generated code in 5 minutes
- [Philosophy](https://goldziher.github.io/scythe/philosophy/) -- why compile SQL instead of using an ORM
- [Alternatives](https://goldziher.github.io/scythe/comparisons/alternatives/) -- how scythe compares to sqlc, SQLDelight, jOOQ, and ORMs
- [Custom Types](https://goldziher.github.io/scythe/guide/custom-types/) -- type overrides for PostgreSQL extensions
- [Configuration](https://goldziher.github.io/scythe/guide/configuration/) -- full scythe.toml reference
- [Annotations](https://goldziher.github.io/scythe/guide/annotations/) -- @name, @returns, @optional, @nullable, @json, and more
- [Lint Rules](https://goldziher.github.io/scythe/reference/lint-rules/) -- all 93 rules with codes and examples

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, architecture, and how to add backends/engines/lint rules.

## License

[MIT](LICENSE)
