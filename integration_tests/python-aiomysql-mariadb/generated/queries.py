# scythe:provenance v=0.18.1 backend=python-aiomysql engine=mariadb schema=sch2:262bec5a0954c973 queries=q1:2f37bd0f0a685c79 options=opt1:cbf29ce484222325  # noqa: E501
import datetime  # noqa: F401
import decimal  # noqa: F401
from dataclasses import dataclass
from enum import Enum  # noqa: F401
from typing import Any  # noqa: F401

import aiomysql  # noqa: F401


class ScytheNoRowsError(Exception):
    """Raised by a `:one` query when no row matches."""



class UsersStatus(str, Enum):
    """Database enum type users_status."""

    ACTIVE = "active"
    INACTIVE = "inactive"
    BANNED = "banned"


@dataclass(frozen=True, slots=True)
class CreateOrderRow:
    """Row type for CreateOrder query."""

    id: int
    user_id: str
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


async def create_order(conn: aiomysql.Connection, *, user_id: str, total: decimal.Decimal, notes: str | None) -> CreateOrderRow:
    """Execute CreateOrder query."""
    async with conn.cursor() as cur:
        await cur.execute("""INSERT INTO orders (user_id, total, notes) VALUES (%s, %s, %s) RETURNING id, user_id, total, notes, created_at""", (user_id, total, notes))
        row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("CreateOrder: no rows returned")
    return CreateOrderRow(
        id=row[0],
        user_id=row[1],
        total=row[2],
        notes=row[3],
        created_at=row[4],
    )


@dataclass(frozen=True, slots=True)
class GetOrdersByUserRow:
    """Row type for GetOrdersByUser query."""

    id: int
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


async def get_orders_by_user(conn: aiomysql.Connection, *, user_id: str) -> list[GetOrdersByUserRow]:
    """Execute GetOrdersByUser query."""
    async with conn.cursor() as cur:
        await cur.execute("""SELECT id, total, notes, created_at FROM orders WHERE user_id = %s ORDER BY created_at DESC""", (user_id,))
        rows = await cur.fetchall()
    return [
        GetOrdersByUserRow(
            id=r[0],
            total=r[1],
            notes=r[2],
            created_at=r[3],
        )
        for r in rows
    ]


@dataclass(frozen=True, slots=True)
class GetOrderTotalRow:
    """Row type for GetOrderTotal query."""

    total_sum: decimal.Decimal | None


async def get_order_total(conn: aiomysql.Connection, *, user_id: str) -> GetOrderTotalRow:
    """Execute GetOrderTotal query."""
    async with conn.cursor() as cur:
        await cur.execute("""SELECT SUM(total) AS total_sum FROM orders WHERE user_id = %s""", (user_id,))
        row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetOrderTotal: no rows returned")
    return GetOrderTotalRow(total_sum=row[0])


async def delete_orders_by_user(conn: aiomysql.Connection, *, user_id: str) -> int:
    """Execute DeleteOrdersByUser query."""
    async with conn.cursor() as cur:
        await cur.execute("""DELETE FROM orders WHERE user_id = %s""", (user_id,))
        return cur.rowcount


@dataclass(frozen=True, slots=True)
class GetUserByIdRow:
    """Row type for GetUserById query."""

    id: str
    name: str
    email: str | None
    status: UsersStatus
    created_at: datetime.datetime


async def get_user_by_id(conn: aiomysql.Connection, *, id: str) -> GetUserByIdRow:
    """Execute GetUserById query."""
    async with conn.cursor() as cur:
        await cur.execute("""SELECT id, name, email, status, created_at FROM users WHERE id = %s""", (id,))
        row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetUserById: no rows returned")
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

    id: str
    name: str
    email: str | None


async def list_active_users(conn: aiomysql.Connection, *, status: UsersStatus) -> list[ListActiveUsersRow]:
    """Execute ListActiveUsers query."""
    async with conn.cursor() as cur:
        await cur.execute("""SELECT id, name, email FROM users WHERE status = %s""", (status,))
        rows = await cur.fetchall()
    return [ListActiveUsersRow(id=r[0], name=r[1], email=r[2]) for r in rows]


@dataclass(frozen=True, slots=True)
class CreateUserRow:
    """Row type for CreateUser query."""

    id: str
    name: str
    email: str | None


async def create_user(conn: aiomysql.Connection, *, name: str, email: str | None, status: UsersStatus) -> CreateUserRow:
    """Execute CreateUser query."""
    async with conn.cursor() as cur:
        await cur.execute("""INSERT INTO users (name, email, status) VALUES (%s, %s, %s) RETURNING id, name, email""", (name, email, status))
        row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("CreateUser: no rows returned")
    return CreateUserRow(id=row[0], name=row[1], email=row[2])


async def update_user_email(conn: aiomysql.Connection, *, email: str, id: str) -> None:
    """Execute UpdateUserEmail query."""
    async with conn.cursor() as cur:
        await cur.execute("""UPDATE users SET email = %s WHERE id = %s""", (email, id))


async def delete_user(conn: aiomysql.Connection, *, id: str) -> None:
    """Execute DeleteUser query."""
    async with conn.cursor() as cur:
        await cur.execute("""DELETE FROM users WHERE id = %s RETURNING id""", (id,))


@dataclass(frozen=True, slots=True)
class SearchUsersRow:
    """Row type for SearchUsers query."""

    id: str
    name: str
    email: str | None


async def search_users(conn: aiomysql.Connection, *, name: str) -> list[SearchUsersRow]:
    """Execute SearchUsers query."""
    async with conn.cursor() as cur:
        await cur.execute("""SELECT id, name, email FROM users WHERE name LIKE %s""", (name,))
        rows = await cur.fetchall()
    return [SearchUsersRow(id=r[0], name=r[1], email=r[2]) for r in rows]

