# Python

Backends: `python-psycopg3`, `python-asyncpg` | Engine: PostgreSQL

Both backends share the same type mappings and dataclass DTOs. They differ only in query execution.

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

## Generated code -- shared dataclasses

```python
from __future__ import annotations

import datetime
from dataclasses import dataclass


@dataclass
class GetUserRow:
    id: int
    name: str
    email: str | None
    created_at: datetime.datetime


@dataclass
class ListUsersRow:
    id: int
    name: str
```

## psycopg3

Scythe generates `%(name)s` parameter placeholders for psycopg3.

### `:one`

```python
async def get_user(conn: AsyncConnection, id: int) -> GetUserRow:
    row = await conn.execute(
        "SELECT id, name, email, created_at FROM users WHERE id = %(id)s",
        {"id": id},
    ).fetchone()
    return GetUserRow(
        id=row[0],
        name=row[1],
        email=row[2],
        created_at=row[3],
    )
```

### `:many`

```python
async def list_users(conn: AsyncConnection, limit: int) -> list[ListUsersRow]:
    rows = await conn.execute(
        "SELECT id, name FROM users ORDER BY name LIMIT %(limit)s",
        {"limit": limit},
    ).fetchall()
    return [ListUsersRow(id=r[0], name=r[1]) for r in rows]
```

### `:exec`

```python
async def create_user(conn: AsyncConnection, name: str, email: str | None) -> None:
    await conn.execute(
        "INSERT INTO users (name, email) VALUES (%(name)s, %(email)s)",
        {"name": name, "email": email},
    )
```

## asyncpg

Scythe generates `$N` positional parameter placeholders for asyncpg.

### `:one`

```python
async def get_user(conn: asyncpg.Connection, id: int) -> GetUserRow:
    row = await conn.fetchrow(
        "SELECT id, name, email, created_at FROM users WHERE id = $1",
        id,
    )
    return GetUserRow(
        id=row["id"],
        name=row["name"],
        email=row["email"],
        created_at=row["created_at"],
    )
```

### `:many`

```python
async def list_users(conn: asyncpg.Connection, limit: int) -> list[ListUsersRow]:
    rows = await conn.fetch(
        "SELECT id, name FROM users ORDER BY name LIMIT $1",
        limit,
    )
    return [ListUsersRow(id=r["id"], name=r["name"]) for r in rows]
```

### `:exec`

```python
async def create_user(conn: asyncpg.Connection, name: str, email: str | None) -> None:
    await conn.execute(
        "INSERT INTO users (name, email) VALUES ($1, $2)",
        name, email,
    )
```

## Enum generation

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

```python
import enum

class UserStatus(enum.Enum):
    ACTIVE = "active"
    INACTIVE = "inactive"
    BANNED = "banned"
```

## Type mappings

| SQL Type | Neutral | Python |
|----------|---------|--------|
| `SERIAL` / `INTEGER` | `int32` | `int` |
| `BIGINT` | `int64` | `int` |
| `TEXT` / `VARCHAR` | `string` | `str` |
| `BOOLEAN` | `bool` | `bool` |
| `BYTEA` | `bytes` | `bytes` |
| `UUID` | `uuid` | `uuid.UUID` |
| `NUMERIC` | `decimal` | `decimal.Decimal` |
| `DATE` | `date` | `datetime.date` |
| `TIMESTAMPTZ` | `datetime_tz` | `datetime.datetime` |
| `INTERVAL` | `interval` | `datetime.timedelta` |
| `JSON` / `JSONB` | `json` | `dict[str, Any]` |
| `TEXT[]` | `array<string>` | `list[str]` |
| nullable column | `nullable` | `T \| None` |
