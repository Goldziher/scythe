# Annotations

Scythe uses SQL comment annotations to control code generation. All annotations start with `-- @`.

## @name (required)

Names the query. Used as the generated function and struct name.

```sql
-- @name GetUserById
-- @returns :one
SELECT id, name FROM users WHERE id = $1;
```

Generates: `get_user_by_id()` function and `GetUserByIdRow` struct.

## @returns (required)

Specifies the query return type. Must include a colon prefix.

| Value | Description | Use Case |
|-------|-------------|----------|
| `:one` | Returns exactly one row (errors if missing) | SELECT ... WHERE id = $1 |
| `:opt` | Returns zero or one row (nullable/optional) | SELECT ... WHERE email = $1 |
| `:many` | Returns multiple rows | SELECT ... WHERE status = $1 |
| `:exec` | Returns nothing | INSERT, UPDATE, DELETE without RETURNING |
| `:exec_result` | Returns affected row count | UPDATE/DELETE when you need the count |
| `:exec_rows` | Returns affected rows | Similar to exec_result |
| `:batch` | Batch execution | Bulk inserts |
| `:grouped` | Returns rows grouped by a key | JOIN queries with parent-child nesting |

```sql
-- @name ListActiveUsers
-- @returns :many
SELECT id, name, email FROM users WHERE status = 'active';

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = $1;
```

## @group_by

Specifies which table's columns become the parent struct when using `@returns :grouped`. All other selected columns become children collected into a nested list.

Format: `-- @group_by table.column`

This annotation is required when `@returns :grouped` is used and produces an error if omitted.

```sql
-- @name GetUsersWithOrders
-- @returns :grouped
-- @group_by users.id
SELECT
    u.id,
    u.name,
    u.email,
    o.id AS order_id,
    o.total,
    o.created_at AS order_date
FROM users u
JOIN orders o ON o.user_id = u.id
WHERE u.status = 'active';
```

This generates a parent struct containing the `users` columns (`id`, `name`, `email`) with a nested collection of child structs containing the `orders` columns (`order_id`, `total`, `order_date`). The exact shape depends on the backend language -- for example, Rust generates a `Vec<ChildRow>` field, Python generates a `list[ChildRow]` field, and so on.

## @optional

Marks a query parameter as optional. Scythe rewrites the SQL at generation time so that passing NULL for the parameter skips the filter condition entirely.

```sql
-- @name ListUsers
-- @returns :many
-- @optional status
SELECT id, name, email FROM users WHERE status = $1;
```

Scythe rewrites `WHERE status = $1` into `WHERE ($1 IS NULL OR status = $1)`. At runtime, passing NULL returns all rows; passing a value filters normally.

### Supported operators

`@optional` works with the following comparison operators:

| Operator | Rewritten form |
|----------|----------------|
| `=` | `($1 IS NULL OR col = $1)` |
| `<>` | `($1 IS NULL OR col <> $1)` |
| `!=` | `($1 IS NULL OR col != $1)` |
| `>` | `($1 IS NULL OR col > $1)` |
| `<` | `($1 IS NULL OR col < $1)` |
| `>=` | `($1 IS NULL OR col >= $1)` |
| `<=` | `($1 IS NULL OR col <= $1)` |
| `LIKE` | `($1 IS NULL OR col LIKE $1)` |
| `ILIKE` | `($1 IS NULL OR col ILIKE $1)` |

### Multiple optional parameters

```sql
-- @name SearchUsers
-- @returns :many
-- @optional status
-- @optional name_pattern
SELECT id, name, email FROM users
WHERE status = $1 AND name ILIKE $2;
```

### Parameter name validation

Parameter names in `@optional` are validated against the query. If the name does not match any parameter, scythe produces an error -- catching typos at generation time rather than at runtime.

## @param

Documents a query parameter. Does not affect code generation, but adds documentation to generated code.

```sql
-- @name GetUser
-- @returns :one
-- @param id: the user's unique identifier
-- @param status: filter by account status
SELECT id, name FROM users WHERE id = $1 AND status = $2;
```

Format: `-- @param <name>: <description>` or `-- @param <name>` (without description).

## @nullable

Forces specific columns to be nullable in generated code, overriding the inferred nullability.

```sql
-- @name GetUserProfile
-- @returns :one
-- @nullable bio, avatar_url
SELECT id, name, bio, avatar_url FROM users WHERE id = $1;
```

Accepts a comma-separated list of column names.

## @nonnull

Forces specific columns to be non-nullable in generated code, overriding the inferred nullability.

```sql
-- @name GetUserWithDefaults
-- @returns :one
-- @nonnull status
SELECT id, name, COALESCE(status, 'active') AS status FROM users WHERE id = $1;
```

Useful when you know a column cannot be null due to application logic that the analyzer cannot infer.

## @json

Maps a column to a typed JSON struct instead of a generic JSON value.

```sql
-- @name GetEvent
-- @returns :one
-- @json data = EventData
SELECT id, data FROM events WHERE id = $1;
```

Format: `-- @json <column> = <TypeName>`. The generated code will use `Json<EventData>` (Rust) or equivalent typed wrapper instead of a raw JSON value.

## @deprecated

Marks a query as deprecated. The generated code will include deprecation annotations in languages that support them.

```sql
-- @name GetUserV1
-- @returns :one
-- @deprecated Use GetUserV2 instead
SELECT id, name FROM users WHERE id = $1;
```

In Rust, this generates `#[deprecated(note = "Use GetUserV2 instead")]` on the function.

## Complete Example

```sql
-- @name GetOrderDetails
-- @returns :one
-- @param order_id: the order to look up
-- @optional status
-- @nullable discount_code
-- @nonnull total
-- @json metadata = OrderMetadata
-- @deprecated Use GetOrderDetailsV2 for new code
SELECT
    o.id,
    o.total,
    o.discount_code,
    o.metadata,
    u.name AS customer_name
FROM orders o
JOIN users u ON o.user_id = u.id
WHERE o.id = $1;
```

## Case Insensitivity

Annotation keywords are case-insensitive. These are equivalent:

```sql
-- @name GetUser
-- @Name GetUser
-- @NAME GetUser
```

The annotation value (e.g., the query name) preserves its original casing.
