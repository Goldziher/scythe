---
title: Java + Kotlin (JDBC, R2DBC, Exposed)
description: The java-jdbc, kotlin-jdbc, java-r2dbc, kotlin-r2dbc, and kotlin-exposed backends.
---

Backends: `java-jdbc`, `kotlin-jdbc`, `java-r2dbc`, `kotlin-r2dbc`, `kotlin-exposed`

`java-jdbc` and `kotlin-jdbc` support 9 engines (PostgreSQL, MySQL, MariaDB, SQLite, DuckDB, MSSQL,
Redshift, Snowflake, Oracle). `java-r2dbc` and `kotlin-r2dbc` support PostgreSQL, MySQL, MariaDB, and
SQLite. `kotlin-exposed` supports PostgreSQL only. The examples on this page use PostgreSQL.

Generated files carry a provenance header as their first line, e.g.
`// scythe:provenance v=0.13.0 backend=java-jdbc engine=postgresql schema=sch1:...`
(`integration_tests/java-jdbc/src/main/java/generated/Queries.java:1`).

## Field naming: `field_case`

Generated field and record-component names are `snake_case` by default, mirroring the SQL column name
-- **not** `camelCase` (`field_case` defaults to `snake_case`;
`crates/scythe-backend/src/naming.rs:24-30`). `created_at` stays `created_at`, not `createdAt`.

All five backends on this page accept a `field_case` option -- `snake_case` or `camelCase`, and
nothing else -- to opt into `camelCase` fields. It is a `[[sql.gen]]` target key only; a manifest
cannot set it, because `NamingConfig::field_case` carries `#[serde(skip)]`
(`crates/scythe-backend/src/naming.rs:24-25`). A `field_case` key under a full manifest's `[naming]`
table is ignored, and naming it in a partial manifest override is a parse error.

```toml
[[sql.gen]]
backend = "java-jdbc"
output = "src/generated/java"
field_case = "camelCase"
```

