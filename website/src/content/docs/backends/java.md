---
title: Java (JDBC, R2DBC)
description: The java-jdbc and java-r2dbc backends -- generated records, queries, and type mappings.
---

Backends: `java-jdbc`, `java-r2dbc`

`java-jdbc` supports 9 engines (PostgreSQL, MySQL, MariaDB, SQLite, DuckDB, MSSQL, Redshift,
Snowflake, Oracle). `java-r2dbc` supports PostgreSQL, MySQL, MariaDB, and SQLite. The examples on this
page use PostgreSQL.

Generated files carry a provenance header as their first line, e.g.
`// scythe:provenance v=0.15.0 backend=java-jdbc engine=postgresql schema=sch1:... queries=q1:...`
(`integration_tests/java-jdbc/src/main/java/generated/Queries.java:1`).

## Field naming: `field_case`

Generated record-component names are `snake_case` by default, mirroring the SQL column name -- **not**
`camelCase` (`field_case` defaults to `snake_case`; `crates/scythe-backend/src/naming.rs`).
`created_at` stays `created_at`, not `createdAt`.

Both backends on this page accept a `field_case` option -- `snake_case` or `camelCase`, and nothing
else -- to opt into `camelCase` fields. It is a `[[sql.gen]]` target key only; a manifest cannot set
it, because `NamingConfig::field_case` carries `#[serde(skip)]`
(`crates/scythe-backend/src/naming.rs`). A `field_case` key under a full manifest's `[naming]` table is
ignored, and naming it in a partial manifest override is a parse error.

```toml
[[sql.gen]]
backend = "java-jdbc"
output = "src/generated/java"
field_case = "camelCase"
```

`camelCase` renames record components and query-function parameters -- `created_at` becomes
`createdAt`. It does not change decoding: both backends on this page read the `ResultSet`/`Row` by the
raw SQL column name (`rs.getObject("created_at", ...)`, `row.get("created_at", ...)`), and only the
declared field name changes (`crates/scythe-codegen/src/backends/java_jdbc.rs`, `java_r2dbc.rs`). Two
SQL identifiers that collapse onto the same generated name are a hard error, not last-write-wins. See
[`field_case`](/scythe/guide/configuration/#field_case) in the Configuration guide for the full option
reference.

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

## JDBC

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

## R2DBC

Backend: `java-r2dbc` | Library: R2DBC with Project Reactor | Engines: PostgreSQL, MySQL, MariaDB, SQLite

Generates reactive code using `Mono<T>` for `:one`/`:exec` queries and `Flux<T>` for `:many` queries. Requires a `ConnectionFactory` instead of a JDBC `Connection`.

Row types are plain records with **no `fromRow` method** -- row-to-object mapping is inlined directly
into each query function (`crates/scythe-codegen/src/backends/java_r2dbc.rs`):

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

## Enum generation

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

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

## Type mappings

| SQL Type | Neutral | Java |
|----------|---------|------|
| `INTEGER` | `int32` | `int` |
| `BIGINT` | `int64` | `long` |
| `TEXT` | `string` | `String` |
| `BOOLEAN` | `bool` | `boolean` |
| `BYTEA` | `bytes` | `byte[]` |
| `UUID` | `uuid` | `java.util.UUID` |
| `NUMERIC` | `decimal` | `java.math.BigDecimal` |
| `DATE` | `date` | `java.time.LocalDate` |
| `TIMESTAMPTZ` | `datetime_tz` | `java.time.OffsetDateTime` |
| `JSON` | `json` | `String` |
| `TEXT[]` | `array<string>` | `String` |
| nullable | `nullable` | `@Nullable T` |
