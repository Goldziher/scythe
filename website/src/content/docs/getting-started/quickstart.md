---
title: Quickstart
description: A 5-minute walkthrough from zero to generated code with scythe.
---

A 5-minute walkthrough from zero to generated code.

## 1. Create a Schema

Create `sql/schema.sql`:

    CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');

    CREATE TABLE users (
        id SERIAL PRIMARY KEY,
        name TEXT NOT NULL,
        email TEXT,
        status user_status NOT NULL DEFAULT 'active',
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );

    CREATE TABLE orders (
        id SERIAL PRIMARY KEY,
        user_id INT NOT NULL REFERENCES users (id),
        total NUMERIC(10, 2) NOT NULL,
        weight_kg DOUBLE PRECISION,
        notes TEXT,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );

    CREATE TABLE tags (
        id SERIAL PRIMARY KEY,
        name TEXT NOT NULL UNIQUE
    );

    CREATE TABLE user_tags (
        user_id INT NOT NULL REFERENCES users (id),
        tag_id INT NOT NULL REFERENCES tags (id),
        PRIMARY KEY (user_id, tag_id)
    );

## 2. Write Annotated Queries

Create `sql/queries.sql`:

    -- @name GetUserById
    -- @returns :one
    SELECT id, name, email, status, created_at FROM users WHERE id = $1;

    -- @name ListActiveUsers
    -- @returns :many
    SELECT id, name, email FROM users WHERE status = $1;

    -- @name UpdateUserEmail
    -- @returns :exec
    UPDATE users SET email = $1 WHERE id = $2;

## 3. Create scythe.toml

Configure your target language and database driver. `[[sql.gen]]` is a repeatable table: each entry
needs a `backend` and an `output` directory.

### Rust

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "rust-sqlx"
output = "src/generated"
```

### Python

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "python-psycopg3"
output = "src/generated"
```

### TypeScript

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "typescript-pg"
output = "src/generated"
```

### Go

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "go-pgx"
output = "src/generated"
```

### Java

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "java-jdbc"
output = "src/generated"
```

Java is the one backend whose output filename is capitalized: this writes `src/generated/Queries.java`, not `queries.java`.

### Kotlin

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "kotlin-jdbc"
output = "src/generated"
```

### C\#

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "csharp-npgsql"
output = "src/generated"
```

### Elixir

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "elixir-postgrex"
output = "lib/generated"
```

### Ruby

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "ruby-pg"
output = "lib/generated"
```

Ruby additionally emits a `queries.rbs` type-signature file alongside `queries.rb`.

### PHP

```toml
[scythe]
version = "1"

[[sql]]
name = "main"
engine = "postgresql"
schema = ["sql/schema.sql"]
queries = ["sql/queries.sql"]

[[sql.gen]]
backend = "php-pdo"
output = "src/Generated"
```

**Optional:** Add `row_type` to customize the generated row type style. For Python backends, use `"pydantic"` or `"msgspec"` instead of the default `"dataclass"`. For TypeScript, use `"zod"` instead of the default `"interface"`. For example:

    [[sql.gen]]
    backend = "python-psycopg3"
    output = "src/generated"
    row_type = "pydantic"

See the [Configuration guide](/scythe/guide/configuration/) for all `row_type` options.

## 4. Generate Code

    scythe generate

Output (for the Rust config from step 3):

    [main] Parsing schema...
    [main] Analyzing 3 queries...
    [main] Writing rust-sqlx output to src/generated/queries.rs
    Done.

Each `[[sql.gen]]` target prints its own `Writing ... output to ...` line; a config with multiple
targets prints one per target. Ruby prints a second line for the `.rbs` file:
`[main] Writing ruby-pg RBS signatures to lib/generated/queries.rbs`.

## 5. Generated Code

Scythe produces idiomatic, type-safe code for your target language. Each snippet below shows a representative sample -- the `UserStatus` enum, the `GetUserById` (`:one`) and `ListActiveUsers` (`:many`) queries, and the `UpdateUserEmail` (`:exec`) query -- generated from the schema and queries above. Every generated file's first line is a provenance header (`scythe check` uses it to detect drift between the file and the schema it was generated from).

### Rust

```rust
// scythe:provenance v=0.14.0 backend=rust-sqlx engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
#![allow(dead_code, unused_imports, clippy::needless_question_mark, clippy::redundant_closure)]

