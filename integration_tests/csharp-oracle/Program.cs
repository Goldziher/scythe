using Oracle.ManagedDataAccess.Client;

static string GetConnectionString()
{
    var oracleUrl = Environment.GetEnvironmentVariable("ORACLE_URL");
    if (oracleUrl != null && oracleUrl.StartsWith("oracle://"))
    {
        var uri = new Uri(oracleUrl);
        var userInfo = uri.UserInfo.Split(':');
        var user = userInfo[0];
        var password = userInfo.Length > 1 ? userInfo[1] : "";
        var port = uri.Port > 0 ? uri.Port : 1521;
        var service = uri.AbsolutePath.TrimStart('/');
        return $"User Id={user};Password={password};Data Source={uri.Host}:{port}/{service}";
    }
    return oracleUrl ?? "User Id=scythe;Password=scythe;Data Source=localhost:1521/XEPDB1";
}

await using var conn = new OracleConnection(GetConnectionString());
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

// Drop tables and sequences, ignoring errors if they do not exist
foreach (var drop in new[] {
    "DROP TABLE user_tags",
    "DROP TABLE tags",
    "DROP TABLE orders",
    "DROP TABLE users",
    "DROP SEQUENCE tags_seq",
    "DROP SEQUENCE orders_seq",
    "DROP SEQUENCE users_seq"
})
{
    try
    {
        await using var cmd = new OracleCommand(drop, conn);
        await cmd.ExecuteNonQueryAsync();
    }
    catch (OracleException)
    {
        // object may not exist
    }
}

// Read and execute schema_full.sql, splitting on Oracle PL/SQL delimiter /\n
var schemaPath = Path.Combine(Directory.GetCurrentDirectory(), "../sql/oracle", "schema_full.sql");
var schemaText = await File.ReadAllTextAsync(schemaPath);
foreach (var block in schemaText.Split("/\n"))
{
    var trimmed = block.Trim();
    if (!string.IsNullOrEmpty(trimmed))
    {
        await using var cmd = new OracleCommand(trimmed, conn);
        await cmd.ExecuteNonQueryAsync();
    }
}

// Test: CreateUser
var user = await Queries.CreateUser(conn, "Alice", "alice@example.com", 1);
Assert(user != null, "CreateUser", "returned null");
Assert(user!.Name == "Alice", "CreateUser", $"expected name Alice, got {user.Name}");
Assert(user.Email == "alice@example.com", "CreateUser", $"expected email alice@example.com, got {user.Email}");
Assert(user.Id > 0, "CreateUser", $"expected positive id, got {user.Id}");
Console.WriteLine("PASS: CreateUser");

var userId = user!.Id;

// Test: GetUserById
var fetched = await Queries.GetUserById(conn, userId);
Assert(fetched != null, "GetUserById", "returned null");
Assert(fetched!.Id == userId, "GetUserById", $"expected id {userId}, got {fetched.Id}");
Assert(fetched.Name == "Alice", "GetUserById", $"expected name Alice, got {fetched.Name}");
Assert(fetched.Email == "alice@example.com", "GetUserById", $"expected email alice@example.com, got {fetched.Email}");
Console.WriteLine("PASS: GetUserById");

// Test: ListActiveUsers
var activeUsers = await Queries.ListActiveUsers(conn);
Assert(activeUsers.Count >= 1, "ListActiveUsers", $"expected at least 1 user, got {activeUsers.Count}");
Assert(activeUsers.Any(u => u.Name == "Alice"), "ListActiveUsers", "expected Alice in active users");
Console.WriteLine("PASS: ListActiveUsers");

// Test: CreateOrder
var order = await Queries.CreateOrder(conn, userId, 9999L, "Test order");
Assert(order != null, "CreateOrder", "returned null");
Assert(order!.UserId == userId, "CreateOrder", $"expected user_id {userId}, got {order.UserId}");
Assert(order.Total == 9999L, "CreateOrder", $"expected total 9999, got {order.Total}");
Console.WriteLine("PASS: CreateOrder");

// Test: GetOrdersByUser
var orders = await Queries.GetOrdersByUser(conn, userId);
Assert(orders.Count == 1, "GetOrdersByUser", $"expected 1 order, got {orders.Count}");
Console.WriteLine("PASS: GetOrdersByUser");

// Test: GetOrderTotal
var orderTotal = await Queries.GetOrderTotal(conn, userId);
Assert(orderTotal != null, "GetOrderTotal", "returned null");
Console.WriteLine("PASS: GetOrderTotal");

// Test: DeleteOrdersByUser
var deletedOrders = await Queries.DeleteOrdersByUser(conn, userId);
Assert(deletedOrders == 1, "DeleteOrdersByUser", $"expected 1 deleted, got {deletedOrders}");
Console.WriteLine("PASS: DeleteOrdersByUser");

// Test: DeleteUser
await Queries.DeleteUser(conn, userId);
var deleted = await Queries.GetUserById(conn, userId);
Assert(deleted == null, "DeleteUser", "user should not exist after deletion");
Console.WriteLine("PASS: DeleteUser");

if (exitCode == 0)
{
    Console.WriteLine("ALL TESTS PASSED");
}

Environment.Exit(exitCode);
