using Microsoft.Data.Sqlite;

var conn = new SqliteConnection("Data Source=:memory:");
conn.Open();

var exitCode = 0;

void Assert(bool condition, string testName, string detail)
{
    if (!condition)
    {
        Console.Error.WriteLine($"FAIL: {testName}: {detail}");
        exitCode = 1;
    }
}

// Clean slate
using (var cmd = new SqliteCommand(@"
    CREATE TABLE users (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        email TEXT,
        status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'inactive', 'banned')),
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE orders (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id INTEGER NOT NULL REFERENCES users (id),
        total REAL NOT NULL,
        notes TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE tags (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL UNIQUE
    );
    CREATE TABLE user_tags (
        user_id INTEGER NOT NULL REFERENCES users (id),
        tag_id INTEGER NOT NULL REFERENCES tags (id),
        PRIMARY KEY (user_id, tag_id)
    );", conn))
{
    cmd.ExecuteNonQuery();
}

// Test: CreateUser
await Queries.CreateUser(conn, "Alice", "alice@example.com", "active");

// Get the last inserted user via raw SQL (SQLite has no LAST_INSERT_ID)
long userId;
using (var cmd = new SqliteCommand("SELECT last_insert_rowid()", conn))
{
    userId = (long)cmd.ExecuteScalar()!;
}

var user = await Queries.GetUserById(conn, userId);
Assert(user != null, "CreateUser", "returned null");
Assert(user!.Name == "Alice", "CreateUser", $"expected name Alice, got {user.Name}");
Assert(user.Email == "alice@example.com", "CreateUser", $"expected email, got {user.Email}");
Assert(user.Id > 0, "CreateUser", $"expected positive id, got {user.Id}");
Console.WriteLine("PASS: CreateUser");

// Test: GetUserById
var fetched = await Queries.GetUserById(conn, userId);
Assert(fetched != null, "GetUserById", "returned null");
Assert(fetched!.Id == userId, "GetUserById", $"expected id {userId}, got {fetched.Id}");
Assert(fetched.Name == "Alice", "GetUserById", $"expected name Alice, got {fetched.Name}");
Console.WriteLine("PASS: GetUserById");

// Test: ListActiveUsers
var activeUsers = await Queries.ListActiveUsers(conn, "active");
Assert(activeUsers.Count >= 1, "ListActiveUsers", $"expected at least 1, got {activeUsers.Count}");
Assert(activeUsers.Any(u => u.Name == "Alice"), "ListActiveUsers", "expected Alice");
Console.WriteLine("PASS: ListActiveUsers");

// Test: UpdateUserEmail
await Queries.UpdateUserEmail(conn, "alice-new@example.com", userId);
var updated = await Queries.GetUserById(conn, userId);
Assert(updated != null, "UpdateUserEmail", "user not found after update");
Assert(updated!.Email == "alice-new@example.com", "UpdateUserEmail", $"expected updated email, got {updated.Email}");
Console.WriteLine("PASS: UpdateUserEmail");

// Test: CreateOrder
await Queries.CreateOrder(conn, userId, 99.95, "first order");

// Test: GetOrdersByUser
var orders = await Queries.GetOrdersByUser(conn, userId);
Assert(orders.Count == 1, "GetOrdersByUser", $"expected 1 order, got {orders.Count}");
Console.WriteLine("PASS: GetOrdersByUser");

// Test: GetOrderTotal
var orderTotal = await Queries.GetOrderTotal(conn, userId);
Assert(orderTotal != null, "GetOrderTotal", "returned null");
Console.WriteLine("PASS: GetOrderTotal");

// Test: SearchUsers
var searchResults = await Queries.SearchUsers(conn, "%Ali%");
Assert(searchResults.Count >= 1, "SearchUsers", $"expected at least 1, got {searchResults.Count}");
Console.WriteLine("PASS: SearchUsers");

// Test: DeleteOrdersByUser
var deletedOrders = await Queries.DeleteOrdersByUser(conn, userId);
Assert(deletedOrders == 1, "DeleteOrdersByUser", $"expected 1 deleted, got {deletedOrders}");
await Queries.DeleteUser(conn, userId);
var deleted = await Queries.GetUserById(conn, userId);
Assert(deleted == null, "DeleteUser", "user should not exist after deletion");
Console.WriteLine("PASS: DeleteUser");

if (exitCode == 0)
{
    Console.WriteLine("ALL TESTS PASSED");
}

conn.Close();
Environment.Exit(exitCode);
