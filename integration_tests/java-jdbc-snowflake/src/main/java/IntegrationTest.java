import generated.Queries;

import java.math.BigDecimal;
import java.nio.file.Files;
import java.nio.file.Path;
import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.SQLException;

public class IntegrationTest {

    private static int passed = 0;
    private static int failed = 0;

    private static void pass(String name) {
        System.out.println("PASS: " + name);
        passed++;
    }

    private static void fail(String name, Exception e) {
        System.out.println("FAIL: " + name + " - " + e.getMessage());
        failed++;
    }

    private static void fail(String name, String message) {
        System.out.println("FAIL: " + name + " - " + message);
        failed++;
    }

    public static void main(String[] args) throws Exception {
        String snowflakeUrl = System.getenv("SNOWFLAKE_URL");
        if (snowflakeUrl == null || snowflakeUrl.isEmpty()) {
            System.err.println("SNOWFLAKE_URL environment variable is required");
            System.exit(1);
        }

        // Parse snowflake://user:pass@host:port/database/schema?account=X
        java.net.URI uri = new java.net.URI(snowflakeUrl);
        String userInfo = uri.getUserInfo();
        String user = userInfo != null ? userInfo.split(":")[0] : "";
        String password = userInfo != null && userInfo.contains(":") ? userInfo.split(":")[1] : "";

        String[] pathParts = uri.getPath().split("/");
        String database = pathParts.length > 1 ? pathParts[1] : "";
        String schema = pathParts.length > 2 ? pathParts[2] : "";

        // Parse account from query params
        String account = "";
        String query = uri.getQuery();
        if (query != null) {
            for (String param : query.split("&")) {
                if (param.startsWith("account=")) {
                    account = param.substring("account=".length());
                    break;
                }
            }
        }

        String jdbcUrl = "jdbc:snowflake://" + uri.getHost() + ":" + uri.getPort()
            + "/?account=" + account + "&db=" + database + "&schema=" + schema
            + "&user=" + user + "&password=" + password;

        try (Connection conn = DriverManager.getConnection(jdbcUrl)) {
            runMigration(conn);

            testCreateUser(conn);
            testGetUserById(conn);
            testListActiveUsers(conn);
            testCreateOrder(conn);
            testGetOrdersByUser(conn);
            testDeleteOrdersByUser(conn);
            testDeleteUser(conn);
        }

        System.out.println();
        System.out.println("Results: " + passed + " passed, " + failed + " failed");
        if (failed > 0) {
            System.exit(1);
        }
        System.out.println("ALL TESTS PASSED");
    }

    private static void runMigration(Connection conn) throws Exception {
        Path schemaPath = Path.of(System.getProperty("user.dir"))
            .resolve("../sql/snowflake/schema.sql")
            .normalize();
        String schema = Files.readString(schemaPath);

        try (var stmt = conn.createStatement()) {
            stmt.execute("DROP TABLE IF EXISTS user_tags");
            stmt.execute("DROP TABLE IF EXISTS tags");
            stmt.execute("DROP TABLE IF EXISTS orders");
            stmt.execute("DROP TABLE IF EXISTS users");
        }

        // Snowflake requires executing statements one at a time
        for (String sql : schema.split(";")) {
            String trimmed = sql.trim();
            if (!trimmed.isEmpty()) {
                try (var stmt = conn.createStatement()) {
                    stmt.execute(trimmed);
                }
            }
        }
    }

    private static long createdUserId;

    private static void testCreateUser(Connection conn) {
        String name = "CreateUser";
        try {
            Queries.createUser(conn, "Alice", "alice@example.com");
            var user = Queries.getUserById(conn, 1L);
            if (user == null) {
                fail(name, "returned null");
                return;
            }
            if (!"Alice".equals(user.name())) {
                fail(name, "expected name Alice, got " + user.name());
                return;
            }
            if (!"alice@example.com".equals(user.email())) {
                fail(name, "expected email alice@example.com, got " + user.email());
                return;
            }
            createdUserId = 1L;
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testGetUserById(Connection conn) {
        String name = "GetUserById";
        try {
            var user = Queries.getUserById(conn, createdUserId);
            if (user == null) {
                fail(name, "returned null");
                return;
            }
            if (!"Alice".equals(user.name())) {
                fail(name, "expected name Alice, got " + user.name());
                return;
            }
            if (user.id() != createdUserId) {
                fail(name, "expected id " + createdUserId + ", got " + user.id());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testListActiveUsers(Connection conn) {
        String name = "ListActiveUsers";
        try {
            var users = Queries.listActiveUsers(conn);
            if (users.isEmpty()) {
                fail(name, "expected at least 1 active user");
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testCreateOrder(Connection conn) {
        String name = "CreateOrder";
        try {
            Queries.createOrder(conn, createdUserId, new BigDecimal("99.99"), "Test order");
            var order = Queries.getOrdersByUser(conn, createdUserId);
            if (order.isEmpty()) {
                fail(name, "returned no orders");
                return;
            }
            var o = order.get(0);
            if (o.user_id() != createdUserId) {
                fail(name, "expected user_id " + createdUserId + ", got " + o.user_id());
                return;
            }
            if (o.total().compareTo(new BigDecimal("99.99")) != 0) {
                fail(name, "expected total 99.99, got " + o.total());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testGetOrdersByUser(Connection conn) {
        String name = "GetOrdersByUser";
        try {
            var orders = Queries.getOrdersByUser(conn, createdUserId);
            if (orders.size() != 1) {
                fail(name, "expected 1 order, got " + orders.size());
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testDeleteOrdersByUser(Connection conn) {
        String name = "DeleteOrdersByUser";
        try {
            int count = Queries.deleteOrdersByUser(conn, createdUserId);
            if (count != 1) {
                fail(name, "expected 1 deleted order, got " + count);
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testDeleteUser(Connection conn) {
        String name = "DeleteUser";
        try {
            Queries.deleteUser(conn, createdUserId);
            var user = Queries.getUserById(conn, createdUserId);
            if (user != null) {
                fail(name, "expected null after deletion, but user still exists");
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }
}
