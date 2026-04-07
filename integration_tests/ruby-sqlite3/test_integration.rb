# frozen_string_literal: true

require "sqlite3"
require_relative "generated/queries"

SCHEMA_PATH = File.join(__dir__, "..", "sql", "sqlite", "schema.sql")

def get_database_path
  path = ENV["DATABASE_PATH"]
  if path.nil? || path.empty?
    warn "ERROR: DATABASE_PATH environment variable is not set (use :memory: for in-memory)"
    exit 1
  end
  path
end

def setup_schema(db)
  schema_sql = File.read(SCHEMA_PATH)
  db.execute_batch(schema_sql)
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

def test_create_user(db)
  Queries.create_user(db, "Alice", "alice@example.com", "active")
  user_id = db.last_insert_row_id
  user = Queries.get_user_by_id(db, user_id)
  assert_not_nil(user, "get_user_by_id returned nil after create")
  assert_equal("Alice", user.name, "create_user name")
  assert_equal("alice@example.com", user.email, "create_user email")
  assert_equal("active", user.status, "create_user status")
  assert_true(user.id.positive?, "create_user id should be positive")
  puts "PASS: CreateUser"
  user.id
end

def test_get_user_by_id(db, user_id)
  user = Queries.get_user_by_id(db, user_id)
  assert_not_nil(user, "get_user_by_id returned nil for id=#{user_id}")
  assert_equal("Alice", user.name, "get_user_by_id name")
  assert_equal(user_id, user.id, "get_user_by_id id")
  assert_equal("alice@example.com", user.email, "get_user_by_id email")
  assert_equal("active", user.status, "get_user_by_id status")
  puts "PASS: GetUserById"
end

def test_list_active_users(db)
  users = Queries.list_active_users(db, "active")
  assert_true(users.length >= 1, "Expected at least 1 active user, got #{users.length}")
  names = users.map(&:name)
  assert_true(names.include?("Alice"), "Expected 'Alice' in active users, got #{names}")
  puts "PASS: ListActiveUsers"
end

def test_update_user_email(db, user_id)
  Queries.update_user_email(db, "alice-new@example.com", user_id)
  user = Queries.get_user_by_id(db, user_id)
  assert_not_nil(user, "user not found after update")
  assert_equal("alice-new@example.com", user.email, "update_user_email email")
  puts "PASS: UpdateUserEmail"
end

def test_create_order(db, user_id)
  Queries.create_order(db, user_id, "49.99", "Test order")
  order_id = db.last_insert_row_id
  orders = Queries.get_orders_by_user(db, user_id)
  assert_true(orders.length >= 1, "Expected at least 1 order")
  assert_equal("Test order", orders[0].notes, "create_order notes")
  puts "PASS: CreateOrder"
  order_id
end

def test_get_orders_by_user(db, user_id)
  orders = Queries.get_orders_by_user(db, user_id)
  assert_true(orders.length >= 1, "Expected at least 1 order, got #{orders.length}")
  assert_equal("Test order", orders[0].notes, "get_orders_by_user notes")
  puts "PASS: GetOrdersByUser"
end

def test_get_order_total(db, user_id)
  result = Queries.get_order_total(db, user_id)
  assert_not_nil(result, "get_order_total returned nil")
  puts "PASS: GetOrderTotal"
end

def test_search_users(db)
  results = Queries.search_users(db, "%Ali%")
  assert_true(results.length >= 1, "Expected at least 1 search result, got #{results.length}")
  names = results.map(&:name)
  assert_true(names.include?("Alice"), "Expected 'Alice' in search results, got #{names}")
  puts "PASS: SearchUsers"
end

def test_delete_user(db, user_id)
  deleted_count = Queries.delete_orders_by_user(db, user_id)
  assert_equal(1, deleted_count, "delete_orders_by_user count")
  Queries.delete_user(db, user_id)
  user = Queries.get_user_by_id(db, user_id)
  assert_true(user.nil?, "Expected user to be deleted, but it still exists")
  puts "PASS: DeleteUser"
end

begin
  database_path = get_database_path
  db = SQLite3::Database.new(database_path)

  setup_schema(db)

  user_id = test_create_user(db)
  test_get_user_by_id(db, user_id)
  test_list_active_users(db)
  test_update_user_email(db, user_id)
  order_id = test_create_order(db, user_id)
  test_get_orders_by_user(db, user_id)
  test_get_order_total(db, user_id)
  test_search_users(db)
  test_delete_user(db, user_id)

  puts "\nALL TESTS PASSED"
rescue StandardError => e
  warn "FAIL: #{e.message}"
  warn e.backtrace.first(5).join("\n")
  exit 1
ensure
  db&.close
end
