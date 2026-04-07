package generated;

import java.sql.Connection;
import java.sql.ResultSet;
import java.sql.SQLException;
import java.sql.Types;
import javax.annotation.Nullable;

public class Queries {

    public enum UserStatus {
        active("active"),
        inactive("inactive"),
        banned("banned");

        private final String value;
        UserStatus(String value) { this.value = value; }
        public String getValue() { return value; }

        public static UserStatus fromString(String text) {
            for (UserStatus s : UserStatus.values()) {
                if (s.value.equals(text)) {
                    return s;
                }
            }
            throw new IllegalArgumentException("Unknown status: " + text);
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
                rs.getObject("created_at", java.time.OffsetDateTime.class)
            );
        }
    }

    public static CreateOrderRow createOrder(Connection conn, int user_id, java.math.BigDecimal total, String notes) throws SQLException {
        try (var ps = conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at")) {
            ps.setInt(1, user_id);
            ps.setBigDecimal(2, total);
            ps.setString(3, notes);
            try (ResultSet rs = ps.executeQuery()) {
                if (rs.next()) {
                    return CreateOrderRow.fromResultSet(rs);
                }
                return null;
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
                rs.getObject("created_at", java.time.OffsetDateTime.class)
            );
        }
    }

    public static java.util.List<GetOrdersByUserRow> getOrdersByUser(Connection conn, int user_id) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC")) {
            ps.setInt(1, user_id);
            try (ResultSet rs = ps.executeQuery()) {
                java.util.List<GetOrdersByUserRow> result = new java.util.ArrayList<>();
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
                return null;
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
                UserStatus.fromString(rs.getString("status")),
                rs.getObject("created_at", java.time.OffsetDateTime.class)
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
                return null;
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

    public static java.util.List<ListActiveUsersRow> listActiveUsers(Connection conn, UserStatus status) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?")) {
            ps.setObject(1, status.getValue(), Types.OTHER);
            try (ResultSet rs = ps.executeQuery()) {
                java.util.List<ListActiveUsersRow> result = new java.util.ArrayList<>();
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
                UserStatus.fromString(rs.getString("status")),
                rs.getObject("created_at", java.time.OffsetDateTime.class)
            );
        }
    }

    public static CreateUserRow createUser(Connection conn, String name, String email, UserStatus status) throws SQLException {
        try (var ps = conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email, status, created_at")) {
            ps.setString(1, name);
            ps.setString(2, email);
            ps.setObject(3, status.getValue(), Types.OTHER);
            try (ResultSet rs = ps.executeQuery()) {
                if (rs.next()) {
                    return CreateUserRow.fromResultSet(rs);
                }
                return null;
            }
        }
    }

    public static void updateUserEmail(Connection conn, String email, int id) throws SQLException {
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

    public static java.util.List<GetUserOrdersRow> getUserOrders(Connection conn, UserStatus status) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = ?")) {
            ps.setObject(1, status.getValue(), Types.OTHER);
            try (ResultSet rs = ps.executeQuery()) {
                java.util.List<GetUserOrdersRow> result = new java.util.ArrayList<>();
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
                UserStatus.fromString(rs.getString("status")),
                rs.getLong("user_count")
            );
        }
    }

    public static CountUsersByStatusRow countUsersByStatus(Connection conn, UserStatus status) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ?")) {
            ps.setObject(1, status.getValue(), Types.OTHER);
            try (ResultSet rs = ps.executeQuery()) {
                if (rs.next()) {
                    return CountUsersByStatusRow.fromResultSet(rs);
                }
                return null;
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

    public static java.util.List<SearchUsersRow> searchUsers(Connection conn, String name) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE name LIKE ?")) {
            ps.setString(1, name);
            try (ResultSet rs = ps.executeQuery()) {
                java.util.List<SearchUsersRow> result = new java.util.ArrayList<>();
                while (rs.next()) {
                    result.add(SearchUsersRow.fromResultSet(rs));
                }
                return result;
            }
        }
    }
}
