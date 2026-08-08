---
title: Architecture
description: Scythe's compilation pipeline, workspace crates, and backend trait.
---

## Pipeline

```text
SQL Schema + Annotated Queries
        |
        v
    Parse (sqlparser-rs)
        |
        v
    Build Catalog (tables, types, constraints)
        |
        v
    Analyze (type inference, nullability, parameters)
        |
        v
    Lint (23 built-in rules + sqruff) + Format (sqruff)
        |
        v
    Backend (manifest.toml per (backend, engine) + CodegenBackend trait)
        |
        v
    Generated Code (Rust, Python, TypeScript, Go, ...)
```

## Workspace Crates

| Crate | Purpose |
|-------|---------|
| `scythe-core` | SQL parsing, catalog building, type inference, nullability analysis |
| `scythe-codegen` | Code generation via trait-based backends |
| `scythe-lint` | 23 lint rules + 35 audit rules + sqruff integration + engine |
| `scythe-backend` | Type resolution, naming conventions, MiniJinja rendering |
| `scythe-inspect` | Live-database health checks behind `scythe inspect` |
| `scythe-cli` | CLI binary with generate, check, lint, fmt, migrate commands |
| `scythe-conformance` | Dev-only: runs inferred nullability against live database engines (not published, no CLI surface) |

## Language-Neutral Type System

The analyzer outputs a neutral type vocabulary. Each backend maps these to concrete language types via a manifest:

| Neutral Type | Rust | Python | TypeScript | Go |
|---|---|---|---|---|
| `int32` | `i32` | `int` | `number` | `int32` |
| `string` | `String` | `str` | `string` | `string` |
| `datetime_tz` | `chrono::DateTime<Utc>` | `datetime.datetime` | `Date` | `time.Time` |
| `uuid` | `uuid::Uuid` | `uuid.UUID` | `string` | `uuid.UUID` |
| nullable | `Option<T>` | `T \| None` | `T \| null` | `*T` |

## Backend Trait

Adding a new language requires implementing the `CodegenBackend` trait:

```rust
pub trait CodegenBackend: Send + Sync {
    fn name(&self) -> &str;
    fn manifest(&self) -> &BackendManifest;
    fn generate_row_struct(&self, ...) -> Result<String, ScytheError>;
    fn generate_model_struct(&self, ...) -> Result<String, ScytheError>;
    fn generate_query_fn(&self, ...) -> Result<String, ScytheError>;
    fn generate_enum_def(&self, ...) -> Result<String, ScytheError>;
    fn generate_composite_def(&self, ...) -> Result<String, ScytheError>;
    fn file_header(&self) -> String;
    fn file_footer(&self) -> String;
    fn supported_engines(&self) -> &[&str];
}
```

Each backend also has a `manifest.toml` that maps neutral types to language-specific types. No Rust code is needed to customize type mappings. Manifests are compiled into the `scythe-codegen` binary at build time (from `crates/scythe-codegen/manifests/`), not discovered on the filesystem at generation time. A `[[sql.gen]]` target can merge a *partial* manifest over its backend's compiled-in one with `manifest = "..."`, resolved against the directory containing `scythe.toml` -- see [Configuration](/scythe/guide/configuration/).

### Engine-aware manifests

Backends that support multiple database engines use a manifest-per-(backend, engine) strategy. For example, `java-jdbc` has nine manifests -- one per engine it supports:

- `java-jdbc.toml` (PostgreSQL, the default)
- `java-jdbc.mysql.toml` (MySQL-specific type mappings)
- `java-jdbc.sqlite.toml` (SQLite-specific type mappings)
- and one each for MariaDB, DuckDB, Redshift, SQL Server, Oracle and Snowflake

When `get_backend("java-jdbc", "mysql")` is called, the engine-specific manifest is loaded automatically. This allows a single backend implementation to generate correct type mappings for each database engine without code duplication.

Backends that only support one engine (e.g. `rust-tokio-postgres` for PostgreSQL, `elixir-myxql` for MySQL) reject mismatched engines with a clear error via the `supported_engines()` method on the trait.

