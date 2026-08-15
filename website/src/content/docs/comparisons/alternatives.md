---
title: Alternatives
description: How scythe compares to sqlc, SQLDelight, PgTyped, sqlx, jOOQ, aiosql, and ORMs.
---

Scythe is a SQL-first code generator. This page compares it to the tool it is most similar to (sqlc), to the [wider SQL-first landscape](#the-wider-sql-first-landscape) (SQLDelight, PgTyped, sqlx, aiosql, PugSQL, HugSQL, go-jet, SQLBoiler, Kysely, MyBatis and others), and to ORMs (Hibernate, SQLAlchemy, ActiveRecord, and more).

Claims about scythe on this page are checked against this repository's source. Claims about other
tools reflect their public documentation and behavior as of 2026-08 and are not re-verified against
their source on every scythe release -- they may drift out of date. If something here looks wrong for
a competitor tool, check that tool's own docs.

## Overview

| | Scythe | sqlc | SQLDelight | jOOQ | ORMs |
|---|---|---|---|---|---|
| Approach | SQL files to code | SQL files to code | .sq files to code | Java DSL to SQL | Code to SQL |
| Languages | 10 (Rust, Python, TS, Go, Java, Kotlin, C#, Elixir, Ruby, PHP) | Go (primary), Python, Kotlin, TypeScript (plugin) | Kotlin (JVM, Native, JS via KMP) | Java, Kotlin | Language-specific |
| Databases | PostgreSQL, MySQL, MariaDB, SQLite, DuckDB, CockroachDB, Redshift, SQL Server, Oracle, Snowflake | PostgreSQL, MySQL, SQLite | SQLite, PostgreSQL, MySQL, H2 | 25+ databases | Varies |
| Backends | 56 | ~5 | 4 | N/A (JDBC) | Varies |
| SQL linting | 59 rules | None | None | None | None |
| SQL formatting | sqruff integration | None | None | None | None |
| Nullability inference | JOINs, COALESCE, CASE, window functions, aggregates | Basic (column constraints) | Column constraints | From DB metadata | Varies |
| Optional params | `@optional` annotation (SQL rewriting) | `sqlc.narg()` | None | Runtime DSL | Varies |
| Batch execution | `:batch` return type | `sqlc.slice()` | None | Batch API | Varies |
| Row type options | Pydantic, msgspec, Zod, dataclass, interface | None | None | N/A | Language-native |
| IDE plugin | None (planned) | None | IntelliJ | IntelliJ | Varies |
| Reactive queries | R2DBC (Java, Kotlin) | Not supported | Kotlin Flow, RxJava | Not supported | Varies |
| Migration support | External | External | Built-in (.sqm validation) | External | Usually built-in |
| Custom types | type_overrides config | Override config | ColumnAdapter interface | Binding/Converter | Language-native |
| Build integration | CLI (any build system) | CLI | Gradle plugin | Maven/Gradle | Language-specific |
| Licensing | MIT | MIT (core), BSD | Apache 2.0 | Apache 2.0 (open DBs), Commercial (Oracle, MSSQL, etc.) | Varies |
| When SQL runs | Compiled at build time | Compiled at build time | Compiled at build time | DSL builds SQL at runtime | Generated at runtime |

## The wider SQL-first landscape

Scythe is one point in a crowded space. The tools below all reject "let an ORM write your SQL", but
they disagree on **where the types come from** and **whether you write SQL or build queries in your
host language** — usually the two decisions that actually separate them.

### SQL files compiled to typed code

The same category as scythe: `.sql` files are the source of truth and a build step emits typed functions.

| Tool | Languages | Types derived from | Notes |
|---|---|---|---|
| **scythe** | 10 | Static analysis of schema + query | No database needed to generate |
| [sqlc](https://sqlc.dev) | Go, plus Python/Kotlin/TypeScript plugins | Static analysis | The direct inspiration; strongest in Go |
| [SQLDelight](https://sqldelight.github.io/sqldelight/) | Kotlin (JVM/Native/JS) | Static analysis of `.sq` files | Reactive queries via Flow, IntelliJ plugin |
| [PgTyped](https://pgtyped.dev) | TypeScript | **A running Postgres** | Types come from the server, not a parser |
| [aiosql](https://nackjicholson.github.io/aiosql/) | Python | Not typed | SQL files become callable functions |
| [PugSQL](https://pugsql.org) | Python | Not typed | Python port of HugSQL |
| [HugSQL](https://hugsql.org) / [Yesql](https://github.com/krisajenkins/yesql) | Clojure | Not typed | Where this pattern started |

aiosql, PugSQL, HugSQL and Yesql give you SQL as the source of truth *without* types. scythe, sqlc,
SQLDelight and PgTyped add the type layer. If all you want is your SQL out of string literals, the
untyped tools are smaller and simpler.

### Verified against a live database

These check queries against a real server, so a parser that has drifted from the engine cannot fool
them — at the cost of needing that server available at build time.

| Tool | Languages | How |
|---|---|---|
| [sqlx](https://github.com/launchbadge/sqlx) | Rust | `query!` macros check against `DATABASE_URL` at compile time; `cargo sqlx prepare` caches metadata into `.sqlx/*.json` for offline and CI builds |
| [PgTyped](https://pgtyped.dev) | TypeScript | Generates types from a running Postgres |
| [SafeQL](https://safeql.dev) | TypeScript | ESLint rule validating inline SQL against a live database |

This is the sharpest trade-off against scythe, and worth being honest about. Live-database checking is
authoritative about column types; static analysis is not, and a parser lagging a dialect can be
confidently wrong. What the database does *not* hand you is nullability for arbitrary query columns —
Postgres describes result columns as type OIDs with no nullability — so outer-join and aggregate
nullability still has to be inferred either way. Scythe takes the static route so generation needs no
database at all. Cross-checking that inference against a live server is
[open work](https://github.com/Goldziher/scythe/issues/65), not a shipped feature.

### Schema-first code generation

Here the *schema* is introspected and a typed query builder is generated; you compose queries in the
host language rather than writing SQL.

| Tool | Languages | Notes |
|---|---|---|
| [jOOQ](https://www.jooq.org) | Java, Kotlin | See [below](#vs-jooq); commercial licence for Oracle/SQL Server/DB2 |
| [go-jet](https://github.com/go-jet/jet) | Go | Type-safe builder plus result mapping; PostgreSQL, MySQL, MariaDB, SQLite |
| [SQLBoiler](https://github.com/aarondl/sqlboiler) | Go | Database-first ORM generated from your schema |
| [xo](https://github.com/xo/xo) | Go | Idiomatic Go for PostgreSQL, MySQL, SQLite, Oracle, SQL Server |
| [Kysely](https://kysely.dev) + `kysely-codegen` | TypeScript | Builder with types generated from the schema |
| [Prisma](https://www.prisma.io), [Drizzle](https://orm.drizzle.team) | TypeScript | Schema-first, closer to the ORM end |
| [Exposed](https://github.com/JetBrains/Exposed), [Ktorm](https://www.ktorm.org) | Kotlin | DSL over a typed schema |
| [QueryDSL](http://querydsl.com) | Java | Generated metamodel, fluent queries |

The difference is what you write. These generate the *vocabulary* for building queries; scythe
generates the *queries themselves* from SQL you already wrote. Builders win on dynamic composition,
which scythe deliberately does not do. Scythe does ship `typescript-kysely` and `kotlin-exposed`
backends, if you want generated code that lands inside those ecosystems rather than replacing them.

### SQL mapping frameworks

| Tool | Languages | Notes |
|---|---|---|
| [MyBatis](https://mybatis.org) | Java | SQL in XML or annotations, mapped to objects; no generated types |
| [Dapper](https://github.com/DapperLib/Dapper) | C# | Micro-ORM; you write SQL, it maps rows onto types you declare |
| [JDBI](https://jdbi.org) | Java | Fluent and declarative SQL object APIs |

These keep SQL first-class but leave the result types to you — you declare them by hand and they are
checked at runtime, if at all. A hand-written type drifting from its query is the specific failure
scythe exists to prevent.

---

## vs sqlc

sqlc is the tool scythe is most directly inspired by. Both compile SQL files into typed code.

**Key differences:**

- **Language support.** sqlc primarily targets Go, with community plugins for Python/Kotlin/TypeScript. Scythe generates native, idiomatic code for 10 languages from a single codebase.
- **SQL linting.** Scythe includes 59 built-in rules (24 lint + 35 audit). sqlc has none -- you need a separate tool.
- **Type inference.** Scythe infers nullability from JOINs, COALESCE, CASE, window functions. sqlc infers from column constraints and has `sqlc.narg()` for nullable params.
- **Optional parameters.** Scythe's `@optional` annotation rewrites SQL conditions to skip filters when NULL is passed. sqlc uses `sqlc.narg()` for a similar effect.
- **Row types.** Scythe supports configurable row types -- Pydantic/msgspec for Python, Zod for TypeScript. sqlc generates fixed types per language.
- **Multi-database.** Both support PostgreSQL, MySQL, SQLite. Scythe also supports MariaDB, DuckDB, CockroachDB, Redshift, SQL Server, Oracle, and Snowflake. Scythe's engine-aware backends generate optimized code per database.
- **Configuration.** sqlc uses `sqlc.yaml`, scythe uses `scythe.toml`. Both are CLI tools.

**When to choose sqlc:** Go-only teams, existing sqlc investment, need `sqlc.narg()` for dynamic queries.

**When to choose scythe:** Polyglot teams, want SQL linting/formatting, need better nullability inference.

---

## vs SQLDelight

SQLDelight is the closest tool to scythe in philosophy -- both generate code from SQL files at compile time. SQLDelight targets the Kotlin ecosystem exclusively.

**Key differences:**

- **Languages.** SQLDelight generates Kotlin only (JVM, Native, JS via KMP). Scythe generates 10 languages.
- **Build system.** SQLDelight requires Gradle. Scythe is a CLI that works with any build system.
- **Reactive queries.** SQLDelight integrates with Kotlin Flow and RxJava -- queries re-emit when data changes, i.e. a live subscription. Scythe's `java-r2dbc`/`kotlin-r2dbc` backends generate one-shot reactive types (`Flux<T>`/`Flow<T>` for `:many`, `Mono<T>`/`suspend fun` for `:one`) but do not re-emit on data change -- see the [Overview table](#overview) above.
- **IDE support.** SQLDelight has an IntelliJ plugin with autocomplete and refactoring. Scythe does not (planned).
- **Migrations.** SQLDelight validates .sqm migration files at build time. Scythe delegates migrations to external tools.
- **SQL linting.** Scythe has 59 built-in rules. SQLDelight has none.

**When to choose SQLDelight:** Kotlin/KMP stack, need reactive queries (Flow), need IDE plugin, building cross-platform mobile.

**When to choose scythe:** Polyglot teams, want SQL linting/formatting, not using Gradle, need non-JVM languages.

---

## vs jOOQ

jOOQ takes a different approach -- instead of SQL files, you write queries using a Java DSL that generates SQL at runtime.

**Key differences:**

- **SQL authoring.** jOOQ uses a Java/Kotlin fluent API. Scythe uses plain SQL files.
- **When SQL runs.** jOOQ builds SQL strings at runtime. Scythe compiles SQL at build time -- zero runtime overhead.
- **Schema source.** jOOQ requires a live database connection for code generation. Scythe reads .sql files.
- **Dynamic queries.** jOOQ excels at runtime query composition (conditional WHERE, dynamic columns). Scythe queries are static.
- **Languages.** jOOQ targets Java/Kotlin. Scythe targets 10 languages.
- **Licensing.** jOOQ is free for open-source databases, commercial license required for Oracle/SQL Server/DB2. Scythe is MIT for everything.

jOOQ's runtime DSL allows conditional query construction:

```java
var query = ctx.select(USERS.ID, USERS.NAME).from(USERS);
if (filterByStatus) {
    query = query.where(USERS.STATUS.eq(status));
}
if (filterByDate) {
    query = query.and(USERS.CREATED_AT.gt(minDate));
}
```

Scythe does not support dynamic query composition. Queries are fixed at compile time. If you need conditional logic, you write separate queries or use SQL-level conditionals (`WHERE (:filter_status IS NULL OR status = :filter_status)`). This is a deliberate trade-off: dynamic queries are powerful but harder to analyze statically, harder to lint, and harder to optimize.

**When to choose jOOQ:** Java/Kotlin only, need dynamic query composition, prefer query builder over SQL files.

**When to choose scythe:** Want plain SQL files, polyglot teams, need all databases without commercial licensing, want SQL linting.

---

## vs ORMs

ORMs (Hibernate, SQLAlchemy, ActiveRecord, Entity Framework, GORM, Diesel, Ecto) map database tables to objects in application code. They generate SQL at runtime based on your object model.

### Common ORM pain points scythe solves

| Problem | ORMs | Scythe |
|---------|------|--------|
| N+1 queries | Lazy loading fires individual SELECTs per row | You write the JOIN -- one query, one round trip |
| Opaque SQL | Generated SQL is unpredictable and hard to debug | The SQL you write is the SQL that runs |
| Migration drift | Model != schema divergence is common | Schema is SQL -- scythe reads it directly |
| Limited SQL | Window functions, CTEs, lateral joins require raw SQL escape hatches | Write any SQL your database supports |
| Type safety | Struggles with aggregations, conditional expressions, nullable joins | Static analysis with precise nullability inference |

**When to choose an ORM:** Database portability matters (BYOD — your users pick the database), simple CRUD, rapid prototyping, team prefers not to write SQL.

**When to choose scythe:** You control the database, complex queries, performance-sensitive, want full SQL control with type safety.

### Per-framework notes

- **Hibernate/JPA** (Java) -- `LazyInitializationException`, N+1 queries, HQL limitations for window functions and CTEs. Scythe gives you full SQL with JDBC types. Choose Hibernate when you need JPA portability or are invested in the JPA ecosystem.
- **SQLAlchemy** (Python) -- Powerful but ORM mode obscures queries. Session complexity causes stale reads and detached instance errors. Async lazy loading does not work. Scythe generates dataclasses with async support. Choose SQLAlchemy Core when you need runtime query composition.
- **ActiveRecord** (Ruby) -- Convention-over-configuration works until queries get complex. N+1 by default, `Arel` is undocumented, no compile-time type safety. Scythe generates module-wrapped `Data.define` types with RBS annotations. Choose ActiveRecord for standard Rails CRUD.
- **Entity Framework** (C#) -- LINQ is expressive but not all expressions translate to SQL. Migration conflicts across branches are common. Change tracker adds overhead for bulk operations. Scythe generates async record types. Choose EF when you need tight Visual Studio integration.
- **GORM** (Go) -- Auto-migration is dangerous in production. Runtime reflection loses compile-time guarantees. Silent failures on missing rows. Scythe generates structs with explicit error returns for pgx/database-sql. Choose GORM for quick data access layers with simple CRUD.
- **Diesel** (Rust) -- Compile-time safety but the DSL has a steep learning curve. Compiler errors for malformed queries can be hundreds of lines. Window functions and CTEs require raw SQL. Sync only without `diesel-async`. Scythe uses plain SQL with sqlx/tokio-postgres. Choose Diesel when you want compile-time SQL verification within Rust's type system.
- **Ecto** (Elixir) -- One of the better-designed ORMs. Composable query DSL, clean changesets, tight Phoenix integration. Complex queries still require `fragment/1` escape hatches. Scythe generates Postgrex query functions with typespecs. Choose Ecto when your stack is Elixir/Phoenix and queries are straightforward.

### Summary table

| | Source of truth | Query language | Type safety | SQL feature coverage | Runtime cost |
|---|---|---|---|---|---|
| **Scythe** | SQL files | SQL | Generated, precise | Full (CTEs, window functions, etc.) | Zero (static code) |
| **Hibernate** | Java annotations | JPQL / Criteria | Partial | Limited | Session management, dirty checking |
| **SQLAlchemy** | Python classes | Python DSL / Core | Partial (`Any` leaks) | Moderate | Session, identity map |
| **ActiveRecord** | Ruby conventions | Ruby DSL | None (dynamic typing) | Limited | Lazy loading, callbacks |
| **Entity Framework** | C# classes | LINQ | Good (LINQ-level) | Moderate | Change tracking |
| **GORM** | Go struct tags | Go DSL | None (reflection) | Limited | Reflection |
| **Diesel** | Rust macros | Rust DSL | Strong (type-level) | Moderate | Minimal |
| **Ecto** | Elixir modules | Elixir DSL | Moderate (compile-time) | Moderate | Minimal |
