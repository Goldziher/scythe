# Scythe vs ORMs

ORMs map database tables to objects in your programming language. They generate SQL at runtime based on your object model. Scythe takes the opposite approach: you write SQL, and scythe generates the object model.

This page compares scythe against the major ORMs by language ecosystem.

---

## Hibernate / JPA (Java)

**How it works:** Entity classes annotated with `@Entity`, `@Table`, `@Column` define the schema. JPQL or the Criteria API express queries. Hibernate generates SQL at runtime and manages object state (dirty checking, lazy loading, caching).

**Pain points scythe solves:**

- **Lazy loading exceptions.** `LazyInitializationException` when accessing a relationship outside a session. With scythe, there are no sessions and no lazy loading — your query returns exactly the data it selects.
- **N+1 queries.** Traversing `@OneToMany` collections triggers individual SELECT statements per parent row. Scythe queries contain explicit JOINs, so the data arrives in one round trip.
- **HQL/JPQL limitations.** Window functions, recursive CTEs, lateral joins, and database-specific syntax require native queries, which lose type safety. Scythe handles all of these with full type inference.
- **Opaque SQL.** Enabling `hibernate.show_sql` reveals generated queries that are verbose, aliased unpredictably, and hard to map back to your Java code. Scythe's SQL is the SQL you wrote.
- **Schema drift.** Hibernate's `hbm2ddl.auto=update` is explicitly documented as unsuitable for production. Real migrations require Flyway or Liquibase on top of the ORM. Scythe reads your migration SQL directly.

**When Hibernate is still the right choice:** You need JPA portability across databases, your team is invested in the JPA ecosystem, or you are working with an existing Hibernate codebase where rewriting queries is not practical.

---

## SQLAlchemy (Python)

**How it works:** Two layers. The Core layer provides a Python DSL for SQL expression construction. The ORM layer adds declarative models, sessions, identity maps, and relationship loading. Most projects use the ORM.

**Pain points scythe solves:**

- **Session complexity.** SQLAlchemy's `Session` manages object identity, transaction scope, and flush timing. Misunderstanding session lifecycle causes bugs: stale reads, unexpected flushes, detached instance errors. Scythe has no session — each function call is a standalone query.
- **Type inference gaps.** SQLAlchemy's type stubs are improving, but complex queries involving `func`, `case`, `label`, and joins often resolve to `Any`. Scythe generates precise types for every query.
- **Async friction.** SQLAlchemy 2.0 supports async, but lazy loading does not work in async contexts. Async code requires `selectinload` or `joinedload` on every relationship. Scythe generates async code that returns flat result types with no loading concerns.
- **Two query languages.** Teams end up mixing ORM queries (session.query, select), Core expressions, and raw SQL strings in the same codebase. Scythe uses one language: SQL.

**When SQLAlchemy is still the right choice:** You need runtime query composition (dynamic filters, search builders), the Core layer meets your needs without the ORM, or your project requires database-agnostic query construction.

---

## ActiveRecord (Ruby)

**How it works:** Convention-over-configuration. Models inherit from `ApplicationRecord`. Table names, primary keys, and foreign keys are inferred from naming conventions. Queries use a chainable Ruby DSL (`where`, `joins`, `includes`).

**Pain points scythe solves:**

- **N+1 by default.** ActiveRecord relationships are lazy-loaded. Without explicit `includes` or `eager_load`, every relationship access fires a query. Bullet helps detect this, but the default behavior is wrong. Scythe queries are explicit — no implicit loading.
- **Callbacks and implicit behavior.** `before_save`, `after_create`, `around_update` — model callbacks create invisible control flow that makes it hard to reason about what a single database operation does. Scythe functions do one thing: execute the SQL you wrote.
- **Limited SQL expressiveness.** Complex queries require `Arel` (undocumented internal API) or raw SQL via `find_by_sql`, which loses type information. Scythe handles complex SQL natively with typed results.
- **No compile-time type safety.** Ruby is dynamically typed. Column type mismatches, missing columns, and renamed fields are caught at runtime, often in production. Scythe generates typed code with Sorbet RBS type annotations.

**When ActiveRecord is still the right choice:** You are building a Rails application where conventions reduce setup time, your queries are simple CRUD, or your team values rapid prototyping over type safety.

---

## Entity Framework (C#)

**How it works:** DbContext and entity classes define the model. LINQ queries translate to SQL at runtime. EF Core supports code-first (model drives schema) and database-first (schema drives model) workflows.

**Pain points scythe solves:**

