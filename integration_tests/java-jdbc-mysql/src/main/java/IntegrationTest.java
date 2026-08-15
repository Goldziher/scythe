import generated.Queries;
import generated.Queries.UsersStatus;

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
        String mysqlUrl = System.getenv("MYSQL_URL");
        if (mysqlUrl == null || mysqlUrl.isEmpty()) {
            System.err.println("MYSQL_URL environment variable is required");
            System.exit(1);
        }

        // Convert mysql://user:pass@host:port/db to JDBC format
        java.net.URI uri = new java.net.URI(mysqlUrl);
        String userInfo = uri.getUserInfo();
        String user = userInfo != null ? userInfo.split(":")[0] : "";
        String password = userInfo != null && userInfo.contains(":") ? userInfo.split(":")[1] : "";
        String jdbcUrl = "jdbc:mysql://" + uri.getHost() + ":" + uri.getPort() + uri.getPath();

        try (Connection conn = DriverManager.getConnection(jdbcUrl, user, password)) {
            runMigration(conn);

            testCreateUser(conn);
            testGetUserById(conn);
            testUpdateUserEmail(conn);
            testListActiveUsers(conn);
            testSearchUsers(conn);
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

    // Splits a SQL script into statements on top-level ';' only -- unlike a
    // naive `schema.split(";")`, this tracks single- and double-quoted
    // spans, PostgreSQL dollar-quoted bodies, and "--" line comments (an
    // apostrophe in a comment must not open a phantom string -- board
    // #224 follow-up) so a ';' inside a string literal, a `$$ ... $$`
    // function body, or a comment does not split the statement in half.
    // "/* ... */" block comments are not handled -- no schema under
    // integration_tests/sql/ uses them today.
    private static java.util.List<String> splitSqlStatements(String sql) {
        java.util.List<String> statements = new java.util.ArrayList<>();
        StringBuilder current = new StringBuilder();
        boolean inSingle = false;
        boolean inDouble = false;
        boolean inLineComment = false;
        String dollarTag = null;
        int i = 0;
        while (i < sql.length()) {
            char ch = sql.charAt(i);
            if (inLineComment) {
                current.append(ch);
                if (ch == '\n') inLineComment = false;
                i++;
                continue;
            }
            if (dollarTag != null) {
                current.append(ch);
                if (ch == '$' && sql.regionMatches(i, dollarTag, 0, dollarTag.length())) {
                    current.append(dollarTag.substring(1));
                    i += dollarTag.length();
                    dollarTag = null;
                    continue;
                }
                i++;
                continue;
            }
            if (inSingle) {
                current.append(ch);
                if (ch == '\'') inSingle = false;
                i++;
                continue;
            }
            if (inDouble) {
                current.append(ch);
                if (ch == '"') inDouble = false;
                i++;
                continue;
            }
            if (ch == '-' && i + 1 < sql.length() && sql.charAt(i + 1) == '-') {
                inLineComment = true;
                current.append(ch);
                i++;
                continue;
            }
            if (ch == '\'') {
                inSingle = true;
                current.append(ch);
                i++;
                continue;
            }
            if (ch == '"') {
                inDouble = true;
                current.append(ch);
                i++;
                continue;
            }
            if (ch == '$') {
                java.util.regex.Matcher matcher =
                    java.util.regex.Pattern.compile("^\\$[A-Za-z0-9_]*\\$").matcher(sql.substring(i));
                if (matcher.find()) {
                    dollarTag = matcher.group();
                    current.append(dollarTag);
                    i += dollarTag.length();
                    continue;
                }
            }
            if (ch == ';') {
                statements.add(current.toString());
                current.setLength(0);
                i++;
                continue;
            }
            current.append(ch);
            i++;
        }
        if (!current.toString().trim().isEmpty()) {
            statements.add(current.toString());
        }
        java.util.List<String> trimmed = new java.util.ArrayList<>();
        for (String s : statements) {
            String t = s.trim();
            if (!t.isEmpty()) trimmed.add(t);
        }
        return trimmed;
    }

    private static void runMigration(Connection conn) throws Exception {
        Path schemaPath = Path.of(System.getProperty("user.dir"))
            .resolve("../sql/mysql/schema.sql")
            .normalize();
        String schema = Files.readString(schemaPath);

        try (var stmt = conn.createStatement()) {
            stmt.execute("DROP TABLE IF EXISTS user_tags");
            stmt.execute("DROP TABLE IF EXISTS tags");
            stmt.execute("DROP TABLE IF EXISTS orders");
            stmt.execute("DROP TABLE IF EXISTS users");
        }

        // MySQL requires executing statements one at a time
        for (String sql : splitSqlStatements(schema)) {
            try (var stmt = conn.createStatement()) {
                stmt.execute(sql);
            }
        }
    }

    private static int createdUserId;

    private static void testCreateUser(Connection conn) {
        String name = "CreateUser";
        try {
            Queries.createUser(conn, "Alice", "alice@example.com", UsersStatus.ACTIVE);
            var user = Queries.getLastInsertUser(conn);
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

    private static void testUpdateUserEmail(Connection conn) {
        String name = "UpdateUserEmail";
        try {
            Queries.updateUserEmail(conn, "alice-updated@example.com", createdUserId);
            var user = Queries.getUserById(conn, createdUserId);
            if (user == null) {
                fail(name, "user not found after update");
                return;
            }
            if (!"alice-updated@example.com".equals(user.email())) {
                fail(name, "expected updated email, got " + user.email());
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
            var users = Queries.listActiveUsers(conn, UsersStatus.ACTIVE);
            if (users.isEmpty()) {
                fail(name, "expected at least 1 active user");
                return;
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }

    private static void testSearchUsers(Connection conn) {
        String name = "SearchUsers";
        try {
            var users = Queries.searchUsers(conn, "%Alice%");
            if (users.isEmpty()) {
                fail(name, "expected at least 1 user matching Alice");
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
            var order = Queries.getLastInsertOrder(conn);
            if (order == null) {
                fail(name, "returned null");
                return;
            }
            if (order.user_id() != createdUserId) {
                fail(name, "expected user_id " + createdUserId + ", got " + order.user_id());
                return;
            }
            if (order.total().compareTo(new BigDecimal("99.99")) != 0) {
                fail(name, "expected total 99.99, got " + order.total());
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
            if (result == null || result.total_sum() == null) {
                fail(name, "returned null");
                return;
            }
            if (result.total_sum().compareTo(new BigDecimal("99.99")) != 0) {
                fail(name, "expected total_sum 99.99, got " + result.total_sum());
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
            // GetUserById is `:one`, so a missing row throws
            // NoSuchElementException rather than returning null.
            try {
                Queries.getUserById(conn, createdUserId);
                fail(name, "expected getUserById to throw after deletion, but it returned a row");
                return;
            } catch (java.util.NoSuchElementException expected) {
                // expected: the user was deleted
            }
            pass(name);
        } catch (Exception e) {
            fail(name, e);
        }
    }
}
