# scythe:provenance v=0.18.0 backend=python-duckdb engine=duckdb schema=sch2:a58e9693abcdb5e7 queries=q1:3fcd9a387f9d569e options=opt1:cbf29ce484222325  # noqa: E501
import datetime  # noqa: F401
import decimal  # noqa: F401
from dataclasses import dataclass
from enum import Enum  # noqa: F401

import duckdb  # noqa: F401


class ScytheNoRowsError(Exception):
    """Raised by a `:one` query when no row matches."""



def create_order(
    conn: duckdb.DuckDBPyConnection,
    *,
    user_id: int,
    total: decimal.Decimal,
    notes: str | None,
) -> None:
    """Execute CreateOrder query."""
    conn.execute(
        """INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)""",
        [user_id, total, notes],
    )


@dataclass(frozen=True, slots=True)
class GetOrdersByUserRow:
    """Row type for GetOrdersByUser query."""

    id: int
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


def get_orders_by_user(
    conn: duckdb.DuckDBPyConnection,
    *,
    user_id: int,
) -> list[GetOrdersByUserRow]:
    """Execute GetOrdersByUser query."""
    rows = conn.execute(
        """
SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC
""",
        [user_id],
    ).fetchall()
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


def get_order_total(
    conn: duckdb.DuckDBPyConnection,
    *,
    user_id: int,
) -> GetOrderTotalRow:
    """Execute GetOrderTotal query."""
    _res = conn.execute(
        """SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?""",
        [user_id],
    )
    row = _res.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetOrderTotal: no rows returned")
    return GetOrderTotalRow(total_sum=row[0])


def delete_orders_by_user(conn: duckdb.DuckDBPyConnection, *, user_id: int) -> int:
    """Execute DeleteOrdersByUser query."""
    _res = conn.execute(
        """DELETE FROM orders WHERE user_id = ?""",
        [user_id],
    )
    row = _res.fetchone()
    return row[0] if row else 0


@dataclass(frozen=True, slots=True)
class GetUserByIdRow:
    """Row type for GetUserById query."""

    id: int
    name: str
    email: str | None
    status: str
    created_at: datetime.datetime


def get_user_by_id(conn: duckdb.DuckDBPyConnection, *, id: int) -> GetUserByIdRow:
    """Execute GetUserById query."""
    _res = conn.execute(
        """SELECT id, name, email, status, created_at FROM users WHERE id = ?""",
        [id],
    )
    row = _res.fetchone()
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

    id: int
    name: str
    email: str | None


def list_active_users(
    conn: duckdb.DuckDBPyConnection,
    *,
    status: str,
) -> list[ListActiveUsersRow]:
    """Execute ListActiveUsers query."""
    rows = conn.execute(
        """SELECT id, name, email FROM users WHERE status = ?""",
        [status],
    ).fetchall()
    return [ListActiveUsersRow(id=r[0], name=r[1], email=r[2]) for r in rows]


def create_user(
    conn: duckdb.DuckDBPyConnection,
    *,
    name: str,
    email: str | None,
    status: str,
) -> None:
    """Execute CreateUser query."""
    conn.execute(
        """INSERT INTO users (name, email, status) VALUES (?, ?, ?)""",
        [name, email, status],
    )


def update_user_email(conn: duckdb.DuckDBPyConnection, *, email: str, id: int) -> None:
    """Execute UpdateUserEmail query."""
    conn.execute(
        """UPDATE users SET email = ? WHERE id = ?""",
        [email, id],
    )


def delete_user(conn: duckdb.DuckDBPyConnection, *, id: int) -> None:
    """Execute DeleteUser query."""
    conn.execute(
        """DELETE FROM users WHERE id = ?""",
        [id],
    )


@dataclass(frozen=True, slots=True)
class SearchUsersRow:
    """Row type for SearchUsers query."""

    id: int
    name: str
    email: str | None


def search_users(conn: duckdb.DuckDBPyConnection, *, name: str) -> list[SearchUsersRow]:
    """Execute SearchUsers query."""
    rows = conn.execute(
        """SELECT id, name, email FROM users WHERE name LIKE ?""",
        [name],
    ).fetchall()
    return [SearchUsersRow(id=r[0], name=r[1], email=r[2]) for r in rows]

