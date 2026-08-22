# scythe:provenance v=0.18.1 backend=python-asyncpg engine=postgresql schema=sch2:59e0edaa3ac94824 queries=q1:861cdfc5df3ece62 options=opt1:cbf29ce484222325  # noqa: E501
import datetime  # noqa: F401
import decimal  # noqa: F401
import uuid  # noqa: F401
from dataclasses import dataclass
from enum import Enum  # noqa: F401
from typing import Any  # noqa: F401

from asyncpg import Connection  # noqa: F401


class ScytheNoRowsError(Exception):
    """Raised by a `:one` query when no row matches."""



class UserStatus(str, Enum):
    """Database enum type user_status."""

    ACTIVE = "active"
    INACTIVE = "inactive"
    BANNED = "banned"


@dataclass(frozen=True, slots=True)
class UserAddress:
    """Composite type user_address."""

    street: str | None
    city: str | None
    zip: str | None

    @classmethod
    def _from_record(cls, record: Any) -> "UserAddress | None":
        """~keep board #204: asyncpg decodes a composite column to its own
        `Record` (tuple-like, not our declared type) -- wrap it into this class."""
        if record is None:
            return None
        return cls(
            street=None if record["street"] is None else record["street"],
            city=None if record["city"] is None else record["city"],
            zip=None if record["zip"] is None else record["zip"],
        )

    def _to_record(self) -> tuple[Any, ...]:
        return (self.street, self.city, self.zip)


@dataclass(frozen=True, slots=True)
class CreateOrderRow:
    """Row type for CreateOrder query."""

    id: int
    user_id: int
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


async def create_order(conn: Connection, *, user_id: int, total: decimal.Decimal, notes: str | None) -> CreateOrderRow:
    """Execute CreateOrder query."""
    row = await conn.fetchrow(
        """INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at""",
        user_id, total, notes,
    )
    if row is None:
        raise ScytheNoRowsError("CreateOrder: no rows returned")
    return CreateOrderRow(
        id=row["id"],
        user_id=row["user_id"],
        total=row["total"],
        notes=row["notes"],
        created_at=row["created_at"],
    )


@dataclass(frozen=True, slots=True)
class GetOrdersByUserRow:
    """Row type for GetOrdersByUser query."""

    id: int
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


async def get_orders_by_user(conn: Connection, *, user_id: int) -> list[GetOrdersByUserRow]:
    """Execute GetOrdersByUser query."""
    rows = await conn.fetch(
        """SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC""",
        user_id,
    )
    return [
        GetOrdersByUserRow(
            id=r["id"],
            total=r["total"],
            notes=r["notes"],
            created_at=r["created_at"],
        )
        for r in rows
    ]


@dataclass(frozen=True, slots=True)
class GetOrderTotalRow:
    """Row type for GetOrderTotal query."""

    total_sum: decimal.Decimal | None


async def get_order_total(conn: Connection, *, user_id: int) -> GetOrderTotalRow:
    """Execute GetOrderTotal query."""
    row = await conn.fetchrow(
        """SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1""",
        user_id,
    )
    if row is None:
        raise ScytheNoRowsError("GetOrderTotal: no rows returned")
    return GetOrderTotalRow(total_sum=row["total_sum"])


@dataclass(frozen=True, slots=True)
class GetOrderWeightTotalRow:
    """Row type for GetOrderWeightTotal query."""

    weight_total: float | None


async def get_order_weight_total(conn: Connection, *, user_id: int) -> GetOrderWeightTotalRow:
    """Execute GetOrderWeightTotal query."""
    row = await conn.fetchrow(
        """SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = $1""",
        user_id,
    )
    if row is None:
        raise ScytheNoRowsError("GetOrderWeightTotal: no rows returned")
    return GetOrderWeightTotalRow(weight_total=row["weight_total"])


async def delete_orders_by_user(conn: Connection, *, user_id: int) -> int:
    """Execute DeleteOrdersByUser query."""
    result = await conn.execute(
        """DELETE FROM orders WHERE user_id = $1""",
        user_id,
    )
    return int(result.split()[-1])


