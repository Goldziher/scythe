using Snowflake.Data.Client;

static string GetConnectionString()
{
    var snowflakeUrl = Environment.GetEnvironmentVariable("SNOWFLAKE_URL")
        ?? "snowflake://scythe:scythe@localhost:443/scythe_test/public?account=test";

    // Parse snowflake://user:pass@host:port/database/schema?account=X
    var uri = new Uri(snowflakeUrl);
    var userInfo = uri.UserInfo.Split(':');
    var user = userInfo[0];
    var password = userInfo.Length > 1 ? userInfo[1] : "";

    var pathParts = uri.AbsolutePath.Split('/');
    var database = pathParts.Length > 1 ? pathParts[1] : "";
    var schema = pathParts.Length > 2 ? pathParts[2] : "";

    // Parse account from query params
    var account = "";
    if (!string.IsNullOrEmpty(uri.Query))
    {
        foreach (var param in uri.Query.TrimStart('?').Split('&'))
        {
            if (param.StartsWith("account="))
            {
                account = param.Substring("account=".Length);
                break;
            }
        }
    }

    return $"account={account};host={uri.Host};user={user};password={password};db={database};schema={schema}";
}

await using var conn = new SnowflakeDbConnection();
conn.ConnectionString = GetConnectionString();
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
await using (var cmd = new SnowflakeCommand(@"
    DROP TABLE IF EXISTS user_tags;
    DROP TABLE IF EXISTS tags;
    DROP TABLE IF EXISTS orders;
    DROP TABLE IF EXISTS users;", conn))
{
    await cmd.ExecuteNonQueryAsync();
}

// Load schema
var schemaPath = Path.Combine(Directory.GetCurrentDirectory(), "../sql/snowflake", "schema.sql");
var schemaText = await File.ReadAllTextAsync(schemaPath);
foreach (var block in schemaText.Split(";"))
{
    var trimmed = block.Trim();
    if (!string.IsNullOrEmpty(trimmed))
    {
        await using var cmd = new SnowflakeCommand(trimmed, conn);
        await cmd.ExecuteNonQueryAsync();
    }
}

// Test: CreateUser
await Queries.CreateUser(conn, "Alice", "alice@example.com");
var user = await Queries.GetUserById(conn, 1);
Assert(user != null, "CreateUser", "returned null");
Assert(user!.Name == "Alice", "CreateUser", $"expected name Alice, got {user.Name}");
Assert(user.Email == "alice@example.com", "CreateUser", $"expected email alice@example.com, got {user.Email}");
Assert(user.Id == 1, "CreateUser", $"expected id 1, got {user.Id}");
Console.WriteLine("PASS: CreateUser");

var userId = 1;

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
await Queries.CreateOrder(conn, userId, 99.95m, "first order");
var orders = await Queries.GetOrdersByUser(conn, userId);
Assert(orders.Count == 1, "CreateOrder", $"expected 1 order created, got {orders.Count}");
var order = orders[0];
Assert(order.UserId == userId, "CreateOrder", $"expected user_id {userId}, got {order.UserId}");
Assert(order.Total == 99.95m, "CreateOrder", $"expected total 99.95, got {order.Total}");
Assert(order.Notes == "first order", "CreateOrder", $"expected notes 'first order', got {order.Notes}");
Console.WriteLine("PASS: CreateOrder");

// Test: GetOrdersByUser
var ordersList = await Queries.GetOrdersByUser(conn, userId);
Assert(ordersList.Count == 1, "GetOrdersByUser", $"expected 1 order, got {ordersList.Count}");
Assert(ordersList[0].Total == 99.95m, "GetOrdersByUser", $"expected total 99.95, got {ordersList[0].Total}");
Assert(ordersList[0].Notes == "first order", "GetOrdersByUser", $"expected notes 'first order', got {ordersList[0].Notes}");
Console.WriteLine("PASS: GetOrdersByUser");

// Test: DeleteOrdersByUser (delete orders first due to FK)
var deletedOrders = await Queries.DeleteOrdersByUser(conn, userId);
Assert(deletedOrders == 1, "DeleteOrdersByUser", $"expected 1 deleted order, got {deletedOrders}");
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
