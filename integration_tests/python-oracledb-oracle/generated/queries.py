# scythe:provenance v=0.18.0 backend=python-oracledb engine=oracle schema=sch2:51c12e41405f20c2 queries=q1:9b9c257a90458ab4 options=opt1:cbf29ce484222325  # noqa: E501
import datetime  # noqa: F401
import decimal  # noqa: F401
from dataclasses import dataclass
from enum import Enum  # noqa: F401

import oracledb  # noqa: F401


class ScytheNoRowsError(Exception):
    """Raised by a `:one` query when no row matches."""



@dataclass(frozen=True, slots=True)
class CreateAttachmentRow:
    """Row type for CreateAttachment query."""

    id: int
    order_id: int
    filename: str


async def create_attachment(conn: oracledb.AsyncConnection, *, order_id: int, filename: str, payload: bytes, description: str | None) -> CreateAttachmentRow:
    """Execute CreateAttachment query."""
    async with conn.cursor() as cur:
        out_id = cur.var(oracledb.NUMBER)
        out_order_id = cur.var(oracledb.NUMBER)
        out_filename = cur.var(oracledb.STRING, 4000)
        await cur.execute(
            """INSERT INTO attachments (order_id, filename, payload, description) VALUES (:1, :2, :3, :4) RETURNING id, order_id, filename INTO :5, :6, :7""",
            [order_id, filename, payload, description, out_id, out_order_id, out_filename],
        )
        return CreateAttachmentRow(
            id=out_id.getvalue()[0],
            order_id=out_order_id.getvalue()[0],
            filename=out_filename.getvalue()[0],
        )


@dataclass(frozen=True, slots=True)
class GetAttachmentsByOrderRow:
    """Row type for GetAttachmentsByOrder query."""

    id: int
    order_id: int
    filename: str
    payload: bytes
    description: str | None


async def get_attachments_by_order(conn: oracledb.AsyncConnection, *, order_id: int) -> list[GetAttachmentsByOrderRow]:
    """Execute GetAttachmentsByOrder query."""
    async with conn.cursor() as cur:
        await cur.execute(
            """SELECT id, order_id, filename, payload, description FROM attachments WHERE order_id = :1 ORDER BY id""",
            [order_id],
        )
        rows = await cur.fetchall()
        return [GetAttachmentsByOrderRow(id=r[0], order_id=r[1], filename=r[2], payload=r[3], description=r[4]) for r in rows]


@dataclass(frozen=True, slots=True)
class GetAttachmentByIdRow:
    """Row type for GetAttachmentById query."""

    id: int
    order_id: int
    filename: str
    payload: bytes
    description: str | None


async def get_attachment_by_id(conn: oracledb.AsyncConnection, *, id: int) -> GetAttachmentByIdRow | None:
    """Execute GetAttachmentById query."""
    async with conn.cursor() as cur:
        await cur.execute(
            """SELECT id, order_id, filename, payload, description FROM attachments WHERE id = :1""",
            [id],
        )
        row = await cur.fetchone()
        if row is None:
            return None
        return GetAttachmentByIdRow(
            id=row[0],
            order_id=row[1],
            filename=row[2],
            payload=row[3],
            description=row[4],
        )


async def delete_attachments_by_order(conn: oracledb.AsyncConnection, *, order_id: int) -> int:
    """Execute DeleteAttachmentsByOrder query."""
    async with conn.cursor() as cur:
        await cur.execute("""DELETE FROM attachments WHERE order_id = :1""", [order_id])
        return cur.rowcount


@dataclass(frozen=True, slots=True)
class CreateOrderRow:
    """Row type for CreateOrder query."""

    id: int
    user_id: int
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


async def create_order(conn: oracledb.AsyncConnection, *, user_id: int, total: decimal.Decimal, notes: str | None) -> CreateOrderRow:
    """Execute CreateOrder query."""
    async with conn.cursor() as cur:
        out_id = cur.var(oracledb.NUMBER)
        out_user_id = cur.var(oracledb.NUMBER)
        out_total = cur.var(oracledb.NUMBER)
        out_notes = cur.var(oracledb.STRING, 4000)
        out_created_at = cur.var(oracledb.DATETIME)
        await cur.execute(
            """INSERT INTO orders (user_id, total, notes) VALUES (:1, :2, :3) RETURNING id, user_id, total, notes, created_at INTO :4, :5, :6, :7, :8""",
            [user_id, total, notes, out_id, out_user_id, out_total, out_notes, out_created_at],
        )
        return CreateOrderRow(
            id=out_id.getvalue()[0],
            user_id=out_user_id.getvalue()[0],
            total=out_total.getvalue()[0],
            notes=out_notes.getvalue()[0],
            created_at=out_created_at.getvalue()[0],
        )


@dataclass(frozen=True, slots=True)
class GetOrdersByUserRow:
    """Row type for GetOrdersByUser query."""

    id: int
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


