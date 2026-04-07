# Java + Kotlin (JDBC)

Backends: `java-jdbc`, `kotlin-jdbc` | Library: JDBC | Engine: PostgreSQL

## SQL input

```sql
-- @name GetUser
-- @returns :one
SELECT id, name, email, created_at FROM users WHERE id = $1;

-- @name ListUsers
-- @returns :many
SELECT id, name FROM users ORDER BY name LIMIT $1;

-- @name CreateUser
-- @returns :exec
INSERT INTO users (name, email) VALUES ($1, $2);
```

Schema:

```sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

---

## Java

### Record with `fromResultSet`

<!-- snippet:skip -->

```java
public record GetUserRow(
    int id,
    String name,
    @Nullable String email,
    java.time.OffsetDateTime createdAt
) {
    public static GetUserRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetUserRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getString("email"),
            rs.getObject("created_at", java.time.OffsetDateTime.class)
        );
    }
}
```

### `:one`

```java
public static GetUserRow getUser(Connection conn, int id) throws SQLException {
    try (var stmt = conn.prepareStatement(
            "SELECT id, name, email, created_at FROM users WHERE id = ?")) {
        stmt.setInt(1, id);
        try (var rs = stmt.executeQuery()) {
            rs.next();
            return GetUserRow.fromResultSet(rs);
        }
    }
}
```

### `:many`

<!-- snippet:skip -->

```java
public record ListUsersRow(int id, String name) {
    public static ListUsersRow fromResultSet(ResultSet rs) throws SQLException {
        return new ListUsersRow(rs.getInt("id"), rs.getString("name"));
    }
}

public static List<ListUsersRow> listUsers(Connection conn, long limit) throws SQLException {
    try (var stmt = conn.prepareStatement(
            "SELECT id, name FROM users ORDER BY name LIMIT ?")) {
        stmt.setLong(1, limit);
        try (var rs = stmt.executeQuery()) {
            var result = new ArrayList<ListUsersRow>();
            while (rs.next()) {
                result.add(ListUsersRow.fromResultSet(rs));
            }
            return result;
        }
    }
}
```

### `:exec`

```java
public static void createUser(Connection conn, String name, @Nullable String email)
        throws SQLException {
    try (var stmt = conn.prepareStatement(
            "INSERT INTO users (name, email) VALUES (?, ?)")) {
        stmt.setString(1, name);
        stmt.setString(2, email);
        stmt.executeUpdate();
    }
}
```

---

## Kotlin

### Data class with `.use {}`

```kotlin
data class GetUserRow(
    val id: Int,
    val name: String,
    val email: String?,
    val createdAt: java.time.OffsetDateTime,
)
```

### `:one`

```kotlin
fun getUser(conn: Connection, id: Int): GetUserRow {
    conn.prepareStatement(
        "SELECT id, name, email, created_at FROM users WHERE id = ?"
    ).use { stmt ->
        stmt.setInt(1, id)
        stmt.executeQuery().use { rs ->
            rs.next()
            return GetUserRow(
                id = rs.getInt("id"),
                name = rs.getString("name"),
                email = rs.getString("email"),
                createdAt = rs.getObject("created_at", java.time.OffsetDateTime::class.java),
            )
        }
    }
}
```

### `:many`

```kotlin
data class ListUsersRow(val id: Int, val name: String)

fun listUsers(conn: Connection, limit: Long): List<ListUsersRow> {
    conn.prepareStatement(
        "SELECT id, name FROM users ORDER BY name LIMIT ?"
    ).use { stmt ->
        stmt.setLong(1, limit)
        stmt.executeQuery().use { rs ->
            val result = mutableListOf<ListUsersRow>()
            while (rs.next()) {
                result.add(ListUsersRow(id = rs.getInt("id"), name = rs.getString("name")))
            }
            return result
        }
    }
}
```

### `:exec`

```kotlin
fun createUser(conn: Connection, name: String, email: String?) {
    conn.prepareStatement(
        "INSERT INTO users (name, email) VALUES (?, ?)"
    ).use { stmt ->
        stmt.setString(1, name)
        stmt.setString(2, email)
        stmt.executeUpdate()
    }
}
```

## Enum generation

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

**Java:**

```java
public enum UserStatus {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");

    private final String value;
    UserStatus(String value) { this.value = value; }
    public String getValue() { return value; }
}
```

**Kotlin:**

```kotlin
enum class UserStatus(val value: String) {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned"),
}
```

## Type mappings

| SQL Type | Neutral | Java | Kotlin |
|----------|---------|------|--------|
| `INTEGER` | `int32` | `int` | `Int` |
| `BIGINT` | `int64` | `long` | `Long` |
| `TEXT` | `string` | `String` | `String` |
| `BOOLEAN` | `bool` | `boolean` | `Boolean` |
| `BYTEA` | `bytes` | `byte[]` | `ByteArray` |
| `UUID` | `uuid` | `java.util.UUID` | `java.util.UUID` |
| `NUMERIC` | `decimal` | `java.math.BigDecimal` | `java.math.BigDecimal` |
| `DATE` | `date` | `java.time.LocalDate` | `java.time.LocalDate` |
| `TIMESTAMPTZ` | `datetime_tz` | `java.time.OffsetDateTime` | `java.time.OffsetDateTime` |
| `JSON` | `json` | `String` | `String` |
| `TEXT[]` | `array<string>` | `java.util.List<String>` | `List<String>` |
| nullable | `nullable` | `@Nullable T` | `T?` |
