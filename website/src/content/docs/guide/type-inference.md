---
title: Type Inference
description: How scythe infers nullability from JOINs, COALESCE, CASE, window functions, and aggregates.
---

Scythe infers types from your SQL schema and query structure. The key insight: not all columns are nullable, and not all nullable columns stay nullable after transformation.

## Neutral Type System

Scythe uses a language-neutral type vocabulary internally. Each backend maps these to concrete types.

| Neutral Type | PostgreSQL Source |
|---|---|
| `bool` | `BOOLEAN` |
| `int16` | `SMALLINT` |
| `int32` | `INTEGER`, `SERIAL` |
| `int64` | `BIGINT`, `BIGSERIAL` |
| `float32` | `REAL` |
| `float64` | `DOUBLE PRECISION` |
| `string` | `TEXT`, `VARCHAR`, `CHAR` |
| `bytes` | `BYTEA` |
| `decimal` | `NUMERIC`, `DECIMAL` |
| `uuid` | `UUID` |
| `date` | `DATE` |
| `time` | `TIME` |
| `time_tz` | `TIME WITH TIME ZONE` |
| `datetime` | `TIMESTAMP` |
| `datetime_tz` | `TIMESTAMPTZ` |
| `interval` | `INTERVAL` |
| `json` | `JSON`, `JSONB` |
| `inet` | `INET`, `CIDR` |

## Nullability from JOINs

Columns from the right side of a `LEFT JOIN` are always nullable, even if the column is defined as `NOT NULL`:

```sql
-- @name GetUserOrders
-- @returns :many
SELECT u.id, u.name, o.total
FROM users u
LEFT JOIN orders o ON u.id = o.user_id;
```

| Column | Type | Nullable | Reason |
|--------|------|----------|--------|
| `u.id` | `int32` | no | Left side of LEFT JOIN |
| `u.name` | `string` | no | Left side of LEFT JOIN |
| `o.total` | `decimal` | **yes** | Right side of LEFT JOIN |

Similarly, columns from the left side of a `RIGHT JOIN` become nullable.

The same widening applies to `LEFT SEMI JOIN` and `LEFT ANTI JOIN` (the non-preserved side's columns
are not projected, but the widening rule is the same as `LEFT JOIN`). `FULL OUTER JOIN` widens **both**
sides -- every column from either table can be NULL, since a row might exist on only one side. This
also applies inside nested joins (parenthesized `FROM` clauses).

## Nullability from COALESCE

`COALESCE` strips nullability when **any** argument -- not just the last one -- is non-nullable:

```sql
-- @name GetUserDisplayName
-- @returns :one
SELECT COALESCE(nickname, name, 'Anonymous') AS display_name
FROM users WHERE id = $1;
```

`display_name` is non-nullable because the final fallback `'Anonymous'` is a non-null literal. The
same result holds if any earlier argument -- not only the last -- is itself non-nullable (for example
a `NOT NULL` column), even if later arguments are nullable.

```sql
-- @name GetUserNickname
-- @returns :one
SELECT COALESCE(nickname, name) AS display_name
FROM users WHERE id = $1;
```

If both `nickname` and `name` are nullable columns, `display_name` remains nullable.

