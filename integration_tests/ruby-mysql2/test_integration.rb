# frozen_string_literal: true

require "mysql2"
require_relative "generated/queries"

SCHEMA_PATH = File.join(__dir__, "..", "sql", "mysql", "schema.sql")

def get_database_url
  url = ENV["DATABASE_URL"]
  if url.nil? || url.empty?
    warn "ERROR: DATABASE_URL environment variable is not set"
    exit 1
  end
  url
end

def parse_mysql_url(url)
  uri = URI.parse(url)
  {
    host: uri.host,
    port: uri.port || 3306,
    username: uri.user,
    password: uri.password,
    database: uri.path&.sub(%r{^/}, "")
  }
end

def setup_schema(client)
  client.query("DROP TABLE IF EXISTS user_tags")
  client.query("DROP TABLE IF EXISTS tags")
  client.query("DROP TABLE IF EXISTS orders")
  client.query("DROP TABLE IF EXISTS users")
  schema_sql = File.read(SCHEMA_PATH)
  schema_sql.split(";").each do |stmt|
    stmt = stmt.strip
    client.query(stmt) unless stmt.empty?
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

def test_create_user(client)
  Queries.create_user(client, "Alice", "alice@example.com", "active")
  user = Queries.get_last_insert_user(client)
  assert_not_nil(user, "get_last_insert_user returned nil")
  assert_equal("Alice", user.name, "create_user name")
  assert_equal("alice@example.com", user.email, "create_user email")
  assert_equal("active", user.status, "create_user status")
  assert_true(user.id.positive?, "create_user id should be positive")
  puts "PASS: CreateUser"
  user.id
end

def test_get_user_by_id(client, user_id)
  user = Queries.get_user_by_id(client, user_id)
  assert_not_nil(user, "get_user_by_id returned nil for id=#{user_id}")
  assert_equal("Alice", user.name, "get_user_by_id name")
  assert_equal(user_id, user.id, "get_user_by_id id")
  assert_equal("alice@example.com", user.email, "get_user_by_id email")
  assert_equal("active", user.status, "get_user_by_id status")
  puts "PASS: GetUserById"
end

def test_list_active_users(client)
  users = Queries.list_active_users(client, "active")
  assert_true(users.length >= 1, "Expected at least 1 active user, got #{users.length}")
  names = users.map(&:name)
  assert_true(names.include?("Alice"), "Expected 'Alice' in active users, got #{names}")
  puts "PASS: ListActiveUsers"
end

def test_update_user_email(client, user_id)
  Queries.update_user_email(client, "alice-new@example.com", user_id)
  user = Queries.get_user_by_id(client, user_id)
  assert_not_nil(user, "user not found after update")
  assert_equal("alice-new@example.com", user.email, "update_user_email email")
  puts "PASS: UpdateUserEmail"
end

def test_create_order(client, user_id)
  Queries.create_order(client, user_id, "49.99", "Test order")
  order = Queries.get_last_insert_order(client)
  assert_not_nil(order, "get_last_insert_order returned nil")
  assert_equal(user_id, order.user_id, "create_order user_id")
  assert_equal("Test order", order.notes, "create_order notes")
  puts "PASS: CreateOrder"
  order.id
end

def test_get_orders_by_user(client, user_id)
  orders = Queries.get_orders_by_user(client, user_id)
  assert_true(orders.length >= 1, "Expected at least 1 order, got #{orders.length}")
  assert_equal("Test order", orders[0].notes, "get_orders_by_user notes")
  puts "PASS: GetOrdersByUser"
end

def test_get_order_total(client, user_id)
  result = Queries.get_order_total(client, user_id)
  assert_not_nil(result, "get_order_total returned nil")
  puts "PASS: GetOrderTotal"
end

def test_search_users(client)
  results = Queries.search_users(client, "%Ali%")
  assert_true(results.length >= 1, "Expected at least 1 search result, got #{results.length}")
  names = results.map(&:name)
  assert_true(names.include?("Alice"), "Expected 'Alice' in search results, got #{names}")
  puts "PASS: SearchUsers"
end

def test_delete_user(client, user_id)
  deleted_count = Queries.delete_orders_by_user(client, user_id)
  assert_equal(1, deleted_count, "delete_orders_by_user count")
  Queries.delete_user(client, user_id)
  user = Queries.get_user_by_id(client, user_id)
  assert_true(user.nil?, "Expected user to be deleted, but it still exists")
  puts "PASS: DeleteUser"
end

require "uri"

begin
  database_url = get_database_url
  opts = parse_mysql_url(database_url)
  client = Mysql2::Client.new(
    host: opts[:host],
    port: opts[:port],
    username: opts[:username],
    password: opts[:password],
    database: opts[:database]
  )

  setup_schema(client)

  user_id = test_create_user(client)
  test_get_user_by_id(client, user_id)
  test_list_active_users(client)
  test_update_user_email(client, user_id)
  order_id = test_create_order(client, user_id)
  test_get_orders_by_user(client, user_id)
  test_get_order_total(client, user_id)
  test_search_users(client)
  test_delete_user(client, user_id)

  puts "\nALL TESTS PASSED"
rescue StandardError => e
  warn "FAIL: #{e.message}"
  warn e.backtrace.first(5).join("\n")
  exit 1
ensure
  client&.close
end
