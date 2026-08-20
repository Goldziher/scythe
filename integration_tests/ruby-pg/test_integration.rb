# frozen_string_literal: true

require "uri"

require "pg"
require_relative "generated/queries"

SCHEMA_PATH = File.join(__dir__, "..", "sql", "pg", "schema.sql")

def get_database_url
  url = ENV["DATABASE_URL"]
  if url.nil? || url.empty?
    warn "ERROR: DATABASE_URL environment variable is not set"
    exit 1
  end
  url
end

# Splits a SQL script into statements on top-level ';' only -- unlike a
# naive `sql.split(";")`, this tracks single- and double-quoted spans,
# PostgreSQL dollar-quoted bodies, and '--' line comments (an apostrophe in
# a comment must not open a phantom string -- board #224 follow-up) so a
# ';' inside a string literal, a `$$ ... $$` function body, or a comment
# does not split the statement in half. '/* ... */' block comments are not
# handled -- no schema under integration_tests/sql/ uses them today.
def split_sql_statements(sql)
  statements = []
  current = +""
  in_single = false
  in_double = false
  in_line_comment = false
  dollar_tag = nil
  i = 0
  length = sql.length
  while i < length
    ch = sql[i]
    if in_line_comment
      current << ch
      in_line_comment = false if ch == "\n"
      i += 1
      next
    end
    if dollar_tag
      current << ch
      if ch == "$" && sql[i, dollar_tag.length] == dollar_tag
        current << dollar_tag[1..]
        i += dollar_tag.length
        dollar_tag = nil
        next
      end
      i += 1
      next
    end
    if in_single
      current << ch
      in_single = false if ch == "'"
      i += 1
      next
    end
    if in_double
      current << ch
      in_double = false if ch == '"'
      i += 1
      next
    end
    if ch == "-" && sql[i + 1] == "-"
      in_line_comment = true
      current << ch
      i += 1
      next
    end
    case ch
    when "'"
      in_single = true
      current << ch
    when '"'
      in_double = true
      current << ch
    when "$"
      if (match = /\$[A-Za-z0-9_]*\$/.match(sql[i..]))&.begin(0) == 0
        dollar_tag = match[0]
        current << dollar_tag
        i += dollar_tag.length
        next
      else
        current << ch
      end
    when ";"
      statements << current
      current = +""
    else
      current << ch
    end
    i += 1
  end
  statements << current unless current.strip.empty?
  statements.map(&:strip).reject(&:empty?)
end

def setup_schema(conn)
  conn.exec("DROP TABLE IF EXISTS user_tags CASCADE")
  conn.exec("DROP TABLE IF EXISTS tags CASCADE")
  conn.exec("DROP TABLE IF EXISTS orders CASCADE")
  conn.exec("DROP TABLE IF EXISTS users CASCADE")
  conn.exec("DROP TYPE IF EXISTS user_status CASCADE")
  conn.exec("DROP TYPE IF EXISTS user_address CASCADE")
  schema_sql = File.read(SCHEMA_PATH)
  conn.exec(schema_sql)
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

def assert_nil(value, message)
  return if value.nil?

  raise "Assertion failed: #{message} (expected nil, got #{value.inspect})"
end

def test_create_user(conn)
  user = Queries.create_user(conn, "Alice", "alice@example.com", "active")
  assert_not_nil(user, "create_user returned nil")
  assert_equal("Alice", user.name, "create_user name")
  assert_equal("alice@example.com", user.email, "create_user email")
  assert_equal("active", user.status, "create_user status")
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
  assert_equal("active", user.status, "get_user_by_id status")
  puts "PASS: GetUserById"
end

def test_list_active_users(conn)
  users = Queries.list_active_users(conn, "active")
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
  order = Queries.create_order(conn, user_id, "49.99", "Test order")
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
  assert_true(result.total_sum.to_f == 49.99, "get_order_total total_sum (got #{result.total_sum})")
  puts "PASS: GetOrderTotal"
end

def test_search_users(conn)
  results = Queries.search_users(conn, "%Ali%")
  assert_true(results.length >= 1, "Expected at least 1 search result, got #{results.length}")
  names = results.map(&:name)
  assert_true(names.include?("Alice"), "Expected 'Alice' in search results, got #{names}")
  puts "PASS: SearchUsers"
end
def test_count_users_by_status(conn)
  result = Queries.count_users_by_status(conn, "active")
  assert_not_nil(result, "count_users_by_status returned nil")
  assert_true(result.user_count >= 1, "Expected count >= 1, got #{result.user_count}")
  assert_equal("active", result.status, "count_users_by_status status")
  puts "PASS: CountUsersByStatus"
