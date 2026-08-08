// scythe:provenance v=0.14.0 backend=csharp-oracle engine=oracle schema=sch1:51c12e41405f20c2
#nullable enable

using Oracle.ManagedDataAccess.Client;

public static class Queries {

public record CreateAttachmentRow(
    long Id,
    long OrderId,
    string Filename
);

public static async Task<CreateAttachmentRow?> CreateAttachment(OracleConnection conn, long order_id, string filename, byte[] payload, string? description) {
    using var cmd = new OracleCommand("INSERT INTO attachments (order_id, filename, payload, description) VALUES (:1, :2, :3, :4) RETURNING id, order_id, filename INTO :out0, :out1, :out2", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)order_id ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { Value = (object)filename ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { Value = (object)payload ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { Value = (object)description ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out0", OracleDbType = OracleDbType.Int64, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out1", OracleDbType = OracleDbType.Int64, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out2", OracleDbType = OracleDbType.Varchar2, Size = 4000, Direction = System.Data.ParameterDirection.Output });
    await cmd.ExecuteNonQueryAsync();
    return new CreateAttachmentRow(
        ((Oracle.ManagedDataAccess.Types.OracleDecimal)cmd.Parameters["out0"].Value).ToInt64(),
        ((Oracle.ManagedDataAccess.Types.OracleDecimal)cmd.Parameters["out1"].Value).ToInt64(),
        ((Oracle.ManagedDataAccess.Types.OracleString)cmd.Parameters["out2"].Value).Value
    );
}

public record GetAttachmentsByOrderRow(
    long Id,
    long OrderId,
    string Filename,
    byte[] Payload,
    string? Description
);

public static async Task<List<GetAttachmentsByOrderRow>> GetAttachmentsByOrder(OracleConnection conn, long order_id) {
    using var cmd = new OracleCommand("SELECT id, order_id, filename, payload, description FROM attachments WHERE order_id = :1 ORDER BY id", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)order_id ?? DBNull.Value });
    using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<GetAttachmentsByOrderRow>();
    while (await reader.ReadAsync()) {
        results.Add(new GetAttachmentsByOrderRow(
            reader.GetInt64(0),
            reader.GetInt64(1),
            reader.GetString(2),
            reader.GetFieldValue<byte[]>(3),
            reader.IsDBNull(4) ? null : reader.GetString(4)
        ));
    }
    return results;
}

public record GetAttachmentByIdRow(
    long Id,
    long OrderId,
    string Filename,
    byte[] Payload,
    string? Description
);

public static async Task<GetAttachmentByIdRow?> GetAttachmentById(OracleConnection conn, long id) {
    using var cmd = new OracleCommand("SELECT id, order_id, filename, payload, description FROM attachments WHERE id = :1", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)id ?? DBNull.Value });
    using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) return null;
    return new GetAttachmentByIdRow(
        reader.GetInt64(0),
        reader.GetInt64(1),
        reader.GetString(2),
        reader.GetFieldValue<byte[]>(3),
        reader.IsDBNull(4) ? null : reader.GetString(4)
    );
}

public static async Task<int> DeleteAttachmentsByOrder(OracleConnection conn, long order_id) {
    using var cmd = new OracleCommand("DELETE FROM attachments WHERE order_id = :1", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)order_id ?? DBNull.Value });
    return await cmd.ExecuteNonQueryAsync();
}

public record CreateOrderRow(
    long Id,
    long UserId,
    decimal Total,
    string? Notes,
    DateTime CreatedAt
);

public static async Task<CreateOrderRow?> CreateOrder(OracleConnection conn, long user_id, decimal total, string? notes) {
    using var cmd = new OracleCommand("INSERT INTO orders (user_id, total, notes) VALUES (:1, :2, :3) RETURNING id, user_id, total, notes, created_at INTO :out0, :out1, :out2, :out3, :out4", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)user_id ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { Value = (object)total ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { Value = (object)notes ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out0", OracleDbType = OracleDbType.Int64, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out1", OracleDbType = OracleDbType.Int64, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out2", OracleDbType = OracleDbType.Decimal, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out3", OracleDbType = OracleDbType.Varchar2, Size = 4000, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out4", OracleDbType = OracleDbType.Date, Direction = System.Data.ParameterDirection.Output });
    await cmd.ExecuteNonQueryAsync();
    return new CreateOrderRow(
        ((Oracle.ManagedDataAccess.Types.OracleDecimal)cmd.Parameters["out0"].Value).ToInt64(),
        ((Oracle.ManagedDataAccess.Types.OracleDecimal)cmd.Parameters["out1"].Value).ToInt64(),
        ((Oracle.ManagedDataAccess.Types.OracleDecimal)cmd.Parameters["out2"].Value).Value,
        ((Oracle.ManagedDataAccess.Types.OracleString)cmd.Parameters["out3"].Value).Value,
        ((Oracle.ManagedDataAccess.Types.OracleDate)cmd.Parameters["out4"].Value).Value
    );
}

public record GetOrdersByUserRow(
    long Id,
    decimal Total,
    string? Notes,
    DateTime CreatedAt
);

public static async Task<List<GetOrdersByUserRow>> GetOrdersByUser(OracleConnection conn, long user_id) {
    using var cmd = new OracleCommand("SELECT id, total, notes, created_at FROM orders WHERE user_id = :1 ORDER BY created_at DESC", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)user_id ?? DBNull.Value });
    using var reader = await cmd.ExecuteReaderAsync();
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

public static async Task<GetOrderTotalRow?> GetOrderTotal(OracleConnection conn, long user_id) {
    using var cmd = new OracleCommand("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :1", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)user_id ?? DBNull.Value });
    using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) return null;
    return new GetOrderTotalRow(
        reader.IsDBNull(0) ? null : reader.GetDecimal(0)
    );
}

