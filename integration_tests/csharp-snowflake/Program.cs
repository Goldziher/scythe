using Snowflake.Data.Client;

static string GetConnectionString()
{
    var snowflakeUrl = Environment.GetEnvironmentVariable("SNOWFLAKE_URL")
        ?? "snowflake://scythe:scythe@localhost:443/scythe_test/public?account=test";

    // Parse snowflake://user:pass@host:port/database/schema?account=X&protocol=Y
    var uri = new Uri(snowflakeUrl);
    var userInfo = uri.UserInfo.Split(':');
    var user = userInfo[0];
    var password = userInfo.Length > 1 ? userInfo[1] : "";

    var pathParts = uri.AbsolutePath.Split('/');
    var database = pathParts.Length > 1 ? pathParts[1] : "";
    var schema = pathParts.Length > 2 ? pathParts[2] : "";

    // Parse account/protocol from query params
    var account = "";
    var scheme = "https";
    if (!string.IsNullOrEmpty(uri.Query))
    {
        foreach (var param in uri.Query.TrimStart('?').Split('&'))
        {
            if (param.StartsWith("account="))
            {
                account = param.Substring("account=".Length);
            }
            else if (param.StartsWith("protocol="))
            {
                scheme = param.Substring("protocol=".Length);
            }
        }
    }

    var port = uri.Port > 0 ? uri.Port : (scheme == "http" ? 80 : 443);
    var insecureMode = scheme == "http" ? ";insecuremode=true" : "";
    return $"account={account};host={uri.Host};port={port};scheme={scheme};user={user};password={password};db={database};schema={schema}{insecureMode}";
}

await using var conn = new SnowflakeDbConnection();
conn.ConnectionString = GetConnectionString();
await conn.OpenAsync();

var exitCode = 0;
var failedTests = new HashSet<string>();

void Assert(bool condition, string testName, string detail)
{
    if (!condition)
    {
        Console.Error.WriteLine($"FAIL: {testName}: {detail}");
        exitCode = 1;
        failedTests.Add(testName);
    }
}

void Pass(string testName, string? label = null)
{
    if (!failedTests.Contains(testName))
    {
        Console.WriteLine($"PASS: {label ?? testName}");
    }
}

// Splits a SQL script into statements on top-level ';' only -- unlike a
// naive `schemaText.Split(";")`, this tracks single- and double-quoted
// spans, PostgreSQL dollar-quoted bodies, and "--" line comments (an
// apostrophe in a comment must not open a phantom string -- board #224
// follow-up) so a ';' inside a string literal, a `$$ ... $$` function
// body, or a comment does not split the statement in half. "/* ... */"
// block comments are not handled -- no schema under integration_tests/sql/
// uses them today.
static IEnumerable<string> SplitSqlStatements(string sql)
{
    var statements = new List<string>();
    var current = new System.Text.StringBuilder();
    var inSingle = false;
    var inDouble = false;
    var inLineComment = false;
    string? dollarTag = null;
    var i = 0;
    while (i < sql.Length)
    {
        var ch = sql[i];
        if (inLineComment)
        {
            current.Append(ch);
            if (ch == '\n') inLineComment = false;
            i++;
            continue;
        }
        if (dollarTag is not null)
        {
            current.Append(ch);
            if (ch == '$' && string.CompareOrdinal(sql, i, dollarTag, 0, dollarTag.Length) == 0)
            {
                current.Append(dollarTag.AsSpan(1));
                i += dollarTag.Length;
                dollarTag = null;
                continue;
            }
            i++;
            continue;
        }
        if (inSingle)
        {
            current.Append(ch);
            if (ch == '\'') inSingle = false;
            i++;
            continue;
        }
        if (inDouble)
        {
            current.Append(ch);
            if (ch == '"') inDouble = false;
            i++;
            continue;
        }
        if (ch == '\'')
        {
            inSingle = true;
            current.Append(ch);
            i++;
            continue;
        }
        if (ch == '"')
        {
            inDouble = true;
            current.Append(ch);
            i++;
            continue;
        }
        if (ch == '-' && i + 1 < sql.Length && sql[i + 1] == '-')
        {
            inLineComment = true;
            current.Append(ch);
            i++;
            continue;
        }
        if (ch == '$')
        {
            var match = System.Text.RegularExpressions.Regex.Match(sql[i..], @"^\$[A-Za-z0-9_]*\$");
            if (match.Success)
            {
                dollarTag = match.Value;
                current.Append(dollarTag);
                i += dollarTag.Length;
                continue;
            }
        }
        if (ch == ';')
        {
            statements.Add(current.ToString());
            current.Clear();
            i++;
            continue;
        }
        current.Append(ch);
        i++;
    }
    if (current.ToString().Trim() != string.Empty)
    {
        statements.Add(current.ToString());
    }
    return statements.Select(s => s.Trim()).Where(s => s.Length > 0);
}

