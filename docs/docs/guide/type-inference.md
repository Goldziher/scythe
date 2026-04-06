# Type Inference

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

## Nullability from COALESCE

`COALESCE` strips nullability when the last argument is a non-null literal or expression:

```sql
-- @name GetUserDisplayName
-- @returns :one
SELECT COALESCE(nickname, name, 'Anonymous') AS display_name
FROM users WHERE id = $1;
```

`display_name` is non-nullable because the final fallback `'Anonymous'` is a non-null literal.

```sql
-- @name GetUserNickname
-- @returns :one
SELECT COALESCE(nickname, name) AS display_name
FROM users WHERE id = $1;
```

If both `nickname` and `name` are nullable columns, `display_name` remains nullable.

## Nullability from Aggregates

Aggregate functions have specific nullability rules:

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

## Nullability from Expressions

Binary operations propagate nullability:

```sql
SELECT a + b AS sum FROM t;
```

If either `a` or `b` is nullable, `sum` is nullable.

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

See [Annotations](annotations.md) for details on `@nullable` and `@nonnull`.
