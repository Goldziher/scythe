// scythe:provenance v=0.16.0 backend=java-jdbc engine=postgresql schema=sch1:c247390d575b8f71 queries=q1:a78685f58b075ff5 options=opt1:cbf29ce484222325
package generated;

import java.math.BigDecimal;
import java.sql.*;
import java.time.*;
import java.util.ArrayList;
import java.util.List;
import javax.annotation.Nonnull;
import javax.annotation.Nullable;

public class Queries {

public enum UserStatus {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");

    private final String value;
    UserStatus(String value) { this.value = value; }
    public String getValue() { return value; }

    public static UserStatus fromValue(String value) {
        for (UserStatus v : values()) {
            if (v.value.equals(value)) {
                return v;
            }
        }
        throw new IllegalArgumentException("Unknown UserStatus value: " + value);
    }
}

public record CreateOrderRow(
    int id,
    int user_id,
    java.math.BigDecimal total,
    @Nullable String notes,
    java.time.OffsetDateTime created_at
) {
    public static CreateOrderRow fromResultSet(ResultSet rs) throws SQLException {
        return new CreateOrderRow(
            rs.getInt("id"),
            rs.getInt("user_id"),
            rs.getBigDecimal("total"),
            rs.getString("notes"),
            rs.getObject("created_at", OffsetDateTime.class)
        );
    }
}

public static CreateOrderRow createOrder(Connection conn, int user_id, @Nonnull java.math.BigDecimal total, @Nullable String notes) throws SQLException {
    try (var ps = conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at")) {
        ps.setInt(1, user_id);
        ps.setBigDecimal(2, total);
        ps.setString(3, notes);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return CreateOrderRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("createOrder: no rows returned");
        }
    }
}

public record GetOrdersByUserRow(
    int id,
    java.math.BigDecimal total,
    @Nullable String notes,
    java.time.OffsetDateTime created_at
) {
    public static GetOrdersByUserRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetOrdersByUserRow(
            rs.getInt("id"),
            rs.getBigDecimal("total"),
            rs.getString("notes"),
            rs.getObject("created_at", OffsetDateTime.class)
        );
    }
}

public static List<GetOrdersByUserRow> getOrdersByUser(Connection conn, int user_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC")) {
        ps.setInt(1, user_id);
        try (ResultSet rs = ps.executeQuery()) {
            List<GetOrdersByUserRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(GetOrdersByUserRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public record GetOrderTotalRow(
    @Nullable java.math.BigDecimal total_sum
) {
    public static GetOrderTotalRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetOrderTotalRow(
            rs.getBigDecimal("total_sum")
        );
    }
}

public static GetOrderTotalRow getOrderTotal(Connection conn, int user_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?")) {
        ps.setInt(1, user_id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetOrderTotalRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("getOrderTotal: no rows returned");
        }
    }
}

public record GetOrderWeightTotalRow(
    @Nullable Double weight_total
) {
    public static GetOrderWeightTotalRow fromResultSet(ResultSet rs) throws SQLException {
        var weight_totalRaw = rs.getDouble("weight_total");
        Double weight_total = rs.wasNull() ? null : weight_totalRaw;
        return new GetOrderWeightTotalRow(
            weight_total
        );
    }
}

public static GetOrderWeightTotalRow getOrderWeightTotal(Connection conn, int user_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = ?")) {
        ps.setInt(1, user_id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetOrderWeightTotalRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("getOrderWeightTotal: no rows returned");
        }
    }
}

public static int deleteOrdersByUser(Connection conn, int user_id) throws SQLException {
    try (var ps = conn.prepareStatement("DELETE FROM orders WHERE user_id = ?")) {
        ps.setInt(1, user_id);
        return ps.executeUpdate();
    }
}

