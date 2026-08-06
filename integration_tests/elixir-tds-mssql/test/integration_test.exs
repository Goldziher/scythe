alias Scythe.Queries


database_url =
  System.get_env("MSSQL_URL", "sqlserver://sa:Scythe_Test1@localhost:1433?database=scythe_test")

uri = URI.parse(database_url)
userinfo = uri.userinfo || "sa"
parts = String.split(userinfo, ":")
username = List.first(parts)
password = Enum.at(parts, 1) || ""
query_params = URI.decode_query(uri.query || "")
database = Map.get(query_params, "database", "master")

{:ok, conn} =
  Tds.start_link(
    hostname: uri.host,
    port: uri.port || 1433,
    username: username,
    password: password,
    database: database
  )

# Clean slate
Tds.query!(conn, "IF OBJECT_ID('user_tags','U') IS NOT NULL DROP TABLE user_tags", [])
Tds.query!(conn, "IF OBJECT_ID('tags','U') IS NOT NULL DROP TABLE tags", [])
Tds.query!(conn, "IF OBJECT_ID('orders','U') IS NOT NULL DROP TABLE orders", [])
Tds.query!(conn, "IF OBJECT_ID('users','U') IS NOT NULL DROP TABLE users", [])

schema_sql = File.read!(Path.join([__DIR__, "..", "..", "sql", "mssql", "schema.sql"]))

schema_sql
|> String.split("GO\n")
|> Enum.map(&String.trim/1)
|> Enum.filter(&(&1 != ""))
|> Enum.each(fn stmt -> Tds.query!(conn, stmt, []) end)

exit_code = 0

assert = fn condition, test_name, detail ->
  unless condition do
    IO.puts(:stderr, "FAIL: #{test_name}: #{detail}")
    Process.put(:exit_code, 1)
  end
end

Process.put(:exit_code, 0)

# Test: CreateUser
{:ok, user} = Queries.create_user(conn, 1, "Alice", "alice@example.com", true)
assert.(user.name == "Alice", "CreateUser", "expected name Alice, got #{user.name}")
assert.(user.email == "alice@example.com", "CreateUser", "expected email alice@example.com")
user_id = user.id
IO.puts("PASS: CreateUser")

# Test: GetUserById
{:ok, fetched} = Queries.get_user_by_id(conn, user_id)
assert.(fetched.id == user_id, "GetUserById", "expected id #{user_id}")
assert.(fetched.name == "Alice", "GetUserById", "expected name Alice")
assert.(fetched.email == "alice@example.com", "GetUserById", "expected email alice@example.com")
IO.puts("PASS: GetUserById")

# Test: ListActiveUsers
{:ok, active_users} = Queries.list_active_users(conn)
assert.(length(active_users) > 0, "ListActiveUsers", "should have at least one user")
first = List.first(active_users)
assert.(first.name == "Alice", "ListActiveUsers", "first user should be Alice")
IO.puts("PASS: ListActiveUsers")

# Test: CreateOrder
{:ok, order} = Queries.create_order(conn, 1, user_id, Decimal.new("99.95"), "first order")
assert.(order.user_id == user_id, "CreateOrder", "expected user_id #{user_id}")
assert.(Decimal.equal?(order.total, Decimal.new("99.95")), "CreateOrder", "expected total 99.95, got #{order.total}")
assert.(order.notes == "first order", "CreateOrder", "expected notes 'first order'")
IO.puts("PASS: CreateOrder")

# Test: GetOrdersByUser
{:ok, orders} = Queries.get_orders_by_user(conn, user_id)
assert.(length(orders) == 1, "GetOrdersByUser", "expected 1 order, got #{length(orders)}")
first_order = List.first(orders)
assert.(Decimal.equal?(first_order.total, Decimal.new("99.95")), "GetOrdersByUser", "expected total 99.95")
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
