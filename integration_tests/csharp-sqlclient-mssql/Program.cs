using Microsoft.Data.SqlClient;

static string GetConnectionString()
{
    var databaseUrl = Environment.GetEnvironmentVariable("MSSQL_URL");
    if (databaseUrl != null && databaseUrl.StartsWith("sqlserver://"))
    {
        var uri = new Uri(databaseUrl);
        var userInfo = uri.UserInfo.Split(':');
        var user = userInfo[0];
        var password = userInfo.Length > 1 ? userInfo[1] : "";
        var database = uri.AbsolutePath.TrimStart('/');
        var port = uri.Port > 0 ? uri.Port : 1433;
        return $"Server={uri.Host},{port};Database={database};User Id={user};Password={password};TrustServerCertificate=true";
    }
    return databaseUrl ?? "Server=localhost,1433;Database=scythe_test;User Id=sa;Password=Scythe_Test1;TrustServerCertificate=true";
}

await using var conn = new SqlConnection(GetConnectionString());
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
await using (var cmd = new SqlCommand(@"
    IF OBJECT_ID('user_tags','U') IS NOT NULL DROP TABLE user_tags;
    IF OBJECT_ID('tags','U') IS NOT NULL DROP TABLE tags;
    IF OBJECT_ID('orders','U') IS NOT NULL DROP TABLE orders;
    IF OBJECT_ID('users','U') IS NOT NULL DROP TABLE users;", conn))
{
    await cmd.ExecuteNonQueryAsync();
}

// Read and execute schema_full.sql
var schemaPath = Path.Combine(Directory.GetCurrentDirectory(), "../sql/mssql", "schema_full.sql");
var schemaText = await File.ReadAllTextAsync(schemaPath);
foreach (var block in schemaText.Split("GO\n"))
{
    var trimmed = block.Trim();
    if (!string.IsNullOrEmpty(trimmed))
    {
        await using var cmd = new SqlCommand(trimmed, conn);
        await cmd.ExecuteNonQueryAsync();
    }
}

// Test: CreateUser
var user = await Queries.CreateUser(conn, "Alice", "alice@example.com", true);
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
var activeUsers = await Queries.ListActiveUsers(conn, true);
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

// Test: SearchUsers
var searchResults = await Queries.SearchUsers(conn, "%Ali%");
Assert(searchResults.Count >= 1, "SearchUsers", $"expected at least 1 result, got {searchResults.Count}");
Assert(searchResults.Any(u => u.Name == "Alice"), "SearchUsers", "expected Alice in search results");
Console.WriteLine("PASS: SearchUsers");

// Test: CountUsersByStatus
var countResult = await Queries.CountUsersByStatus(conn, true);
Assert(countResult != null, "CountUsersByStatus", "returned null");
Assert(countResult!.UserCount >= 1, "CountUsersByStatus", $"expected count >= 1, got {countResult.UserCount}");
Console.WriteLine("PASS: CountUsersByStatus");

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
