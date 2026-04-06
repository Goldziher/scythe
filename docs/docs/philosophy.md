# Philosophy

## SQL is the Source of Truth

SQL is a battle-tested, mature, and type-safe database language. It has been refined over 50 years and is supported by every major database engine. It should be the source of truth for your data layer.

ORMs try to abstract SQL to make application code the source of truth. In the process, they:

- Can make complex database operations difficult
- May introduce unnecessary abstractions
- Can generate suboptimal queries behind opaque APIs
- May not provide the same level of type safety as native SQL

## Write SQL, Generate Code

Scythe follows the philosophy pioneered by [sqlc](https://github.com/sqlc-dev/sqlc): **write real SQL, generate type-safe code**.

Your SQL queries are the contract between your application and your database. Scythe:

See the [Architecture](architecture.md) page for the full pipeline.

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

Scythe provides 22 custom lint rules plus sqruff integration and integrated SQL formatting to ensure your SQL is clean, correct, and consistent.

## Why Not an ORM?

| Aspect | ORM | Scythe |
|--------|-----|--------|
| Source of truth | Application code | SQL |
| Complex queries | Difficult/impossible | Natural |
| Performance | Opaque, often suboptimal | You control the SQL |
| Type safety | Compile-time (limited) | Compile-time (precise) |
| Learning curve | ORM API + SQL | Just SQL |
| Generated code | Hidden, hard to debug | Visible, readable |