@dataclass(frozen=True, slots=True)
class GetUserByIdRow:
    """Row type for GetUserById query."""

    id: int
    name: str
    email: str | None
    status: UserStatus
    created_at: datetime.datetime


async def get_user_by_id(conn: Connection, *, id: int) -> GetUserByIdRow:
    """Execute GetUserById query."""
    row = await conn.fetchrow(
        """SELECT id, name, email, status, created_at FROM users WHERE id = $1""",
        id,
    )
    if row is None:
        raise ScytheNoRowsError("GetUserById: no rows returned")
    return GetUserByIdRow(
        id=row["id"],
        name=row["name"],
        email=row["email"],
        status=UserStatus(row["status"]),
        created_at=row["created_at"],
    )


@dataclass(frozen=True, slots=True)
class ListActiveUsersRow:
    """Row type for ListActiveUsers query."""

    id: int
    name: str
    email: str | None


async def list_active_users(conn: Connection, *, status: UserStatus) -> list[ListActiveUsersRow]:
    """Execute ListActiveUsers query."""
    rows = await conn.fetch(
        """SELECT id, name, email FROM users WHERE status = $1""",
        status,
    )
    return [
        ListActiveUsersRow(
            id=r["id"],
            name=r["name"],
            email=r["email"],
        )
        for r in rows
    ]


@dataclass(frozen=True, slots=True)
class CreateUserRow:
    """Row type for CreateUser query."""

    id: int
    name: str
    email: str | None
    status: UserStatus
    created_at: datetime.datetime


async def create_user(conn: Connection, *, name: str, email: str | None, status: UserStatus) -> CreateUserRow:
    """Execute CreateUser query."""
    row = await conn.fetchrow(
        """INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at""",
        name, email, status,
    )
    if row is None:
        raise ScytheNoRowsError("CreateUser: no rows returned")
    return CreateUserRow(
        id=row["id"],
        name=row["name"],
        email=row["email"],
        status=UserStatus(row["status"]),
        created_at=row["created_at"],
    )


async def update_user_email(conn: Connection, *, email: str, id: int) -> None:
    """Execute UpdateUserEmail query."""
    await conn.execute(
        """UPDATE users SET email = $1 WHERE id = $2""",
        email, id,
    )


async def delete_user(conn: Connection, *, id: int) -> None:
    """Execute DeleteUser query."""
    await conn.execute(
        """DELETE FROM users WHERE id = $1""",
        id,
    )


@dataclass(frozen=True, slots=True)
class GetUserOrdersRow:
    """Row type for GetUserOrders query."""

    id: int
    name: str
    total: decimal.Decimal | None
    notes: str | None


async def get_user_orders(conn: Connection, *, status: UserStatus) -> list[GetUserOrdersRow]:
    """Execute GetUserOrders query."""
    rows = await conn.fetch(
        """SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1""",
        status,
    )
    return [
        GetUserOrdersRow(
            id=r["id"],
            name=r["name"],
            total=r["total"],
            notes=r["notes"],
        )
        for r in rows
    ]


@dataclass(frozen=True, slots=True)
class CountUsersByStatusRow:
    """Row type for CountUsersByStatus query."""

    status: UserStatus
    user_count: int


async def count_users_by_status(conn: Connection, *, status: UserStatus) -> CountUsersByStatusRow:
    """Execute CountUsersByStatus query."""
    row = await conn.fetchrow(
        """SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1""",
        status,
    )
    if row is None:
        raise ScytheNoRowsError("CountUsersByStatus: no rows returned")
    return CountUsersByStatusRow(
        status=UserStatus(row["status"]),
        user_count=row["user_count"],
    )


@dataclass(frozen=True, slots=True)
class GetUserWithTagsRow:
    """Row type for GetUserWithTags query."""

    id: int
    name: str
    tag_name: str