`camelCase` renames record components, data-class properties, and query-function parameters --
`created_at` becomes `createdAt`. It does not change decoding: every backend on this page reads the
`ResultSet`/`Row` by the raw SQL column name (`rs.getObject("created_at", ...)`,
`row.get("created_at", ...)`), and only the declared field name changes
(`crates/scythe-codegen/src/backends/java_jdbc.rs`, `kotlin_jdbc.rs`). Two SQL identifiers that
collapse onto the same generated name are a hard error, not last-write-wins. See
[`field_case`](/scythe/guide/configuration/#field_case) in the Configuration guide for the full
option reference.

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
    java.time.OffsetDateTime created_at
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

### `:one` -- returns `@Nullable T`, null on no rows

```java
public static @Nullable GetUserRow getUser(Connection conn, int id) throws SQLException {
    try (var ps = conn.prepareStatement(
            "SELECT id, name, email, created_at FROM users WHERE id = ?")) {
        ps.setInt(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetUserRow.fromResultSet(rs);
            }
            return null;
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
    try (var ps = conn.prepareStatement(
            "SELECT id, name FROM users ORDER BY name LIMIT ?")) {
        ps.setLong(1, limit);
        try (ResultSet rs = ps.executeQuery()) {
            List<ListUsersRow> result = new ArrayList<>();
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

`kotlin-jdbc` also accepts an `extension_functions` option (default `false`): when `true`, query
functions are generated as `Connection.query(...)` extension functions instead of taking `conn` as a
parameter.

### Data class with `.use {}`

```kotlin
data class GetUserRow(
    val id: Int,
    val name: String,
    val email: String?,
    val created_at: java.time.OffsetDateTime,
)
```

### `:one` -- returns `T?`, null on no rows

```kotlin
fun getUser(
    conn: Connection,
    id: Int,
): GetUserRow? {
    conn.prepareStatement("SELECT id, name, email, created_at FROM users WHERE id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                GetUserRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    created_at = rs.getObject("created_at", OffsetDateTime::class.java),
                )
            } else {
                null
            }
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

Backend: `java-r2dbc` | Library: R2DBC with Project Reactor | Engines: PostgreSQL, MySQL, MariaDB, SQLite

Generates reactive code using `Mono<T>` for `:one`/`:exec` queries and `Flux<T>` for `:many` queries. Requires a `ConnectionFactory` instead of a JDBC `Connection`.

Row types are plain records with **no `fromRow` method** -- row-to-object mapping is inlined directly
into each query function (`crates/scythe-codegen/src/backends/java_r2dbc.rs:189-210`):

```java
public record GetUserRow(
    int id,
    String name,
    @Nullable String email,
    java.time.OffsetDateTime created_at
) {}
```

Resource handling uses `Mono.usingWhen(...)` / `Flux.usingWhen(...)`, not `flatMap` + `doFinally`.
Binds are **zero-based positional integers** -- `.bind(0, x)`, `.bind(1, y)`, ... -- not `"$1"` string
keys, even against PostgreSQL:

### `:one`

```java
public static Mono<GetUserRow> getUser(ConnectionFactory cf, int id) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("SELECT id, name, email, created_at FROM users WHERE id = $1");
            stmt.bind(0, id);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.map((row, meta) ->
                    new GetUserRow(
                        row.get("id", Integer.class),
                        row.get("name", String.class),
                        row.get("email", String.class),
                        row.get("created_at", java.time.OffsetDateTime.class)
                    ))));
        },
        conn -> Mono.from(conn.close())
    );
}
```

### `:many`

```java
public static Flux<ListUsersRow> listUsers(ConnectionFactory cf, long limit) {
    return Flux.usingWhen(
        cf.create(),
        conn -> {
            var stmt = conn.createStatement("SELECT id, name FROM users ORDER BY name LIMIT $1");
            stmt.bind(0, limit);
            return Flux.from(stmt.execute())
                .flatMap(result -> result.map((row, meta) ->
                    new ListUsersRow(
                        row.get("id", Integer.class),
                        row.get("name", String.class)
                    )));
        },
        conn -> Mono.from(conn.close())
    );
}
```

### `:exec`

```java
public static Mono<Void> createUser(ConnectionFactory cf, String name, @Nullable String email) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("INSERT INTO users (name, email) VALUES ($1, $2)");
            stmt.bind(0, name);
            stmt.bind(1, email);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.getRowsUpdated()))
                .then();
        },
        conn -> Mono.from(conn.close())
    );
}
```

---

## Kotlin R2DBC

Backend: `kotlin-r2dbc` | Library: R2DBC with Kotlin coroutines | Engines: PostgreSQL, MySQL, MariaDB, SQLite

Generates coroutine-based code using `suspend fun` for `:one` and `:exec` queries, and `Flow<T>` for
`:many` queries, via `kotlinx-coroutines-reactor`'s `awaitFirst` / `awaitFirstOrNull` / `asFlow`. Like
`java-r2dbc`, there is no separate row-mapping method and binds are zero-based positional integers.
Acquiring the connection is `Mono.from(cf.create()).awaitFirst()` -- not `cf.create().awaitFirst()`,
since `create()` returns a reactive-streams `Publisher`, not a `Mono` directly.

An `extension_functions` option (default `false`) generates `Connection.query(...)` extension
functions instead of top-level functions taking a `ConnectionFactory` parameter.

### `:one`

```kotlin
suspend fun getUser(
    cf: ConnectionFactory,
    id: Int,
): GetUserRow? {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("SELECT id, name, email, created_at FROM users WHERE id = $1")
        stmt.bind(0, id)
        return Mono
            .from(stmt.execute())
            .flatMap { result ->
                Mono.from(
                    result.map { row, _ ->
                        GetUserRow(
                            id = row.get("id", Int::class.javaObjectType),
                            name = row.get("name", String::class.java),
                            email = row.get("email", String::class.java),
                            created_at = row.get("created_at", java.time.OffsetDateTime::class.java),
                        )
                    },
                )
            }.awaitFirstOrNull()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}
```

### `:many`

```kotlin
fun listUsers(
    cf: ConnectionFactory,
    limit: Long,
): Flow<ListUsersRow> =
    Flux
        .usingWhen(
            cf.create(),
            { conn ->
                val stmt = conn.createStatement("SELECT id, name FROM users ORDER BY name LIMIT $1")
                stmt.bind(0, limit)
                Flux
                    .from(stmt.execute())
                    .flatMap { result ->
                        result.map { row, _ ->
                            ListUsersRow(
                                id = row.get("id", Int::class.javaObjectType),
                                name = row.get("name", String::class.java),
                            )
                        }
                    }
            },
            { conn -> Mono.from(conn.close()) },
        ).asFlow()
```

### `:exec`

```kotlin
suspend fun createUser(
    cf: ConnectionFactory,
    name: String,
    email: String?,
) {
    val conn = Mono.from(cf.create()).awaitFirst()
    try {
        val stmt = conn.createStatement("INSERT INTO users (name, email) VALUES ($1, $2)")
        stmt.bind(0, name)
        stmt.bind(1, email)
        Mono.from(stmt.execute()).flatMap { result -> Mono.from(result.rowsUpdated) }.awaitFirstOrNull()
    } finally {
        Mono.from(conn.close()).awaitFirstOrNull()
    }
}
```

---

## Kotlin Exposed

Backend: `kotlin-exposed` | Library: JetBrains Exposed | Engine: PostgreSQL

`kotlin-exposed` does **not** generate Exposed's type-safe query DSL (`selectAll().where {}`,
`insert {}`). Every query -- `:one`, `:many`, and `:exec` alike -- is emitted as raw SQL passed to
`exec()` inside a `transaction {}` block, decoding rows with plain JDBC `ResultSet` getters
(`crates/scythe-codegen/src/backends/kotlin_exposed.rs:228-330`). Table objects are still generated for
table declarations (not queries), but as `IntIdTable`/`LongIdTable`/`UUIDTable` subclasses -- never a
bare `Table` -- and without a `PrimaryKey` override, since the id table base class already declares one:

