// scythe:provenance v=0.13.0 backend=java-jdbc engine=oracle schema=sch1:51c12e41405f20c2
package generated;

import java.math.BigDecimal;
import java.sql.*;
import java.time.*;
import java.util.ArrayList;
import java.util.List;
import javax.annotation.Nonnull;
import javax.annotation.Nullable;

public class Queries {

public record CreateAttachmentRow(
    long id,
    long order_id,
    String filename
) {
    public static CreateAttachmentRow fromResultSet(ResultSet rs) throws SQLException {
        return new CreateAttachmentRow(
            rs.getLong("id"),
            rs.getLong("order_id"),
            rs.getString("filename")
        );
    }
}

public static @Nullable CreateAttachmentRow createAttachment(Connection conn, long order_id, @Nonnull String filename, @Nonnull byte[] payload, @Nullable String description) throws SQLException {
    try (var cs = conn.prepareCall("BEGIN INSERT INTO attachments (order_id, filename, payload, description) VALUES (?, ?, ?, ?) RETURNING id, order_id, filename INTO ?, ?, ?; END;")) {
        cs.setLong(1, order_id);
        cs.setString(2, filename);
        cs.setBytes(3, payload);
        cs.setString(4, description);
        cs.registerOutParameter(5, java.sql.Types.NUMERIC);
        cs.registerOutParameter(6, java.sql.Types.NUMERIC);
        cs.registerOutParameter(7, java.sql.Types.VARCHAR);
        cs.execute();
        return new CreateAttachmentRow(
            cs.getLong(5),
            cs.getLong(6),
            cs.getString(7)
        );
    }
}

public record GetAttachmentsByOrderRow(
    long id,
    long order_id,
    String filename,
    byte[] payload,
    @Nullable String description
) {
    public static GetAttachmentsByOrderRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetAttachmentsByOrderRow(
            rs.getLong("id"),
            rs.getLong("order_id"),
            rs.getString("filename"),
            rs.getBytes("payload"),
            rs.getString("description")
        );
    }
}

public static List<GetAttachmentsByOrderRow> getAttachmentsByOrder(Connection conn, long order_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, order_id, filename, payload, description FROM attachments WHERE order_id = ? ORDER BY id")) {
        ps.setLong(1, order_id);
        try (ResultSet rs = ps.executeQuery()) {
            List<GetAttachmentsByOrderRow> result = new ArrayList<>();
            while (rs.next()) {
                result.add(GetAttachmentsByOrderRow.fromResultSet(rs));
            }
            return result;
        }
    }
}

public record GetAttachmentByIdRow(
    long id,
    long order_id,
    String filename,
    byte[] payload,
    @Nullable String description
) {
    public static GetAttachmentByIdRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetAttachmentByIdRow(
            rs.getLong("id"),
            rs.getLong("order_id"),
            rs.getString("filename"),
            rs.getBytes("payload"),
            rs.getString("description")
        );
    }
}

public static @Nullable GetAttachmentByIdRow getAttachmentById(Connection conn, long id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, order_id, filename, payload, description FROM attachments WHERE id = ?")) {
        ps.setLong(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetAttachmentByIdRow.fromResultSet(rs);
            }
            return null;
        }
    }
}

public static int deleteAttachmentsByOrder(Connection conn, long order_id) throws SQLException {
    try (var ps = conn.prepareStatement("DELETE FROM attachments WHERE order_id = ?")) {
        ps.setLong(1, order_id);
        return ps.executeUpdate();
    }
}

public record CreateOrderRow(
    long id,
    long user_id,
    java.math.BigDecimal total,
    @Nullable String notes,
    java.time.LocalDateTime created_at
) {
    public static CreateOrderRow fromResultSet(ResultSet rs) throws SQLException {
        return new CreateOrderRow(
            rs.getLong("id"),
            rs.getLong("user_id"),
            rs.getBigDecimal("total"),
            rs.getString("notes"),
            rs.getObject("created_at", LocalDateTime.class)
        );
    }
}

public static @Nullable CreateOrderRow createOrder(Connection conn, long user_id, @Nonnull java.math.BigDecimal total, @Nullable String notes) throws SQLException {
    try (var cs = conn.prepareCall("BEGIN INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at INTO ?, ?, ?, ?, ?; END;")) {
        cs.setLong(1, user_id);
        cs.setBigDecimal(2, total);
        cs.setString(3, notes);
        cs.registerOutParameter(4, java.sql.Types.NUMERIC);
        cs.registerOutParameter(5, java.sql.Types.NUMERIC);
        cs.registerOutParameter(6, java.sql.Types.NUMERIC);
        cs.registerOutParameter(7, java.sql.Types.VARCHAR);
        cs.registerOutParameter(8, java.sql.Types.TIMESTAMP);
        cs.execute();
        return new CreateOrderRow(
            cs.getLong(4),
            cs.getLong(5),
            cs.getBigDecimal(6),
            cs.getString(7),
            cs.getObject(8, LocalDateTime.class)
        );
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
            rs.getObject("created_at", LocalDateTime.class)
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

public static @Nullable GetOrderTotalRow getOrderTotal(Connection conn, long user_id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?")) {
        ps.setLong(1, user_id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetOrderTotalRow.fromResultSet(rs);
            }
            return null;
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
    long active,
    java.time.LocalDateTime created_at
) {
    public static GetUserByIdRow fromResultSet(ResultSet rs) throws SQLException {
        return new GetUserByIdRow(
            rs.getLong("id"),
            rs.getString("name"),
            rs.getString("email"),
            rs.getLong("active"),
            rs.getObject("created_at", LocalDateTime.class)
        );
    }
}

public static @Nullable GetUserByIdRow getUserById(Connection conn, long id) throws SQLException {
    try (var ps = conn.prepareStatement("SELECT id, name, email, active, created_at FROM users WHERE id = ?")) {
        ps.setLong(1, id);
        try (ResultSet rs = ps.executeQuery()) {
            if (rs.next()) {
                return GetUserByIdRow.fromResultSet(rs);
            }
            return null;
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
    try (var ps = conn.prepareStatement("SELECT id, name, email FROM users WHERE active = 1")) {
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
    long id,
    String name,
    @Nullable String email,
    long active,
    java.time.LocalDateTime created_at
) {
    public static CreateUserRow fromResultSet(ResultSet rs) throws SQLException {
        return new CreateUserRow(
            rs.getLong("id"),
            rs.getString("name"),
            rs.getString("email"),
            rs.getLong("active"),
            rs.getObject("created_at", LocalDateTime.class)
        );
    }
}

public static @Nullable CreateUserRow createUser(Connection conn, @Nonnull String name, @Nullable String email, long active) throws SQLException {
    try (var cs = conn.prepareCall("BEGIN INSERT INTO users (name, email, active) VALUES (?, ?, ?) RETURNING id, name, email, active, created_at INTO ?, ?, ?, ?, ?; END;")) {
        cs.setString(1, name);
        cs.setString(2, email);
        cs.setLong(3, active);
        cs.registerOutParameter(4, java.sql.Types.NUMERIC);
        cs.registerOutParameter(5, java.sql.Types.VARCHAR);
        cs.registerOutParameter(6, java.sql.Types.VARCHAR);
        cs.registerOutParameter(7, java.sql.Types.NUMERIC);
        cs.registerOutParameter(8, java.sql.Types.TIMESTAMP);
        cs.execute();
        return new CreateUserRow(
            cs.getLong(4),
            cs.getString(5),
            cs.getString(6),
            cs.getLong(7),
            cs.getObject(8, LocalDateTime.class)
        );
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

