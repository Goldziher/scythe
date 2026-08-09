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

Scythe compiles annotated SQL into type-safe database access code — structs, functions and type
mappings — for **10 languages** across **10 databases** via **56 backends**. Built-in linting
(58 rules), security auditing and formatting catch SQL bugs before they ship.

## Installation

```bash
cargo install scythe-cli
# or, using pre-built binaries for a faster install:
cargo binstall scythe-cli
# or
brew install Goldziher/tap/scythe
```

No Rust toolchain required — both wrappers download the prebuilt binary for your platform and
verify its checksum:

```bash
npm install --save-dev scythe-cli   # Node.js
pip install scythe-sql              # Python
```

See [Installation](https://goldziher.github.io/scythe/getting-started/installation/) for supported
platforms, proxy configuration and cache control.

## Quick Start

**1. Write annotated SQL:**

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

**3. Generate:**

```bash
scythe generate
```

**4. Use it.** Scythe infers that `o.total` and `o.notes` are nullable — they sit on the right side
of a `LEFT JOIN` — and types them as `Option<T>` while `u.id` and `u.name` stay non-null:

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserOrdersRow {
    pub id: i32,
    pub name: String,
    pub total: Option<rust_decimal::Decimal>,
    pub notes: Option<String>,
}
```

Every generated file opens with a `scythe:provenance` header recording the schema and queries it
came from, so `scythe check` can tell you when it has drifted.

The [quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) shows the same
query in all ten languages, with full function bodies.

## Pre-commit / prek

```yaml
repos:
  - repo: https://github.com/Goldziher/scythe
    rev: v0.14.0
    hooks:
      - id: scythe-fmt
      - id: scythe-lint
      - id: scythe-generate
```

Six hooks are available — see
[Pre-commit Hooks](https://goldziher.github.io/scythe/guide/pre-commit-hooks/) for `scythe-audit`,
`scythe-inspect` and `scythe-check`, and for their configuration.

## What you get

- **Type inference that reads the query** — nullability from JOINs, COALESCE, window functions,
  CASE WHEN and aggregates; CTEs, enums, composites and arrays mapped to language-native types.
- **[58 built-in rules](https://goldziher.github.io/scythe/reference/lint-rules/)** — 23 lint and
  35 audit, plus sqruff's style rules and 15 more drift checks run by `scythe check`.
- **[`scythe audit`](https://goldziher.github.io/scythe/guide/audit/)** — a security scanner for
  SQL: dangerous functions, privilege escalation, literal passwords, `SELECT *` over PII. Human,
  SARIF or JSON output for CI.
- **[`scythe inspect`](https://goldziher.github.io/scythe/guide/inspect/)** — live-database health
  checks: unindexed foreign keys, policies without RLS, duplicate indexes. PostgreSQL only.
- **Configurable output** — Pydantic, msgspec, Zod, dataclasses or plain interfaces per backend;
  `structs_only` for a types-only package; custom type overrides for ltree, citext or PostGIS.
- **Annotations beyond the basics** — `@optional` parameters compile to conditional filters,
  `:batch` for bulk operations, `@returns :grouped` with `@group_by` for nested results.

## Language and database support

Ten languages — Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby and PHP, plus plain
JavaScript via the TypeScript backends' JSDoc emit mode — across PostgreSQL, MySQL, MariaDB,
SQLite, DuckDB, CockroachDB, MSSQL, Oracle, Redshift and Snowflake.

See the [backend overview](https://goldziher.github.io/scythe/backends/overview/) for the full
driver matrix and per-backend notes.

## Documentation

Full documentation at [goldziher.github.io/scythe](https://goldziher.github.io/scythe):

- [Quickstart](https://goldziher.github.io/scythe/getting-started/quickstart/) — zero to generated code in 5 minutes
- [Philosophy](https://goldziher.github.io/scythe/philosophy/) — why compile SQL instead of using an ORM
- [Alternatives](https://goldziher.github.io/scythe/comparisons/alternatives/) — how scythe compares to sqlc, SQLDelight, jOOQ, and ORMs
- [Configuration](https://goldziher.github.io/scythe/guide/configuration/) — full `scythe.toml` reference
- [Annotations](https://goldziher.github.io/scythe/guide/annotations/) — `@name`, `@returns`, `@optional`, `@nullable`, `@json`, and more
- [CLI Reference](https://goldziher.github.io/scythe/guide/cli-reference/) — every command, flag and exit code

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, architecture, and how to add backends/engines/lint rules.

## License

[MIT](LICENSE)