public record GetUserByIdRow(
    int id,
    String name,
    @Nullable String email,
    UserStatus status,
    java.time.OffsetDateTime created_at
) {
    public static GetUserByIdRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetUserByIdRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getString("email"),
            UserStatus.fromValue(rs.getString("status")),
            rs.getObject("created_at", OffsetDateTime.class)
        );
    }
}

public static GetUserByIdRow getUserById(Connection conn, int id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?")) {
        ps.setInt(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetUserByIdRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("getUserById: no rows returned");
        }
    }
}

public record ListActiveUsersRow(
    int id,
    String name,
    @Nullable String email
) {
    public static ListActiveUsersRow fromResultSet(ResultSet rs) throws SQLException {
        return new ListActiveUsersRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getString("email")
        );
    }
}

public static List<ListActiveUsersRow> listActiveUsers(Connection conn, @Nonnull UserStatus status) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?")) {
        ps.setObject(1, status.getValue(), java.sql.Types.OTHER);
        try (ResultSet rs = ps.executeQuery()) {
            List<ListActiveUsersRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(ListActiveUsersRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public record CreateUserRow(
    int id,
    String name,
    @Nullable String email,
    UserStatus status,
    java.time.OffsetDateTime created_at
) {
    public static CreateUserRow fromResultSet(ResultSet rs) throws SQLException {
        return new CreateUserRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getString("email"),
            UserStatus.fromValue(rs.getString("status")),
            rs.getObject("created_at", OffsetDateTime.class)
        );
    }
}

public static CreateUserRow createUser(Connection conn, @Nonnull String name, @Nullable String email, @Nonnull UserStatus status) throws SQLException {
    try (var ps = conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email, status, created_at")) {
        ps.setString(1, name);
        ps.setString(2, email);
        ps.setObject(3, status.getValue(), java.sql.Types.OTHER);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return CreateUserRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("createUser: no rows returned");
        }
    }
}

public static void updateUserEmail(Connection conn, @Nonnull String email, int id) throws SQLException {
    try (var ps = conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?")) {
        ps.setString(1, email);
        ps.setInt(2, id);
        ps.executeUpdate();
    }
}

public static void deleteUser(Connection conn, int id) throws SQLException {
    try (var ps = conn.prepareStatement("DELETE FROM users WHERE id = ?")) {
        ps.setInt(1, id);
        ps.executeUpdate();
    }
}

public record GetUserOrdersRow(
    int id,
    String name,
    @Nullable java.math.BigDecimal total,
    @Nullable String notes
) {
    public static GetUserOrdersRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetUserOrdersRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getBigDecimal("total"),
            rs.getString("notes")
        );
    }
}

public static List<GetUserOrdersRow> getUserOrders(Connection conn, @Nonnull UserStatus status) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = ?")) {
        ps.setObject(1, status.getValue(), java.sql.Types.OTHER);
        try (ResultSet rs = ps.executeQuery()) {
            List<GetUserOrdersRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(GetUserOrdersRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public record CountUsersByStatusRow(
    UserStatus status,
    long user_count
) {
    public static CountUsersByStatusRow fromResultSet(ResultSet rs) throws SQLException {
        return new CountUsersByStatusRow(
            UserStatus.fromValue(rs.getString("status")),
            rs.getLong("user_count")
        );
    }
}

public static CountUsersByStatusRow countUsersByStatus(Connection conn, @Nonnull UserStatus status) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ?")) {
        ps.setObject(1, status.getValue(), java.sql.Types.OTHER);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return CountUsersByStatusRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("countUsersByStatus: no rows returned");
        }
    }
}

public record GetUserWithTagsRow(
    int id,
    String name,
    String tag_name
) {
    public static GetUserWithTagsRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetUserWithTagsRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getString("tag_name")
        );
    }
}