#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "user_status", rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Inactive,
    Banned,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GetUserByIdRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub status: UserStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub async fn get_user_by_id(pool: &sqlx::PgPool, id: i32) -> Result<GetUserByIdRow, sqlx::Error> {
    sqlx::query_as!(GetUserByIdRow, "SELECT id, name, email, status AS \"status: UserStatus\", created_at FROM users WHERE id = $1", id)
        .fetch_one(pool)
        .await
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ListActiveUsersRow {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
}

pub async fn list_active_users(pool: &sqlx::PgPool, status: &UserStatus) -> Result<Vec<ListActiveUsersRow>, sqlx::Error> {
    sqlx::query_as!(ListActiveUsersRow, "SELECT id, name, email FROM users WHERE status = $1", status as &UserStatus)
        .fetch_all(pool)
        .await
}

pub async fn update_user_email(pool: &sqlx::PgPool, email: &str, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE users SET email = $1 WHERE id = $2", email, id)
        .execute(pool)
        .await?;
    Ok(())
}
```

`scythe generate` runs `rustfmt` over Rust output when it is available on `PATH`, which reflows long lines like the macro calls above.

### Python

```python
# scythe:provenance v=0.14.0 backend=python-psycopg3 engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582  # noqa: E501
import datetime  # noqa: F401
import decimal  # noqa: F401
import uuid  # noqa: F401
from dataclasses import dataclass
from enum import Enum  # noqa: F401
from typing import Any  # noqa: F401

from psycopg import AsyncConnection  # noqa: F401



class UserStatus(str, Enum):
    """Database enum type user_status."""

    ACTIVE = "active"
    INACTIVE = "inactive"
    BANNED = "banned"


@dataclass(frozen=True, slots=True)
class GetUserByIdRow:
    """Row type for GetUserById query."""

    id: int
    name: str
    email: str | None
    status: UserStatus
    created_at: datetime.datetime


async def get_user_by_id(conn: AsyncConnection, *, id: int) -> GetUserByIdRow | None:
    """Execute GetUserById query."""
    cur = await conn.execute(
        """SELECT id, name, email, status, created_at FROM users WHERE id = %(id)s""",
        {"id": id},
    )
    row = await cur.fetchone()
    if row is None:
        return None
    return GetUserByIdRow(
        id=row[0],
        name=row[1],
        email=row[2],
        status=row[3],
        created_at=row[4],
    )


@dataclass(frozen=True, slots=True)
class ListActiveUsersRow:
    """Row type for ListActiveUsers query."""

    id: int
    name: str
    email: str | None


async def list_active_users(conn: AsyncConnection, *, status: UserStatus) -> list[ListActiveUsersRow]:
    """Execute ListActiveUsers query."""
    cur = await conn.execute(
        """SELECT id, name, email FROM users WHERE status = %(status)s""",
        {"status": status},
    )
    rows = await cur.fetchall()
    return [ListActiveUsersRow(id=r[0], name=r[1], email=r[2]) for r in rows]


async def update_user_email(conn: AsyncConnection, *, email: str, id: int) -> None:
    """Execute UpdateUserEmail query."""
    await conn.execute(
        """UPDATE users SET email = %(email)s WHERE id = %(id)s""",
        {"email": email, "id": id},
    )
```

Parameters are keyword-only (the `*,`), and `:one` queries return `Row | None`.

### TypeScript

```typescript
// scythe:provenance v=0.14.0 backend=typescript-pg engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
import type { PoolClient } from "pg";


export const UserStatusValues = {
    Active: "active",
    Inactive: "inactive",
    Banned: "banned",
} as const;

export type UserStatus = typeof UserStatusValues[keyof typeof UserStatusValues];

/** Row type for GetUserById queries. */
export interface GetUserByIdRow {
    id: number;
    name: string;
    email: string | null;
    status: UserStatus;
    created_at: Date;
}

/** Fetch a single GetUserByIdRow or null. */
export async function getUserById(
    client: PoolClient,
    id: number,
): Promise<GetUserByIdRow | null> {
    const { rows } = await client.query<GetUserByIdRow>(
        `SELECT id, name, email, status, created_at FROM users WHERE id = $1`,
        [id],
    );
    return rows[0] ?? null;
}

/** Row type for ListActiveUsers queries. */
export interface ListActiveUsersRow {
    id: number;
    name: string;
    email: string | null;
}

/** Fetch all ListActiveUsersRow rows. */
export async function listActiveUsers(
    client: PoolClient,
    status: UserStatus,
): Promise<ListActiveUsersRow[]> {
    const { rows } = await client.query<ListActiveUsersRow>(
        `SELECT id, name, email FROM users WHERE status = $1`,
        [status],
    );
    return rows;
}

/** Execute a query returning no rows. */
export async function updateUserEmail(
    client: PoolClient,
    email: string,
    id: number,
): Promise<void> {
    await client.query(
        `UPDATE users SET email = $1 WHERE id = $2`,
        [email, id],
    );
}
```

`typescript-pg` emits a `const` object plus a derived `type`, not a TypeScript `enum` -- that's what the `typescript-postgres` backend emits instead.

### Go

```go
// scythe:provenance v=0.14.0 backend=go-pgx engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
// Code generated by scythe. DO NOT EDIT.
// Run `goimports -w .` to fix imports.
package queries

import (
    "context"
    "time"

    "github.com/jackc/pgx/v5/pgxpool"
    "github.com/shopspring/decimal"
)


type UserStatus string

const (
    UserStatusActive UserStatus = "active"
    UserStatusInactive UserStatus = "inactive"
    UserStatusBanned UserStatus = "banned"
)

type GetUserByIdRow struct {
    Id int32 `json:"id"`
    Name string `json:"name"`
    Email *string `json:"email"`
    Status UserStatus `json:"status"`
    CreatedAt time.Time `json:"created_at"`
}

// Returns the zero value of the struct if no row is found.
// Use pgx.ErrNoRows to distinguish not-found from other errors.
func GetUserById(ctx context.Context, db *pgxpool.Pool, Id int32) (GetUserByIdRow, error) {
    row := db.QueryRow(ctx, "SELECT id, name, email, status, created_at FROM users WHERE id = $1", Id)
    var r GetUserByIdRow
    err := row.Scan(&r.Id, &r.Name, &r.Email, &r.Status, &r.CreatedAt)
    return r, err
}

type ListActiveUsersRow struct {
    Id int32 `json:"id"`
    Name string `json:"name"`
    Email *string `json:"email"`
}

func ListActiveUsers(ctx context.Context, db *pgxpool.Pool, Status UserStatus) ([]ListActiveUsersRow, error) {
    rows, err := db.Query(ctx, "SELECT id, name, email FROM users WHERE status = $1", Status)
    if err != nil {
        return nil, err
    }
    defer rows.Close()
    var result []ListActiveUsersRow
    for rows.Next() {
        var r ListActiveUsersRow
        if err := rows.Scan(&r.Id, &r.Name, &r.Email); err != nil {
            return nil, err
        }
        result = append(result, r)
    }
    return result, rows.Err()
}

func UpdateUserEmail(ctx context.Context, db *pgxpool.Pool, Email string, Id int32) error {
    _, err := db.Exec(ctx, "UPDATE users SET email = $1 WHERE id = $2", Email, Id)
    return err
}
```

The `decimal` import is unused by these three queries but always emitted -- the header comment tells you to run `goimports -w .` to clean it up.

### Java

<!-- snippet:skip -->

```java
// scythe:provenance v=0.14.0 backend=java-jdbc engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
package generated;

import java.math.BigDecimal;
import java.sql.*;
import java.time.*;
import java.util.ArrayList;
import java.util.List;
import javax.annotation.Nonnull;
import javax.annotation.Nullable;

public class Queries {

public enum UserStatus {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");

    private final String value;
    UserStatus(String value) { this.value = value; }
    public String getValue() { return value; }
}

public record GetUserByIdRow(
    int id,
    String name,
    @Nullable String email,
    UserStatus status,
    java.time.OffsetDateTime created_at
) {
    public static GetUserByIdRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetUserByIdRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getString("email"),
            UserStatus.valueOf(rs.getString("status").toUpperCase()),
            rs.getObject("created_at", OffsetDateTime.class)
        );
    }
}

