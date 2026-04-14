"""Integration tests for scythe-generated pyodbc queries."""

import asyncio
import os
import sys
from decimal import Decimal
from pathlib import Path

import pyodbc

from generated.queries import (
    create_order,
    create_user,
    delete_orders_by_user,
    delete_user,
    get_orders_by_user,
    get_user_by_id,
    list_active_users,
)


SCHEMA_PATH = Path(__file__).parent.parent / "sql" / "mssql" / "schema.sql"


def get_database_url() -> str:
    """Read MSSQL_URL from environment."""
    url = os.environ.get("MSSQL_URL")
    if not url:
        print("ERROR: MSSQL_URL environment variable is not set", file=sys.stderr)
        sys.exit(1)
    return url


def setup_schema(conn: pyodbc.Connection) -> None:
    """Drop all tables and recreate schema from SQL file."""
    cursor = conn.cursor()
    for table in ("user_tags", "tags", "orders", "users"):
        cursor.execute(f"IF OBJECT_ID('{table}', 'U') IS NOT NULL DROP TABLE {table}")
    conn.commit()
    schema_sql = SCHEMA_PATH.read_text()
    for stmt in schema_sql.split(";"):
        stmt = stmt.strip()
        if stmt:
            cursor.execute(stmt)
    conn.commit()


def test_create_user(conn: pyodbc.Connection) -> int:
    """Test CreateUser query. Returns created user ID."""
    user = create_user(conn, name="Alice", email="alice@example.com")
    assert user is not None, "CreateUser returned None"
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.email == "alice@example.com", f"Expected email 'alice@example.com', got '{user.email}'"
    conn.commit()
    print("PASS: CreateUser")
    return user.id


def test_get_user_by_id(conn: pyodbc.Connection, user_id: int) -> None:
    """Test GetUserById query."""
    user = get_user_by_id(conn, id=user_id)
    assert user is not None, f"GetUserById returned None for id={user_id}"
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.id == user_id, f"Expected id {user_id}, got {user.id}"
    print("PASS: GetUserById")


def test_list_active_users(conn: pyodbc.Connection) -> None:
    """Test ListActiveUsers query."""
    users = list_active_users(conn)
    assert len(users) >= 1, f"Expected at least 1 active user, got {len(users)}"
    names = [u.name for u in users]
    assert "Alice" in names, f"Expected 'Alice' in active users, got {names}"
    print("PASS: ListActiveUsers")


def test_create_order(conn: pyodbc.Connection, user_id: int) -> int:
    """Test CreateOrder query. Returns created order ID."""
    order = create_order(conn, user_id=user_id, total=Decimal("49.99"), notes="Test order")
    assert order is not None, "CreateOrder returned None"
    assert order.user_id == user_id, f"Expected user_id {user_id}, got {order.user_id}"
    assert order.notes == "Test order", f"Expected notes 'Test order', got '{order.notes}'"
    conn.commit()
    print("PASS: CreateOrder")
    return order.id


def test_get_orders_by_user(conn: pyodbc.Connection, user_id: int) -> None:
    """Test GetOrdersByUser query."""
    orders = get_orders_by_user(conn, user_id=user_id)
    assert len(orders) >= 1, f"Expected at least 1 order, got {len(orders)}"
    assert orders[0].notes == "Test order", f"Expected notes 'Test order', got '{orders[0].notes}'"
    print("PASS: GetOrdersByUser")


def test_delete_user(conn: pyodbc.Connection, user_id: int) -> None:
    """Test DeleteUser query."""
    # Delete orders first due to FK constraint
    delete_orders_by_user(conn, user_id=user_id)
    delete_user(conn, id=user_id)
    conn.commit()
    user = get_user_by_id(conn, id=user_id)
    assert user is None, f"Expected user to be deleted, but got {user}"
    print("PASS: DeleteUser")


def run_tests() -> None:
    """Run all integration tests."""
    database_url = get_database_url()
    from urllib.parse import urlparse
    parsed = urlparse(database_url)
    server = f"{parsed.hostname},{parsed.port}" if parsed.port else parsed.hostname
    database = parsed.path.lstrip("/") or "master"
    conn_str = "DRIVER={ODBC Driver 18 for SQL Server};SERVER=%s;DATABASE=%s;UID=%s;PWD=%s;TrustServerCertificate=yes" % (server, database, parsed.username, parsed.password)
    conn = pyodbc.connect(conn_str)
    try:
        setup_schema(conn)

        user_id = test_create_user(conn)
        test_get_user_by_id(conn, user_id)
        test_list_active_users(conn)
        order_id = test_create_order(conn, user_id)
        test_get_orders_by_user(conn, user_id)
        test_delete_user(conn, user_id)
    finally:
        conn.close()

    print("\nALL TESTS PASSED")


if __name__ == "__main__":
    try:
        run_tests()
    except Exception as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
