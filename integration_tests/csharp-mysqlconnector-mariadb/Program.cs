using MySqlConnector;

static string GetConnectionString()
{
    var databaseUrl = Environment.GetEnvironmentVariable("MARIADB_URL");
    if (databaseUrl != null && databaseUrl.StartsWith("mysql://"))
    {
        var uri = new Uri(databaseUrl);
        var userInfo = uri.UserInfo.Split(':');
        var user = userInfo[0];
        var password = userInfo.Length > 1 ? userInfo[1] : "";
        var database = uri.AbsolutePath.TrimStart('/');
        var port = uri.Port > 0 ? uri.Port : 3306;
        return $"Server={uri.Host};Port={port};Database={database};User={user};Password={password}";
    }
    return databaseUrl ?? "Server=localhost;Port=3306;Database=scythe_test;User=scythe;Password=scythe";
}

await using var conn = new MySqlConnection(GetConnectionString());
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
await using (var cmd = new MySqlCommand(@"
    DROP TABLE IF EXISTS user_tags;
    DROP TABLE IF EXISTS tags;
    DROP TABLE IF EXISTS orders;
    DROP TABLE IF EXISTS users;
    CREATE TABLE users (
        id UUID DEFAULT UUID() PRIMARY KEY,
        name VARCHAR(255) NOT NULL,
        email VARCHAR(255),
        home_ip INET4,
        status ENUM('active', 'inactive', 'banned') NOT NULL DEFAULT 'active',
        created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
    );
    CREATE TABLE orders (
        id INT AUTO_INCREMENT PRIMARY KEY,
        user_id UUID NOT NULL,
        total DECIMAL(10, 2) NOT NULL,
        notes TEXT,
        created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
        FOREIGN KEY (user_id) REFERENCES users (id)
    );
    CREATE TABLE tags (
        id INT AUTO_INCREMENT PRIMARY KEY,
        name VARCHAR(255) NOT NULL UNIQUE
    );
    CREATE TABLE user_tags (
        user_id UUID NOT NULL,
        tag_id INT NOT NULL,
        PRIMARY KEY (user_id, tag_id),
        FOREIGN KEY (user_id) REFERENCES users (id),
        FOREIGN KEY (tag_id) REFERENCES tags (id)
    );", conn))
{
    await cmd.ExecuteNonQueryAsync();
}

// Test: CreateUser
var user = await Queries.CreateUser(conn, "Alice", "alice@example.com", Queries.UsersStatus.Active);
Assert(user != null, "CreateUser", "returned null");
Assert(user!.Name == "Alice", "CreateUser", $"expected name Alice, got {user.Name}");
Assert(user.Email == "alice@example.com", "CreateUser", $"expected email alice@example.com, got {user.Email}");
Assert(!string.IsNullOrEmpty(user!.Id), "CreateUser", "expected non-empty id");
Console.WriteLine("PASS: CreateUser");

var userId = user.Id;

// Test: GetUserById
var fetched = await Queries.GetUserById(conn, userId);
Assert(fetched != null, "GetUserById", "returned null");
Assert(fetched!.Id == userId, "GetUserById", $"expected id {userId}, got {fetched.Id}");
Assert(fetched.Name == "Alice", "GetUserById", $"expected name Alice, got {fetched.Name}");
Console.WriteLine("PASS: GetUserById");

// Test: ListActiveUsers
var activeUsers = await Queries.ListActiveUsers(conn, Queries.UsersStatus.Active);
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
var order = await Queries.CreateOrder(conn, userId, 99.95m, "first order");
Assert(order != null, "CreateOrder", "returned null");
Assert(order!.UserId == userId, "CreateOrder", $"expected user_id {userId}, got {order.UserId}");
Assert(order.Total == 99.95m, "CreateOrder", $"expected total 99.95, got {order.Total}");
Assert(order.Notes == "first order", "CreateOrder", $"expected notes 'first order', got {order.Notes}");
Console.WriteLine("PASS: CreateOrder");

// Test: GetOrdersByUser
var orders = await Queries.GetOrdersByUser(conn, userId);
Assert(orders.Count == 1, "GetOrdersByUser", $"expected 1 order, got {orders.Count}");
Console.WriteLine("PASS: GetOrdersByUser");

// Test: SearchUsers
var searchResults = await Queries.SearchUsers(conn, "%Ali%");
Assert(searchResults.Count >= 1, "SearchUsers", $"expected at least 1, got {searchResults.Count}");
Console.WriteLine("PASS: SearchUsers");

// Test: DeleteUser (delete orders first due to FK)
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

Environment.Exit(exitCode);