public static @Nullable GetUserByIdRow getUserById(Connection conn, int id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?")) {
        ps.setInt(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetUserByIdRow.fromResultSet(rs);
            }
            return null;
        }
    }
}

public record ListActiveUsersRow(
    int id,
    String name,
    @Nullable String email
) {
    public static ListActiveUsersRow fromResultSet(ResultSet rs) throws SQLException {
        return new ListActiveUsersRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getString("email")
        );
    }
}

public static List<ListActiveUsersRow> listActiveUsers(Connection conn, @Nonnull UserStatus status) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?")) {
        ps.setObject(1, status.getValue(), java.sql.Types.OTHER);
        try (ResultSet rs = ps.executeQuery()) {
            List<ListActiveUsersRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(ListActiveUsersRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public static void updateUserEmail(Connection conn, @Nonnull String email, int id) throws SQLException {
    try (var ps = conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?")) {
        ps.setString(1, email);
        ps.setInt(2, id);
        ps.executeUpdate();
    }
}

}
```

### Kotlin

<!-- snippet:skip -->

```kotlin
// scythe:provenance v=0.14.0 backend=kotlin-jdbc engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
package generated

import java.math.BigDecimal
import java.sql.Connection
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.LocalTime
import java.time.OffsetDateTime
import java.time.OffsetTime
import java.util.UUID


enum class UserStatus(val value: String) {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");
}


data class GetUserByIdRow(
    val id: Int,
    val name: String,
    val email: String?,
    val status: UserStatus,
    val created_at: java.time.OffsetDateTime,
)


fun getUserById(
    conn: Connection,
    id: Int,
): GetUserByIdRow? {
    conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?").use { ps ->
        ps.setInt(1, id)
        ps.executeQuery().use { rs ->
            return if (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                GetUserByIdRow(
                    id = rs.getInt("id"),
                    name = rs.getString("name"),
                    email = email,
                    status = UserStatus.valueOf(rs.getString("status").uppercase()),
                    created_at = rs.getObject("created_at", OffsetDateTime::class.java),
                )
            } else {
                null
            }
        }
    }
}


