"""Integration tests for scythe-generated snowflake queries."""

import asyncio
import os
import re
import sys
from decimal import Decimal
from pathlib import Path

import snowflake.connector

try:
    import fakesnow
except ImportError:
    fakesnow = None

from generated.queries import (
    ScytheNoRowsError,
    create_order,
    create_user,
    delete_orders_by_user,
    delete_user,
    get_orders_by_user,
    get_user_by_id,
    list_active_users,
)


SCHEMA_PATH = Path(__file__).parent.parent / "sql" / "snowflake" / "schema.sql"


def get_database_url() -> str:
    """Read SNOWFLAKE_URL from environment."""
    url = os.environ.get("SNOWFLAKE_URL")
    if not url:
        print("ERROR: SNOWFLAKE_URL environment variable is not set", file=sys.stderr)
        sys.exit(1)
    return url


def split_sql_statements(sql: str) -> list[str]:
    """Split a SQL script into statements on top-level ';' only.

    A naive `sql.split(";")` breaks on a ';' inside a string literal, a
    PostgreSQL `$$ ... $$` dollar-quoted function body, or a '--' line
    comment (an apostrophe in a comment must not open a phantom string --
    board #224 follow-up). This tracks that state instead so none of those
    split the statement in half. '/* ... */' block comments are not
    handled -- no schema under integration_tests/sql/ uses them today.
    """
    statements: list[str] = []
    current: list[str] = []
    in_single = False
    in_double = False
    in_line_comment = False
    dollar_tag: str | None = None
    i = 0
    length = len(sql)
    while i < length:
        ch = sql[i]
        if in_line_comment:
            current.append(ch)
            if ch == "\n":
                in_line_comment = False
            i += 1
            continue
        if dollar_tag is not None:
            current.append(ch)
            if ch == "$" and sql.startswith(dollar_tag, i):
                current.append(dollar_tag[1:])
                i += len(dollar_tag)
                continue
            i += 1
            continue
        if in_single:
            current.append(ch)
            if ch == "'":
                in_single = False
            i += 1
            continue
        if in_double:
            current.append(ch)
            if ch == '"':
                in_double = False
            i += 1
            continue
        if ch == "-" and sql[i + 1 : i + 2] == "-":
            in_line_comment = True
            current.append(ch)
            i += 1
            continue
        if ch == "'":
            in_single = True
            current.append(ch)
            i += 1
            continue
        if ch == '"':
            in_double = True
            current.append(ch)
            i += 1
            continue
        if ch == "$":
            match = re.match(r"\$[A-Za-z0-9_]*\$", sql[i:])
            if match:
                dollar_tag = match.group(0)
                current.append(dollar_tag)
                i += len(dollar_tag)
                continue
        if ch == ";":
            statements.append("".join(current))
            current = []
            i += 1
            continue
        current.append(ch)
        i += 1
    if "".join(current).strip():
        statements.append("".join(current))
    return [s.strip() for s in statements if s.strip()]


def setup_schema(conn) -> None:
    """Drop all tables and recreate schema from SQL file."""
    cursor = conn.cursor()
    for table in ("user_tags", "tags", "orders", "users"):
        try:
            cursor.execute(f"DROP TABLE IF EXISTS {table}")
        except Exception:
            pass
    schema_sql = SCHEMA_PATH.read_text()
    for stmt in split_sql_statements(schema_sql):
        cursor.execute(stmt)
    conn.commit()


def test_create_user(conn) -> int:
    """Test CreateUser query. Returns created user ID."""
    create_user(conn, name="Alice", email="alice@example.com", active=True)
    cursor = conn.cursor()
    cursor.execute("SELECT MAX(id) FROM users")
    max_id_row = cursor.fetchone()
    user_id = max_id_row[0] if max_id_row and max_id_row[0] else 1
    user = get_user_by_id(conn, id=user_id)
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.email == "alice@example.com", f"Expected email 'alice@example.com', got '{user.email}'"
    conn.commit()
    print("PASS: CreateUser")
    return user.id


