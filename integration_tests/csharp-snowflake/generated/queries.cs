// scythe:provenance v=0.14.0 backend=csharp-snowflake engine=snowflake schema=sch1:c91500313602fb46 queries=q1:4bc3d50da85e2742
#nullable enable

using System.Data;
using Snowflake.Data.Client;

public static class Queries {

public static async Task CreateOrder(SnowflakeDbConnection conn, long user_id, decimal total, string? notes) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.Int64, Value = user_id });
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "2", DbType = System.Data.DbType.Decimal, Value = total });
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "3", DbType = System.Data.DbType.String, Value = notes });
    await cmd.ExecuteNonQueryAsync();
}

public record GetOrdersByUserRow(
    long Id,
    decimal Total,
    string? Notes,
    DateTime CreatedAt
);

public static async Task<List<GetOrdersByUserRow>> GetOrdersByUser(SnowflakeDbConnection conn, long user_id) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.Int64, Value = user_id });
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<GetOrdersByUserRow>();
    while (await reader.ReadAsync()) {
        results.Add(new GetOrdersByUserRow(
            reader.GetInt64(0),
            reader.GetDecimal(1),
            reader.IsDBNull(2) ? null : reader.GetString(2),
            reader.GetDateTime(3)
        ));
    }
    return results;
}

public record GetOrderTotalRow(
    decimal? TotalSum
);

public static async Task<GetOrderTotalRow> GetOrderTotal(SnowflakeDbConnection conn, long user_id) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.Int64, Value = user_id });
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetOrderTotal expected exactly one row but found none");
    return new GetOrderTotalRow(
        reader.IsDBNull(0) ? null : reader.GetDecimal(0)
    );
}

public static async Task<int> DeleteOrdersByUser(SnowflakeDbConnection conn, long user_id) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"DELETE FROM orders WHERE id IN (SELECT id FROM orders WHERE user_id = ?)";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.Int64, Value = user_id });
    return await cmd.ExecuteNonQueryAsync();
}

public record GetUserByIdRow(
    long Id,
    string Name,
    string? Email,
    bool Active,
    string? Metadata,
    DateTime CreatedAt,
    DateTimeOffset? UpdatedAt
);

public static async Task<GetUserByIdRow> GetUserById(SnowflakeDbConnection conn, long id) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"SELECT id, name, email, active, metadata, created_at, updated_at FROM users WHERE id = ?";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.Int64, Value = id });
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetUserById expected exactly one row but found none");
    return new GetUserByIdRow(
        reader.GetInt64(0),
        reader.GetString(1),
        reader.IsDBNull(2) ? null : reader.GetString(2),
        reader.GetBoolean(3),
        reader.IsDBNull(4) ? null : reader.GetString(4),
        reader.GetDateTime(5),
        reader.IsDBNull(6) ? null : reader.GetFieldValue<DateTimeOffset>(6)
    );
}

public record ListActiveUsersRow(
    long Id,
    string Name,
    string? Email
);

public static async Task<List<ListActiveUsersRow>> ListActiveUsers(SnowflakeDbConnection conn) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"SELECT id, name, email FROM users WHERE active = TRUE";
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<ListActiveUsersRow>();
    while (await reader.ReadAsync()) {
        results.Add(new ListActiveUsersRow(
            reader.GetInt64(0),
            reader.GetString(1),
            reader.IsDBNull(2) ? null : reader.GetString(2)
        ));
    }
    return results;
}

public static async Task CreateUser(SnowflakeDbConnection conn, string name, string? email, bool active) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"INSERT INTO users (name, email, active) VALUES (?, ?, ?)";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.String, Value = name });
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "2", DbType = System.Data.DbType.String, Value = email });
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "3", DbType = System.Data.DbType.Boolean, Value = active });
    await cmd.ExecuteNonQueryAsync();
}

public static async Task UpdateUserEmail(SnowflakeDbConnection conn, string email, long id) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"UPDATE users SET email = ?, updated_at = CURRENT_TIMESTAMP() WHERE id = ?";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.String, Value = email });
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "2", DbType = System.Data.DbType.Int64, Value = id });
    await cmd.ExecuteNonQueryAsync();
}

public static async Task DeleteUser(SnowflakeDbConnection conn, long id) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"DELETE FROM users WHERE id = ?";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.Int64, Value = id });
    await cmd.ExecuteNonQueryAsync();
}

public record SearchUsersRow(
    long Id,
    string Name,
    string? Email
);

public static async Task<List<SearchUsersRow>> SearchUsers(SnowflakeDbConnection conn, string name) {
    await using var cmd = new SnowflakeDbCommand(conn);
    cmd.CommandText = @"SELECT id, name, email FROM users WHERE name LIKE ?";
    cmd.Parameters.Add(new SnowflakeDbParameter { ParameterName = "1", DbType = System.Data.DbType.String, Value = name });
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<SearchUsersRow>();
    while (await reader.ReadAsync()) {
        results.Add(new SearchUsersRow(
            reader.GetInt64(0),
            reader.GetString(1),
            reader.IsDBNull(2) ? null : reader.GetString(2)
        ));
    }
    return results;
}

}
