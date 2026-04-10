# Load generated queries module
Code.require_file("generated/queries.ex")

alias Scythe.Queries

database_url =
  System.get_env("DATABASE_URL", "postgres://scythe:scythe@localhost:5432/scythe_test")

uri = URI.parse(database_url)
[username, password] = String.split(uri.userinfo, ":")
database = String.trim_leading(uri.path, "/")

defmodule ScytheTestRepo do
  use Ecto.Repo,
    otp_app: :scythe_ecto_integration_test,
    adapter: Ecto.Adapters.Postgres
end

Application.put_env(:scythe_ecto_integration_test, ScytheTestRepo,
  hostname: uri.host,
  port: uri.port,
  username: username,
  password: password,
  database: database,
  pool_size: 1
)

{:ok, _} = ScytheTestRepo.start_link()

repo = ScytheTestRepo

# Clean slate
Ecto.Adapters.SQL.query!(repo, "DROP TABLE IF EXISTS user_tags CASCADE", [])
Ecto.Adapters.SQL.query!(repo, "DROP TABLE IF EXISTS tags CASCADE", [])
Ecto.Adapters.SQL.query!(repo, "DROP TABLE IF EXISTS orders CASCADE", [])
Ecto.Adapters.SQL.query!(repo, "DROP TABLE IF EXISTS users CASCADE", [])
Ecto.Adapters.SQL.query!(repo, "DROP TYPE IF EXISTS user_status CASCADE", [])

Ecto.Adapters.SQL.query!(repo, "CREATE TYPE user_status AS ENUM ('active', 'inactive', 'banned')", [])

Ecto.Adapters.SQL.query!(
  repo,
  """
  CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT,
    status user_status NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
  )
  """,
  []
)

Ecto.Adapters.SQL.query!(
  repo,
  """
  CREATE TABLE orders (
    id SERIAL PRIMARY KEY,
    user_id INT NOT NULL REFERENCES users (id),
    total NUMERIC(10, 2) NOT NULL,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
  )
  """,
  []
)

Ecto.Adapters.SQL.query!(
  repo,
  """
  CREATE TABLE tags (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
  )
  """,
  []
)

Ecto.Adapters.SQL.query!(
  repo,
  """
  CREATE TABLE user_tags (
    user_id INT NOT NULL REFERENCES users (id),
    tag_id INT NOT NULL REFERENCES tags (id),
    PRIMARY KEY (user_id, tag_id)
  )
  """,
  []
)

assert = fn condition, test_name, detail ->
  unless condition do
    IO.puts(:stderr, "FAIL: #{test_name}: #{detail}")
    Process.put(:exit_code, 1)
  end
end

Process.put(:exit_code, 0)

# Test: CreateUser
{:ok, user} = Queries.create_user(repo, "Alice", "alice@example.com", "active")
assert.(user.name == "Alice", "CreateUser", "expected name Alice, got #{user.name}")
assert.(user.email == "alice@example.com", "CreateUser", "expected email alice@example.com")
assert.(user.status == "active", "CreateUser", "expected status active, got #{user.status}")
user_id = user.id
IO.puts("PASS: CreateUser")

# Test: GetUserById
{:ok, fetched} = Queries.get_user_by_id(repo, user_id)
assert.(fetched.id == user_id, "GetUserById", "expected id #{user_id}")
assert.(fetched.name == "Alice", "GetUserById", "expected name Alice")
assert.(fetched.email == "alice@example.com", "GetUserById", "expected email alice@example.com")
IO.puts("PASS: GetUserById")

# Test: ListActiveUsers
{:ok, active_users} = Queries.list_active_users(repo, "active")
assert.(length(active_users) > 0, "ListActiveUsers", "should have at least one user")
first = List.first(active_users)
assert.(first.name == "Alice", "ListActiveUsers", "first user should be Alice")
IO.puts("PASS: ListActiveUsers")

# Test: CreateOrder
{:ok, order} = Queries.create_order(repo, user_id, Decimal.new("99.95"), "first order")
assert.(order.user_id == user_id, "CreateOrder", "expected user_id #{user_id}")
assert.(Decimal.equal?(order.total, Decimal.new("99.95")), "CreateOrder", "expected total 99.95, got #{order.total}")
assert.(order.notes == "first order", "CreateOrder", "expected notes 'first order'")
IO.puts("PASS: CreateOrder")

# Test: GetOrdersByUser
{:ok, orders} = Queries.get_orders_by_user(repo, user_id)
assert.(length(orders) == 1, "GetOrdersByUser", "expected 1 order, got #{length(orders)}")
first_order = List.first(orders)
assert.(Decimal.equal?(first_order.total, Decimal.new("99.95")), "GetOrdersByUser", "expected total 99.95")
IO.puts("PASS: GetOrdersByUser")

# Test: DeleteUser (delete orders first due to FK)
{:ok, deleted_orders} = Queries.delete_orders_by_user(repo, user_id)
assert.(deleted_orders == 1, "DeleteUser", "expected 1 deleted order, got #{deleted_orders}")
:ok = Queries.delete_user(repo, user_id)
result = Queries.get_user_by_id(repo, user_id)
assert.(result == {:ok, nil}, "DeleteUser", "user should be nil after deletion")
IO.puts("PASS: DeleteUser")

final_exit_code = Process.get(:exit_code, 0)

if final_exit_code == 0 do
  IO.puts("ALL TESTS PASSED")
end

System.halt(final_exit_code)
