alias Scythe.Queries


database_path = System.get_env("SQLITE_PATH", ":memory:")

{:ok, conn} = Exqlite.Sqlite3.open(database_path)

# Clean slate
:ok = Exqlite.Sqlite3.execute(conn, "DROP TABLE IF EXISTS user_tags")
:ok = Exqlite.Sqlite3.execute(conn, "DROP TABLE IF EXISTS tags")
:ok = Exqlite.Sqlite3.execute(conn, "DROP TABLE IF EXISTS orders")
:ok = Exqlite.Sqlite3.execute(conn, "DROP TABLE IF EXISTS users")

schema_sql = File.read!(Path.join([__DIR__, "..", "..", "sql", "sqlite", "schema.sql"]))

schema_sql
|> String.split(";")
|> Enum.map(&String.trim/1)
|> Enum.filter(&(&1 != ""))
|> Enum.each(fn stmt -> :ok = Exqlite.Sqlite3.execute(conn, stmt) end)

exit_code = 0

assert = fn condition, test_name, detail ->
  unless condition do
    IO.puts(:stderr, "FAIL: #{test_name}: #{detail}")
    Process.put(:exit_code, 1)
  end
end

Process.put(:exit_code, 0)

# Test: CreateUser
:ok = Queries.create_user(conn, "Alice", "alice@example.com", "active")
{:ok, user_id} = Exqlite.Sqlite3.last_insert_rowid(conn)
{:ok, user} = Queries.get_user_by_id(conn, user_id)
assert.(user.name == "Alice", "CreateUser", "expected name Alice, got #{user.name}")
assert.(user.email == "alice@example.com", "CreateUser", "expected email alice@example.com")
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
{:ok, orders_after_insert} = Queries.get_orders_by_user(conn, user_id)
order = List.first(orders_after_insert)
assert.(abs(order.total - 99.95) < 0.001, "CreateOrder", "expected total 99.95, got #{order.total}")
assert.(order.notes == "first order", "CreateOrder", "expected notes 'first order'")
IO.puts("PASS: CreateOrder")

# Test: GetOrdersByUser
{:ok, orders} = Queries.get_orders_by_user(conn, user_id)
assert.(length(orders) == 1, "GetOrdersByUser", "expected 1 order, got #{length(orders)}")
first_order = List.first(orders)
assert.(abs(first_order.total - 99.95) < 0.001, "GetOrdersByUser", "expected total 99.95")
IO.puts("PASS: GetOrdersByUser")

# Test: DeleteUser (delete orders first due to FK)
{:ok, deleted_orders} = Queries.delete_orders_by_user(conn, user_id)
assert.(deleted_orders == 1, "DeleteUser", "expected 1 deleted order, got #{deleted_orders}")
:ok = Queries.delete_user(conn, user_id)
result = Queries.get_user_by_id(conn, user_id)
assert.(result == {:error, :not_found}, "DeleteUser", "user should not exist after deletion")
IO.puts("PASS: DeleteUser")

final_exit_code = Process.get(:exit_code, 0)

if final_exit_code == 0 do
  IO.puts("ALL TESTS PASSED")
end

System.halt(final_exit_code)
