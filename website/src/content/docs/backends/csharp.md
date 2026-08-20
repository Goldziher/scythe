---
title: C# + Npgsql
description: The csharp-npgsql backend -- generated records, queries, and type mappings.
---

Backend: `csharp-npgsql` | Library: [Npgsql](https://www.npgsql.org/) | Engine: PostgreSQL

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

## Generated code

Every generated file starts with the provenance header, then `#nullable enable`, then `using Npgsql;`,
then a single `public static class Queries { ... }` wrapping every row record and query function
(`integration_tests/csharp-npgsql/generated/queries.cs:1-6`):

```csharp
// scythe:provenance v=0.16.1 backend=csharp-npgsql engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
#nullable enable

using Npgsql;

public static class Queries {
  // row records and query functions below
}
```

### Record types

```csharp
public record GetUserRow(
    int Id,
    string Name,
    string? Email,
    DateTimeOffset CreatedAt
);

public record ListUsersRow(
    int Id,
    string Name
);
```

Field names use `PascalCase`. Nullable columns use `T?`. Record construction is positional, not
named-argument -- see the `:one` example below.

There is no `CancellationToken` parameter anywhere in the generated API -- no backend source file
emits one.

### `:one` -- returns `Task<Row?>`, null on no rows

```csharp
public static async Task<GetUserRow?> GetUser(NpgsqlConnection conn, int id) {
    await using var cmd = new NpgsqlCommand("SELECT id, name, email, created_at FROM users WHERE id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", id);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) return null;
    return new GetUserRow(
        reader.GetInt32(0),
        reader.GetString(1),
        reader.IsDBNull(2) ? null : reader.GetString(2),
        reader.GetFieldValue<DateTimeOffset>(3)
    );
}
```

Placeholders are `@p1`, `@p2`, ... (not `$1`); binding is the two-argument
`cmd.Parameters.AddWithValue("p1", value)` -- the single-argument overload used to bind by position
does not appear anywhere in the generated code.

### `:many`

```csharp
public static async Task<List<ListUsersRow>> ListUsers(NpgsqlConnection conn, long limit) {
    await using var cmd = new NpgsqlCommand("SELECT id, name FROM users ORDER BY name LIMIT @p1", conn);
    cmd.Parameters.AddWithValue("p1", limit);
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<ListUsersRow>();
    while (await reader.ReadAsync()) {
        results.Add(new ListUsersRow(
            reader.GetInt32(0),
            reader.GetString(1)
        ));
    }
    return results;
}
```

### `:exec`

```csharp
public static async Task CreateUser(NpgsqlConnection conn, string name, string? email) {
    await using var cmd = new NpgsqlCommand("INSERT INTO users (name, email) VALUES (@p1, @p2)", conn);
    cmd.Parameters.AddWithValue("p1", name);
    cmd.Parameters.AddWithValue("p2", email);
    await cmd.ExecuteNonQueryAsync();
}
```

## Enum generation

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

```csharp
public enum UserStatus {
    Active,
    Inactive,
    Banned,
}
```

## Type mappings

| SQL Type | Neutral | C# (Npgsql) |
|----------|---------|-------------|
| `INTEGER` | `int32` | `int` |
| `BIGINT` | `int64` | `long` |
| `SMALLINT` | `int16` | `short` |
| `REAL` | `float32` | `float` |
| `DOUBLE PRECISION` | `float64` | `double` |
| `TEXT` / `VARCHAR` | `string` | `string` |
| `BOOLEAN` | `bool` | `bool` |
| `BYTEA` | `bytes` | `byte[]` |
| `UUID` | `uuid` | `Guid` |
| `NUMERIC` | `decimal` | `decimal` |
| `DATE` | `date` | `DateOnly` |
| `TIME` | `time` | `TimeOnly` |
| `TIMESTAMPTZ` | `datetime_tz` | `DateTimeOffset` |
| `TIMESTAMP` | `datetime` | `DateTime` |
| `INTERVAL` | `interval` | `TimeSpan` |
| `JSON` / `JSONB` | `json` | `string` |
| `INET` | `inet` | `System.Net.IPAddress` |
| `TEXT[]` | `array<string>` | `List<string>` |
| nullable column | `nullable` | `T?` |
