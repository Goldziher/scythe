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
| `:one` | Returns a single row | SELECT ... WHERE id = $1 |
| `:many` | Returns multiple rows | SELECT ... WHERE status = $1 |
| `:exec` | Returns nothing | INSERT, UPDATE, DELETE without RETURNING |
| `:exec_result` | Returns affected row count | UPDATE/DELETE when you need the count |
| `:exec_rows` | Returns affected rows | Similar to exec_result |
| `:batch` | Batch execution | Bulk inserts |

```sql
-- @name ListActiveUsers
-- @returns :many
SELECT id, name, email FROM users WHERE status = 'active';

-- @name DeleteUser
-- @returns :exec
DELETE FROM users WHERE id = $1;
```

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
