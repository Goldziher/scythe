# C# + Npgsql

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

Field names use `PascalCase`. Nullable columns use `T?`.

### `:one` -- async pattern

```csharp
public static async Task<GetUserRow> GetUser(
    NpgsqlConnection conn,
    int id,
    CancellationToken ct = default)
{
    await using var cmd = new NpgsqlCommand(
        "SELECT id, name, email, created_at FROM users WHERE id = $1", conn);
    cmd.Parameters.AddWithValue(id);

    await using var reader = await cmd.ExecuteReaderAsync(ct);
    await reader.ReadAsync(ct);

    return new GetUserRow(
        Id: reader.GetInt32(0),
        Name: reader.GetString(1),
        Email: reader.IsDBNull(2) ? null : reader.GetString(2),
        CreatedAt: reader.GetFieldValue<DateTimeOffset>(3)
    );
}
```

### `:many`

```csharp
public static async Task<List<ListUsersRow>> ListUsers(
    NpgsqlConnection conn,
    long limit,
    CancellationToken ct = default)
{
    await using var cmd = new NpgsqlCommand(
        "SELECT id, name FROM users ORDER BY name LIMIT $1", conn);
    cmd.Parameters.AddWithValue(limit);

    await using var reader = await cmd.ExecuteReaderAsync(ct);
    var result = new List<ListUsersRow>();

    while (await reader.ReadAsync(ct))
    {
        result.Add(new ListUsersRow(
            Id: reader.GetInt32(0),
            Name: reader.GetString(1)
        ));
    }

    return result;
}
```

### `:exec`

```csharp
public static async Task CreateUser(
    NpgsqlConnection conn,
    string name,
    string? email,
    CancellationToken ct = default)
{
    await using var cmd = new NpgsqlCommand(
        "INSERT INTO users (name, email) VALUES ($1, $2)", conn);
    cmd.Parameters.AddWithValue(name);
    cmd.Parameters.AddWithValue((object?)email ?? DBNull.Value);

    await cmd.ExecuteNonQueryAsync(ct);
}
```

## Enum generation

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

```csharp
public enum UserStatus
{
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
