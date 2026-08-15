// scythe:provenance v=0.16.0 backend=java-jdbc engine=sqlite schema=sch1:588fb635332179bc queries=q1:f7199f36438b6396 options=opt1:cbf29ce484222325
package generated;

import java.math.BigDecimal;
import java.sql.*;
import java.time.*;
import java.util.ArrayList;
import java.util.List;
import javax.annotation.Nonnull;
import javax.annotation.Nullable;

public class Queries {

public static void createOrder(Connection conn, long user_id, double total, @Nullable String notes) throws SQLException {
    try (var ps = conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)")) {
        ps.setLong(1, user_id);
        ps.setDouble(2, total);
        ps.setString(3, notes);
        ps.executeUpdate();
    }
}

public record GetOrdersByUserRow(
    long id,
    double total,
    @Nullable String notes,
    String created_at
) {
    public static GetOrdersByUserRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetOrdersByUserRow(
            rs.getLong("id"),
            rs.getDouble("total"),
            rs.getString("notes"),
            rs.getString("created_at")
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
    @Nullable Double total_sum
) {
    public static GetOrderTotalRow fromResultSet(ResultSet rs) throws SQLException {
        var total_sumRaw = rs.getDouble("total_sum");
        Double total_sum = rs.wasNull() ? null : total_sumRaw;
        return new GetOrderTotalRow(
            total_sum
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
    try (var ps = conn.prepareStatement("DELETE FROM orders WHERE user_id = ?")) {
        ps.setLong(1, user_id);
        return ps.executeUpdate();
    }
}

public record GetUserByIdRow(
    long id,
    String name,
    @Nullable String email,
    String status,
    String created_at
) {
    public static GetUserByIdRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetUserByIdRow(
            rs.getLong("id"),
            rs.getString("name"),
            rs.getString("email"),
            rs.getString("status"),
            rs.getString("created_at")
        );
    }
}

public static GetUserByIdRow getUserById(Connection conn, long id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?")) {
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

public static List<ListActiveUsersRow> listActiveUsers(Connection conn, @Nonnull String status) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?")) {
        ps.setString(1, status);
        try (ResultSet rs = ps.executeQuery()) {
            List<ListActiveUsersRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(ListActiveUsersRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public static void createUser(Connection conn, @Nonnull String name, @Nullable String email, @Nonnull String status) throws SQLException {
    try (var ps = conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?)")) {
        ps.setString(1, name);
        ps.setString(2, email);
        ps.setString(3, status);
        ps.executeUpdate();
    }
}

public static void updateUserEmail(Connection conn, @Nonnull String email, long id) throws SQLException {
    try (var ps = conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?")) {
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