def test_get_user_by_id(conn, user_id: int) -> None:
    """Test GetUserById query."""
    user = get_user_by_id(conn, id=user_id)
    assert user.name == "Alice", f"Expected name 'Alice', got '{user.name}'"
    assert user.id == user_id, f"Expected id {user_id}, got {user.id}"
    print("PASS: GetUserById")


def test_list_active_users(conn) -> None:
    """Test ListActiveUsers query."""
    users = list_active_users(conn)
    assert len(users) >= 1, f"Expected at least 1 active user, got {len(users)}"
    names = [u.name for u in users]
    assert "Alice" in names, f"Expected 'Alice' in active users, got {names}"
    print("PASS: ListActiveUsers")


def test_create_order(conn, user_id: int) -> int:
    """Test CreateOrder query. Returns created order ID."""
    create_order(conn, user_id=user_id, total=Decimal("49.99"), notes="Test order")
    orders = get_orders_by_user(conn, user_id=user_id)
    order = orders[0] if orders else None
    assert order is not None, "CreateOrder returned None"
    assert order.notes == "Test order", f"Expected notes 'Test order', got '{order.notes}'"
    conn.commit()
    print("PASS: CreateOrder")
    return order.id


def test_get_orders_by_user(conn, user_id: int, order_id: int) -> None:
    """Test GetOrdersByUser query."""
    orders = get_orders_by_user(conn, user_id=user_id)
    assert len(orders) >= 1, f"Expected at least 1 order, got {len(orders)}"
    assert orders[0].notes == "Test order", f"Expected notes 'Test order', got '{orders[0].notes}'"
    assert any(o.id == order_id for o in orders), f"Expected order {order_id} in results, got {[o.id for o in orders]}"
    print("PASS: GetOrdersByUser")


def test_delete_user(conn, user_id: int) -> None:
    """Test DeleteUser query."""
    # Delete orders first due to FK constraint
    delete_orders_by_user(conn, user_id=user_id)
    delete_user(conn, id=user_id)
    conn.commit()
    try:
        user = get_user_by_id(conn, id=user_id)
    except ScytheNoRowsError:
        pass
    else:
        raise AssertionError(f"Expected user to be deleted, but got {user}")
    print("PASS: DeleteUser")


def run_tests() -> None:
    """Run all integration tests."""
    use_fakesnow = os.environ.get("SNOWFLAKE_USE_FAKESNOW", "0") == "1"
    if use_fakesnow:
        if not fakesnow:
            print("ERROR: fakesnow not installed but SNOWFLAKE_USE_FAKESNOW=1", file=sys.stderr)
            sys.exit(1)
        snowflake.connector.paramstyle = "qmark"
        with fakesnow.patch():
            conn = snowflake.connector.connect(database="testdb", schema="public")
            try:
                setup_schema(conn)

                user_id = test_create_user(conn)
                test_get_user_by_id(conn, user_id)
                test_list_active_users(conn)
                order_id = test_create_order(conn, user_id)
                test_get_orders_by_user(conn, user_id, order_id)
                test_delete_user(conn, user_id)
            finally:
                conn.close()
    else:
        database_url = get_database_url()
        from urllib.parse import urlparse, parse_qs
        parsed = urlparse(database_url)
        # Parse snowflake://user:password@host:port/database/schema?account=X&protocol=http
        query_params = parse_qs(parsed.query or "")
        account = query_params.get("account", [parsed.hostname])[0]
        protocol = query_params.get("protocol", ["https"])[0]
        path_parts = parsed.path.strip("/").split("/")
        database = path_parts[0] if len(path_parts) > 0 else "testdb"
        schema = path_parts[1] if len(path_parts) > 1 else "public"
        conn = snowflake.connector.connect(
            account=account,
            user=parsed.username or "test",
            password=parsed.password or "test",
            host=parsed.hostname or "localhost",
            port=parsed.port or 443,
            database=database,
            schema=schema,
            protocol=protocol,
        )
        try:
            setup_schema(conn)

            user_id = test_create_user(conn)
            test_get_user_by_id(conn, user_id)
            test_list_active_users(conn)
            order_id = test_create_order(conn, user_id)
            test_get_orders_by_user(conn, user_id, order_id)
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