async def get_orders_by_user(conn: oracledb.AsyncConnection, *, user_id: int) -> list[GetOrdersByUserRow]:
    """Execute GetOrdersByUser query."""
    async with conn.cursor() as cur:
        await cur.execute(
            """SELECT id, total, notes, created_at FROM orders WHERE user_id = :1 ORDER BY created_at DESC""",
            [user_id],
        )
        rows = await cur.fetchall()
        return [GetOrdersByUserRow(id=r[0], total=r[1], notes=r[2], created_at=r[3]) for r in rows]


@dataclass(frozen=True, slots=True)
class GetOrderTotalRow:
    """Row type for GetOrderTotal query."""

    total_sum: decimal.Decimal | None


async def get_order_total(conn: oracledb.AsyncConnection, *, user_id: int) -> GetOrderTotalRow:
    """Execute GetOrderTotal query."""
    async with conn.cursor() as cur:
        await cur.execute(
            """SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :1""",
            [user_id],
        )
        row = await cur.fetchone()
        if row is None:
            raise ScytheNoRowsError("GetOrderTotal: no rows returned")
        return GetOrderTotalRow(total_sum=row[0])


async def delete_orders_by_user(conn: oracledb.AsyncConnection, *, user_id: int) -> int:
    """Execute DeleteOrdersByUser query."""
    async with conn.cursor() as cur:
        await cur.execute("""DELETE FROM orders WHERE user_id = :1""", [user_id])
        return cur.rowcount


@dataclass(frozen=True, slots=True)
class GetUserByIdRow:
    """Row type for GetUserById query."""

    id: int
    name: str
    email: str | None
    active: int
    created_at: datetime.datetime


async def get_user_by_id(conn: oracledb.AsyncConnection, *, id: int) -> GetUserByIdRow:
    """Execute GetUserById query."""
    async with conn.cursor() as cur:
        await cur.execute(
            """SELECT id, name, email, active, created_at FROM users WHERE id = :1""",
            [id],
        )
        row = await cur.fetchone()
        if row is None:
            raise ScytheNoRowsError("GetUserById: no rows returned")
        return GetUserByIdRow(
            id=row[0],
            name=row[1],
            email=row[2],
            active=row[3],
            created_at=row[4],
        )


@dataclass(frozen=True, slots=True)
class ListActiveUsersRow:
    """Row type for ListActiveUsers query."""

    id: int
    name: str
    email: str | None


async def list_active_users(conn: oracledb.AsyncConnection) -> list[ListActiveUsersRow]:
    """Execute ListActiveUsers query."""
    async with conn.cursor() as cur:
        await cur.execute("""SELECT id, name, email FROM users WHERE active = 1""")
        rows = await cur.fetchall()
        return [ListActiveUsersRow(id=r[0], name=r[1], email=r[2]) for r in rows]


@dataclass(frozen=True, slots=True)
class CreateUserRow:
    """Row type for CreateUser query."""

    id: int
    name: str
    email: str | None
    active: int
    created_at: datetime.datetime


async def create_user(conn: oracledb.AsyncConnection, *, name: str, email: str | None, active: int) -> CreateUserRow:
    """Execute CreateUser query."""
    async with conn.cursor() as cur:
        out_id = cur.var(oracledb.NUMBER)
        out_name = cur.var(oracledb.STRING, 4000)
        out_email = cur.var(oracledb.STRING, 4000)
        out_active = cur.var(oracledb.NUMBER)
        out_created_at = cur.var(oracledb.DATETIME)
        await cur.execute(
            """INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :4, :5, :6, :7, :8""",
            [name, email, active, out_id, out_name, out_email, out_active, out_created_at],
        )
        return CreateUserRow(
            id=out_id.getvalue()[0],
            name=out_name.getvalue()[0],
            email=out_email.getvalue()[0],
            active=out_active.getvalue()[0],
            created_at=out_created_at.getvalue()[0],
        )


async def update_user_email(conn: oracledb.AsyncConnection, *, email: str, id: int) -> None:
    """Execute UpdateUserEmail query."""
    async with conn.cursor() as cur:
        await cur.execute("""UPDATE users SET email = :1 WHERE id = :2""", [email, id])


async def delete_user(conn: oracledb.AsyncConnection, *, id: int) -> None:
    """Execute DeleteUser query."""
    async with conn.cursor() as cur:
        await cur.execute("""DELETE FROM users WHERE id = :1""", [id])


@dataclass(frozen=True, slots=True)
class SearchUsersRow:
    """Row type for SearchUsers query."""

    id: int
    name: str
    email: str | None


async def search_users(conn: oracledb.AsyncConnection, *, name: str) -> list[SearchUsersRow]:
    """Execute SearchUsers query."""
    async with conn.cursor() as cur:
        await cur.execute(
            """SELECT id, name, email FROM users WHERE name LIKE :1""",
            [name],
        )
        rows = await cur.fetchall()
        return [SearchUsersRow(id=r[0], name=r[1], email=r[2]) for r in rows]

