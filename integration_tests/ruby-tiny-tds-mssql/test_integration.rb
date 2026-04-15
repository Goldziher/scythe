# frozen_string_literal: true

require "uri"

require "tiny_tds"
require_relative "generated/queries"

SCHEMA_PATH = File.join(__dir__, "..", "sql", "mssql", "schema.sql")

def get_database_url
  url = ENV["MSSQL_URL"]
  if url.nil? || url.empty?
    warn "ERROR: MSSQL_URL environment variable is not set"
    exit 1
  end
  url
end

def setup_schema(conn)
  conn.execute("IF OBJECT_ID('user_tags','U') IS NOT NULL DROP TABLE user_tags").do
  conn.execute("IF OBJECT_ID('tags','U') IS NOT NULL DROP TABLE tags").do
  conn.execute("IF OBJECT_ID('orders','U') IS NOT NULL DROP TABLE orders").do
  conn.execute("IF OBJECT_ID('users','U') IS NOT NULL DROP TABLE users").do
  schema_sql = File.read(SCHEMA_PATH)
  schema_sql.split("GO\n").each do |stmt|
    stmt = stmt.strip
    conn.execute(stmt).do unless stmt.empty?
  end
end

def assert_equal(expected, actual, message)
  return if expected == actual

  raise "Assertion failed: #{message} (expected #{expected.inspect}, got #{actual.inspect})"
end

def assert_not_nil(value, message)
  return unless value.nil?

  raise "Assertion failed: #{message} (got nil)"
end

def assert_true(value, message)
  return if value

  raise "Assertion failed: #{message}"
end

def test_create_user(conn)
  user = Queries.create_user(conn, 1, "Alice", "alice@example.com", true)
  assert_not_nil(user, "create_user returned nil")
  assert_equal("Alice", user.name, "create_user name")
  assert_equal("alice@example.com", user.email, "create_user email")
  assert_true(user.id.positive?, "create_user id should be positive")
  puts "PASS: CreateUser"
  user.id
end

def test_get_user_by_id(conn, user_id)
  user = Queries.get_user_by_id(conn, user_id)
  assert_not_nil(user, "get_user_by_id returned nil for id=#{user_id}")
  assert_equal("Alice", user.name, "get_user_by_id name")
  assert_equal(user_id, user.id, "get_user_by_id id")
  assert_equal("alice@example.com", user.email, "get_user_by_id email")
  puts "PASS: GetUserById"
end

def test_list_active_users(conn)
  users = Queries.list_active_users(conn)
  assert_true(users.length >= 1, "Expected at least 1 active user, got #{users.length}")
  names = users.map(&:name)
  assert_true(names.include?("Alice"), "Expected 'Alice' in active users, got #{names}")
  puts "PASS: ListActiveUsers"
end

def test_update_user_email(conn, user_id)
  Queries.update_user_email(conn, "alice-new@example.com", user_id)
  user = Queries.get_user_by_id(conn, user_id)
  assert_not_nil(user, "user not found after update")
  assert_equal("alice-new@example.com", user.email, "update_user_email email")
  puts "PASS: UpdateUserEmail"
end

def test_create_order(conn, user_id)
  order = Queries.create_order(conn, 1, user_id, "49.99", "Test order")
  assert_not_nil(order, "create_order returned nil")
  assert_equal(user_id, order.user_id, "create_order user_id")
  assert_equal("Test order", order.notes, "create_order notes")
  puts "PASS: CreateOrder"
  order.id
end

def test_get_orders_by_user(conn, user_id)
  orders = Queries.get_orders_by_user(conn, user_id)
  assert_true(orders.length >= 1, "Expected at least 1 order, got #{orders.length}")
  assert_equal("Test order", orders[0].notes, "get_orders_by_user notes")
  puts "PASS: GetOrdersByUser"
end

def test_get_order_total(conn, user_id)
  result = Queries.get_order_total(conn, user_id)
  assert_not_nil(result, "get_order_total returned nil")
  assert_equal("49.99", result.total_sum.to_s("F"), "get_order_total total_sum")
  puts "PASS: GetOrderTotal"
end

def test_search_users(conn)
  results = Queries.search_users(conn, "%Ali%")
  assert_true(results.length >= 1, "Expected at least 1 search result, got #{results.length}")
  names = results.map(&:name)
  assert_true(names.include?("Alice"), "Expected 'Alice' in search results, got #{names}")
  puts "PASS: SearchUsers"
end

def test_delete_user(conn, user_id)
  # Delete orders first due to FK constraint
  deleted_count = Queries.delete_orders_by_user(conn, user_id)
  Queries.delete_user(conn, user_id)
  user = Queries.get_user_by_id(conn, user_id)
  assert_true(user.nil?, "Expected user to be deleted, but it still exists")
  puts "PASS: DeleteUser"
end

begin
  database_url = get_database_url
  uri = URI.parse(database_url)
  query_params = URI.decode_www_form(uri.query || "").to_h
  conn = TinyTds::Client.new(
    host: uri.host,
    port: uri.port || 1433,
    username: uri.user,
    password: uri.password,
    database: query_params["database"] || "master"
  )

  setup_schema(conn)

  user_id = test_create_user(conn)
  test_get_user_by_id(conn, user_id)
  test_list_active_users(conn)
  test_update_user_email(conn, user_id)
  order_id = test_create_order(conn, user_id)
  test_get_orders_by_user(conn, user_id)
  test_get_order_total(conn, user_id)
  test_search_users(conn)
  test_delete_user(conn, user_id)

  puts "\nALL TESTS PASSED"
rescue StandardError => e
  warn "FAIL: #{e.message}"
  warn e.backtrace.first(5).join("\n")
  exit 1
ensure
  conn&.close
end
