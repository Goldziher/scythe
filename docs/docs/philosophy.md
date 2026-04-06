# Philosophy

## SQL is the Source of Truth

SQL is a battle-tested, mature, and type-safe database language. It has been refined over 50 years and is supported by every major database engine. It should be the source of truth for your data layer.

ORMs try to abstract SQL to make application code the source of truth. In the process, they:

- Make complex database operations difficult or impossible
- Introduce unnecessary abstractions and bloat
- Generate inefficient queries behind opaque APIs
- Create a false sense of type safety while losing SQL's real type system

## Write SQL, Generate Code

Scythe follows the philosophy pioneered by [sqlc](https://github.com/sqlc-dev/sqlc): **write real SQL, generate type-safe code**.

Your SQL queries are the contract between your application and your database. Scythe:

1. **Parses** your SQL schema and annotated queries
2. **Infers** types with precision -- including nullability from JOINs, COALESCE, aggregates, and CASE expressions
3. **Generates** idiomatic, type-safe code in your language of choice
4. **Lints** your SQL for correctness, performance, and style
5. **Formats** your SQL for consistency

## Database-Facing Code Should Be

- **Simple** -- no magic, no hidden queries, no lazy loading surprises
- **Type-safe** -- compile-time guarantees that your code matches your schema
- **Performant** -- you write the SQL, you control the execution plan
- **Transparent** -- what you write is what runs

## SQL Should Be Linted and Formatted

Since SQL is the source of truth, it deserves the same treatment as application code:

- **Linted** for correctness (UPDATE without WHERE, ambiguous columns, NULL comparisons)
- **Linted** for performance (ORDER BY without LIMIT, leading wildcard LIKE)
- **Formatted** for consistency (indentation, capitalization, spacing)

Scythe provides 93 lint rules and integrated SQL formatting to ensure your SQL is clean, correct, and consistent.

## Why Not an ORM?

| Aspect | ORM | Scythe |
|--------|-----|--------|
| Source of truth | Application code | SQL |
| Complex queries | Difficult/impossible | Natural |
| Performance | Opaque, often suboptimal | You control the SQL |
| Type safety | Compile-time (limited) | Compile-time (precise) |
| Learning curve | ORM API + SQL | Just SQL |
| Generated code | Hidden, hard to debug | Visible, readable |
