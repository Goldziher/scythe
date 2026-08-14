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
  System.get_env("ORACLE_URL", "oracle://system:oracle@localhost:1521/FREEPDB1")

uri = URI.parse(database_url)
userinfo = uri.userinfo || "system"
parts = String.split(userinfo, ":")
username = List.first(parts)
password = Enum.at(parts, 1) || ""
database = String.trim_leading(uri.path, "/")

{:ok, conn} =
  Jamdb.Oracle.start_link(
    hostname: uri.host,
    port: uri.port || 1521,
    database: database,
    username: username,
    password: password
  )

# Allow the DBConnection pool to finish establishing the connection before querying.
Process.sleep(500)

# Clean slate
# Helper to execute raw SQL via DBConnection
run_sql = fn conn, sql ->
  DBConnection.execute(conn, %Jamdb.Oracle.Query{statement: sql}, [])
end

# Clean slate: drop tables and sequences, ignoring errors if they do not exist.
for table <- ["attachments", "user_tags", "tags", "orders", "users"] do
  run_sql.(conn, "DROP TABLE #{table} CASCADE CONSTRAINTS")
end

for seq <- ["attachments_seq", "tags_seq", "orders_seq", "users_seq"] do
  run_sql.(conn, "DROP SEQUENCE #{seq}")
end

schema_sql = File.read!(Path.join([__DIR__, "..", "..", "sql", "oracle", "schema_full.sql"]))

schema_sql
|> String.split("/\n")
|> Enum.map(fn block ->
  block
  |> String.split("\n")
  |> Enum.reject(&String.starts_with?(String.trim(&1), "--"))
  |> Enum.join("\n")
  |> String.trim()
end)
|> Enum.filter(&(&1 != ""))
|> Enum.each(fn stmt ->
  case run_sql.(conn, stmt) do
    {:ok, _, _} -> :ok
    {:ok, _} -> :ok
    {:error, reason} -> raise "Schema statement failed: #{inspect(reason)}\nSQL: #{stmt}"
  end
end)

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
{:ok, user} = Queries.create_user(conn, "Alice", "alice@example.com", 1)
assert.(user.name == "Alice", "CreateUser", "expected name Alice, got #{user.name}")
assert.(user.email == "alice@example.com", "CreateUser", "expected email alice@example.com")
user_id = user.id
pass.("CreateUser", "CreateUser")

# Test: GetUserById
{:ok, fetched} = Queries.get_user_by_id(conn, user_id)
assert.(fetched.id == user_id, "GetUserById", "expected id #{user_id}")
assert.(fetched.name == "Alice", "GetUserById", "expected name Alice")
assert.(fetched.email == "alice@example.com", "GetUserById", "expected email alice@example.com")
pass.("GetUserById", "GetUserById")

# Test: ListActiveUsers
{:ok, active_users} = Queries.list_active_users(conn)
assert.(length(active_users) > 0, "ListActiveUsers", "should have at least one user")
first = List.first(active_users)
assert.(first.name == "Alice", "ListActiveUsers", "first user should be Alice")
pass.("ListActiveUsers", "ListActiveUsers")

# Test: CreateOrder
{:ok, order} = Queries.create_order(conn, user_id, Decimal.new("99.99"), "first order")
assert.(order.user_id == user_id, "CreateOrder", "expected user_id #{user_id}")
assert.(Decimal.equal?(order.total, Decimal.new("99.99")), "CreateOrder", "expected total 99.99, got #{order.total}")
assert.(order.notes == "first order", "CreateOrder", "expected notes 'first order'")
pass.("CreateOrder", "CreateOrder")

# Test: GetOrdersByUser
{:ok, orders} = Queries.get_orders_by_user(conn, user_id)
assert.(length(orders) == 1, "GetOrdersByUser", "expected 1 order, got #{length(orders)}")
first_order = List.first(orders)
assert.(Decimal.equal?(first_order.total, Decimal.new("99.99")), "GetOrdersByUser", "expected total 99.99")
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