- **LINQ translation failures.** Not all LINQ expressions translate to SQL. EF Core silently falls back to client-side evaluation (EF Core 3.0+ throws instead, but the diagnostics are not always clear). Scythe compiles SQL directly — if it parses, it runs.
- **Migration conflicts.** Code-first migrations generate migration files from model diffs. Concurrent developers creating migrations on different branches cause merge conflicts in the migration history. Scythe reads SQL schema files and does not generate migrations.
- **Tracking overhead.** EF's change tracker monitors every loaded entity for modifications. This is convenient for simple updates but adds memory overhead and makes bulk operations slow. Scythe functions execute queries without object tracking.
- **Expression tree complexity.** Advanced queries require understanding `IQueryable` expression trees, `Include`/`ThenInclude` chains, and projection semantics. Scythe requires understanding SQL.

**When Entity Framework is still the right choice:** You are building a .NET application and want tight Visual Studio integration, your team prefers LINQ over SQL, or you need automatic change tracking for complex update workflows.

---

## GORM (Go)

**How it works:** Struct tags define column mappings. A chainable API (`db.Where(...).Find(...)`) builds and executes queries. GORM uses reflection to map results to structs.

**Pain points scythe solves:**

- **Runtime reflection.** GORM maps query results to structs using reflection, which is slow and loses compile-time guarantees. Misspelled column tags, wrong types, and missing fields are caught at runtime. Scythe generates structs that match the query at compile time.
- **Silent failures.** GORM does not return errors by default for many operations. `db.First(&user)` returns a zero-value struct if no row is found (unless you check `db.Error`). Scythe's generated Go code uses explicit `error` returns.
- **Limited query support.** Subqueries, CTEs, window functions, and complex joins require raw SQL via `db.Raw()`, which returns `*sql.Rows` with no type mapping. Scythe generates typed structs for all queries.
- **Struct tag coupling.** The same struct serves as the schema definition, query result, and serialization target. Changes to the JSON API affect the database layer. Scythe generates per-query result types, decoupled from your API types.

**When GORM is still the right choice:** You need a quick data access layer for a Go service with simple CRUD operations, or your team prefers not to maintain separate SQL files.

---

## Diesel (Rust)

**How it works:** A Rust DSL for building SQL queries. Schema is defined via `table!` macros (generated from migrations). Queries are built using Rust method chains that are type-checked at compile time.

**Pain points scythe solves:**

- **DSL learning curve.** Diesel's type-level query builder uses advanced Rust generics. Compiler errors for malformed queries can be hundreds of lines long and difficult to interpret. Scythe: write SQL, get generated code.
- **Limited SQL coverage.** Window functions, CTEs, and some PostgreSQL-specific features require custom extensions or raw SQL via `sql_query`, which loses type safety. Scythe infers types for these features.
- **Schema macro rigidity.** The `table!` macro defines one struct per table. Queries that select a subset of columns or join multiple tables require custom type definitions. Scythe generates a result struct per query automatically.
- **Sync only.** Diesel is synchronous. Async support requires `diesel-async`, which is a separate crate with its own API surface. Scythe generates async Rust code for async backends (sqlx, tokio-postgres) natively.

**When Diesel is still the right choice:** You want compile-time SQL verification within Rust's type system, your queries are straightforward, or you prefer a Rust-native DSL over maintaining SQL files.

---

## Ecto (Elixir)

**How it works:** Schemas are defined as Elixir modules with typed fields. Queries use a composable DSL built on Elixir macros. Ecto.Repo manages database connections and transactions. Changesets handle validation and casting.

**Pain points scythe solves:**

- **DSL escape hatches.** Complex queries require `fragment/1` for raw SQL, which is untyped and not validated at compile time. Scythe validates all SQL statically.
- **Schema coupling.** Ecto schemas couple the database representation to the application representation. If your API response differs from your table layout, you need intermediate structs. Scythe generates per-query result types.
- **Virtual fields and associations.** Preloading associations uses separate queries (`Repo.preload`) or join-based loading. The preloading behavior is clearer than most ORMs, but still requires explicit management. Scythe queries return flat results from explicit JOINs.

**When Ecto is still the right choice:** Ecto is one of the better-designed ORMs. Its query DSL is composable and explicit, changesets provide clean validation, and it integrates tightly with Phoenix. If your stack is Elixir/Phoenix and your queries are straightforward, Ecto is a strong default. Use scythe when your SQL outgrows the DSL or when you need to share query definitions across multiple languages.

---

## Summary

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
