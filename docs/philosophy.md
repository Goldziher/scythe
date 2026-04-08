# Philosophy

Scythe compiles SQL into type-safe database access code. You write SQL queries and schema, scythe generates the boilerplate — structs, functions, type mappings — in 10 languages.

## Why Compile SQL?

SQL is a 50-year-old language supported by every major database. It is expressive, optimizable, and well-understood. Developers already know it. Tools already support it. Database engines already optimize it.

The problem is the glue code. Every application that talks to a database needs code that maps query parameters in, maps result rows out, and keeps types aligned between the two worlds. This code is tedious, error-prone, and changes every time the schema or queries change.

Scythe eliminates that glue code. You write `.sql` files — schema definitions and annotated queries — and scythe compiles them into fully typed functions and data structures for your target language. The generated code is readable, has no runtime dependencies beyond your database driver, and stays in sync with your SQL automatically.

The result: you get the full power of SQL with the type safety of generated code, without maintaining the mapping layer by hand.

## Where ORMs Still Win

ORMs provide database portability. If your application supports bring-your-own-database (BYOD) — letting users choose between PostgreSQL, MySQL, SQLite, or others — an ORM abstracts the dialect differences at runtime. The same application code works across databases without maintaining separate SQL files per engine.

Scythe takes the opposite approach: you write SQL for a specific database engine. This gives you full access to engine-specific features (PostgreSQL arrays, MySQL JSON functions, SQLite pragmas) and lets the database optimizer do its job. But it means targeting multiple engines requires separate SQL files and configuration blocks per engine.

If your application must run on whichever database the end user provides, an ORM is the right tool. If you control the database and want type-safe, optimized SQL, scythe is the right tool.

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