public static List<GetUserWithTagsRow> getUserWithTags(Connection conn, int id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = ?")) {
        ps.setInt(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            List<GetUserWithTagsRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(GetUserWithTagsRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public record SearchUsersRow(
    int id,
    String name,
    @Nullable String email
) {
    public static SearchUsersRow fromResultSet(ResultSet rs) throws SQLException {
        return new SearchUsersRow(
            rs.getInt("id"),
            rs.getString("name"),
            rs.getString("email")
        );
    }
}

public static List<SearchUsersRow> searchUsers(Connection conn, @Nonnull String name) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE name LIKE ?")) {
        ps.setString(1, name);
        try (ResultSet rs = ps.executeQuery()) {
            List<SearchUsersRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(SearchUsersRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public record UserAddress(String street, String city, String zip) {

    /**
     * ~keep board #196: pgjdbc registers no `getObject(col, UserAddress.class)` type map for
     * this composite -- it throws `PSQLException: conversion to class UserAddress` at runtime.
     * Parse the driver's composite text form instead.
     */
    public static UserAddress fromText(String text) {
        if (text == null) {
            return null;
        }
        java.util.List<String> f = parseCompositeFields(text);
        return new UserAddress(
            f.get(0),
            f.get(1),
            f.get(2)
        );
    }

    /**
     * ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
     * honoring its escaping rules: an empty unquoted field is SQL NULL (returned as `null`); a
     * field needing quoting (containing a comma, paren, quote, backslash, or leading/trailing
     * space, or the empty string) is wrapped in double quotes; every other field is unquoted and
     * taken literally. A nested composite's own "(x,y)" text form always contains parens, so it
     * always comes back quoted here, ready for that type's own `fromText` to parse recursively.
     *
     * Inside a quoted field `record_out` writes a literal `"` as `""` and a literal `\` as `\\`.
     * Both spellings must be accepted: reading `""` as "closing quote, then a new field" both
     * truncates the value and desynchronizes every field after it. Verified against
     * PostgreSQL 16 -- ROW('he said "hi"', 'back\slash', NULL) renders as
     * ("he said ""hi""","back\\slash",).
     */
    private static java.util.List<String> parseCompositeFields(String text) {
        java.util.List<String> fields = new java.util.ArrayList<>();
        String inner = text.substring(1, text.length() - 1);
        int i = 0;
        int n = inner.length();
        while (true) {
            StringBuilder field = new StringBuilder();
            boolean isNull = false;
            if (i < n && inner.charAt(i) == '"') {
                i++;
                while (i < n) {
                    char c = inner.charAt(i);
                    if (c == '\\' && i + 1 < n) {
                        field.append(inner.charAt(i + 1));
                        i += 2;
                    } else if (c == '"' && i + 1 < n && inner.charAt(i + 1) == '"') {
                        field.append('"');
                        i += 2;
                    } else if (c == '"') {
                        i++;
                        break;
                    } else {
                        field.append(c);
                        i++;
                    }
                }
            } else {
                int start = i;
                while (i < n && inner.charAt(i) != ',') {
                    i++;
                }
                field.append(inner, start, i);
                isNull = field.length() == 0;
            }
            fields.add(isNull ? null : field.toString());
            if (i < n && inner.charAt(i) == ',') {
                i++;
                continue;
            }
            break;
        }
        return fields;
    }
}

public record GetUserProfileRow(
    int id,
    @Nullable UserStatus secondary_status,
    @Nullable UserAddress address
) {
    public static GetUserProfileRow fromResultSet(ResultSet rs) throws SQLException {
        var secondary_statusRaw = rs.getString("secondary_status");
        UserStatus secondary_status = secondary_statusRaw == null ? null : UserStatus.fromValue(secondary_statusRaw);
        return new GetUserProfileRow(
            rs.getInt("id"),
            secondary_status,
            UserAddress.fromText(rs.getString("address"))
        );
    }
}

public static GetUserProfileRow getUserProfile(Connection conn, int id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, secondary_status, address FROM users WHERE id = ?")) {
        ps.setInt(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetUserProfileRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("getUserProfile: no rows returned");
        }
    }
}

}

