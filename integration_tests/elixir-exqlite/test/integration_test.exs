alias Scythe.Queries

db_path = System.get_env("DATABASE_PATH", ":memory:")

{:ok, conn} = Exqlite.Sqlite3.open(db_path)

# Clean slate
Exqlite.Sqlite3.execute(conn, "DROP TABLE IF EXISTS user_tags")
Exqlite.Sqlite3.execute(conn, "DROP TABLE IF EXISTS tags")
Exqlite.Sqlite3.execute(conn, "DROP TABLE IF EXISTS orders")
Exqlite.Sqlite3.execute(conn, "DROP TABLE IF EXISTS users")

Exqlite.Sqlite3.execute(
  conn,
  """
  CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    email TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'inactive', 'banned')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
  )
  """
)

Exqlite.Sqlite3.execute(
  conn,
  """
  CREATE TABLE orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users (id),
    total REAL NOT NULL,
    notes TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
  )
  """
)

Exqlite.Sqlite3.execute(
  conn,
  """
  CREATE TABLE tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
  )
  """
)

Exqlite.Sqlite3.execute(
  conn,
  """
  CREATE TABLE user_tags (
    user_id INTEGER NOT NULL REFERENCES users (id),
    tag_id INTEGER NOT NULL REFERENCES tags (id),
    PRIMARY KEY (user_id, tag_id)
  )
  """
)

assert = fn condition, test_name, detail ->
  unless condition do
    IO.puts(:stderr, "FAIL: #{test_name}: #{detail}")
    Process.put(:exit_code, 1)
  end
end

Process.put(:exit_code, 0)

# Test: CreateUser
:ok = Queries.create_user(conn, "Alice", "alice@example.com", "active")

# SQLite: get last inserted user via last_insert_rowid
{:ok, stmt} = Exqlite.Sqlite3.prepare(conn, "SELECT last_insert_rowid()")
{:ok, [[last_id]]} = Exqlite.Sqlite3.fetch_all(conn, stmt)
Exqlite.Sqlite3.release(conn, stmt)
{:ok, user} = Queries.get_user_by_id(conn, last_id)
assert.(user.name == "Alice", "CreateUser", "expected name Alice, got #{user.name}")
assert.(user.email == "alice@example.com", "CreateUser", "expected email alice@example.com")
assert.(user.status == "active", "CreateUser", "expected status active, got #{user.status}")
user_id = user.id
IO.puts("PASS: CreateUser")

# Test: GetUserById
{:ok, fetched} = Queries.get_user_by_id(conn, user_id)
assert.(fetched.id == user_id, "GetUserById", "expected id #{user_id}")
assert.(fetched.name == "Alice", "GetUserById", "expected name Alice")
assert.(fetched.email == "alice@example.com", "GetUserById", "expected email alice@example.com")
IO.puts("PASS: GetUserById")

# Test: ListActiveUsers
{:ok, active_users} = Queries.list_active_users(conn, "active")
assert.(length(active_users) > 0, "ListActiveUsers", "should have at least one user")
first = List.first(active_users)
assert.(first.name == "Alice", "ListActiveUsers", "first user should be Alice")
IO.puts("PASS: ListActiveUsers")

# Test: CreateOrder
:ok = Queries.create_order(conn, user_id, 99.95, "first order")
{:ok, orders} = Queries.get_orders_by_user(conn, user_id)
assert.(length(orders) == 1, "CreateOrder", "expected 1 order, got #{length(orders)}")
first_order = List.first(orders)
assert.(first_order.total == 99.95, "CreateOrder", "expected total 99.95, got #{first_order.total}")
assert.(first_order.notes == "first order", "CreateOrder", "expected notes 'first order'")
IO.puts("PASS: CreateOrder")

# Test: GetOrdersByUser
{:ok, orders} = Queries.get_orders_by_user(conn, user_id)
assert.(length(orders) == 1, "GetOrdersByUser", "expected 1 order, got #{length(orders)}")
IO.puts("PASS: GetOrdersByUser")

# Test: DeleteUser (delete orders first due to FK)
{:ok, deleted_orders} = Queries.delete_orders_by_user(conn, user_id)
assert.(deleted_orders >= 1, "DeleteUser", "expected at least 1 deleted order, got #{deleted_orders}")
:ok = Queries.delete_user(conn, user_id)
result = Queries.get_user_by_id(conn, user_id)
assert.(result == {:error, :not_found}, "DeleteUser", "user should not exist after deletion")
IO.puts("PASS: DeleteUser")

Exqlite.Sqlite3.close(conn)

final_exit_code = Process.get(:exit_code, 0)

if final_exit_code == 0 do
  IO.puts("ALL TESTS PASSED")
end

System.halt(final_exit_code)
