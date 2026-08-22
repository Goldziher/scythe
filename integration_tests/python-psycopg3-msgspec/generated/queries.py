# scythe:provenance v=0.18.0 backend=python-psycopg3 engine=postgresql schema=sch2:59e0edaa3ac94824 queries=q1:861cdfc5df3ece62 options=opt1:44be2db4ed87131f  # noqa: E501
import datetime  # noqa: F401
import decimal  # noqa: F401
import uuid  # noqa: F401
from enum import Enum  # noqa: F401
from typing import Any  # noqa: F401

import msgspec
from psycopg import AsyncConnection  # noqa: F401


class ScytheNoRowsError(Exception):
    """Raised by a `:one` query when no row matches."""



class UserStatus(str, Enum):
    """Database enum type user_status."""

    ACTIVE = "active"
    INACTIVE = "inactive"
    BANNED = "banned"


class UserAddress(msgspec.Struct):
    """Composite type user_address."""

    street: str | None
    city: str | None
    zip: str | None

    @classmethod
    def _from_text(cls, text: str | None) -> "UserAddress | None":
        """~keep board #204: psycopg3 registers no adapter for a
        user-defined composite -- it hands back the driver's raw text form
        as a plain str. Parse it here instead."""
        if text is None:
            return None
        f = cls._parse_composite_fields(text)
        return cls(
            street=None if f[0] is None else cls._require_composite_field(f[0], "street"),
            city=None if f[1] is None else cls._require_composite_field(f[1], "city"),
            zip=None if f[2] is None else cls._require_composite_field(f[2], "zip"),
        )

    def _to_pg_text(self) -> str:
        return "(" + ",".join([self._encode_composite_field(self.street), self._encode_composite_field(self.city), self._encode_composite_field(self.zip)]) + ")"

    @staticmethod
    def _encode_composite_field(value: Any) -> str:
        if value is None:
            return ""
        if hasattr(value, "_to_pg_text"):
            raw = value._to_pg_text()
        elif isinstance(value, Enum):
            raw = str(value.value)
        else:
            raw = str(value)
        if raw and not any(char in raw for char in ',()\"\\') and raw == raw.strip():
            return raw
        escaped = raw.replace("\\", "\\\\").replace('\"', '\"\"')
        return f'\"{escaped}\"'

    @staticmethod
    def _parse_composite_fields(text: str) -> list[str | None]:
        """~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field
        tokens, honoring its escaping rules: an empty unquoted field is SQL NULL (returned
        as None); a field needing quoting (comma, paren, quote, backslash, leading/trailing
        space, or the empty string) is wrapped in double quotes; every other field is
        unquoted and taken literally. A nested composite's own "(x,y)" text form always
        contains parens, so it always comes back quoted here, ready for that type's own
        `_from_text` to parse recursively.

        Inside a quoted field `record_out` doubles a literal double-quote and backslash-
        escapes a literal backslash. Both spellings must be accepted: reading a doubled
        quote as "closing quote, then a new field" both truncates the value and
        desynchronizes every field after it. Verified against PostgreSQL 16.
        """
        fields: list[str | None] = []
        inner = text[1:-1]
        i = 0
        n = len(inner)
        while True:
            chars: list[str] = []
            is_null = False
            if i < n and inner[i] == '"':
                i += 1
                while i < n:
                    c = inner[i]
                    if c == "\\" and i + 1 < n:
                        chars.append(inner[i + 1])
                        i += 2
                    elif c == '"' and i + 1 < n and inner[i + 1] == '"':
                        chars.append('"')
                        i += 2
                    elif c == '"':
                        i += 1
                        break
                    else:
                        chars.append(c)
                        i += 1
            else:
                start = i
                while i < n and inner[i] != ",":
                    i += 1
                chars = list(inner[start:i])
                is_null = len(chars) == 0
            fields.append(None if is_null else "".join(chars))
            if i < n and inner[i] == ",":
                i += 1
                continue
            break
        return fields

    @staticmethod
    def _require_composite_field(raw: str | None, field: str) -> str:
        """~keep A composite's fields are all declared non-nullable -- CompositeFieldInfo
        carries no per-field nullability -- but PostgreSQL happily stores a NULL sub-field,
        which arrives here as None. Raising names the field that was NULL; returning None
        through an annotation that says `str` would hand the caller a value its own type
        says is impossible."""
        if raw is None:
            raise ValueError(f"composite field {field!r} is NULL, which its generated type cannot represent")
        return raw