### Table object

```kotlin
object UsersTable : IntIdTable("users") {
    val id = integer("id")
    val name = text("name")
    val email = text("email").nullable()
    val created_at = timestampWithTimeZone("created_at")
}
```

### `:one` -- raw SQL via `exec()`, decoded manually

```kotlin
data class GetUserRow(
    val id: Int,
    val name: String,
    val email: String?,
    val created_at: java.time.OffsetDateTime,
)

fun getUser(id: Int): GetUserRow? =
    transaction {
        exec("SELECT id, name, email, created_at FROM users WHERE id = ?", listOf(IntegerColumnType() to id)) { rs ->
            if (rs.next()) {
                GetUserRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = rs.getString("email"),
                    created_at = rs.getObject("created_at"),
                )
            } else {
                null
            }
        }
    }
```

### `:many`

```kotlin
data class ListUsersRow(val id: Int, val name: String)

fun listUsers(limit: Long): List<ListUsersRow> =
    transaction {
        val result = mutableListOf<ListUsersRow>()
        exec("SELECT id, name FROM users ORDER BY name LIMIT ?", listOf(LongColumnType() to limit)) { rs ->
            while (rs.next()) {
                result.add(
                    ListUsersRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                    ),
                )
            }
        }
        result
    }
```

### `:exec`

```kotlin
fun createUser(
    name: String,
    email: String?,
) = transaction {
    exec(
        "INSERT INTO users (name, email) VALUES (?, ?)",
        listOf(TextColumnType() to name, TextColumnType() to email),
    )
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
