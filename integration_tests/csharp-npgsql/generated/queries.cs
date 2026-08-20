// scythe:provenance v=0.16.1 backend=csharp-npgsql engine=postgresql schema=sch2:c247390d575b8f71 queries=q1:b6aca93cc722fe32 options=opt1:cbf29ce484222325
#nullable enable

using Npgsql;

public static class Queries {

public enum UserStatus {
    Active,
    Inactive,
    Banned,
}

public record CreateOrderRow(
    int Id,
    int UserId,
    decimal Total,
    string? Notes,
    DateTimeOffset CreatedAt
);

public static async Task<CreateOrderRow> CreateOrder(NpgsqlConnection conn, int user_id, decimal total, string? notes) {
    await using var cmd = new NpgsqlCommand(@"INSERT INTO orders (user_id, total, notes) VALUES (@p1, @p2, @p3) RETURNING id, user_id, total, notes, created_at", conn);
    cmd.Parameters.AddWithValue("p1", user_id);
    cmd.Parameters.AddWithValue("p2", total);
    cmd.Parameters.AddWithValue("p3", notes);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("CreateOrder expected exactly one row but found none");
    return new CreateOrderRow(
        reader.GetInt32(0),
        reader.GetInt32(1),
        reader.GetDecimal(2),
        reader.IsDBNull(3) ? null : reader.GetString(3),
        reader.GetFieldValue<DateTimeOffset>(4)
    );
}

public record GetOrdersByUserRow(
    int Id,
    decimal Total,
    string? Notes,
    DateTimeOffset CreatedAt
);

public static async Task<List<GetOrdersByUserRow>> GetOrdersByUser(NpgsqlConnection conn, int user_id) {
    await using var cmd = new NpgsqlCommand(@"SELECT id, total, notes, created_at FROM orders WHERE user_id = @p1 ORDER BY created_at DESC", conn);
    cmd.Parameters.AddWithValue("p1", user_id);
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<GetOrdersByUserRow>();
    while (await reader.ReadAsync()) {
        results.Add(new GetOrdersByUserRow(
            reader.GetInt32(0),
            reader.GetDecimal(1),
            reader.IsDBNull(2) ? null : reader.GetString(2),
            reader.GetFieldValue<DateTimeOffset>(3)
        ));
    }
    return results;
}

public record GetOrderTotalRow(
    decimal? TotalSum
);

public static async Task<GetOrderTotalRow> GetOrderTotal(NpgsqlConnection conn, int user_id) {
    await using var cmd = new NpgsqlCommand(@"SELECT SUM(total) AS total_sum FROM orders WHERE user_id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", user_id);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetOrderTotal expected exactly one row but found none");
    return new GetOrderTotalRow(
        reader.IsDBNull(0) ? null : reader.GetDecimal(0)
    );
}

public record GetOrderWeightTotalRow(
    double? WeightTotal
);

public static async Task<GetOrderWeightTotalRow> GetOrderWeightTotal(NpgsqlConnection conn, int user_id) {
    await using var cmd = new NpgsqlCommand(@"SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", user_id);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetOrderWeightTotal expected exactly one row but found none");
    return new GetOrderWeightTotalRow(
        reader.IsDBNull(0) ? null : reader.GetDouble(0)
    );
}

public static async Task<int> DeleteOrdersByUser(NpgsqlConnection conn, int user_id) {
    await using var cmd = new NpgsqlCommand(@"DELETE FROM orders WHERE user_id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", user_id);
    return await cmd.ExecuteNonQueryAsync();
}

public record GetUserByIdRow(
    int Id,
    string Name,
    string? Email,
    UserStatus Status,
    DateTimeOffset CreatedAt
);

public static async Task<GetUserByIdRow> GetUserById(NpgsqlConnection conn, int id) {
    await using var cmd = new NpgsqlCommand(@"SELECT id, name, email, status, created_at FROM users WHERE id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", id);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetUserById expected exactly one row but found none");
    return new GetUserByIdRow(
        reader.GetInt32(0),
        reader.GetString(1),
        reader.IsDBNull(2) ? null : reader.GetString(2),
        (Enum.TryParse<UserStatus>(reader.GetString(3), true, out var enumVal3) ? enumVal3 : throw new InvalidOperationException($"Invalid enum value '{reader.GetString(3)}' for UserStatus")),
        reader.GetFieldValue<DateTimeOffset>(4)
    );
}

public record ListActiveUsersRow(
    int Id,
    string Name,
    string? Email
);

public static async Task<List<ListActiveUsersRow>> ListActiveUsers(NpgsqlConnection conn, UserStatus status) {
    await using var cmd = new NpgsqlCommand(@"SELECT id, name, email FROM users WHERE status = @p1::user_status", conn);
    cmd.Parameters.AddWithValue("p1", status.ToDbValue());
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<ListActiveUsersRow>();
    while (await reader.ReadAsync()) {
        results.Add(new ListActiveUsersRow(
            reader.GetInt32(0),
            reader.GetString(1),
            reader.IsDBNull(2) ? null : reader.GetString(2)
        ));
    }
    return results;
}

public record CreateUserRow(
    int Id,
    string Name,
    string? Email,
    UserStatus Status,
    DateTimeOffset CreatedAt
);

public static async Task<CreateUserRow> CreateUser(NpgsqlConnection conn, string name, string? email, UserStatus status) {
    await using var cmd = new NpgsqlCommand(@"INSERT INTO users (name, email, status) VALUES (@p1, @p2, @p3::user_status) RETURNING id, name, email, status, created_at", conn);
    cmd.Parameters.AddWithValue("p1", name);
    cmd.Parameters.AddWithValue("p2", email);
    cmd.Parameters.AddWithValue("p3", status.ToDbValue());
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("CreateUser expected exactly one row but found none");
    return new CreateUserRow(
        reader.GetInt32(0),
        reader.GetString(1),
        reader.IsDBNull(2) ? null : reader.GetString(2),
        (Enum.TryParse<UserStatus>(reader.GetString(3), true, out var enumVal3) ? enumVal3 : throw new InvalidOperationException($"Invalid enum value '{reader.GetString(3)}' for UserStatus")),
        reader.GetFieldValue<DateTimeOffset>(4)
    );
}

public static async Task UpdateUserEmail(NpgsqlConnection conn, string email, int id) {
    await using var cmd = new NpgsqlCommand(@"UPDATE users SET email = @p1 WHERE id = @p2", conn);
    cmd.Parameters.AddWithValue("p1", email);
    cmd.Parameters.AddWithValue("p2", id);
    await cmd.ExecuteNonQueryAsync();
}

public static async Task DeleteUser(NpgsqlConnection conn, int id) {
    await using var cmd = new NpgsqlCommand(@"DELETE FROM users WHERE id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", id);
    await cmd.ExecuteNonQueryAsync();
}

public record GetUserOrdersRow(
    int Id,
    string Name,
    decimal? Total,
    string? Notes
);

public static async Task<List<GetUserOrdersRow>> GetUserOrders(NpgsqlConnection conn, UserStatus status) {
    await using var cmd = new NpgsqlCommand(@"SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = @p1::user_status", conn);
    cmd.Parameters.AddWithValue("p1", status.ToDbValue());
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<GetUserOrdersRow>();
    while (await reader.ReadAsync()) {
        results.Add(new GetUserOrdersRow(
            reader.GetInt32(0),
            reader.GetString(1),
            reader.IsDBNull(2) ? null : reader.GetDecimal(2),
            reader.IsDBNull(3) ? null : reader.GetString(3)
        ));
    }
    return results;
}

public record CountUsersByStatusRow(
    UserStatus Status,
    long UserCount
);

public static async Task<CountUsersByStatusRow> CountUsersByStatus(NpgsqlConnection conn, UserStatus status) {
    await using var cmd = new NpgsqlCommand(@"SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = @p1::user_status", conn);
    cmd.Parameters.AddWithValue("p1", status.ToDbValue());
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("CountUsersByStatus expected exactly one row but found none");
    return new CountUsersByStatusRow(
        (Enum.TryParse<UserStatus>(reader.GetString(0), true, out var enumVal0) ? enumVal0 : throw new InvalidOperationException($"Invalid enum value '{reader.GetString(0)}' for UserStatus")),
        reader.GetInt64(1)
    );
}

public record GetUserWithTagsRow(
    int Id,
    string Name,
    string TagName
);

public static async Task<List<GetUserWithTagsRow>> GetUserWithTags(NpgsqlConnection conn, int id) {
    await using var cmd = new NpgsqlCommand(@"SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", id);
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<GetUserWithTagsRow>();
    while (await reader.ReadAsync()) {
        results.Add(new GetUserWithTagsRow(
            reader.GetInt32(0),
            reader.GetString(1),
            reader.GetString(2)
        ));
    }
    return results;
}

public record SearchUsersRow(
    int Id,
    string Name,
    string? Email
);

public static async Task<List<SearchUsersRow>> SearchUsers(NpgsqlConnection conn, string name) {
    await using var cmd = new NpgsqlCommand(@"SELECT id, name, email FROM users WHERE name LIKE @p1", conn);
    cmd.Parameters.AddWithValue("p1", name);
    await using var reader = await cmd.ExecuteReaderAsync();
    var results = new List<SearchUsersRow>();
    while (await reader.ReadAsync()) {
        results.Add(new SearchUsersRow(
            reader.GetInt32(0),
            reader.GetString(1),
            reader.IsDBNull(2) ? null : reader.GetString(2)
        ));
    }
    return results;
}

public record UserAddress(
    string Street,
    string City,
    string Zip
) {
    /// <summary>
    /// ~keep board #220: Npgsql has no binary decoder for this composite unless the
    /// caller registers one with NpgsqlDataSourceBuilder.MapComposite&lt;UserAddress&gt;() --
    /// this generated code cannot do that on the caller's behalf, so it parses the
    /// driver's composite text form instead.
    /// </summary>
    public static UserAddress? FromText(string? text)
    {
        if (text is null)
        {
            return null;
        }
        var f = ParseCompositeFields(text);
        return new UserAddress(
            f[0]!,
            f[1]!,
            f[2]!
        );
    }

    public string ToPgText()
    {
        return "(" + string.Join(",", new[] { EncodeCompositeField(Street), EncodeCompositeField(City), EncodeCompositeField(Zip) }) + ")";
    }

    private static string EncodeCompositeField(object? value)
    {
        if (value is null) return string.Empty;
        var raw = value is IFormattable formattable
            ? formattable.ToString(null, System.Globalization.CultureInfo.InvariantCulture)
            : value.ToString()!;
        if (raw.Length > 0 && raw.IndexOfAny([',', '(', ')', '"', '\\']) < 0 && raw == raw.Trim()) return raw;
        return "\"" + raw.Replace("\\", "\\\\").Replace("\"", "\"\"") + "\"";
    }

    /// <summary>
    /// ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
    /// honoring its escaping rules: an empty unquoted field is SQL NULL (returned as null); a
    /// field needing quoting (containing a comma, paren, quote, backslash, or leading/trailing
    /// space, or the empty string) is wrapped in double quotes; every other field is unquoted
    /// and taken literally. A nested composite's own "(x,y)" text form always contains parens,
    /// so it always comes back quoted here, ready for that type's own FromText to parse
    /// recursively.
    ///
    /// Inside a quoted field record_out writes a literal '"' as '""' and a literal '\' as '\\'.
    /// Both spellings must be accepted: reading '""' as "closing quote, then a new field" both
    /// truncates the value and desynchronizes every field after it. Verified against
    /// PostgreSQL 16 -- ROW('he said "hi"', 'back\slash', NULL) renders as
    /// ("he said ""hi""","back\\slash",).
    /// </summary>
    private static List<string?> ParseCompositeFields(string text)
    {
        var fields = new List<string?>();
        var inner = text.Substring(1, text.Length - 2);
        int i = 0;
        int n = inner.Length;
        while (true)
        {
            var field = new System.Text.StringBuilder();
            bool isNull = false;
            if (i < n && inner[i] == '"')
            {
                i++;
                while (i < n)
                {
                    char c = inner[i];
                    if (c == '\\' && i + 1 < n)
                    {
                        field.Append(inner[i + 1]);
                        i += 2;
                    }
                    else if (c == '"' && i + 1 < n && inner[i + 1] == '"')
                    {
                        field.Append('"');
                        i += 2;
                    }
                    else if (c == '"')
                    {
                        i++;
                        break;
                    }
                    else
                    {
                        field.Append(c);
                        i++;
                    }
                }
            }
            else
            {
                int start = i;
                while (i < n && inner[i] != ',')
                {
                    i++;
                }
                field.Append(inner, start, i - start);
                isNull = field.Length == 0;
            }
            fields.Add(isNull ? null : field.ToString());
            if (i < n && inner[i] == ',')
            {
                i++;
                continue;
            }
            break;
        }
        return fields;
    }
}

public record GetUserProfileRow(
    int Id,
    UserStatus? SecondaryStatus,
    UserAddress? Address
);

public static async Task<GetUserProfileRow> GetUserProfile(NpgsqlConnection conn, int id) {
    await using var cmd = new NpgsqlCommand(@"SELECT id, secondary_status, address FROM users WHERE id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", id);
    cmd.UnknownResultTypeList = new[] { false, false, true };
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetUserProfile expected exactly one row but found none");
    return new GetUserProfileRow(
        reader.GetInt32(0),
        reader.IsDBNull(1) ? null : (Enum.TryParse<UserStatus>(reader.GetString(1), true, out var enumVal1) ? enumVal1 : throw new InvalidOperationException($"Invalid enum value '{reader.GetString(1)}' for UserStatus")),
        reader.IsDBNull(2) ? null : UserAddress.FromText(reader.GetFieldValue<string>(2))!
    );
}

public record RoundTripUserAddressRow(
    UserAddress? Address
);

public static async Task<RoundTripUserAddressRow> RoundTripUserAddress(NpgsqlConnection conn, UserAddress? address) {
    await using var cmd = new NpgsqlCommand(@"INSERT INTO users (name, status, address) VALUES ('Composite Parameter Round Trip', 'active', (@p1::text::user_address)) RETURNING address", conn);
    cmd.Parameters.AddWithValue("p1", (object?)address?.ToPgText() ?? DBNull.Value);
    cmd.UnknownResultTypeList = new[] { true };
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("RoundTripUserAddress expected exactly one row but found none");
    return new RoundTripUserAddressRow(
        reader.IsDBNull(0) ? null : UserAddress.FromText(reader.GetFieldValue<string>(0))!
    );
}

public record GetUserAsJsonRow(
    string? Payload
);

public static async Task<GetUserAsJsonRow> GetUserAsJson(NpgsqlConnection conn, int id) {
    await using var cmd = new NpgsqlCommand(@"SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = @p1", conn);
    cmd.Parameters.AddWithValue("p1", id);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetUserAsJson expected exactly one row but found none");
    return new GetUserAsJsonRow(
        reader.IsDBNull(0) ? null : reader.GetString(0)
    );
}

public record GetUsersAsJsonRow(
    string? Payload
);

public static async Task<GetUsersAsJsonRow> GetUsersAsJson(NpgsqlConnection conn) {
    await using var cmd = new NpgsqlCommand(@"SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u", conn);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetUsersAsJson expected exactly one row but found none");
    return new GetUsersAsJsonRow(
        reader.IsDBNull(0) ? null : reader.GetString(0)
    );
}

public record GetUserOrdersAsJsonRow(
    string? Payload
);

public static async Task<GetUserOrdersAsJsonRow> GetUserOrdersAsJson(NpgsqlConnection conn, int id) {
    await using var cmd = new NpgsqlCommand(@"SELECT json_agg(o.* ORDER BY o.id) AS payload FROM users u LEFT JOIN orders o ON o.user_id = u.id WHERE u.id = @p1 GROUP BY u.id", conn);
    cmd.Parameters.AddWithValue("p1", id);
    await using var reader = await cmd.ExecuteReaderAsync();
    if (!await reader.ReadAsync()) throw new InvalidOperationException("GetUserOrdersAsJson expected exactly one row but found none");
    return new GetUserOrdersAsJsonRow(
        reader.IsDBNull(0) ? null : reader.GetString(0)
    );
}

}

public static class UserStatusExtensions {
    public static string ToDbValue(this Queries.UserStatus value) => value switch {
        Queries.UserStatus.Active => "active",
        Queries.UserStatus.Inactive => "inactive",
        Queries.UserStatus.Banned => "banned",
        _ => throw new ArgumentOutOfRangeException(nameof(value), value, null),
    };
}
