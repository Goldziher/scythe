<!--
AI-RULEZ :: GENERATED FILE — DO NOT EDIT
Content-Hash: blake3:9faebdc4be8cf6282b4092246fd91032bb3d7b8ef2c1101b16959316ea16a9ef
Source-Hash: blake3:dda6f642ae20dd0ec58dfb83a2337b9997b046a92f543f4dfb9a5ca28f2dd3f7
Schema-Version: v1
-->

# Annotations Reference

All annotations use `-- @` prefix in SQL comments. Keywords are case-insensitive.

## @name (required)

Names the query. Used as the generated function and struct name.

```sql
-- @name GetUserById
```

Generates: `get_user_by_id()` function and `GetUserByIdRow` struct (naming depends on backend).

## @returns (required)

Specifies the query return type. Use the canonical colon-prefixed spelling shown below.

| Value | Description | Use Case |
|-------|-------------|----------|
| `:one` | Requires exactly one row | SELECT ... WHERE id = $1 |
| `:opt` | Returns an optional row | Zero-or-one SELECT |
| `:many` | Returns multiple rows | SELECT ... WHERE status = $1 |
| `:exec` | Returns nothing | INSERT, UPDATE, DELETE |
| `:exec_result` | Driver result or affected count, depending on backend | DML driver result |
| `:exec_rows` | Returns affected row count | UPDATE/DELETE when count matters |
| `:batch` | Batch execution | Bulk inserts |
| `:grouped` | Grouped results | JOIN with parent-child nesting |

## @group_by

Required with `@returns :grouped`. Specifies which table's columns form the parent struct.

```sql
-- @name GetUsersWithOrders
-- @returns :grouped
-- @group_by users.id
SELECT u.id, u.name, o.id AS order_id, o.total
FROM users u JOIN orders o ON o.user_id = u.id;
```

## @optional

Makes a query parameter optional. Scythe rewrites SQL so NULL skips the filter.

```sql
-- @name SearchUsers
-- @returns :many
-- @optional status
-- @optional name
SELECT id, name FROM users WHERE status = $1 AND name ILIKE $2;
```

Rewritten: `WHERE ($1 IS NULL OR status = $1) AND ($2 IS NULL OR name ILIKE $2)`

Supported operators: `=`, `<>`, `!=`, `>`, `<`, `>=`, `<=`, `LIKE`, `ILIKE`

Parameter names are validated against the query -- typos cause errors.

## @param

Documents a query parameter (does not affect code generation).

```sql
-- @param id: the user's unique identifier
-- @param status: filter by account status
```

## @nullable

Forces columns to be nullable in generated code.

```sql
-- @nullable bio, avatar_url
```

## @nonnull

Forces columns to be non-nullable in generated code.

```sql
-- @nonnull status
```

## @json

Maps a column to a typed JSON struct.

```sql
-- @json data = EventData
```

Generates `Json<EventData>` (Rust), typed wrapper in other languages.

## @deprecated

Marks a query as deprecated with an optional message.

```sql
-- @deprecated Use GetUserV2 instead
```

## Complete Example

```sql
-- @name GetOrderDetails
-- @returns :one
-- @param $1 order_id: the order to look up
-- @nullable discount_code
-- @nonnull total
-- @json metadata = OrderMetadata
-- @deprecated Use GetOrderDetailsV2 for new code
SELECT
    o.id, o.total, o.discount_code, o.metadata,
    u.name AS customer_name
FROM orders o
JOIN users u ON o.user_id = u.id
WHERE o.id = $1;
```