class GetUserAsJsonRowPayload(msgspec.Struct):
    """Nested struct for get_user_as_json_row_payload."""

    id: int
    name: str
    email: str | None
    status: UserStatus
    secondary_status: UserStatus | None
    address: UserAddress | None
    created_at: datetime.datetime

    @classmethod
    def _from_json(cls, obj: dict[str, Any]) -> "GetUserAsJsonRowPayload":
        """Build from one decoded JSON object."""
        return cls(
            id=obj["id"],
            name=obj["name"],
            email=obj["email"],
            status=obj["status"],
            secondary_status=obj["secondary_status"],
            address=obj["address"],
            created_at=obj["created_at"],
        )


class GetUsersAsJsonRowPayload(msgspec.Struct):
    """Nested struct for get_users_as_json_row_payload."""

    id: int
    name: str
    email: str | None
    status: UserStatus
    secondary_status: UserStatus | None
    address: UserAddress | None
    created_at: datetime.datetime

    @classmethod
    def _from_json(cls, obj: dict[str, Any]) -> "GetUsersAsJsonRowPayload":
        """Build from one decoded JSON object."""
        return cls(
            id=obj["id"],
            name=obj["name"],
            email=obj["email"],
            status=obj["status"],
            secondary_status=obj["secondary_status"],
            address=obj["address"],
            created_at=obj["created_at"],
        )


class GetUserOrdersAsJsonRowPayload(msgspec.Struct):
    """Nested struct for get_user_orders_as_json_row_payload."""

    id: int
    user_id: int
    total: decimal.Decimal
    weight_kg: float | None
    notes: str | None
    created_at: datetime.datetime

    @classmethod
    def _from_json(cls, obj: dict[str, Any]) -> "GetUserOrdersAsJsonRowPayload":
        """Build from one decoded JSON object."""
        return cls(
            id=obj["id"],
            user_id=obj["user_id"],
            total=obj["total"],
            weight_kg=obj["weight_kg"],
            notes=obj["notes"],
            created_at=obj["created_at"],
        )


class CreateOrderRow(msgspec.Struct):
    """Row type for CreateOrder query."""

    id: int
    user_id: int
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


async def create_order(conn: AsyncConnection, *, user_id: int, total: decimal.Decimal, notes: str | None) -> CreateOrderRow:
    """Execute CreateOrder query."""
    cur = await conn.execute(
        """INSERT INTO orders (user_id, total, notes) VALUES (%(user_id)s, %(total)s, %(notes)s) RETURNING id, user_id, total, notes, created_at""",
        {"user_id": user_id, "total": total, "notes": notes},
    )
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


class GetOrdersByUserRow(msgspec.Struct):
    """Row type for GetOrdersByUser query."""

    id: int
    total: decimal.Decimal
    notes: str | None
    created_at: datetime.datetime


async def get_orders_by_user(conn: AsyncConnection, *, user_id: int) -> list[GetOrdersByUserRow]:
    """Execute GetOrdersByUser query."""
    cur = await conn.execute(
        """SELECT id, total, notes, created_at FROM orders WHERE user_id = %(user_id)s ORDER BY created_at DESC""",
        {"user_id": user_id},
    )
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


class GetOrderTotalRow(msgspec.Struct):
    """Row type for GetOrderTotal query."""

    total_sum: decimal.Decimal | None


