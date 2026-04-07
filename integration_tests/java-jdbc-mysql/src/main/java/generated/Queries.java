package generated;

import java.sql.Connection;
import java.sql.ResultSet;
import java.sql.SQLException;
import javax.annotation.Nullable;

public class Queries {

    public enum UsersStatus {
        active("active"),
        inactive("inactive"),
        banned("banned");

        private final String value;
        UsersStatus(String value) { this.value = value; }
        public String getValue() { return value; }

        public static UsersStatus fromString(String text) {
            for (UsersStatus s : UsersStatus.values()) {
                if (s.value.equals(text)) {
                    return s;
                }
            }
            throw new IllegalArgumentException("Unknown UsersStatus: " + text);
        }
    }

    // --- Orders ---

    public static void createOrder(Connection conn, int user_id, java.math.BigDecimal total, String notes) throws SQLException {
        try (var ps = conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)")) {
            ps.setInt(1, user_id);
            ps.setBigDecimal(2, total);
            ps.setString(3, notes);
            ps.executeUpdate();
        }
    }

    public record GetLastInsertOrderRow(
        int id,
        int user_id,
        java.math.BigDecimal total,
        @Nullable String notes,
        java.time.LocalDateTime created_at
    ) {
        public static GetLastInsertOrderRow fromResultSet(ResultSet rs) throws SQLException {
            return new GetLastInsertOrderRow(
                rs.getInt("id"),
                rs.getInt("user_id"),
                rs.getBigDecimal("total"),
                rs.getString("notes"),
                rs.getTimestamp("created_at").toLocalDateTime()
            );
        }
    }

    public static GetLastInsertOrderRow getLastInsertOrder(Connection conn) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, user_id, total, notes, created_at FROM orders WHERE id = LAST_INSERT_ID()")) {
            try (ResultSet rs = ps.executeQuery()) {
                if (rs.next()) {
                    return GetLastInsertOrderRow.fromResultSet(rs);
                }
                return null;
            }
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
                rs.getTimestamp("created_at").toLocalDateTime()
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

    // --- Users ---

    public record GetUserByIdRow(
        int id,
        String name,
        @Nullable String email,
        UsersStatus status,
        java.time.LocalDateTime created_at
    ) {
        public static GetUserByIdRow fromResultSet(ResultSet rs) throws SQLException {
            return new GetUserByIdRow(
                rs.getInt("id"),
                rs.getString("name"),
                rs.getString("email"),
                UsersStatus.fromString(rs.getString("status")),
                rs.getTimestamp("created_at").toLocalDateTime()
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

    public static java.util.List<ListActiveUsersRow> listActiveUsers(Connection conn, UsersStatus status) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?")) {
            ps.setString(1, status.getValue());
            try (ResultSet rs = ps.executeQuery()) {
                java.util.List<ListActiveUsersRow> result = new java.util.ArrayList<>();
                while (rs.next()) {
                    result.add(ListActiveUsersRow.fromResultSet(rs));
                }
                return result;
            }
        }
    }

    public static void createUser(Connection conn, String name, String email, UsersStatus status) throws SQLException {
        try (var ps = conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?)")) {
            ps.setString(1, name);
            ps.setString(2, email);
            ps.setString(3, status.getValue());
            ps.executeUpdate();
        }
    }

    public record GetLastInsertUserRow(
        int id,
        String name,
        @Nullable String email,
        UsersStatus status,
        java.time.LocalDateTime created_at
    ) {
        public static GetLastInsertUserRow fromResultSet(ResultSet rs) throws SQLException {
            return new GetLastInsertUserRow(
                rs.getInt("id"),
                rs.getString("name"),
                rs.getString("email"),
                UsersStatus.fromString(rs.getString("status")),
                rs.getTimestamp("created_at").toLocalDateTime()
            );
        }
    }

    public static GetLastInsertUserRow getLastInsertUser(Connection conn) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = LAST_INSERT_ID()")) {
            try (ResultSet rs = ps.executeQuery()) {
                if (rs.next()) {
                    return GetLastInsertUserRow.fromResultSet(rs);
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
