---
title: Python
description: The python-psycopg3 and python-asyncpg backends -- generated dataclasses, queries, and type mappings.
---

Backends: `python-psycopg3`, `python-asyncpg` | Engines: PostgreSQL, Redshift

Both backends share the same type mappings and row-type generation. They differ only in query
execution and imports. All parameters on generated functions are **keyword-only** -- every call site
after `conn` requires `*,` in the signature (`crates/scythe-codegen/src/backends/python_psycopg3.rs`,
`python_asyncpg.rs`). A positional call like `get_user(conn, 1)` raises `TypeError` at runtime; the
correct call is `get_user(conn, id=1)`.

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

The default `row_type` is `dataclass`; the decorator is `@dataclass(frozen=True, slots=True)`, and
every row and function carries a one-line docstring. There is no `from __future__ import annotations`
-- the module starts directly with the provenance header, then typed stdlib imports, then the driver
import (`integration_tests/python-psycopg3/generated/queries.py:1-9`):

```python
# scythe:provenance v=0.18.1 backend=python-psycopg3 engine=postgresql schema=sch1:... queries=q1:...  # noqa: E501
import datetime  # noqa: F401
import decimal  # noqa: F401
from dataclasses import dataclass
from enum import Enum  # noqa: F401

from psycopg import AsyncConnection  # noqa: F401


@dataclass(frozen=True, slots=True)
class GetUserRow:
    """Row type for GetUser query."""

    id: int
    name: str
    email: str | None
    created_at: datetime.datetime


@dataclass(frozen=True, slots=True)
class ListUsersRow:
    """Row type for ListUsers query."""

    id: int
    name: str
```

`row_type` also accepts `pydantic` (`class GetUserRow(BaseModel):`) and `msgspec`
(`class GetUserRow(msgspec.Struct):`) -- see `crates/scythe-codegen/src/backends/python_common.rs`.

## psycopg3

Scythe generates `%(name)s` parameter placeholders for psycopg3. Fetching is two statements, not a
chained call: `cur = await conn.execute(...)` followed by `row = await cur.fetchone()` --
`conn.execute(...).fetchone()` is not valid psycopg3.

### `:one`

```python
async def get_user(conn: AsyncConnection, *, id: int) -> GetUserRow | None:
    """Execute GetUser query."""
    cur = await conn.execute(
        """SELECT id, name, email, created_at FROM users WHERE id = %(id)s""",
        {"id": id},
    )
    row = await cur.fetchone()
    if row is None:
        return None
    return GetUserRow(
        id=row[0],
        name=row[1],
        email=row[2],
        created_at=row[3],
    )
```

### `:many`

```python
async def list_users(conn: AsyncConnection, *, limit: int) -> list[ListUsersRow]:
    """Execute ListUsers query."""
    cur = await conn.execute(
        """SELECT id, name FROM users ORDER BY name LIMIT %(limit)s""",
        {"limit": limit},
    )
    rows = await cur.fetchall()
    return [ListUsersRow(id=r[0], name=r[1]) for r in rows]
```

### `:exec`

```python
async def create_user(conn: AsyncConnection, *, name: str, email: str | None) -> None:
    """Execute CreateUser query."""
    await conn.execute(
        """INSERT INTO users (name, email) VALUES (%(name)s, %(email)s)""",
        {"name": name, "email": email},
    )
```

## asyncpg

Scythe generates `$N` positional parameter placeholders for asyncpg -- but the **generated function's
own parameters** are still keyword-only, the same as psycopg3; only the SQL placeholders and the
underlying driver call (`conn.fetchrow(sql, id)`) are positional.

### `:one`

```python
async def get_user(conn: Connection, *, id: int) -> GetUserRow | None:
    """Execute GetUser query."""
    row = await conn.fetchrow(
        """SELECT id, name, email, created_at FROM users WHERE id = $1""",
        id,
    )
    if row is None:
        return None
    return GetUserRow(
        id=row["id"],
        name=row["name"],
        email=row["email"],
        created_at=row["created_at"],
    )
```

### `:many`

```python
async def list_users(conn: Connection, *, limit: int) -> list[ListUsersRow]:
    """Execute ListUsers query."""
    rows = await conn.fetch(
        """SELECT id, name FROM users ORDER BY name LIMIT $1""",
        limit,
    )
    return [ListUsersRow(id=r["id"], name=r["name"]) for r in rows]
```

### `:exec`

```python
async def create_user(conn: Connection, *, name: str, email: str | None) -> None:
    """Execute CreateUser query."""
    await conn.execute(
        """INSERT INTO users (name, email) VALUES ($1, $2)""",
        name, email,
    )
```

## Enum generation

```sql
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned');
```

```python
class UserStatus(str, Enum):
    """Database enum type user_status."""

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
