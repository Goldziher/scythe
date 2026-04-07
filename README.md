<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/Goldziher/scythe/main/logo.svg">
    <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/Goldziher/scythe/main/logo-dark.svg">
    <img width="400" alt="Scythe" src="https://raw.githubusercontent.com/Goldziher/scythe/main/logo-dark.svg" />
  </picture>

  **SQL Compiler and Linter.**

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

Scythe is an SQL compiler and linter that generates type-safe code from your SQL queries. Inspired by [sqlc](https://github.com/sqlc-dev/sqlc) and [sqlfluff](https://github.com/sqlfluff/sqlfluff), it makes SQL the source of truth -- giving you zero bloat, max performance, and safer code across 10 languages and 3 databases.

## Installation

```bash
# Cargo
cargo install scythe-cli

# Homebrew
brew install Goldziher/tap/scythe
```

## Quick Start

**1. Write SQL queries with annotations:**

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;
```

**2. Configure `scythe.toml`:**

```toml
[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries/*.sql"]
output = "src/db/generated"

[sql.gen.rust]
target = "sqlx"
```

**3. Generate code:**

```bash
scythe generate
```

## Supported Databases

| Database   | Status    |
|------------|-----------|
| PostgreSQL | Supported |
| MySQL      | Supported |
| SQLite     | Supported |

## Supported Languages

| Language   | PostgreSQL | MySQL | SQLite |
|------------|:----------:|:-----:|:------:|
| Rust       | x          | x     | x      |
| Python     | x          | x     | x      |
| TypeScript | x          | x     | x      |
| Go         | x          | x     | x      |
| Java       | x          | x     | x      |
| Kotlin     | x          | x     | x      |
| C#         | x          | x     | x      |
| Elixir     | x          | x     | x      |
| Ruby       | x          | x     | x      |
| PHP        | x          | x     | x      |

## Documentation

Full documentation is available at [goldziher.github.io/scythe](https://goldziher.github.io/scythe), including:

- [Configuration](https://goldziher.github.io/scythe/configuration/)
- [Annotations](https://goldziher.github.io/scythe/annotations/)
- [Type Inference](https://goldziher.github.io/scythe/type-inference/)
- [Linting](https://goldziher.github.io/scythe/linting/)
- [CLI Reference](https://goldziher.github.io/scythe/cli/)
- [Migration from sqlc](https://goldziher.github.io/scythe/migration-from-sqlc/)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

[MIT](LICENSE)