end

def test_get_user_profile(conn)
  # Test: GetUserProfile (board #197/#204) -- a nullable enum and a nullable
  # composite column, each observed both present and as SQL NULL, plus a
  # composite field containing a double quote and a comma to prove
  # UserAddress.from_text handles record_out's doubled-quote escaping
  # (board #204) rather than truncating on it.
  present = conn.exec_params(
    "INSERT INTO users (name, email, status, secondary_status, address) " \
    "VALUES ('Carol', 'carol@example.com', 'active', 'inactive', " \
    "ROW('1 Main St', 'Springfield', '12345')) RETURNING id"
  )
  present_id = present[0]["id"].to_i
  absent = conn.exec_params(
    "INSERT INTO users (name, email, status, secondary_status, address) " \
    "VALUES ('Dave', 'dave@example.com', 'active', NULL, NULL) RETURNING id"
  )
  absent_id = absent[0]["id"].to_i
  quoted = conn.exec_params(
    "INSERT INTO users (name, email, status, secondary_status, address) " \
    "VALUES ('Eve', 'eve@example.com', 'active', 'inactive', " \
    "ROW('12 \"Main\", Apt 3', 'Berlin', '10115')) RETURNING id"
  )
  quoted_id = quoted[0]["id"].to_i

  profile = Queries.get_user_profile(conn, present_id)
  assert_equal("inactive", profile.secondary_status, "get_user_profile secondary_status present")
  assert_not_nil(profile.address, "get_user_profile address should be present")
  assert_equal("1 Main St", profile.address.street, "get_user_profile address.street")
  assert_equal("Springfield", profile.address.city, "get_user_profile address.city")
  assert_equal("12345", profile.address.zip, "get_user_profile address.zip")

  null_profile = Queries.get_user_profile(conn, absent_id)
  assert_true(null_profile.secondary_status.nil?, "get_user_profile secondary_status null")
  assert_true(null_profile.address.nil?, "get_user_profile address null")

  quoted_profile = Queries.get_user_profile(conn, quoted_id)
  assert_not_nil(quoted_profile.address, "get_user_profile quoted address should be present")
  assert_equal('12 "Main", Apt 3', quoted_profile.address.street, "get_user_profile quoted address.street")
  assert_equal("Berlin", quoted_profile.address.city, "get_user_profile quoted address.city")
  assert_equal("10115", quoted_profile.address.zip, "get_user_profile quoted address.zip")

  Queries.delete_user(conn, present_id)
  Queries.delete_user(conn, absent_id)
  Queries.delete_user(conn, quoted_id)

  puts "PASS: GetUserProfile"
end

def test_round_trip_user_address(conn)
  address = Queries::UserAddress.new(street: '12 "Main", Apt \\3', city: "", zip: "10115")
  present = Queries.round_trip_user_address(conn, address)
  assert_equal(address, present.address, "round_trip_user_address escaped composite")
  absent = Queries.round_trip_user_address(conn, nil)
  assert_true(absent.address.nil?, "round_trip_user_address whole-composite NULL")
  puts "PASS: RoundTripUserAddress"
end

def test_delete_user(conn, user_id)
  # Delete orders first due to FK constraint
  deleted_count = Queries.delete_orders_by_user(conn, user_id)
  assert_equal(1, deleted_count, "delete_orders_by_user count")
  Queries.delete_user(conn, user_id)
  # GetUserById is `:one`, so a missing row raises Queries::RecordNotFound
  # rather than returning nil.
  begin
    Queries.get_user_by_id(conn, user_id)
    raise "Expected get_user_by_id to raise Queries::RecordNotFound, but it returned a row"
  rescue Queries::RecordNotFound
    # expected: the user was deleted
  end
  puts "PASS: DeleteUser"
end

begin
  database_url = get_database_url
  conn = PG.connect(database_url)

  setup_schema(conn)

  user_id = test_create_user(conn)
  test_get_user_by_id(conn, user_id)
  test_list_active_users(conn)
  test_update_user_email(conn, user_id)
  order_id = test_create_order(conn, user_id)
  test_get_orders_by_user(conn, user_id)
  test_get_order_total(conn, user_id)
  test_search_users(conn)
  test_count_users_by_status(conn)
  test_get_user_profile(conn)
  test_round_trip_user_address(conn)
  test_delete_user(conn, user_id)

  puts "\nALL TESTS PASSED"
rescue StandardError => e
  warn "FAIL: #{e.message}"
  warn e.backtrace.first(5).join("\n")
  exit 1
ensure
  conn&.close
end
