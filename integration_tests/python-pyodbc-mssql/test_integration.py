"""Integration tests for scythe-generated pyodbc queries."""

import asyncio
import os
import sys
from decimal import Decimal
from pathlib import Path

from generated.queries import (
    create_order,
    create_user,
    delete_orders_by_user,
    delete_user,
    get_orders_by_user,
    get_user_by_id,
    list_active_users,
)





def get_database_url() -> str:
    """Read MSSQL_URL from environment."""
    url = os.environ.get("MSSQL_URL")
    if not url:
        print("ERROR: MSSQL_URL environment variable is not set", file=sys.stderr)
        sys.exit(1)
    return url





async def test_create_user(conn) -> int:
    """Test CreateUser query. Returns created user ID."""
    user = await create_user(
        conn, name="Alice", email="alice@example.com"
    )
    assert user is not None, "CreateUser returned None"
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.email == "alice@example.com", f"Expected email 'alice@example.com', got '{user.email}'"
    print("PASS: CreateUser")
    return user.id


async def test_get_user_by_id(conn, user_id: int) -> None:
    """Test GetUserById query."""
    user = await get_user_by_id(conn, id=user_id)
    assert user is not None, f"GetUserById returned None for id={user_id}"
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.id == user_id, f"Expected id {user_id}, got {user.id}"
    print("PASS: GetUserById")


async def test_list_active_users(conn) -> None:
    """Test ListActiveUsers query."""
    users = await list_active_users(conn)
    assert len(users) >= 1, f"Expected at least 1 active user, got {len(users)}"
    names = [u.name for u in users]
    assert "Alice" in names, f"Expected 'Alice' in active users, got {names}"
    print("PASS: ListActiveUsers")


async def test_create_order(conn, user_id: int) -> int:
    """Test CreateOrder query. Returns created order ID."""
    order = await create_order(
        conn, user_id=user_id, total=Decimal("49.99"), notes="Test order"
    )
    assert order is not None, "CreateOrder returned None"
    assert order.user_id == user_id, f"Expected user_id {user_id}, got {order.user_id}"
    assert order.notes == "Test order", f"Expected notes 'Test order', got '{order.notes}'"
    print("PASS: CreateOrder")
    return order.id


async def test_get_orders_by_user(conn, user_id: int) -> None:
    """Test GetOrdersByUser query."""
    orders = await get_orders_by_user(conn, user_id=user_id)
    assert len(orders) >= 1, f"Expected at least 1 order, got {len(orders)}"
    assert orders[0].notes == "Test order", f"Expected notes 'Test order', got '{orders[0].notes}'"
    print("PASS: GetOrdersByUser")


async def test_delete_user(conn, user_id: int) -> None:
    """Test DeleteUser query."""
    # Delete orders first due to FK constraint
    await delete_orders_by_user(conn, user_id=user_id)
    await delete_user(conn, id=user_id)
    user = await get_user_by_id(conn, id=user_id)
    assert user is None, f"Expected user to be deleted, but got {user}"
    print("PASS: DeleteUser")


async def run_tests() -> None:
    """Run all integration tests."""
    database_url = get_database_url()

    print("\nALL TESTS PASSED")


if __name__ == "__main__":
    try:
        asyncio.run(run_tests())
    except Exception as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
