# Scythe

**Polyglot SQL-to-code generator with built-in linting and formatting.**

Write SQL. Get type-safe code. In any language.

## Why Scythe

- **10 languages, 13 backend drivers** -- Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby, PHP
- **3 databases** -- PostgreSQL, MySQL, SQLite
- **22 custom lint rules plus sqruff integration** -- catch bugs before they ship
- **SQL formatting** -- via sqruff integration
- **Smart type inference** -- nullability from JOINs, COALESCE, aggregates

## Quick Install

```bash
cargo install scythe-cli
# or
brew install Goldziher/tap/scythe
```

## 30-Second Example

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1;
```

Scythe knows `o.total` is nullable (right side of LEFT JOIN) and generates type-safe code in your language of choice.
