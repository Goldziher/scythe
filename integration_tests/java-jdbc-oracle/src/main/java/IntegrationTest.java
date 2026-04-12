import generated.Queries;

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
        String oracleUrl = System.getenv("ORACLE_URL");
        if (oracleUrl == null || oracleUrl.isEmpty()) {
            System.err.println("ORACLE_URL environment variable is required");
            System.exit(1);
        }

        // Convert oracle://user:pass@host:port/service to JDBC thin format
        java.net.URI uri = new java.net.URI(oracleUrl);
        String userInfo = uri.getUserInfo();
        String user = userInfo != null ? userInfo.split(":")[0] : "";
        String password = userInfo != null && userInfo.contains(":") ? userInfo.split(":")[1] : "";
        String jdbcUrl = "jdbc:oracle:thin:@" + uri.getHost() + ":" + uri.getPort() + uri.getPath();

        try (Connection conn = DriverManager.getConnection(jdbcUrl, user, password)) {
            runMigration(conn);

            testCreateUser(conn);
            testGetUserById(conn);
            testListActiveUsers(conn);
            testCreateOrder(conn);
            testGetOrdersByUser(conn);
            testGetOrderTotal(conn);
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
        // Drop tables and sequences, ignoring errors if they do not exist
        String[] drops = {
            "DROP TABLE user_tags",
            "DROP TABLE tags",
            "DROP TABLE orders",
            "DROP TABLE users",
            "DROP SEQUENCE tags_seq",
            "DROP SEQUENCE orders_seq",
            "DROP SEQUENCE users_seq"
        };
        for (String drop : drops) {
            try (var stmt = conn.createStatement()) {
                stmt.execute(drop);
            } catch (SQLException ignored) {
                // object may not exist
            }
        }

        Path schemaPath = Path.of(System.getProperty("user.dir"))
            .resolve("../sql/oracle/schema_full.sql")
            .normalize();
        String schema = Files.readString(schemaPath);

        // Oracle PL/SQL blocks are delimited by /\n
        for (String block : schema.split("/\n")) {
            String trimmed = block.trim();
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
            var user = Queries.createUser(conn, "Alice", "alice@example.com", 1L);
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
            createdUserId = user.id();
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
            var order = Queries.createOrder(conn, createdUserId, 9999L, "Test order");
            if (order == null) {
                fail(name, "returned null");
                return;
            }
            if (order.user_id() != createdUserId) {
                fail(name, "expected user_id " + createdUserId + ", got " + order.user_id());
                return;
            }
            if (order.total() != 9999L) {
                fail(name, "expected total 9999, got " + order.total());
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

    private static void testGetOrderTotal(Connection conn) {
        String name = "GetOrderTotal";
        try {
            var result = Queries.getOrderTotal(conn, createdUserId);
            if (result == null) {
                fail(name, "returned null");
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