:::caution[Oracle exception]
On Oracle, empty string literals (`''`, `N''`) evaluate to NULL rather than an empty value. A literal
fallback like `COALESCE(email, '')` is treated as a non-null literal on every other dialect, but on
Oracle the fallback itself is NULL, so `display_name` stays **nullable** there. See
[Oracle: Empty string is NULL](/scythe/databases/oracle/#notes) and the live conformance fixture
`testing_data/nullability_live/coalesce_non_null/live_coalesce_with_empty_string_default_is_null_on_oracle.json`.
:::

## Nullability from Aggregates

Aggregate functions have specific nullability rules for ordinary (non-windowed) aggregation:

| Function | Nullable? | Reason |
|----------|-----------|--------|
| `COUNT(*)` | no | Always returns a number |
| `COUNT(col)` | no | Always returns a number |
| `SUM(col)` | yes | Returns NULL for empty sets |
| `AVG(col)` | yes | Returns NULL for empty sets |
| `MIN(col)` | yes | Returns NULL for empty sets |
| `MAX(col)` | yes | Returns NULL for empty sets |

```sql
-- @name GetUserStats
-- @returns :one
SELECT
    COUNT(*) AS total_orders,
    SUM(total) AS revenue,
    MAX(created_at) AS last_order
FROM orders WHERE user_id = $1;
```

| Column | Nullable | Reason |
|--------|----------|--------|
| `total_orders` | no | COUNT is never null |
| `revenue` | yes | SUM returns NULL for empty result |
| `last_order` | yes | MAX returns NULL for empty result |

### Windowed aggregates

A windowed aggregate (`OVER (...)`) produces one output row per input row, so an empty result set
means zero output rows rather than a NULL aggregate value. Nullability differs from the non-windowed
case above:

| Function | Windowed nullability | Reason |
|----------|----------------------|--------|
| `SUM(col) OVER (...)` | no | Cannot be evaluated over zero rows within the window |
| `AVG(col) OVER (...)` | no | Same as `SUM` |
| `MIN(col) OVER (...)` / `MAX(col) OVER (...)` | same as `col` | Takes the nullability of the argument column, not `true` |

### Aggregate result types

Aggregates can widen the argument's neutral type, not just its nullability:

| Function | Argument type | Result type |
|----------|---------------|-------------|
| `SUM` | `int32` (or narrower) | `int64` |
| `SUM` | `int64` | `decimal` (arbitrary precision, since `int64` has no wider integer type) |
| `SUM` | float type | Same float type, unwidened |
| `AVG` | non-float | `decimal` |
| `AVG` | float type | `float64` |

## Nested aggregates

On PostgreSQL, `json_agg(alias.*)`, `jsonb_agg(alias.*)`, `row_to_json(alias.*)`,
`to_json(alias.*)` and `to_jsonb(alias.*)` resolve to a struct scythe synthesizes from the
aggregated relation, not to an opaque `json` scalar. The neutral type is `json_nested<T>`:

| Expression | Join | Neutral type |
|---|---|---|
| `json_agg(o.*)`, `jsonb_agg(o.*)` | inner | `json_nested<array<GetUserOrdersRowOrders>>` |
| `json_agg(o.*)`, `jsonb_agg(o.*)` | outer | `json_nested<array<nullable<GetUserOrdersOuterRowOrders>>>` |
| `row_to_json(o.*)`, `to_json(o.*)`, `to_jsonb(o.*)` | -- | `json_nested<GetOrderPayloadRowPayload>` |

`jsonb_agg` differs from `json_agg` only in storage type, and `to_json`/`to_jsonb` over a whole-row
reference return the same document `row_to_json` does, so each pair infers the same shape.

The struct name is the query name, then `Row`, then the output column name: query `GetUserOrders` with
an `AS orders` column produces `GetUserOrdersRowOrders`. A name that collides with a composite or enum
in the catalog gains a numeric suffix. Fields are the aggregated relation's columns in schema order,
each keeping its own schema nullability.

The bare-identifier form (`json_agg(o)`, where `o` is a table alias rather than a column) is
recognized the same way.

### Generated code

```sql
-- @name GetUsersWithOrders
-- @returns :many
SELECT u.id, u.name, json_agg(o.*) AS orders
FROM users u
JOIN orders o ON o.user_id = u.id
GROUP BY u.id, u.name;
```

Against an `orders` table of `id`, `user_id`, `total`, `weight_kg`, `notes`, `created_at`, the
`rust-sqlx` backend emits:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GetUsersWithOrdersRowOrders {
    pub id: i32,
    pub user_id: i32,
    pub total: rust_decimal::Decimal,
    pub weight_kg: Option<f64>,
    pub notes: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUsersWithOrdersRow {
    pub id: i32,
    pub name: String,
    pub orders: Option<sqlx::types::Json<Vec<GetUsersWithOrdersRowOrders>>>,
}
```

`row_to_json` has no array wrapper -- one JSON object per output row rather than one element per
aggregated row:

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserAsJsonRow {
    pub payload: Option<sqlx::types::Json<GetUserAsJsonRowPayload>>,
}
```

### Nullability of the aggregate

The column is always nullable: `json_agg`/`jsonb_agg` return NULL for an empty group, and
`row_to_json`/`to_json`/`to_jsonb` over a null-extended row return SQL NULL.

An outer join widens the array *element*, not the fields. For a non-matching row PostgreSQL makes the
whole-row variable itself NULL, so `json_agg(o.*)` aggregates one JSON `null` and the column's value
is `[null]` -- never `[{"id": null, ...}]`. The same query over a `LEFT JOIN` therefore emits:

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUsersWithOrdersOuterRow {
    pub id: i32,
    pub orders: Option<sqlx::types::Json<Vec<Option<GetUsersWithOrdersOuterRowOrders>>>>,
}
```

`json_agg(o.*) FILTER (WHERE o.id IS NOT NULL)` -- the idiom for suppressing that `[null]` -- cannot
produce a null element, but scythe does not prove that. The element stays optional.

### JSON keys

The keys `json_agg` and `row_to_json` emit are the raw SQL column names, which need not match the
generated field names. Each backend spells the mapping out:

| Backend | Row field type for `json_nested<T>` | Key mapping |
|---|---|---|
| `rust-sqlx` | `sqlx::types::Json<T>` | `#[serde(rename = "...")]` where the field name differs |
| `rust-tokio-postgres` | `postgres_types::Json<T>` | same |
| `go-pgx` | `T` | `json:"..."` struct tag |
| `python-psycopg3` | `T` | a `_from_json` classmethod |

A quoted mixed-case column keeps its key and renames the field:

```rust
    #[serde(rename = "createdAt")]
    pub created_at: chrono::DateTime<chrono::Utc>,
```

An enum reachable only through a nested struct is emitted with serde derives and per-variant renames,
because inside a JSON result the value is decoded by serde rather than off the wire by the driver:

```rust
#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type, serde::Serialize, serde::Deserialize)]
#[sqlx(type_name = "user_status", rename_all = "snake_case")]
pub enum UserStatus {
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "inactive")]
    Inactive,
    #[serde(rename = "banned")]
    Banned,
}
```

### Where it does not apply

Only the four backends in the table above emit nested structs. Every other backend rewrites the column
to plain `json`, byte-identical to what it produced before nested inference existed -- including the
enums and composites reachable only through the discarded struct, which are not emitted.

Inference is also gated on the engine. PostgreSQL and CockroachDB infer nested structs; on every other
engine `json_agg` keeps its plain `json` mapping. That includes Redshift and DuckDB, which map onto
the PostgreSQL dialect but have no `json_agg`.

These are not covered:

- `json_object_agg` and `jsonb_object_agg` -- plain `json`. They build an object keyed by the
  runtime values of their first argument (`json_object_agg(o.id, o.status)` yields
  `{"1": "shipped"}`), so there is no fixed field set to synthesize a struct from.
- `json_agg`/`jsonb_agg` over a scalar expression, or over a bare `*` -- plain `json`.
- `to_json`/`to_jsonb` over a scalar expression -- plain `json`, nullable exactly when the argument
  is (both are strict: `to_json(NULL)` is SQL NULL, not the JSON document `null`).
- `json_build_object`, `json_build_array` and their `jsonb` spellings -- plain, non-nullable `json`.
  They are not strict, so a NULL argument becomes a JSON `null` inside a non-NULL document.
- `json_strip_nulls`/`jsonb_strip_nulls` -- plain `json`, nullable exactly when the argument is.
  They are strict, unlike the `json_build_*` family they sit next to.
- `array_agg` and `string_agg` -- unaffected, still `array<T>` and `string`.
- A nested aggregate over a nested aggregate -- rejected with an error. Wrap one level per query.
- `@json` annotations, which produce `json_typed<T>` -- untouched.

Two queries in one output file that derive the same struct name emit one definition when their field
shapes match, and fail with an error naming both when they do not.

See [Neutral Types](/scythe/reference/neutral-types/) for the full container table.

## Nullability from CASE

CASE expressions are nullable if any branch can produce NULL:

```sql
-- @name GetUserTier
-- @returns :one
SELECT
    CASE
        WHEN total_spent > 1000 THEN 'gold'
        WHEN total_spent > 100 THEN 'silver'
        ELSE 'bronze'
    END AS tier
FROM users WHERE id = $1;
```

`tier` is non-nullable because all branches (including ELSE) produce non-null values.

```sql
SELECT
    CASE
        WHEN total_spent > 1000 THEN 'gold'
    END AS tier
FROM users WHERE id = $1;
```

`tier` is nullable because the implicit ELSE returns NULL.

### The `IS NOT NULL` guard exception

A branch that returns a nullable column is not counted as nullable if its condition is exactly
`WHEN <col> IS NOT NULL THEN <col>` -- guarding a column with its own null check proves that branch's
result cannot be NULL:

```sql
SELECT
    CASE WHEN bio IS NOT NULL THEN bio ELSE 'No bio' END AS bio_display
FROM users WHERE id = $1;
```

`bio_display` is non-nullable: the `bio` branch is exempted by the guard, and the `ELSE` branch is a
non-null literal. This exception only matches the exact `col IS NOT NULL THEN col` shape (same
expression on both sides); it does not generalize to other guard conditions.

## Nullability from Expressions

Binary operators fall into three groups:

- **Arithmetic (`+`, `-`, `*`, `/`, `%`) and string concatenation (`||`)** propagate nullability: if
  either operand is nullable, the result is nullable.

  ```sql
  SELECT a + b AS sum FROM t;
  ```

  If either `a` or `b` is nullable, `sum` is nullable.

- **Comparison (`=`, `<>`, `<`, `<=`, `>`, `>=`) and boolean (`AND`, `OR`)** always produce a
  non-nullable `bool`, regardless of operand nullability.

- **JSON operators (`->`, `->>`, `#>`, `#>>`)** are always nullable, regardless of operand nullability
  -- a missing key or path segment naturally produces SQL NULL.

### Scalar subqueries

A scalar subquery (a subquery used as a single expression value) is nullable if it can return zero
rows, because a zero-row result evaluates to SQL NULL regardless of the projected column's own
nullability. The one exception is an ungrouped aggregate query with no `HAVING` clause and a single
projected column (e.g. `(SELECT COUNT(*) FROM ...)`) -- that shape always returns exactly one row, so
its nullability is the aggregate's own nullability from the tables above, not forced to nullable.

## Manual Overrides

When the analyzer cannot determine nullability correctly, use annotations:

```sql
-- @name GetUser
-- @returns :one
-- @nullable bio
-- @nonnull computed_status
SELECT id, bio, some_complex_expression() AS computed_status
FROM users WHERE id = $1;
```

See [Annotations](/scythe/guide/annotations/) for details on `@nullable` and `@nonnull`.
