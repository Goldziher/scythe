using Npgsql;

var databaseUrl = Environment.GetEnvironmentVariable("REDSHIFT_URL")
    ?? "postgres://scythe:scythe@localhost:5433/scythe_test";

// Parse postgres:// URL into Npgsql connection string
var uri = new Uri(databaseUrl);
var userInfo = uri.UserInfo.Split(':');
var connString = $"Host={uri.Host};Port={uri.Port};Database={uri.AbsolutePath.TrimStart('/')};Username={userInfo[0]};Password={userInfo[1]}";

await using var conn = new NpgsqlConnection(connString);
await conn.OpenAsync();

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
await using (var cmd = new NpgsqlCommand(@"
    DROP TABLE IF EXISTS user_tags CASCADE;
    DROP TABLE IF EXISTS tags CASCADE;
    DROP TABLE IF EXISTS orders CASCADE;
    DROP TABLE IF EXISTS users CASCADE;
    CREATE TABLE users (
        id SERIAL PRIMARY KEY,
        name TEXT NOT NULL,
        email TEXT,
        status VARCHAR(50) NOT NULL DEFAULT 'active',
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE orders (
        id SERIAL PRIMARY KEY,
        user_id INT NOT NULL REFERENCES users (id),
        total NUMERIC(10, 2) NOT NULL,
        notes TEXT,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    );
    CREATE TABLE tags (
        id SERIAL PRIMARY KEY,
        name TEXT NOT NULL UNIQUE
    );
    CREATE TABLE user_tags (
        user_id INT NOT NULL REFERENCES users (id),
        tag_id INT NOT NULL REFERENCES tags (id),
        PRIMARY KEY (user_id, tag_id)
    );", conn))
{
    await cmd.ExecuteNonQueryAsync();
}

// Test: CreateUser
var user = await Queries.CreateUser(conn, "Alice", "alice@example.com", "active");
Assert(user != null, "CreateUser", "returned null");
Assert(user!.Name == "Alice", "CreateUser", $"expected name Alice, got {user.Name}");
Assert(user.Email == "alice@example.com", "CreateUser", $"expected email alice@example.com, got {user.Email}");
Assert(user.Id > 0, "CreateUser", $"expected positive id, got {user.Id}");
Console.WriteLine("PASS: CreateUser");

var userId = user.Id;

// Test: GetUserById
var fetched = await Queries.GetUserById(conn, userId);
Assert(fetched != null, "GetUserById", "returned null");
Assert(fetched!.Id == userId, "GetUserById", $"expected id {userId}, got {fetched.Id}");
Assert(fetched.Name == "Alice", "GetUserById", $"expected name Alice, got {fetched.Name}");
Assert(fetched.Email == "alice@example.com", "GetUserById", $"expected email alice@example.com, got {fetched.Email}");
Console.WriteLine("PASS: GetUserById");

// Test: ListActiveUsers
var activeUsers = await Queries.ListActiveUsers(conn, "active");
Assert(activeUsers.Count >= 1, "ListActiveUsers", $"expected at least 1 user, got {activeUsers.Count}");
Assert(activeUsers.Any(u => u.Name == "Alice"), "ListActiveUsers", "expected Alice in active users");
Console.WriteLine("PASS: ListActiveUsers");

// Test: UpdateUserEmail
await Queries.UpdateUserEmail(conn, "alice-new@example.com", userId);
var updated = await Queries.GetUserById(conn, userId);
Assert(updated != null, "UpdateUserEmail", "user not found after update");
Assert(updated!.Email == "alice-new@example.com", "UpdateUserEmail", $"expected updated email, got {updated.Email}");
Console.WriteLine("PASS: UpdateUserEmail");

// Test: CreateOrder
var order = await Queries.CreateOrder(conn, userId, 99.95m, "first order");
Assert(order != null, "CreateOrder", "returned null");
Assert(order!.UserId == userId, "CreateOrder", $"expected user_id {userId}, got {order.UserId}");
Assert(order.Total == 99.95m, "CreateOrder", $"expected total 99.95, got {order.Total}");
Assert(order.Notes == "first order", "CreateOrder", $"expected notes 'first order', got {order.Notes}");
Console.WriteLine("PASS: CreateOrder");

// Test: GetOrdersByUser
var orders = await Queries.GetOrdersByUser(conn, userId);
Assert(orders.Count == 1, "GetOrdersByUser", $"expected 1 order, got {orders.Count}");
Assert(orders[0].Total == 99.95m, "GetOrdersByUser", $"expected total 99.95, got {orders[0].Total}");
Assert(orders[0].Notes == "first order", "GetOrdersByUser", $"expected notes 'first order', got {orders[0].Notes}");
Console.WriteLine("PASS: GetOrdersByUser");

// Test: GetOrderTotal
var orderTotal = await Queries.GetOrderTotal(conn, userId);
Assert(orderTotal != null, "GetOrderTotal", "returned null");
Assert(orderTotal!.TotalSum == 99.95m, "GetOrderTotal", $"expected total 99.95, got {orderTotal.TotalSum}");
Console.WriteLine("PASS: GetOrderTotal");

// Test: DeleteUser (delete orders first due to FK)
var deletedOrders = await Queries.DeleteOrdersByUser(conn, userId);
Assert(deletedOrders == 1, "DeleteUser", $"expected 1 deleted order, got {deletedOrders}");
await Queries.DeleteUser(conn, userId);
var deleted = await Queries.GetUserById(conn, userId);
Assert(deleted == null, "DeleteUser", "user should not exist after deletion");
Console.WriteLine("PASS: DeleteUser");

if (exitCode == 0)
{
    Console.WriteLine("ALL TESTS PASSED");
}

Environment.Exit(exitCode);