async def get_user_with_tags(conn: Connection, *, id: int) -> list[GetUserWithTagsRow]:
    """Execute GetUserWithTags query."""
    rows = await conn.fetch(
        """SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = $1""",
        id,
    )
    return [
        GetUserWithTagsRow(
            id=r["id"],
            name=r["name"],
            tag_name=r["tag_name"],
        )
        for r in rows
    ]


@dataclass(frozen=True, slots=True)
class SearchUsersRow:
    """Row type for SearchUsers query."""

    id: int
    name: str
    email: str | None


async def search_users(conn: Connection, *, name: str) -> list[SearchUsersRow]:
    """Execute SearchUsers query."""
    rows = await conn.fetch(
        """SELECT id, name, email FROM users WHERE name LIKE $1""",
        name,
    )
    return [SearchUsersRow(id=r["id"], name=r["name"], email=r["email"]) for r in rows]


@dataclass(frozen=True, slots=True)
class GetUserProfileRow:
    """Row type for GetUserProfile query."""

    id: int
    secondary_status: UserStatus | None
    address: UserAddress | None


async def get_user_profile(conn: Connection, *, id: int) -> GetUserProfileRow:
    """Execute GetUserProfile query."""
    row = await conn.fetchrow(
        """SELECT id, secondary_status, address FROM users WHERE id = $1""",
        id,
    )
    if row is None:
        raise ScytheNoRowsError("GetUserProfile: no rows returned")
    return GetUserProfileRow(
        id=row["id"],
        secondary_status=None if row["secondary_status"] is None else UserStatus(row["secondary_status"]),
        address=UserAddress._from_record(row["address"]),
    )


@dataclass(frozen=True, slots=True)
class RoundTripUserAddressRow:
    """Row type for RoundTripUserAddress query."""

    address: UserAddress | None


async def round_trip_user_address(conn: Connection, *, address: UserAddress | None) -> RoundTripUserAddressRow:
    """Execute RoundTripUserAddress query."""
    row = await conn.fetchrow(
        """INSERT INTO users (name, status, address)
VALUES ('Composite Parameter Round Trip', 'active', ($1))
RETURNING address""",
        None if address is None else address._to_record(),
    )
    if row is None:
        raise ScytheNoRowsError("RoundTripUserAddress: no rows returned")
    return RoundTripUserAddressRow(address=UserAddress._from_record(row["address"]))


@dataclass(frozen=True, slots=True)
class GetUserAsJsonRow:
    """Row type for GetUserAsJson query."""

    payload: dict[str, Any] | None


async def get_user_as_json(conn: Connection, *, id: int) -> GetUserAsJsonRow:
    """Execute GetUserAsJson query."""
    row = await conn.fetchrow(
        """SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = $1""",
        id,
    )
    if row is None:
        raise ScytheNoRowsError("GetUserAsJson: no rows returned")
    return GetUserAsJsonRow(payload=row["payload"])


@dataclass(frozen=True, slots=True)
class GetUsersAsJsonRow:
    """Row type for GetUsersAsJson query."""

    payload: list[dict[str, Any]] | None


async def get_users_as_json(conn: Connection) -> GetUsersAsJsonRow:
    """Execute GetUsersAsJson query."""
    row = await conn.fetchrow(
        """SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u""",
    )
    if row is None:
        raise ScytheNoRowsError("GetUsersAsJson: no rows returned")
    return GetUsersAsJsonRow(payload=row["payload"])


@dataclass(frozen=True, slots=True)
class GetUserOrdersAsJsonRow:
    """Row type for GetUserOrdersAsJson query."""

    payload: list[dict[str, Any]] | None


async def get_user_orders_as_json(conn: Connection, *, id: int) -> GetUserOrdersAsJsonRow:
    """Execute GetUserOrdersAsJson query."""
    row = await conn.fetchrow(
        """SELECT json_agg(o.* ORDER BY o.id) AS payload
FROM users u
LEFT JOIN orders o ON o.user_id = u.id
WHERE u.id = $1
GROUP BY u.id""",
        id,
    )
    if row is None:
        raise ScytheNoRowsError("GetUserOrdersAsJson: no rows returned")
    return GetUserOrdersAsJsonRow(payload=row["payload"])