public static async Task<int> DeleteOrdersByUser(OracleConnection conn, long user_id) {
    using var cmd = new OracleCommand("DELETE FROM orders WHERE user_id = :1", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)user_id ?? DBNull.Value });
    return await cmd.ExecuteNonQueryAsync();
}

public record GetUserByIdRow(
    long Id,
    string Name,
    string? Email,
    long Active,
    DateTime CreatedAt
);

public static async Task<GetUserByIdRow?> GetUserById(OracleConnection conn, long id) {
    using var cmd = new OracleCommand("SELECT id, name, email, active, created_at FROM users WHERE id = :1", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)id ?? DBNull.Value });
    using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) return null;
    return new GetUserByIdRow(
        reader.GetInt64(0),
        reader.GetString(1),
        reader.IsDBNull(2) ? null : reader.GetString(2),
        reader.GetInt64(3),
        reader.GetDateTime(4)
    );
}

public record ListActiveUsersRow(
    long Id,
    string Name,
    string? Email
);

public static async Task<List<ListActiveUsersRow>> ListActiveUsers(OracleConnection conn) {
    using var cmd = new OracleCommand("SELECT id, name, email FROM users WHERE active = 1", conn);
    using var reader = await cmd.ExecuteReaderAsync();
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

public record CreateUserRow(
    long Id,
    string Name,
    string? Email,
    long Active,
    DateTime CreatedAt
);

public static async Task<CreateUserRow?> CreateUser(OracleConnection conn, string name, string? email, long active) {
    using var cmd = new OracleCommand("INSERT INTO users (name, email, active) VALUES (:1, :2, :3) RETURNING id, name, email, active, created_at INTO :out0, :out1, :out2, :out3, :out4", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)name ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { Value = (object)email ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { Value = (object)active ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out0", OracleDbType = OracleDbType.Int64, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out1", OracleDbType = OracleDbType.Varchar2, Size = 4000, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out2", OracleDbType = OracleDbType.Varchar2, Size = 4000, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out3", OracleDbType = OracleDbType.Int64, Direction = System.Data.ParameterDirection.Output });
    cmd.Parameters.Add(new OracleParameter { ParameterName = "out4", OracleDbType = OracleDbType.Date, Direction = System.Data.ParameterDirection.Output });
    await cmd.ExecuteNonQueryAsync();
    return new CreateUserRow(
        ((Oracle.ManagedDataAccess.Types.OracleDecimal)cmd.Parameters["out0"].Value).ToInt64(),
        ((Oracle.ManagedDataAccess.Types.OracleString)cmd.Parameters["out1"].Value).Value,
        ((Oracle.ManagedDataAccess.Types.OracleString)cmd.Parameters["out2"].Value).Value,
        ((Oracle.ManagedDataAccess.Types.OracleDecimal)cmd.Parameters["out3"].Value).ToInt64(),
        ((Oracle.ManagedDataAccess.Types.OracleDate)cmd.Parameters["out4"].Value).Value
    );
}

public static async Task UpdateUserEmail(OracleConnection conn, string email, long id) {
    using var cmd = new OracleCommand("UPDATE users SET email = :1 WHERE id = :2", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)email ?? DBNull.Value });
    cmd.Parameters.Add(new OracleParameter { Value = (object)id ?? DBNull.Value });
    await cmd.ExecuteNonQueryAsync();
}

public static async Task DeleteUser(OracleConnection conn, long id) {
    using var cmd = new OracleCommand("DELETE FROM users WHERE id = :1", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)id ?? DBNull.Value });
    await cmd.ExecuteNonQueryAsync();
}

public record SearchUsersRow(
    long Id,
    string Name,
    string? Email
);

public static async Task<List<SearchUsersRow>> SearchUsers(OracleConnection conn, string name) {
    using var cmd = new OracleCommand("SELECT id, name, email FROM users WHERE name LIKE :1", conn);
    cmd.Parameters.Add(new OracleParameter { Value = (object)name ?? DBNull.Value });
    using var reader = await cmd.ExecuteReaderAsync();
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