data class ListActiveUsersRow(
    val id: Int,
    val name: String,
    val email: String?,
)


fun listActiveUsers(
    conn: Connection,
    status: UserStatus,
): List<ListActiveUsersRow> {
    conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?").use { ps ->
        ps.setObject(1, status.value, java.sql.Types.OTHER)
        ps.executeQuery().use { rs ->
            val result = mutableListOf<ListActiveUsersRow>()
            while (rs.next()) {
                val emailValue = rs.getString("email")
                val email = if (rs.wasNull()) null else emailValue
                result.add(
                    ListActiveUsersRow(
                        id = rs.getInt("id"),
                        name = rs.getString("name"),
                        email = email,
                    ),
                )
            }
            return result
        }
    }
}


fun updateUserEmail(
    conn: Connection,
    email: String,
    id: Int,
) {
    conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?").use { ps ->
        ps.setString(1, email)
        ps.setInt(2, id)
        ps.executeUpdate()
    }
}
```

### C\#

```csharp
// scythe:provenance v=0.14.0 backend=csharp-npgsql engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
#nullable enable

using Npgsql;

public static class Queries {

public enum UserStatus {
    Active,
    Inactive,
    Banned,
}

public record GetUserByIdRow(
    int Id,
    string Name,
    string? Email,
    UserStatus Status,
    DateTimeOffset CreatedAt
);

public static async Task<GetUserByIdRow?> GetUserById(NpgsqlConnection conn, int id) {
    await using var cmd = new NpgsqlCommand("SELECT id, name, email, status, created_at FROM users WHERE id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", id);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) return null;
    return new GetUserByIdRow(
        reader.GetInt32(0),
        reader.GetString(1),
        reader.IsDBNull(2) ? null : reader.GetString(2),
        (Enum.TryParse<UserStatus>(reader.GetString(3), true, out var enumVal3) ? enumVal3 : throw new InvalidOperationException($"Invalid enum value '{reader.GetString(3)}' for UserStatus")),
        reader.GetFieldValue<DateTimeOffset>(4)
    );
}