async def get_order_total(conn: AsyncConnection, *, user_id: int) -> GetOrderTotalRow:
    """Execute GetOrderTotal query."""
    cur = await conn.execute(
        """SELECT SUM(total) AS total_sum FROM orders WHERE user_id = %(user_id)s""",
        {"user_id": user_id},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetOrderTotal: no rows returned")
    return GetOrderTotalRow(total_sum=row[0])


class GetOrderWeightTotalRow(msgspec.Struct):
    """Row type for GetOrderWeightTotal query."""

    weight_total: float | None


async def get_order_weight_total(conn: AsyncConnection, *, user_id: int) -> GetOrderWeightTotalRow:
    """Execute GetOrderWeightTotal query."""
    cur = await conn.execute(
        """SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = %(user_id)s""",
        {"user_id": user_id},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetOrderWeightTotal: no rows returned")
    return GetOrderWeightTotalRow(weight_total=row[0])


async def delete_orders_by_user(conn: AsyncConnection, *, user_id: int) -> int:
    """Execute DeleteOrdersByUser query."""
    cur = await conn.execute(
        """DELETE FROM orders WHERE user_id = %(user_id)s""",
        {"user_id": user_id},
    )
    return cur.rowcount


class GetUserByIdRow(msgspec.Struct):
    """Row type for GetUserById query."""

    id: int
    name: str
    email: str | None
    status: UserStatus
    created_at: datetime.datetime


async def get_user_by_id(conn: AsyncConnection, *, id: int) -> GetUserByIdRow:
    """Execute GetUserById query."""
    cur = await conn.execute(
        """SELECT id, name, email, status, created_at FROM users WHERE id = %(id)s""",
        {"id": id},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetUserById: no rows returned")
    return GetUserByIdRow(
        id=row[0],
        name=row[1],
        email=row[2],
        status=UserStatus(row[3]),
        created_at=row[4],
    )


class ListActiveUsersRow(msgspec.Struct):
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


class CreateUserRow(msgspec.Struct):
    """Row type for CreateUser query."""

    id: int
    name: str
    email: str | None
    status: UserStatus
    created_at: datetime.datetime


async def create_user(conn: AsyncConnection, *, name: str, email: str | None, status: UserStatus) -> CreateUserRow:
    """Execute CreateUser query."""
    cur = await conn.execute(
        """INSERT INTO users (name, email, status) VALUES (%(name)s, %(email)s, %(status)s) RETURNING id, name, email, status, created_at""",
        {"name": name, "email": email, "status": status},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("CreateUser: no rows returned")
    return CreateUserRow(
        id=row[0],
        name=row[1],
        email=row[2],
        status=UserStatus(row[3]),
        created_at=row[4],
    )


async def update_user_email(conn: AsyncConnection, *, email: str, id: int) -> None:
    """Execute UpdateUserEmail query."""
    await conn.execute(
        """UPDATE users SET email = %(email)s WHERE id = %(id)s""",
        {"email": email, "id": id},
    )


async def delete_user(conn: AsyncConnection, *, id: int) -> None:
    """Execute DeleteUser query."""
    await conn.execute(
        """DELETE FROM users WHERE id = %(id)s""",
        {"id": id},
    )


class GetUserOrdersRow(msgspec.Struct):
    """Row type for GetUserOrders query."""

    id: int
    name: str
    total: decimal.Decimal | None
    notes: str | None


async def get_user_orders(conn: AsyncConnection, *, status: UserStatus) -> list[GetUserOrdersRow]:
    """Execute GetUserOrders query."""
    cur = await conn.execute(
        """SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = %(status)s""",
        {"status": status},
    )
    rows = await cur.fetchall()
    return [GetUserOrdersRow(id=r[0], name=r[1], total=r[2], notes=r[3]) for r in rows]


class CountUsersByStatusRow(msgspec.Struct):
    """Row type for CountUsersByStatus query."""

    status: UserStatus
    user_count: int


async def count_users_by_status(conn: AsyncConnection, *, status: UserStatus) -> CountUsersByStatusRow:
    """Execute CountUsersByStatus query."""
    cur = await conn.execute(
        """SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = %(status)s""",
        {"status": status},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("CountUsersByStatus: no rows returned")
    return CountUsersByStatusRow(status=UserStatus(row[0]), user_count=row[1])


class GetUserWithTagsRow(msgspec.Struct):
    """Row type for GetUserWithTags query."""

    id: int
    name: str
    tag_name: str


async def get_user_with_tags(conn: AsyncConnection, *, id: int) -> list[GetUserWithTagsRow]:
    """Execute GetUserWithTags query."""
    cur = await conn.execute(
        """SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = %(id)s""",
        {"id": id},
    )
    rows = await cur.fetchall()
    return [GetUserWithTagsRow(id=r[0], name=r[1], tag_name=r[2]) for r in rows]


class SearchUsersRow(msgspec.Struct):
    """Row type for SearchUsers query."""

    id: int
    name: str
    email: str | None


async def search_users(conn: AsyncConnection, *, name: str) -> list[SearchUsersRow]:
    """Execute SearchUsers query."""
    cur = await conn.execute(
        """SELECT id, name, email FROM users WHERE name LIKE %(name)s""",
        {"name": name},
    )
    rows = await cur.fetchall()
    return [SearchUsersRow(id=r[0], name=r[1], email=r[2]) for r in rows]


class GetUserProfileRow(msgspec.Struct):
    """Row type for GetUserProfile query."""

    id: int
    secondary_status: UserStatus | None
    address: UserAddress | None


async def get_user_profile(conn: AsyncConnection, *, id: int) -> GetUserProfileRow:
    """Execute GetUserProfile query."""
    cur = await conn.execute(
        """SELECT id, secondary_status, address FROM users WHERE id = %(id)s""",
        {"id": id},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetUserProfile: no rows returned")
    return GetUserProfileRow(
        id=row[0],
        secondary_status=None if row[1] is None else UserStatus(row[1]),
        address=UserAddress._from_text(row[2]),
    )


class RoundTripUserAddressRow(msgspec.Struct):
    """Row type for RoundTripUserAddress query."""

    address: UserAddress | None


async def round_trip_user_address(conn: AsyncConnection, *, address: UserAddress | None) -> RoundTripUserAddressRow:
    """Execute RoundTripUserAddress query."""
    cur = await conn.execute(
        """INSERT INTO users (name, status, address)
VALUES ('Composite Parameter Round Trip', 'active', (%(address)s::text::user_address))
RETURNING address""",
        {"address": None if address is None else address._to_pg_text()},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("RoundTripUserAddress: no rows returned")
    return RoundTripUserAddressRow(address=UserAddress._from_text(row[0]))


class GetUserAsJsonRow(msgspec.Struct):
    """Row type for GetUserAsJson query."""

    payload: GetUserAsJsonRowPayload | None


async def get_user_as_json(conn: AsyncConnection, *, id: int) -> GetUserAsJsonRow:
    """Execute GetUserAsJson query."""
    cur = await conn.execute(
        """SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = %(id)s""",
        {"id": id},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetUserAsJson: no rows returned")
    return GetUserAsJsonRow(
        payload=None if row[0] is None else GetUserAsJsonRowPayload._from_json(row[0]),
    )


class GetUsersAsJsonRow(msgspec.Struct):
    """Row type for GetUsersAsJson query."""

    payload: list[GetUsersAsJsonRowPayload] | None


async def get_users_as_json(conn: AsyncConnection) -> GetUsersAsJsonRow:
    """Execute GetUsersAsJson query."""
    cur = await conn.execute(
        """SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u""",
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetUsersAsJson: no rows returned")
    return GetUsersAsJsonRow(
        payload=None if row[0] is None else [GetUsersAsJsonRowPayload._from_json(item) for item in row[0]],
    )


class GetUserOrdersAsJsonRow(msgspec.Struct):
    """Row type for GetUserOrdersAsJson query."""

    payload: list[GetUserOrdersAsJsonRowPayload | None] | None


async def get_user_orders_as_json(conn: AsyncConnection, *, id: int) -> GetUserOrdersAsJsonRow:
    """Execute GetUserOrdersAsJson query."""
    cur = await conn.execute(
        """SELECT json_agg(o.* ORDER BY o.id) AS payload
FROM users u
LEFT JOIN orders o ON o.user_id = u.id
WHERE u.id = %(id)s
GROUP BY u.id""",
        {"id": id},
    )
    row = await cur.fetchone()
    if row is None:
        raise ScytheNoRowsError("GetUserOrdersAsJson: no rows returned")
    return GetUserOrdersAsJsonRow(
        payload=None if row[0] is None else [None if item is None else GetUserOrdersAsJsonRowPayload._from_json(item) for item in row[0]],
    )

