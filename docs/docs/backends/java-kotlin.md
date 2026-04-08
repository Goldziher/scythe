# Java + Kotlin (JDBC, R2DBC, Exposed)

Backends: `java-jdbc`, `kotlin-jdbc`, `java-r2dbc`, `kotlin-r2dbc`, `kotlin-exposed` | Engine: PostgreSQL

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

---

## Java R2DBC

Backend: `java-r2dbc` | Library: R2DBC with Project Reactor | Engine: PostgreSQL

Generates reactive code using `Mono<T>` for `:one` queries and `Flux<T>` for `:many` queries. Requires a `ConnectionFactory` instead of a JDBC `Connection`.

### Record with row mapping

<!-- snippet:skip -->

```java
public record GetUserRow(
    int id,
    String name,
    @Nullable String email,
    java.time.OffsetDateTime createdAt
) {
    public static GetUserRow fromRow(io.r2dbc.spi.Row row) {
        return new GetUserRow(
            row.get("id", Integer.class),
            row.get("name", String.class),
            row.get("email", String.class),
            row.get("created_at", java.time.OffsetDateTime.class)
        );
    }
}
```

### `:one`

```java
public static Mono<GetUserRow> getUser(ConnectionFactory cf, int id) {
    return Mono.from(cf.create())
        .flatMap(conn -> Mono.from(conn.createStatement(
                "SELECT id, name, email, created_at FROM users WHERE id = $1")
            .bind("$1", id)
            .execute())
        .flatMap(result -> Mono.from(result.map((row, meta) -> GetUserRow.fromRow(row))))
        .doFinally(sig -> conn.close()));
}
```

### `:many`

```java
public static Flux<ListUsersRow> listUsers(ConnectionFactory cf, long limit) {
    return Mono.from(cf.create())
        .flatMapMany(conn -> Flux.from(conn.createStatement(
                "SELECT id, name FROM users ORDER BY name LIMIT $1")
            .bind("$1", limit)
            .execute())
        .flatMap(result -> result.map((row, meta) -> ListUsersRow.fromRow(row)))
        .doFinally(sig -> conn.close()));
}
```

### `:exec`

```java
public static Mono<Void> createUser(ConnectionFactory cf, String name, @Nullable String email) {
    return Mono.from(cf.create())
        .flatMap(conn -> Mono.from(conn.createStatement(
                "INSERT INTO users (name, email) VALUES ($1, $2)")
            .bind("$1", name)
            .bind("$2", email)
            .execute())
        .then()
        .doFinally(sig -> conn.close()));
}
```

---

## Kotlin R2DBC

Backend: `kotlin-r2dbc` | Library: R2DBC with Kotlin coroutines | Engine: PostgreSQL

Generates coroutine-based code using `suspend fun` for `:one` and `:exec` queries, and `Flow<T>` for `:many` queries. Uses `awaitFirst` / `asFlow` extension functions from `kotlinx-coroutines-reactor`.

### `:one`

```kotlin
suspend fun getUser(cf: ConnectionFactory, id: Int): GetUserRow {
    val conn = cf.create().awaitFirst()
    try {
        val result = conn.createStatement(
            "SELECT id, name, email, created_at FROM users WHERE id = \$1"
        ).bind("\$1", id)
            .execute()
            .awaitFirst()
        return result.map { row, _ ->
            GetUserRow(
                id = row.get("id", Int::class.java)!!,
                name = row.get("name", String::class.java)!!,
                email = row.get("email", String::class.java),
                createdAt = row.get("created_at", java.time.OffsetDateTime::class.java)!!,
            )
        }.awaitFirst()
    } finally {
        conn.close().awaitFirstOrNull()
    }
}
```

### `:many`

```kotlin
fun listUsers(cf: ConnectionFactory, limit: Long): Flow<ListUsersRow> = flow {
    val conn = cf.create().awaitFirst()
    try {
        val result = conn.createStatement(
            "SELECT id, name FROM users ORDER BY name LIMIT \$1"
        ).bind("\$1", limit)
            .execute()
            .awaitFirst()
        emitAll(
            result.map { row, _ ->
                ListUsersRow(
                    id = row.get("id", Int::class.java)!!,
                    name = row.get("name", String::class.java)!!,
                )
            }.asFlow()
        )
    } finally {
        conn.close().awaitFirstOrNull()
    }
}
```

### `:exec`

```kotlin
suspend fun createUser(cf: ConnectionFactory, name: String, email: String?) {
    val conn = cf.create().awaitFirst()
    try {
        conn.createStatement(
            "INSERT INTO users (name, email) VALUES (\$1, \$2)"
        ).bind("\$1", name)
            .bind("\$2", email)
            .execute()
            .awaitFirst()
    } finally {
        conn.close().awaitFirstOrNull()
    }
}
```

---

## Kotlin Exposed

Backend: `kotlin-exposed` | Library: JetBrains Exposed | Engine: PostgreSQL

Generates Exposed Table objects and query functions using the `transaction {}` DSL. Table definitions mirror the SQL schema, and queries use Exposed's type-safe DSL or raw SQL via `exec()`.

### Table object

```kotlin
object UsersTable : Table("users") {
    val id = integer("id").autoIncrement()
    val name = text("name")
    val email = text("email").nullable()
    val createdAt = timestampWithTimeZone("created_at")
        .defaultExpression(CurrentTimestampWithTimeZone)

    override val primaryKey = PrimaryKey(id)
}
```

### `:one`

```kotlin
data class GetUserRow(
    val id: Int,
    val name: String,
    val email: String?,
    val createdAt: java.time.OffsetDateTime,
)

fun getUser(id: Int): GetUserRow = transaction {
    UsersTable.selectAll()
        .where { UsersTable.id eq id }
        .single()
        .let { row ->
            GetUserRow(
                id = row[UsersTable.id],
                name = row[UsersTable.name],
                email = row[UsersTable.email],
                createdAt = row[UsersTable.createdAt],
            )
        }
}
```

### `:many`

```kotlin
data class ListUsersRow(val id: Int, val name: String)

fun listUsers(limit: Int): List<ListUsersRow> = transaction {
    UsersTable.select(UsersTable.id, UsersTable.name)
        .orderBy(UsersTable.name)
        .limit(limit)
        .map { row ->
            ListUsersRow(id = row[UsersTable.id], name = row[UsersTable.name])
        }
}
```

### `:exec`

```kotlin
fun createUser(name: String, email: String?) {
    transaction {
        UsersTable.insert {
            it[UsersTable.name] = name
            it[UsersTable.email] = email
        }
    }
}
```

---

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
