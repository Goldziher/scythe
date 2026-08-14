// scythe:provenance v=0.15.0 backend=java-jdbc engine=snowflake schema=sch1:c91500313602fb46 queries=q1:4bc3d50da85e2742 options=opt1:cbf29ce484222325
package generated;

import java.math.BigDecimal;
import java.sql.*;
import java.time.*;
import java.util.ArrayList;
import java.util.List;
import javax.annotation.Nonnull;
import javax.annotation.Nullable;

public class Queries {

public static void createOrder(Connection conn, long user_id, @Nonnull java.math.BigDecimal total, @Nullable String notes) throws SQLException {
    try (var ps = conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)")) {
        ps.setLong(1, user_id);
        ps.setBigDecimal(2, total);
        ps.setString(3, notes);
        ps.executeUpdate();
    }
}

public record GetOrdersByUserRow(
    long id,
    java.math.BigDecimal total,
    @Nullable String notes,
    java.time.LocalDateTime created_at
) {
    public static GetOrdersByUserRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetOrdersByUserRow(
            rs.getLong("id"),
            rs.getBigDecimal("total"),
            rs.getString("notes"),
            rs.getTimestamp("created_at").toLocalDateTime()
        );
    }
}

public static List<GetOrdersByUserRow> getOrdersByUser(Connection conn, long user_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC")) {
        ps.setLong(1, user_id);
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

public static GetOrderTotalRow getOrderTotal(Connection conn, long user_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?")) {
        ps.setLong(1, user_id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetOrderTotalRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("getOrderTotal: no rows returned");
        }
    }
}

public static int deleteOrdersByUser(Connection conn, long user_id) throws SQLException {
    try (var ps = conn.prepareStatement("DELETE FROM orders WHERE id IN (SELECT id FROM orders WHERE user_id = ?)")) {
        ps.setLong(1, user_id);
        return ps.executeUpdate();
    }
}

public record GetUserByIdRow(
    long id,
    String name,
    @Nullable String email,
    boolean active,
    @Nullable String metadata,
    java.time.LocalDateTime created_at,
    @Nullable java.time.OffsetDateTime updated_at
) {
    public static GetUserByIdRow fromResultSet(ResultSet rs) throws SQLException {
        var updated_atRaw = rs.getTimestamp("updated_at");
        OffsetDateTime updated_at = rs.wasNull() ? null : updated_atRaw.toInstant().atOffset(ZoneOffset.UTC);
        return new GetUserByIdRow(
            rs.getLong("id"),
            rs.getString("name"),
            rs.getString("email"),
            rs.getBoolean("active"),
            rs.getString("metadata"),
            rs.getTimestamp("created_at").toLocalDateTime(),
            updated_at
        );
    }
}

public static GetUserByIdRow getUserById(Connection conn, long id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email, active, metadata, created_at, updated_at FROM users WHERE id = ?")) {
        ps.setLong(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetUserByIdRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("getUserById: no rows returned");
        }
    }
}

public record ListActiveUsersRow(
    long id,
    String name,
    @Nullable String email
) {
    public static ListActiveUsersRow fromResultSet(ResultSet rs) throws SQLException {
        return new ListActiveUsersRow(
            rs.getLong("id"),
            rs.getString("name"),
            rs.getString("email")
        );
    }
}

public static List<ListActiveUsersRow> listActiveUsers(Connection conn) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE active = TRUE")) {
        try (ResultSet rs = ps.executeQuery()) {
            List<ListActiveUsersRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(ListActiveUsersRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public static void createUser(Connection conn, @Nonnull String name, @Nullable String email, boolean active) throws SQLException {
    try (var ps = conn.prepareStatement("INSERT INTO users (name, email, active) VALUES (?, ?, ?)")) {
        ps.setString(1, name);
        ps.setString(2, email);
        ps.setBoolean(3, active);
        ps.executeUpdate();
    }
}

public static void updateUserEmail(Connection conn, @Nonnull String email, long id) throws SQLException {
    try (var ps = conn.prepareStatement("UPDATE users SET email = ?, updated_at = CURRENT_TIMESTAMP() WHERE id = ?")) {
        ps.setString(1, email);
        ps.setLong(2, id);
        ps.executeUpdate();
    }
}

public static void deleteUser(Connection conn, long id) throws SQLException {
    try (var ps = conn.prepareStatement("DELETE FROM users WHERE id = ?")) {
        ps.setLong(1, id);
        ps.executeUpdate();
    }
}

public record SearchUsersRow(
    long id,
    String name,
    @Nullable String email
) {
    public static SearchUsersRow fromResultSet(ResultSet rs) throws SQLException {
        return new SearchUsersRow(
            rs.getLong("id"),
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

}

