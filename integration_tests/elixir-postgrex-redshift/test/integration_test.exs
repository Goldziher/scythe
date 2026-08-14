alias Scythe.Queries

# Splits a SQL script into statements on top-level ';' only -- unlike a naive
# `String.split(sql, ";")`, this tracks single- and double-quoted spans,
# PostgreSQL bare `$$ ... $$` dollar-quoted bodies (the tagged `$tag$ ... $tag$`
# form is not detected -- this is a script, not a module, so the splitter has
# to be a single-pass Enum.reduce without lookahead, and none of the fixture
# schemas use a tagged delimiter), and '--' line comments (an apostrophe in a
# comment must not open a phantom string -- board #224 follow-up) so a ';'
# inside a string literal, a function body, or a comment does not split the
# statement in half. '/* ... */' block comments are not handled -- no schema
# under integration_tests/sql/ uses them today.
split_sql_statements = fn sql ->
  {statements, current, _mode, _prev} =
    sql
    |> String.graphemes()
    |> Enum.reduce({[], "", :normal, ""}, fn ch, {stmts, cur, mode, prev} ->
      new_cur = cur <> ch

      case mode do
        :line_comment ->
          {stmts, new_cur, (if ch == "\n", do: :normal, else: :line_comment), ch}

        :dollar ->
          if prev == "$" and ch == "$" do
            {stmts, new_cur, :normal, ch}
          else
            {stmts, new_cur, :dollar, ch}
          end

        :single ->
          {stmts, new_cur, (if ch == "'", do: :normal, else: :single), ch}

        :double ->
          {stmts, new_cur, (if ch == "\"", do: :normal, else: :double), ch}

        :normal ->
          cond do
            ch == "'" -> {stmts, new_cur, :single, ch}
            ch == "\"" -> {stmts, new_cur, :double, ch}
            ch == "$" and prev == "$" -> {stmts, new_cur, :dollar, ch}
            ch == "-" and prev == "-" -> {stmts, new_cur, :line_comment, ch}
            ch == ";" -> {[cur | stmts], "", :normal, ch}
            true -> {stmts, new_cur, :normal, ch}
          end
      end
    end)

  [current | statements]
  |> Enum.reverse()
  |> Enum.map(&String.trim/1)
  |> Enum.filter(&(&1 != ""))
end

database_url =
  System.get_env("REDSHIFT_URL", "postgres://scythe:scythe@localhost:5432/scythe_test")

uri = URI.parse(database_url)
[username, password] = String.split(uri.userinfo, ":")
database = String.trim_leading(uri.path, "/")

{:ok, conn} =
  Postgrex.start_link(
    hostname: uri.host,
    port: uri.port || 5439,
    username: username,
    password: password,
    database: database
  )

# Clean slate
Postgrex.query!(conn, "DROP TABLE IF EXISTS user_tags CASCADE", [])
Postgrex.query!(conn, "DROP TABLE IF EXISTS tags CASCADE", [])
Postgrex.query!(conn, "DROP TABLE IF EXISTS orders CASCADE", [])
Postgrex.query!(conn, "DROP TABLE IF EXISTS users CASCADE", [])

schema_sql = File.read!(Path.join([__DIR__, "..", "..", "sql", "redshift/schema_pg_compat.sql"]))
schema_sql
|> split_sql_statements.()
|> Enum.each(fn stmt -> Postgrex.query!(conn, stmt, []) end)

exit_code = 0

assert = fn condition, test_name, detail ->
  unless condition do
    IO.puts(:stderr, "FAIL: #{test_name}: #{detail}")
    Process.put(:exit_code, 1)
    Process.put(:failed_tests, MapSet.put(Process.get(:failed_tests, MapSet.new()), test_name))
  end
end

pass = fn test_name, label ->
  unless MapSet.member?(Process.get(:failed_tests, MapSet.new()), test_name) do
    IO.puts("PASS: #{label}")
  end
end

Process.put(:exit_code, 0)
Process.put(:failed_tests, MapSet.new())

# Test: CreateUser
{:ok, user} = Queries.create_user(conn, "Alice", "alice@example.com", "active")
assert.(user.name == "Alice", "CreateUser", "expected name Alice, got #{user.name}")
assert.(user.email == "alice@example.com", "CreateUser", "expected email alice@example.com")
assert.(user.status == "active", "CreateUser", "expected status active, got #{user.status}")
user_id = user.id
pass.("CreateUser", "CreateUser")

# Test: GetUserById
{:ok, fetched} = Queries.get_user_by_id(conn, user_id)
assert.(fetched.id == user_id, "GetUserById", "expected id #{user_id}")
assert.(fetched.name == "Alice", "GetUserById", "expected name Alice")
assert.(fetched.email == "alice@example.com", "GetUserById", "expected email alice@example.com")
pass.("GetUserById", "GetUserById")

# Test: ListActiveUsers
{:ok, active_users} = Queries.list_active_users(conn, "active")
assert.(length(active_users) > 0, "ListActiveUsers", "should have at least one user")
first = List.first(active_users)
assert.(first.name == "Alice", "ListActiveUsers", "first user should be Alice")
pass.("ListActiveUsers", "ListActiveUsers")

# Test: CreateOrder
{:ok, order} = Queries.create_order(conn, user_id, Decimal.new("99.95"), "first order")
assert.(order.user_id == user_id, "CreateOrder", "expected user_id #{user_id}")
assert.(Decimal.equal?(order.total, Decimal.new("99.95")), "CreateOrder", "expected total 99.95, got #{order.total}")
assert.(order.notes == "first order", "CreateOrder", "expected notes 'first order'")
pass.("CreateOrder", "CreateOrder")

# Test: GetOrdersByUser
{:ok, orders} = Queries.get_orders_by_user(conn, user_id)
assert.(length(orders) == 1, "GetOrdersByUser", "expected 1 order, got #{length(orders)}")
first_order = List.first(orders)
assert.(Decimal.equal?(first_order.total, Decimal.new("99.95")), "GetOrdersByUser", "expected total 99.95")
pass.("GetOrdersByUser", "GetOrdersByUser")

# Test: DeleteUser (delete orders first due to FK)
{:ok, deleted_orders} = Queries.delete_orders_by_user(conn, user_id)
assert.(deleted_orders == 1, "DeleteUser", "expected 1 deleted order, got #{deleted_orders}")
:ok = Queries.delete_user(conn, user_id)
result = Queries.get_user_by_id(conn, user_id)
assert.(result == {:error, :not_found}, "DeleteUser", "user should not exist after deletion")
pass.("DeleteUser", "DeleteUser")

final_exit_code = Process.get(:exit_code, 0)

if final_exit_code == 0 do
  IO.puts("ALL TESTS PASSED")
end

System.halt(final_exit_code)