// Clean slate (Snowflake.Data does not support multi-statement text without
// MULTI_STATEMENT_COUNT, so each DROP runs as its own statement)
foreach (var dropStatement in new[]
{
    "DROP TABLE IF EXISTS user_tags",
    "DROP TABLE IF EXISTS tags",
    "DROP TABLE IF EXISTS orders",
    "DROP TABLE IF EXISTS users",
})
{
    await using var cmd = new SnowflakeDbCommand(conn) { CommandText = dropStatement };
    await cmd.ExecuteNonQueryAsync();
}

// Load schema
var schemaPath = Path.Combine(Directory.GetCurrentDirectory(), "../sql/snowflake", "schema.sql");
var schemaText = await File.ReadAllTextAsync(schemaPath);
foreach (var stmt in SplitSqlStatements(schemaText))
{
    await using var cmd = new SnowflakeDbCommand(conn) { CommandText = stmt };
    await cmd.ExecuteNonQueryAsync();
}

// Test: CreateUser
await Queries.CreateUser(conn, "Alice", "alice@example.com", true);
var user = await Queries.GetUserById(conn, 1);
Assert(user != null, "CreateUser", "returned null");
Assert(user!.Name == "Alice", "CreateUser", $"expected name Alice, got {user.Name}");
Assert(user.Email == "alice@example.com", "CreateUser", $"expected email alice@example.com, got {user.Email}");
Assert(user.Id == 1, "CreateUser", $"expected id 1, got {user.Id}");
Pass("CreateUser");

var userId = 1;

// Test: GetUserById
var fetched = await Queries.GetUserById(conn, userId);
Assert(fetched != null, "GetUserById", "returned null");
Assert(fetched!.Id == userId, "GetUserById", $"expected id {userId}, got {fetched.Id}");
Assert(fetched.Name == "Alice", "GetUserById", $"expected name Alice, got {fetched.Name}");
Assert(fetched.Email == "alice@example.com", "GetUserById", $"expected email alice@example.com, got {fetched.Email}");
Pass("GetUserById");

// Test: ListActiveUsers
var activeUsers = await Queries.ListActiveUsers(conn);
Assert(activeUsers.Count >= 1, "ListActiveUsers", $"expected at least 1 user, got {activeUsers.Count}");
Assert(activeUsers.Any(u => u.Name == "Alice"), "ListActiveUsers", "expected Alice in active users");
Pass("ListActiveUsers");

// Test: CreateOrder
await Queries.CreateOrder(conn, userId, 99.95m, "first order");
var orders = await Queries.GetOrdersByUser(conn, userId);
Assert(orders.Count == 1, "CreateOrder", $"expected 1 order created, got {orders.Count}");
var order = orders[0];
Assert(order.Total == 99.95m, "CreateOrder", $"expected total 99.95, got {order.Total}");
Assert(order.Notes == "first order", "CreateOrder", $"expected notes 'first order', got {order.Notes}");
Pass("CreateOrder");

// Test: GetOrdersByUser
var ordersList = await Queries.GetOrdersByUser(conn, userId);
Assert(ordersList.Count == 1, "GetOrdersByUser", $"expected 1 order, got {ordersList.Count}");
Assert(ordersList[0].Total == 99.95m, "GetOrdersByUser", $"expected total 99.95, got {ordersList[0].Total}");
Assert(ordersList[0].Notes == "first order", "GetOrdersByUser", $"expected notes 'first order', got {ordersList[0].Notes}");
Pass("GetOrdersByUser");

// Test: DeleteOrdersByUser (delete orders first due to FK)
var deletedOrders = await Queries.DeleteOrdersByUser(conn, userId);
Assert(deletedOrders == 1, "DeleteOrdersByUser", $"expected 1 deleted order, got {deletedOrders}");
Pass("DeleteOrdersByUser");

// Test: DeleteUser
await Queries.DeleteUser(conn, userId);
var deletedUserWasFound = true;
try
{
    await Queries.GetUserById(conn, userId);
}
catch (InvalidOperationException)
{
    deletedUserWasFound = false;
}
Assert(!deletedUserWasFound, "DeleteUser", "user should not exist after deletion");
Pass("DeleteUser");

if (exitCode == 0)
{
    Console.WriteLine("ALL TESTS PASSED");
}

Environment.Exit(exitCode);
