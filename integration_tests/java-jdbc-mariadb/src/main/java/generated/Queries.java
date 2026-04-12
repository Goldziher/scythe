package generated;

import java.math.BigDecimal;
import java.sql.*;
import java.time.LocalDateTime;
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

        public static UsersStatus fromString(String text) {
            for (UsersStatus s : UsersStatus.values()) {
                if (s.value.equals(text)) {
                    return s;
                }
            }
            throw new IllegalArgumentException("Unknown UsersStatus: " + text);
        }
    }

    public record CreateOrderRow(
        int id,
        java.util.UUID user_id,
        java.math.BigDecimal total,
        @Nullable String notes,
        java.time.LocalDateTime created_at
    ) {}

    public static @Nullable CreateOrderRow createOrder(Connection conn, @Nonnull java.util.UUID user_id, @Nonnull java.math.BigDecimal total, @Nullable String notes) throws SQLException {
        try (var ps = conn.prepareStatement("INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at")) {
            ps.setString(1, user_id.toString());
            ps.setBigDecimal(2, total);
            ps.setString(3, notes);
            ps.execute();
            try (ResultSet rs = ps.getResultSet()) {
                if (rs != null && rs.next()) {
                    return new CreateOrderRow(
                        rs.getInt("id"),
                        java.util.UUID.fromString(rs.getString("user_id")),
                        rs.getBigDecimal("total"),
                        rs.getString("notes"),
                        rs.getObject("created_at", LocalDateTime.class)
                    );
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
    ) {}

    public static List<GetOrdersByUserRow> getOrdersByUser(Connection conn, @Nonnull java.util.UUID user_id) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC")) {
            ps.setString(1, user_id.toString());
            try (ResultSet rs = ps.executeQuery()) {
                List<GetOrdersByUserRow> result = new ArrayList<>();
                while (rs.next()) {
                    result.add(new GetOrdersByUserRow(
                        rs.getInt("id"),
                        rs.getBigDecimal("total"),
                        rs.getString("notes"),
                        rs.getObject("created_at", LocalDateTime.class)
                    ));
                }
                return result;
            }
        }
    }

    public record GetOrderTotalRow(
        @Nullable java.math.BigDecimal total_sum
    ) {}

    public static @Nullable GetOrderTotalRow getOrderTotal(Connection conn, @Nonnull java.util.UUID user_id) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?")) {
            ps.setString(1, user_id.toString());
            try (ResultSet rs = ps.executeQuery()) {
                if (rs.next()) {
                    return new GetOrderTotalRow(
                        rs.getBigDecimal("total_sum")
                    );
                }
                return null;
            }
        }
    }

    public static int deleteOrdersByUser(Connection conn, @Nonnull java.util.UUID user_id) throws SQLException {
        try (var ps = conn.prepareStatement("DELETE FROM orders WHERE user_id = ?")) {
            ps.setString(1, user_id.toString());
            return ps.executeUpdate();
        }
    }

    public record GetUserByIdRow(
        java.util.UUID id,
        String name,
        @Nullable String email,
        UsersStatus status,
        java.time.LocalDateTime created_at
    ) {}

    public static @Nullable GetUserByIdRow getUserById(Connection conn, @Nonnull java.util.UUID id) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, name, email, status, created_at FROM users WHERE id = ?")) {
            ps.setString(1, id.toString());
            try (ResultSet rs = ps.executeQuery()) {
                if (rs.next()) {
                    return new GetUserByIdRow(
                        java.util.UUID.fromString(rs.getString("id")),
                        rs.getString("name"),
                        rs.getString("email"),
                        UsersStatus.fromString(rs.getString("status")),
                        rs.getObject("created_at", LocalDateTime.class)
                    );
                }
                return null;
            }
        }
    }

    public record ListActiveUsersRow(
        java.util.UUID id,
        String name,
        @Nullable String email
    ) {}

    public static List<ListActiveUsersRow> listActiveUsers(Connection conn, @Nonnull UsersStatus status) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE status = ?")) {
            ps.setString(1, status.getValue());
            try (ResultSet rs = ps.executeQuery()) {
                List<ListActiveUsersRow> result = new ArrayList<>();
                while (rs.next()) {
                    result.add(new ListActiveUsersRow(
                        java.util.UUID.fromString(rs.getString("id")),
                        rs.getString("name"),
                        rs.getString("email")
                    ));
                }
                return result;
            }
        }
    }

    public record CreateUserRow(
        java.util.UUID id,
        String name,
        @Nullable String email
    ) {}

    public static @Nullable CreateUserRow createUser(Connection conn, @Nonnull String name, @Nullable String email, @Nonnull UsersStatus status) throws SQLException {
        try (var ps = conn.prepareStatement("INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email")) {
            ps.setString(1, name);
            ps.setString(2, email);
            ps.setString(3, status.getValue());
            ps.execute();
            try (ResultSet rs = ps.getResultSet()) {
                if (rs != null && rs.next()) {
                    return new CreateUserRow(
                        java.util.UUID.fromString(rs.getString("id")),
                        rs.getString("name"),
                        rs.getString("email")
                    );
                }
                return null;
            }
        }
    }

    public static void updateUserEmail(Connection conn, @Nonnull String email, @Nonnull java.util.UUID id) throws SQLException {
        try (var ps = conn.prepareStatement("UPDATE users SET email = ? WHERE id = ?")) {
            ps.setString(1, email);
            ps.setString(2, id.toString());
            ps.executeUpdate();
        }
    }

    public static void deleteUser(Connection conn, @Nonnull java.util.UUID id) throws SQLException {
        try (var ps = conn.prepareStatement("DELETE FROM users WHERE id = ? RETURNING id")) {
            ps.setString(1, id.toString());
            ps.execute();
        }
    }

    public record SearchUsersRow(
        java.util.UUID id,
        String name,
        @Nullable String email
    ) {}

    public static List<SearchUsersRow> searchUsers(Connection conn, @Nonnull String name) throws SQLException {
        try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE name LIKE ?")) {
            ps.setString(1, name);
            try (ResultSet rs = ps.executeQuery()) {
                List<SearchUsersRow> result = new ArrayList<>();
                while (rs.next()) {
                    result.add(new SearchUsersRow(
                        java.util.UUID.fromString(rs.getString("id")),
                        rs.getString("name"),
                        rs.getString("email")
                    ));
                }
                return result;
            }
        }
    }

}
