// scythe:provenance v=0.17.0 backend=java-jdbc engine=mariadb schema=sch2:262bec5a0954c973 queries=q1:2f37bd0f0a685c79 options=opt1:cbf29ce484222325
package generated;

import java.math.BigDecimal;
import java.sql.*;
import java.time.*;
import java.util.ArrayList;
import java.util.List;
import javax.annotation.Nonnull;
import javax.annotation.Nullable;

public class Queries {

public enum UsersStatus {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");

    private final String value;
    UsersStatus(String value) { this.value = value; }
    public String getValue() { return value; }

    public static UsersStatus fromValue(String value) {
        for (UsersStatus v : values()) {
            if (v.value.equals(value)) {
                return v;
            }
        }
        throw new IllegalArgumentException("Unknown UsersStatus value: " + value);
    }
}

public record CreateOrderRow(
    int id,
    String user_id,
    java.math.BigDecimal total,
    @Nullable String notes,
    java.time.LocalDateTime created_at
) {
    public static CreateOrderRow fromResultSet(ResultSet rs) throws SQLException {
        return new CreateOrderRow(
            rs.getInt("id"),
            rs.getString("user_id"),
            rs.getBigDecimal("total"),
            rs.getString("notes"),
            rs.getObject("created_at", LocalDateTime.class)
        );
    }
}

public static CreateOrderRow createOrder(Connection conn, @Nonnull String user_id, @Nonnull java.math.BigDecimal total, @Nullable String notes) throws SQLException {
    try (var ps = conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at")) {
        ps.setString(1, user_id);
        ps.setBigDecimal(2, total);
        ps.setString(3, notes);
        ps.execute();
        ResultSet rs = ps.getResultSet();
        if (rs != null && rs.next()) {
            return CreateOrderRow.fromResultSet(rs);
        }
        throw new java.util.NoSuchElementException("createOrder: no rows returned");
    }
}

public record GetOrdersByUserRow(
    int id,
    java.math.BigDecimal total,
    @Nullable String notes,
    java.time.LocalDateTime created_at
) {
    public static GetOrdersByUserRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetOrdersByUserRow(
            rs.getInt("id"),
            rs.getBigDecimal("total"),
            rs.getString("notes"),
            rs.getObject("created_at", LocalDateTime.class)
        );
    }
}

public static List<GetOrdersByUserRow> getOrdersByUser(Connection conn, @Nonnull String user_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC")) {
        ps.setString(1, user_id);
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

public static GetOrderTotalRow getOrderTotal(Connection conn, @Nonnull String user_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?")) {
        ps.setString(1, user_id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetOrderTotalRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("getOrderTotal: no rows returned");
        }
    }
}

public static int deleteOrdersByUser(Connection conn, @Nonnull String user_id) throws SQLException {
    try (var ps = conn.prepareStatement("DELETE FROM orders WHERE user_id = ?")) {
        ps.setString(1, user_id);
        return ps.executeUpdate();
    }
}

public record GetUserByIdRow(
    String id,
    String name,
    @Nullable String email,
    UsersStatus status,
    java.time.LocalDateTime created_at
) {
    public static GetUserByIdRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetUserByIdRow(
            rs.getString("id"),
            rs.getString("name"),
            rs.getString("email"),
            UsersStatus.fromValue(rs.getString("status")),
            rs.getObject("created_at", LocalDateTime.class)
        );
    }
}

public static GetUserByIdRow getUserById(Connection conn, @Nonnull String id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?")) {
        ps.setString(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetUserByIdRow.fromResultSet(rs);
            }
            throw new java.util.NoSuchElementException("getUserById: no rows returned");
        }
    }
}

public record ListActiveUsersRow(
    String id,
    String name,
    @Nullable String email
) {
    public static ListActiveUsersRow fromResultSet(ResultSet rs) throws SQLException {
        return new ListActiveUsersRow(
            rs.getString("id"),
            rs.getString("name"),
            rs.getString("email")
        );
    }
}

public static List<ListActiveUsersRow> listActiveUsers(Connection conn, @Nonnull UsersStatus status) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?")) {
        ps.setString(1, status.getValue());
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
    String id,
    String name,
    @Nullable String email
) {
    public static CreateUserRow fromResultSet(ResultSet rs) throws SQLException {
        return new CreateUserRow(
            rs.getString("id"),
            rs.getString("name"),
            rs.getString("email")
        );
    }
}

public static CreateUserRow createUser(Connection conn, @Nonnull String name, @Nullable String email, @Nonnull UsersStatus status) throws SQLException {
    try (var ps = conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email")) {
        ps.setString(1, name);
        ps.setString(2, email);
        ps.setString(3, status.getValue());
        ps.execute();
        ResultSet rs = ps.getResultSet();
        if (rs != null && rs.next()) {
            return CreateUserRow.fromResultSet(rs);
        }
        throw new java.util.NoSuchElementException("createUser: no rows returned");
    }
}

public static void updateUserEmail(Connection conn, @Nonnull String email, @Nonnull String id) throws SQLException {
    try (var ps = conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?")) {
        ps.setString(1, email);
        ps.setString(2, id);
        ps.executeUpdate();
    }
}

public static void deleteUser(Connection conn, @Nonnull String id) throws SQLException {
    try (var ps = conn.prepareStatement("DELETE FROM users WHERE id = ? RETURNING id")) {
        ps.setString(1, id);
        ps.executeUpdate();
    }
}

public record SearchUsersRow(
    String id,
    String name,
    @Nullable String email
) {
    public static SearchUsersRow fromResultSet(ResultSet rs) throws SQLException {
        return new SearchUsersRow(
            rs.getString("id"),
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

