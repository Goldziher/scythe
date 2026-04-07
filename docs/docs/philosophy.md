# Philosophy

Scythe compiles SQL into type-safe database access code. You write SQL queries and schema, scythe generates the boilerplate — structs, functions, type mappings — in 10 languages.

## Why Compile SQL?

SQL is a 50-year-old language supported by every major database. It is expressive, optimizable, and well-understood. Developers already know it. Tools already support it. Database engines already optimize it.

The problem is the glue code. Every application that talks to a database needs code that maps query parameters in, maps result rows out, and keeps types aligned between the two worlds. This code is tedious, error-prone, and changes every time the schema or queries change.

Scythe eliminates that glue code. You write `.sql` files — schema definitions and annotated queries — and scythe compiles them into fully typed functions and data structures for your target language. The generated code is readable, has no runtime dependencies beyond your database driver, and stays in sync with your SQL automatically.

The result: you get the full power of SQL with the type safety of generated code, without maintaining the mapping layer by hand.

## vs ORMs

ORMs map database tables to objects in your application language. They promise productivity by hiding SQL behind a language-native API. In practice, they introduce a set of recurring problems:

| Problem | What happens | Scythe's approach |
|---------|-------------|-------------------|
| **N+1 queries** | Lazy-loading traverses relationships one row at a time. A page listing 50 orders with their items fires 51 queries. | You write the JOIN yourself. One query, one round trip. |
| **Opaque query generation** | The ORM translates method chains into SQL. The generated SQL is often surprising, sometimes incorrect, and hard to predict from reading the application code. | The SQL you write is the SQL that runs. Nothing is generated at runtime. |
| **Migration hell** | Schema changes require updating model classes, writing migration scripts, and hoping the ORM's diff algorithm produces correct DDL. Divergence between the model and the actual schema is common. | Schema is defined in SQL. Scythe reads it directly. Your migration tool manages DDL; scythe generates code from the result. |
| **Debugging complexity** | When a query is slow, you must reverse-engineer the ORM's output, check the query plan, then figure out which API call to change. The abstraction that was supposed to help is now in the way. | You own the SQL. Run `EXPLAIN` on it directly. Optimize it directly. |
| **Limited SQL support** | Window functions, CTEs, lateral joins, recursive queries — ORMs either do not support them or require dropping into raw SQL, defeating the purpose of the abstraction. | Scythe handles these natively. Write any SQL your database supports. |
| **Type safety gaps** | ORM type systems struggle with aggregations, conditional expressions, and nullable joins. Types are often inferred incorrectly or left as `Any`. | Scythe analyzes your SQL statically and infers precise types, including nullability from JOINs, CASE expressions, and aggregations. |

ORMs remain a reasonable choice for simple CRUD applications where SQL complexity is low, rapid prototyping is the priority, and the team prefers not to write SQL. For everything else, compiling SQL gives you more control with less friction.

See the [detailed ORM comparison](comparisons/orms.md) for per-framework analysis.

## vs jOOQ

jOOQ is the closest tool to scythe in philosophy — both are SQL-first and reject the ORM abstraction. The differences are in scope, approach, and licensing.

| Aspect | jOOQ | Scythe |
|--------|------|--------|
| **SQL authoring** | Java/Kotlin DSL that mirrors SQL syntax | Plain `.sql` files — no new API to learn |
| **When SQL runs** | DSL builds SQL strings at runtime | SQL is compiled at build time; generated code calls the driver directly |
| **Language support** | Java, Kotlin | Rust, Python, TypeScript, Go, Java, Kotlin, C#, Elixir, Ruby, PHP |
| **Schema input** | Requires a live database connection for code generation | Reads `.sql` schema files — no running database needed |
| **SQL quality tools** | None | 93 lint rules and integrated formatting |
| **Licensing** | Open source for PostgreSQL, MySQL, SQLite, H2. **Commercial license required** for Oracle, SQL Server, DB2, and others. | MIT license. All databases, all features, no commercial tiers. |
| **Runtime overhead** | DSL evaluation and SQL string construction at runtime | Zero — generated code is static function calls |

Both tools respect SQL as the primary interface to the database. jOOQ is a strong choice if your stack is Java/Kotlin and you prefer composing queries programmatically. Scythe is the better fit if you want plain SQL files, need polyglot support, or want to avoid commercial licensing constraints.

See the [detailed jOOQ comparison](comparisons/jooq.md) for code examples and feature breakdown.

## Custom Types

Scythe maps database types to language-native types automatically. When your schema uses types that scythe does not recognize — extensions like `ltree`, `citext`, or domain types — you define overrides in your configuration:

```toml
[[sql.type_overrides]]
db_type = "ltree"
type = "string"
```

This tells scythe to map any column of type `ltree` to a string in the generated code. Overrides apply globally across all queries in the configuration block.

## SQL Features

Scythe's type inference engine handles the SQL features that ORMs struggle with:

- **CTEs** — basic, recursive, and chained (`WITH a AS (...), b AS (SELECT ... FROM a)`)
- **Window functions** — `ROW_NUMBER`, `RANK`, `DENSE_RANK`, `LAG`, `LEAD`, `NTILE`, `FIRST_VALUE`, `LAST_VALUE` with correct nullability inference for each function
- **Complex JOINs** — `INNER`, `LEFT`, `RIGHT`, `FULL OUTER`, `CROSS` with automatic nullability propagation (left join makes the right side nullable)
- **CASE WHEN** — type widening across branches (`integer` + `null` = `nullable integer`, `integer` + `bigint` = `bigint`)
- **RETURNING clauses** — `INSERT ... RETURNING`, `UPDATE ... RETURNING`, `DELETE ... RETURNING` with full column inference
- **Enums** — PostgreSQL `CREATE TYPE ... AS ENUM` mapped to language-native enum types
- **Composite types** — PostgreSQL `CREATE TYPE ... AS` mapped to structs/classes
- **Arrays** — PostgreSQL array types (`integer[]`, `text[]`) mapped to language-native array/list types
- **JSONB/JSON** — mapped to language-appropriate JSON types with configurable override support

## SQL Should Be Linted and Formatted

SQL is the source of truth. It deserves the same quality tooling as application code:

- **93 lint rules** — correctness checks (UPDATE without WHERE, ambiguous columns, NULL comparisons with `=` instead of `IS`), performance warnings (ORDER BY without LIMIT, leading wildcard LIKE, SELECT *), and style enforcement
- **Integrated formatting** — consistent indentation, keyword capitalization, and spacing via sqruff integration

Scythe runs linting and formatting as part of the compilation pipeline. Bad SQL is caught before code generation, not at runtime.