public record ListActiveUsersRow(
    int Id,
    string Name,
    string? Email
);

public static async Task<List<ListActiveUsersRow>> ListActiveUsers(NpgsqlConnection conn, UserStatus status) {
    await using var cmd = new NpgsqlCommand("SELECT id, name, email FROM users WHERE status = @p1::user_status", conn);
    cmd.Parameters.AddWithValue("p1", status.ToDbValue());
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<ListActiveUsersRow>();
    while (await reader.ReadAsync()) {
        results.Add(new ListActiveUsersRow(
            reader.GetInt32(0),
            reader.GetString(1),
            reader.IsDBNull(2) ? null : reader.GetString(2)
        ));
    }
    return results;
}

public static async Task UpdateUserEmail(NpgsqlConnection conn, string email, int id) {
    await using var cmd = new NpgsqlCommand("UPDATE users SET email = @p1 WHERE id = @p2", conn);
    cmd.Parameters.AddWithValue("p1", email);
    cmd.Parameters.AddWithValue("p2", id);
    await cmd.ExecuteNonQueryAsync();
}

}

public static class UserStatusExtensions {
    public static string ToDbValue(this Queries.UserStatus value) => value switch {
        Queries.UserStatus.Active => "active",
        Queries.UserStatus.Inactive => "inactive",
        Queries.UserStatus.Banned => "banned",
        _ => throw new ArgumentOutOfRangeException(nameof(value), value, null),
    };
}
```

### Elixir

```elixir
# scythe:provenance v=0.14.0 backend=elixir-postgrex engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582
defmodule UserStatus do
  @moduledoc "Enum type for user_status."

  @type t :: String.t()

  @spec active() :: String.t()
  def active(), do: "active"
  @spec inactive() :: String.t()
  def inactive(), do: "inactive"
  @spec banned() :: String.t()
  def banned(), do: "banned"
  @spec values() :: [String.t()]
  def values, do: ["active", "inactive", "banned"]
end

defmodule GetUserByIdRow do
  @moduledoc "Row type for GetUserById queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    status: UserStatus.t(),
    created_at: DateTime.t()
  }
  defstruct [:id, :name, :email, :status, :created_at]
end

defmodule ListActiveUsersRow do
  @moduledoc "Row type for ListActiveUsers queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil
  }
  defstruct [:id, :name, :email]
end

defmodule Scythe.Queries do