> **Note:** SQL parsing and type inference run through six parser dialects -- PostgreSQL, MySQL, SQLite, SQL Server, Oracle and Snowflake -- which the 10 supported engines map onto (CockroachDB, DuckDB and Redshift all parse as PostgreSQL; MariaDB as MySQL). Backend coverage varies by engine: the primary engines (PostgreSQL, MySQL, SQLite) have all 10 languages, but DuckDB has manifests for 5 languages (Java, Kotlin, Go, Python, TypeScript) and Snowflake for 7 (Java, Kotlin, Go, Python, TypeScript, C#, PHP). Code generation backends produce driver-specific code for each language, loading type mappings from their respective engine-aware manifests.

### Example manifest.toml (rust-sqlx)

```toml
[backend]
name = "rust-sqlx"
language = "rust"
file_extension = "rs"
engine = "postgresql"

[types.scalars]
bool = "bool"
int32 = "i32"
int64 = "i64"
string = "String"
uuid = "uuid::Uuid"
datetime_tz = "chrono::DateTime<chrono::Utc>"
json = "serde_json::Value"

[types.containers]
array = "Vec<{T}>"
nullable = "Option<{T}>"

[naming]
struct_case = "PascalCase"
fn_case = "snake_case"
row_suffix = "Row"
```

## Available Backends

56 selectable backend names -- 52 `CodegenBackend` implementations, four of which are also reachable under a `javascript-*` name that switches them to JSDoc emit -- across 10 languages and 10 databases (PostgreSQL, MySQL, MariaDB, SQLite, DuckDB, CockroachDB, Redshift, SQL Server, Oracle, Snowflake), resolved through 6 parser dialects. See [Backend Overview](/scythe/backends/overview/) for the full list organized by engine. The table below shows the three primary engines; see the overview for the rest.

| Language | PostgreSQL | MySQL | SQLite |
|----------|-----------|-------|--------|
| Rust | sqlx, tokio-postgres | sqlx | sqlx |
| Python | psycopg3, asyncpg | aiomysql | aiosqlite |
| TypeScript | postgres.js, pg | mysql2 | better-sqlite3 |
| JavaScript | postgres.js, pg | mysql2 | better-sqlite3 |
| Go | pgx | database/sql | database/sql |
| Java | JDBC | JDBC | JDBC |
| Kotlin | JDBC | JDBC | JDBC |
| C# | Npgsql | MySqlConnector | Microsoft.Data.Sqlite |
| Elixir | Postgrex | MyXQL | Exqlite |
| Ruby | pg | mysql2 | sqlite3 |
| PHP | PDO | PDO | PDO |

## Validating Nullability Inference

Nullability is inferred statically from the schema and the query, so a unit test can only compare
the analyzer against the same model the analyzer is built from. The `scythe-conformance` crate
closes that gap by running fixtures against real database servers. It is a contributor-facing
subsystem: the crate is `publish = false`, has no CLI surface, and nothing in `scythe generate`
touches it.

Per (fixture, engine, column) it compares three facts:

| Fact | Source |
|------|--------|
| **A** | the analyzer's `AnalyzedColumn::nullable` |
| **G** | whether the generated code actually *renders* the column as optional, parsed out of the resolved `full_type` against the backend manifest's `nullable` container pattern |
| **E** | the engine's observed per-row nullness, from the query run against a live server |

Four assertions relate them:

| Assertion | Statement |
|-----------|-----------|
| A1 fidelity | `A == G` -- the analyzer's inference and the generated code's rendered nullability agree |
| A2 soundness | an observed NULL implies the generated code renders that column optional; otherwise the generated code would decode a NULL non-optionally and crash |
| A3 anti-vacuity | a column the analyzer calls nullable must be demonstrated NULL by some run, or the suite is satisfied by marking everything nullable |
| A4 join-group coherence | columns widened by the same outer join -- sharing a `join_group`, not nullable before it -- must be NULL together in no-match rows and non-NULL together in matched rows |

A2 is keyed on **G**, not **A**: the crash risk is a property of the code that was actually
generated, so a fidelity mismatch that under-renders nullability fails A2 as well as A1.

Fixtures live under `testing_data/nullability_live/`. Each declares the portable DDL and query the
analyzer sees, the per-engine live schema profile it seeds, the engines it applies to, and one or
more runs with seed data and per-row expectations.

### Engine coverage

A run selects the engines it will actually dial. The runner draws a line between two outcomes and
allows no silent third:

- **Selected but not runnable** -- driver feature off, no connection configuration, or no driver
  implemented yet -- is a hard error raised before any fixture is evaluated.
- **Listed by a fixture but not selected for this run** is an explicit, recorded skip, printed in
  the report with the fixture, engine, and reason.

Live drivers exist for all six engines -- PostgreSQL, MySQL, MariaDB, SQLite, SQL Server, and
Oracle -- each behind its own Cargo feature (`pg`, `mysql`, `mariadb`, `sqlite`, `mssql`, `oracle`)
plus a `live-tests` gate. No driver is linked by default, so `cargo test --workspace` exercises the
pure modules without a container. Selecting an engine whose feature was not compiled in is a hard
error naming the feature to enable, never a silent skip. CI runs one job per engine, gated on
changes to the analyzer, the catalog, the fixtures, or the suite itself.

Each engine's isolation is its own problem. SQL Server gets a fresh database per connection and
reconnects into it: T-SQL's default schema belongs to the database principal rather than the
session, and `USE` does not survive tiberius routing statements through `sp_executesql`, so tables
would otherwise land in `master`. Oracle gets a user per connection, because in Oracle a schema is
a user; `CREATE SEQUENCE` is granted explicitly, since identity columns are sequence-backed. Oracle
column names are folded to lowercase to match the analyzer's normalization, and two names that
collide once folded are a hard error rather than an arbitrary pick.

### The divergence registry

`testing_data/nullability_live/DIVERGENCES.toml` records accepted gaps where the analyzer is more
pessimistic than a given engine -- it marked a column nullable and no live run there has ever
demonstrated a NULL. `analyzer_over_pessimistic` is the only legal entry kind, so the registry can
suppress an A3 vacuity failure and nothing else. A soundness failure is never suppressible; neither
is a fidelity or join-group failure.

Each entry names a fixture, engine, column, kind, tracking issue URL, and reason. The registry is
capped at 25 entries, and staleness is checked against the raw verdicts *before* suppression is
applied: a registered divergence that stops reproducing on an engine that actually ran fails the
build, so fixing the underlying gap forces deleting the entry that excused it. Entries for engines
that were not dialed in a given run are skipped rather than treated as stale.
