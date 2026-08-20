using Npgsql;

var databaseUrl = Environment.GetEnvironmentVariable("DATABASE_URL")
    ?? "postgres://scythe:scythe@localhost:5433/scythe_test";

// Parse postgres:// URL into Npgsql connection string
var uri = new Uri(databaseUrl);
var userInfo = uri.UserInfo.Split(':');
var connString = $"Host={uri.Host};Port={uri.Port};Database={uri.AbsolutePath.TrimStart('/')};Username={userInfo[0]};Password={userInfo[1]}";

await using var conn = new NpgsqlConnection(connString);
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

// Clean slate
await using (var cmd = new NpgsqlCommand(@"
    DROP TABLE IF EXISTS user_tags CASCADE;
    DROP TABLE IF EXISTS tags CASCADE;
    DROP TABLE IF EXISTS orders CASCADE;
    DROP TABLE IF EXISTS users CASCADE;
    DROP TYPE IF EXISTS user_status CASCADE;
    DROP TYPE IF EXISTS user_address CASCADE;
", conn))
{
    await cmd.ExecuteNonQueryAsync();
}

var schemaPath = Path.Combine(Directory.GetCurrentDirectory(), "../sql/pg", "schema.sql");
var schemaText = await File.ReadAllTextAsync(schemaPath);
await using (var cmd = new NpgsqlCommand(schemaText, conn))
{
    await cmd.ExecuteNonQueryAsync();
}
// Reload types so Npgsql knows about user_status enum
conn.ReloadTypes();

// Test: CreateUser
var user = await Queries.CreateUser(conn, "Alice", "alice@example.com", Queries.UserStatus.Active);
Assert(user != null, "CreateUser", "returned null");
Assert(user!.Name == "Alice", "CreateUser", $"expected name Alice, got {user.Name}");
Assert(user.Email == "alice@example.com", "CreateUser", $"expected email alice@example.com, got {user.Email}");
Assert(user.Status == Queries.UserStatus.Active, "CreateUser", $"expected status Active, got {user.Status}");
Assert(user.Id > 0, "CreateUser", $"expected positive id, got {user.Id}");
Pass("CreateUser");

var userId = user.Id;

// Test: GetUserById
var fetched = await Queries.GetUserById(conn, userId);
Assert(fetched != null, "GetUserById", "returned null");
Assert(fetched!.Id == userId, "GetUserById", $"expected id {userId}, got {fetched.Id}");
Assert(fetched.Name == "Alice", "GetUserById", $"expected name Alice, got {fetched.Name}");
Assert(fetched.Email == "alice@example.com", "GetUserById", $"expected email alice@example.com, got {fetched.Email}");
Assert(fetched.Status == Queries.UserStatus.Active, "GetUserById", $"expected status Active, got {fetched.Status}");
Pass("GetUserById");

// Test: ListActiveUsers
var activeUsers = await Queries.ListActiveUsers(conn, Queries.UserStatus.Active);
Assert(activeUsers.Count >= 1, "ListActiveUsers", $"expected at least 1 user, got {activeUsers.Count}");
Assert(activeUsers.Any(u => u.Name == "Alice"), "ListActiveUsers", "expected Alice in active users");
Pass("ListActiveUsers");

// Test: UpdateUserEmail
await Queries.UpdateUserEmail(conn, "alice-new@example.com", userId);
var updated = await Queries.GetUserById(conn, userId);
Assert(updated != null, "UpdateUserEmail", "user not found after update");
Assert(updated!.Email == "alice-new@example.com", "UpdateUserEmail", $"expected updated email, got {updated.Email}");
Pass("UpdateUserEmail");

// Test: CreateOrder
var order = await Queries.CreateOrder(conn, userId, 99.95m, "first order");
Assert(order != null, "CreateOrder", "returned null");
Assert(order!.UserId == userId, "CreateOrder", $"expected user_id {userId}, got {order.UserId}");
Assert(order.Total == 99.95m, "CreateOrder", $"expected total 99.95, got {order.Total}");
Assert(order.Notes == "first order", "CreateOrder", $"expected notes 'first order', got {order.Notes}");
Pass("CreateOrder");

// Test: GetOrdersByUser
var orders = await Queries.GetOrdersByUser(conn, userId);
Assert(orders.Count == 1, "GetOrdersByUser", $"expected 1 order, got {orders.Count}");
Assert(orders[0].Total == 99.95m, "GetOrdersByUser", $"expected total 99.95, got {orders[0].Total}");
Assert(orders[0].Notes == "first order", "GetOrdersByUser", $"expected notes 'first order', got {orders[0].Notes}");
Pass("GetOrdersByUser");

// Test: GetOrderTotal
var orderTotal = await Queries.GetOrderTotal(conn, userId);
Assert(orderTotal != null, "GetOrderTotal", "returned null");
Assert(orderTotal!.TotalSum == 99.95m, "GetOrderTotal", $"expected total 99.95, got {orderTotal.TotalSum}");
Pass("GetOrderTotal");
// Test: SearchUsers
var searchResults = await Queries.SearchUsers(conn, "%Ali%");
Assert(searchResults.Count >= 1, "SearchUsers", $"expected at least 1 result, got {searchResults.Count}");
Assert(searchResults.Any(u => u.Name == "Alice"), "SearchUsers", "expected Alice in search results");
Pass("SearchUsers");

// Test: CountUsersByStatus
var countResult = await Queries.CountUsersByStatus(conn, Queries.UserStatus.Active);
Assert(countResult != null, "CountUsersByStatus", "returned null");
Assert(countResult!.UserCount >= 1, "CountUsersByStatus", $"expected count >= 1, got {countResult.UserCount}");
Assert(countResult.Status == Queries.UserStatus.Active, "CountUsersByStatus", $"expected status Active, got {countResult.Status}");
Pass("CountUsersByStatus");

// Test: GetUserProfile (board #197/#204) -- a nullable enum and a nullable
// composite column, each observed both present and as SQL NULL, plus a
// composite field containing a double quote and a comma to prove
// UserAddress.FromText handles record_out's doubled-quote escaping (board
// #204) rather than truncating on it. The generated GetUserProfile also
// emits cmd.UnknownResultTypeList for the composite column, which this
// test exercises by reading through it.
int presentId;
await using (var presentCmd = new NpgsqlCommand(
    "INSERT INTO users (name, email, status, secondary_status, address) " +
    "VALUES ('Carol', 'carol@example.com', 'active', 'inactive', " +
    "ROW('1 Main St', 'Springfield', '12345')) RETURNING id", conn))
{
    presentId = (int)(await presentCmd.ExecuteScalarAsync())!;
}
int absentId;
await using (var absentCmd = new NpgsqlCommand(
    "INSERT INTO users (name, email, status, secondary_status, address) " +
    "VALUES ('Dave', 'dave@example.com', 'active', NULL, NULL) RETURNING id", conn))
{
    absentId = (int)(await absentCmd.ExecuteScalarAsync())!;
}
int quotedId;
await using (var quotedCmd = new NpgsqlCommand(
    "INSERT INTO users (name, email, status, secondary_status, address) " +
    "VALUES ('Eve', 'eve@example.com', 'active', 'inactive', " +
    "ROW('12 \"Main\", Apt 3', 'Berlin', '10115')) RETURNING id", conn))
{
    quotedId = (int)(await quotedCmd.ExecuteScalarAsync())!;
}

var profile = await Queries.GetUserProfile(conn, presentId);
Assert(profile.SecondaryStatus == Queries.UserStatus.Inactive, "GetUserProfile", $"expected secondary_status Inactive, got {profile.SecondaryStatus}");
Assert(profile.Address != null, "GetUserProfile", "expected address to be present");
Assert(profile.Address!.Street == "1 Main St", "GetUserProfile", $"expected address.Street '1 Main St', got {profile.Address.Street}");
Assert(profile.Address.City == "Springfield", "GetUserProfile", $"expected address.City 'Springfield', got {profile.Address.City}");
Assert(profile.Address.Zip == "12345", "GetUserProfile", $"expected address.Zip '12345', got {profile.Address.Zip}");

var nullProfile = await Queries.GetUserProfile(conn, absentId);
Assert(nullProfile.SecondaryStatus == null, "GetUserProfile", $"expected secondary_status null, got {nullProfile.SecondaryStatus}");
Assert(nullProfile.Address == null, "GetUserProfile", "expected address null");

var quotedProfile = await Queries.GetUserProfile(conn, quotedId);
Assert(quotedProfile.Address != null, "GetUserProfile", "expected quoted address to be present");
Assert(quotedProfile.Address!.Street == "12 \"Main\", Apt 3", "GetUserProfile", $"expected address.Street '12 \"Main\", Apt 3', got {quotedProfile.Address.Street}");
Assert(quotedProfile.Address.City == "Berlin", "GetUserProfile", $"expected address.City 'Berlin', got {quotedProfile.Address.City}");
Assert(quotedProfile.Address.Zip == "10115", "GetUserProfile", $"expected address.Zip '10115', got {quotedProfile.Address.Zip}");
Pass("GetUserProfile");

await Queries.DeleteUser(conn, presentId);
await Queries.DeleteUser(conn, absentId);
await Queries.DeleteUser(conn, quotedId);

// Test: DeleteUser (delete orders first due to FK)
var deletedOrders = await Queries.DeleteOrdersByUser(conn, userId);
Assert(deletedOrders == 1, "DeleteUser", $"expected 1 deleted order, got {deletedOrders}");
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