@spec get_user_by_id(Postgrex.conn(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, :not_found} | {:error, term()}
def get_user_by_id(conn, id) do
  case Postgrex.query(conn, "SELECT id, name, email, status, created_at FROM users WHERE id = $1", [id]) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, status, created_at] = row
      {:ok, %GetUserByIdRow{id: id, name: name, email: email, status: status, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_active_users(Postgrex.conn(), UserStatus) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn, status) do
  case Postgrex.query(conn, "SELECT id, name, email FROM users WHERE status = $1", [status]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %ListActiveUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(Postgrex.conn(), String.t(), integer()) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  case Postgrex.query(conn, "UPDATE users SET email = $1 WHERE id = $2", [email, id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

end
```

Query functions take a `Postgrex.conn()` and match rows out of `%{rows: [row | _]}}`, not a bare `pid()`.

### Ruby

```ruby
# frozen_string_literal: true
# scythe:provenance v=0.14.0 backend=ruby-pg engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582

module Queries

  module UserStatus
    ACTIVE = "active"
    INACTIVE = "inactive"
    BANNED = "banned"
    ALL = [ACTIVE, INACTIVE, BANNED].freeze
  end

  GetUserByIdRow = Data.define(:id, :name, :email, :status, :created_at)


  def self.get_user_by_id(conn, id)
    result = conn.exec_params("SELECT id, name, email, status, created_at FROM users WHERE id = $1", [id])
    return nil if result.ntuples.zero?
    row = result[0]
    GetUserByIdRow.new(id: row["id"].to_i, name: row["name"], email: row["email"]&.then { |v| v }, status: row["status"], created_at: row["created_at"])
  end

  ListActiveUsersRow = Data.define(:id, :name, :email)


  def self.list_active_users(conn, status)
    result = conn.exec_params("SELECT id, name, email FROM users WHERE status = $1", [status])
    result.map do |row|
      ListActiveUsersRow.new(id: row["id"].to_i, name: row["name"], email: row["email"]&.then { |v| v })
    end
  end

  def self.update_user_email(conn, email, id)
    conn.exec_params("UPDATE users SET email = $1 WHERE id = $2", [email, id])
    nil
  end

end
```

`UserStatus` values are module constants (`ACTIVE`, `INACTIVE`, `BANNED`, plus an `ALL` array), not local variables.

### PHP

<!-- snippet:skip -->

```php
<?php
// scythe:provenance v=0.14.0 backend=php-pdo engine=postgresql schema=sch1:2e813606acee8b51 queries=q1:9c4e1f77a0b3d582

declare(strict_types=1);

namespace App\Generated;



enum UserStatus: string {
    case ACTIVE = "active";
    case INACTIVE = "inactive";
    case BANNED = "banned";
}

readonly class GetUserByIdRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: UserStatus::from($row['status']),
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class ListActiveUsersRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
        );
    }
}

final class Queries {

    /**
     * @param \PDO $pdo
     * @param int $id
     * @return GetUserByIdRow|null
     */
    public static function getUserById(\PDO $pdo, int $id): ?GetUserByIdRow {
        $stmt = $pdo->prepare("SELECT id, name, email, status, created_at FROM users WHERE id = :p1");
        $stmt->execute(["p1" => $id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        return $row ? GetUserByIdRow::fromRow($row) : null;
    }

    /**
     * @param \PDO $pdo
     * @param UserStatus $status
     * @return \Generator<int, ListActiveUsersRow, mixed, void>
     */
    public static function listActiveUsers(\PDO $pdo, UserStatus $status): \Generator {
        $stmt = $pdo->prepare("SELECT id, name, email FROM users WHERE status = :p1");
        $stmt->execute(["p1" => $status->value]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield ListActiveUsersRow::fromRow($row);
        }
    }

    /**
     * @param \PDO $pdo
     * @param string $email
     * @param int $id
     * @return void
     */
    public static function updateUserEmail(\PDO $pdo, string $email, int $id): void {
        $stmt = $pdo->prepare("UPDATE users SET email = :p1 WHERE id = :p2");
        $stmt->execute(["p1" => $email, "p2" => $id]);
    }

}
```

## 6. Validate and Lint

    # Validate SQL parses and types resolve correctly
    scythe check

    # Lint SQL for correctness, performance, and style
    scythe lint

## Next Steps

- [Configuration](/scythe/guide/configuration/) -- full scythe.toml reference
- [Annotations](/scythe/guide/annotations/) -- all nine annotation types
- [Type Inference](/scythe/guide/type-inference/) -- how nullability analysis works
- [Linting](/scythe/guide/linting/) -- 58 built-in rules (23 lint + 35 audit) plus sqruff integration
