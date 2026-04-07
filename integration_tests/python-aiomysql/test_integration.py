"""Integration tests for scythe-generated aiomysql queries."""

import asyncio
import os
import sys
from pathlib import Path

import aiomysql

from generated.queries import (
    create_user,
    delete_orders_by_user,
    delete_user,
    create_order,
    get_last_insert_user,
    get_last_insert_order,
    get_orders_by_user,
    get_user_by_id,
    list_active_users,
)


SCHEMA_PATH = Path(__file__).parent.parent / "sql" / "mysql" / "schema.sql"


def get_database_url() -> str:
    """Read DATABASE_URL from environment."""
    url = os.environ.get("DATABASE_URL")
    if not url:
        print("ERROR: DATABASE_URL environment variable is not set", file=sys.stderr)
        sys.exit(1)
    return url


async def setup_schema(conn: aiomysql.Connection) -> None:
    """Drop all tables and recreate schema from SQL file."""
    async with conn.cursor() as cur:
        await cur.execute("SET FOREIGN_KEY_CHECKS = 0")
        await cur.execute("DROP TABLE IF EXISTS user_tags")
        await cur.execute("DROP TABLE IF EXISTS tags")
        await cur.execute("DROP TABLE IF EXISTS orders")
        await cur.execute("DROP TABLE IF EXISTS users")
        await cur.execute("SET FOREIGN_KEY_CHECKS = 1")
        schema_sql = SCHEMA_PATH.read_text()
        for statement in schema_sql.split(";"):
            statement = statement.strip()
            if statement:
                await cur.execute(statement)
    await conn.commit()


async def test_create_user(conn: aiomysql.Connection) -> int:
    """Test CreateUser + GetLastInsertUser queries. Returns created user ID."""
    await create_user(conn, name="Alice", email="alice@example.com", status="active")
    user = await get_last_insert_user(conn)
    assert user is not None, "GetLastInsertUser returned None"
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.email == "alice@example.com", f"Expected email, got '{user.email}'"
    print("PASS: CreateUser")
    return user.id


async def test_get_user_by_id(conn: aiomysql.Connection, user_id: int) -> None:
    """Test GetUserById query."""
    user = await get_user_by_id(conn, id=user_id)
    assert user is not None, f"GetUserById returned None for id={user_id}"
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.id == user_id, f"Expected id {user_id}, got {user.id}"
    print("PASS: GetUserById")


async def test_list_active_users(conn: aiomysql.Connection) -> None:
    """Test ListActiveUsers query."""
    users = await list_active_users(conn, status="active")
    assert len(users) >= 1, f"Expected at least 1 active user, got {len(users)}"
    names = [u.name for u in users]
    assert "Alice" in names, f"Expected 'Alice' in active users, got {names}"
    print("PASS: ListActiveUsers")


async def test_create_order(conn: aiomysql.Connection, user_id: int) -> int:
    """Test CreateOrder + GetLastInsertOrder queries. Returns created order ID."""
    await create_order(conn, user_id=user_id, total=49.99, notes="Test order")
    order = await get_last_insert_order(conn)
    assert order is not None, "GetLastInsertOrder returned None"
    assert order.user_id == user_id, f"Expected user_id {user_id}, got {order.user_id}"
    assert order.notes == "Test order", f"Expected notes 'Test order', got '{order.notes}'"
    print("PASS: CreateOrder")
    return order.id


async def test_get_orders_by_user(conn: aiomysql.Connection, user_id: int) -> None:
    """Test GetOrdersByUser query."""
    orders = await get_orders_by_user(conn, user_id=user_id)
    assert len(orders) >= 1, f"Expected at least 1 order, got {len(orders)}"
    assert orders[0].notes == "Test order", f"Expected notes 'Test order', got '{orders[0].notes}'"
    print("PASS: GetOrdersByUser")


async def test_delete_user(conn: aiomysql.Connection, user_id: int) -> None:
    """Test DeleteUser query."""
    await delete_orders_by_user(conn, user_id=user_id)
    await delete_user(conn, id=user_id)
    user = await get_user_by_id(conn, id=user_id)
    assert user is None, f"Expected user to be deleted, but got {user}"
    print("PASS: DeleteUser")


async def run_tests() -> None:
    """Run all integration tests."""
    database_url = get_database_url()
    # Parse mysql://user:pass@host:port/db
    # aiomysql uses individual params, not a URL
    import urllib.parse

    parsed = urllib.parse.urlparse(database_url)
    conn = await aiomysql.connect(
        host=parsed.hostname or "localhost",
        port=parsed.port or 3306,
        user=parsed.username or "root",
        password=parsed.password or "",
        db=parsed.path.lstrip("/"),
        autocommit=True,
    )
    try:
        await setup_schema(conn)

        user_id = await test_create_user(conn)
        await test_get_user_by_id(conn, user_id)
        await test_list_active_users(conn)
        order_id = await test_create_order(conn, user_id)
        await test_get_orders_by_user(conn, user_id)
        await test_delete_user(conn, user_id)
    finally:
        conn.close()

    print("\nALL TESTS PASSED")


if __name__ == "__main__":
    try:
        asyncio.run(run_tests())
    except Exception as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
