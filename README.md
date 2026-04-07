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

Scythe compiles annotated SQL into type-safe database access code. You write SQL queries, scythe generates the boilerplate -- structs, functions, type mappings -- in 10 languages across 3 databases. Built-in linting (93 rules) and formatting catch SQL bugs before they ship.

## Installation

```bash
cargo install scythe-cli
# or
brew install Goldziher/tap/scythe
```

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

See the [full quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) for all 10 languages.

## Features

- **10 languages** -- Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby, PHP
- **3 databases** -- PostgreSQL, MySQL, SQLite
- **25 backend drivers** -- sqlx, tokio-postgres, psycopg3, asyncpg, pg, postgres.js, pgx, JDBC, Npgsql, PDO, and more
- **93 lint rules** -- UPDATE without WHERE, SELECT *, NULL comparisons, leading wildcard LIKE, plus 71 sqruff rules
- **SQL formatting** -- consistent style via sqruff integration
- **Smart type inference** -- nullability from JOINs, COALESCE, window functions, CASE WHEN, aggregates
- **CTEs and window functions** -- ROW_NUMBER, RANK, LAG, LEAD, recursive CTEs with correct type inference
- **Enums, composites, arrays** -- PostgreSQL types mapped to language-native equivalents
- **Custom type overrides** -- map ltree, citext, PostGIS geometry to any target type

## Supported Languages

| Language   | PostgreSQL | MySQL | SQLite |
|------------|:----------:|:-----:|:------:|
| Rust       | sqlx, tokio-postgres | sqlx | sqlx |
| Python     | psycopg3, asyncpg | aiomysql | aiosqlite |
| TypeScript | pg, postgres.js | mysql2 | better-sqlite3 |
| Go         | pgx | database/sql | database/sql |
| Java       | JDBC | JDBC | JDBC |
| Kotlin     | JDBC | JDBC | JDBC |
| C#         | Npgsql | MySqlConnector | Microsoft.Data.Sqlite |
| Elixir     | Postgrex | MyXQL | Exqlite |
| Ruby       | pg | mysql2 | sqlite3 |
| PHP        | PDO | PDO | PDO |

## Documentation

Full documentation at [goldziher.github.io/scythe](https://goldziher.github.io/scythe):

- [Quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) -- zero to generated code in 5 minutes
- [Philosophy](https://goldziher.github.io/scythe/philosophy/) -- why compile SQL instead of using an ORM
- [vs jOOQ](https://goldziher.github.io/scythe/comparisons/jooq/) -- plain SQL vs Java DSL, licensing, polyglot support
- [vs ORMs](https://goldziher.github.io/scythe/comparisons/orms/) -- Hibernate, SQLAlchemy, ActiveRecord, Entity Framework, GORM, Diesel, Ecto
- [Custom Types](https://goldziher.github.io/scythe/guide/custom-types/) -- type overrides for PostgreSQL extensions
- [Configuration](https://goldziher.github.io/scythe/guide/configuration/) -- full scythe.toml reference
- [Annotations](https://goldziher.github.io/scythe/guide/annotations/) -- @name, @returns, @nullable, @json, and more
- [Lint Rules](https://goldziher.github.io/scythe/reference/lint-rules/) -- all 93 rules with codes and examples

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, architecture, and how to add backends/engines/lint rules.

## License

[MIT](LICENSE)
