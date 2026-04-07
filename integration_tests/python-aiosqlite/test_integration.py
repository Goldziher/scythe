"""Integration tests for scythe-generated aiosqlite queries."""

import asyncio
import os
import sys
from pathlib import Path

import aiosqlite

from generated.queries import (
    create_user,
    delete_orders_by_user,
    delete_user,
    create_order,
    get_orders_by_user,
    get_user_by_id,
    list_active_users,
)


SCHEMA_PATH = Path(__file__).parent.parent / "sql" / "sqlite" / "schema.sql"


async def setup_schema(conn: aiosqlite.Connection) -> None:
    """Drop all tables and recreate schema from SQL file."""
    await conn.execute("DROP TABLE IF EXISTS user_tags")
    await conn.execute("DROP TABLE IF EXISTS tags")
    await conn.execute("DROP TABLE IF EXISTS orders")
    await conn.execute("DROP TABLE IF EXISTS users")
    schema_sql = SCHEMA_PATH.read_text()
    for statement in schema_sql.split(";"):
        statement = statement.strip()
        if statement:
            await conn.execute(statement)
    await conn.commit()


async def test_create_user(conn: aiosqlite.Connection) -> int:
    """Test CreateUser query. Returns created user ID."""
    await create_user(conn, name="Alice", email="alice@example.com", status="active")
    await conn.commit()
    user = await get_user_by_id(conn, id=1)
    assert user is not None, "GetUserById returned None after create"
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.email == "alice@example.com", f"Expected email, got '{user.email}'"
    print("PASS: CreateUser")
    return user.id


async def test_get_user_by_id(conn: aiosqlite.Connection, user_id: int) -> None:
    """Test GetUserById query."""
    user = await get_user_by_id(conn, id=user_id)
    assert user is not None, f"GetUserById returned None for id={user_id}"
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.id == user_id, f"Expected id {user_id}, got {user.id}"
    print("PASS: GetUserById")


async def test_list_active_users(conn: aiosqlite.Connection) -> None:
    """Test ListActiveUsers query."""
    users = await list_active_users(conn, status="active")
    assert len(users) >= 1, f"Expected at least 1 active user, got {len(users)}"
    names = [u.name for u in users]
    assert "Alice" in names, f"Expected 'Alice' in active users, got {names}"
    print("PASS: ListActiveUsers")


async def test_create_order(conn: aiosqlite.Connection, user_id: int) -> int:
    """Test CreateOrder query. Returns created order ID."""
    await create_order(conn, user_id=user_id, total=49.99, notes="Test order")
    await conn.commit()
    orders = await get_orders_by_user(conn, user_id=user_id)
    assert len(orders) >= 1, "No orders found after create"
    order = orders[0]
    assert order.notes == "Test order", f"Expected notes 'Test order', got '{order.notes}'"
    print("PASS: CreateOrder")
    return order.id


async def test_get_orders_by_user(conn: aiosqlite.Connection, user_id: int) -> None:
    """Test GetOrdersByUser query."""
    orders = await get_orders_by_user(conn, user_id=user_id)
    assert len(orders) >= 1, f"Expected at least 1 order, got {len(orders)}"
    assert orders[0].notes == "Test order", f"Expected notes 'Test order', got '{orders[0].notes}'"
    print("PASS: GetOrdersByUser")


async def test_delete_user(conn: aiosqlite.Connection, user_id: int) -> None:
    """Test DeleteUser query."""
    await delete_orders_by_user(conn, user_id=user_id)
    await delete_user(conn, id=user_id)
    await conn.commit()
    user = await get_user_by_id(conn, id=user_id)
    assert user is None, f"Expected user to be deleted, but got {user}"
    print("PASS: DeleteUser")


async def run_tests() -> None:
    """Run all integration tests."""
    db_path = os.environ.get("DATABASE_PATH", ":memory:")
    async with aiosqlite.connect(db_path) as conn:
        await conn.execute("PRAGMA foreign_keys = ON")
        await setup_schema(conn)

        user_id = await test_create_user(conn)
        await test_get_user_by_id(conn, user_id)
        await test_list_active_users(conn)
        order_id = await test_create_order(conn, user_id)
        await test_get_orders_by_user(conn, user_id)
        await test_delete_user(conn, user_id)

    print("\nALL TESTS PASSED")


if __name__ == "__main__":
    try:
        asyncio.run(run_tests())
    except Exception as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
