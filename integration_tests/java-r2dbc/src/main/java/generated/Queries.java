// scythe:provenance v=0.16.1 backend=java-r2dbc engine=postgresql schema=sch1:c247390d575b8f71 queries=q1:a78685f58b075ff5 options=opt1:cbf29ce484222325
package generated;

import io.r2dbc.spi.ConnectionFactory;
import io.r2dbc.spi.Row;
import io.r2dbc.spi.RowMetadata;
import io.r2dbc.spi.Statement;
import java.math.BigDecimal;
import java.time.LocalDate;
import java.time.LocalTime;
import java.time.OffsetDateTime;
import java.util.UUID;
import javax.annotation.Nonnull;
import javax.annotation.Nullable;
import reactor.core.publisher.Flux;
import reactor.core.publisher.Mono;

public class Queries {

public enum UserStatus {
    ACTIVE("active"),
    INACTIVE("inactive"),
    BANNED("banned");

    private final String value;
    UserStatus(String value) { this.value = value; }
    public String getValue() { return value; }

    public static UserStatus fromValue(String value) {
        for (UserStatus v : values()) {
            if (v.value.equals(value)) {
                return v;
            }
        }
        throw new IllegalArgumentException("Unknown UserStatus value: " + value);
    }
}

public record CreateOrderRow(
    int id,
    int user_id,
    java.math.BigDecimal total,
    @Nullable String notes,
    java.time.OffsetDateTime created_at
) {}

public record GetOrdersByUserRow(
    int id,
    java.math.BigDecimal total,
    @Nullable String notes,
    java.time.OffsetDateTime created_at
) {}

public record GetOrderTotalRow(
    @Nullable java.math.BigDecimal total_sum
) {}

public record GetOrderWeightTotalRow(
    @Nullable Double weight_total
) {}

public record GetUserByIdRow(
    int id,
    String name,
    @Nullable String email,
    UserStatus status,
    java.time.OffsetDateTime created_at
) {}

public record ListActiveUsersRow(
    int id,
    String name,
    @Nullable String email
) {}

public record CreateUserRow(
    int id,
    String name,
    @Nullable String email,
    UserStatus status,
    java.time.OffsetDateTime created_at
) {}

public record GetUserOrdersRow(
    int id,
    String name,
    @Nullable java.math.BigDecimal total,
    @Nullable String notes
) {}

public record CountUsersByStatusRow(
    UserStatus status,
    long user_count
) {}

public record GetUserWithTagsRow(
    int id,
    String name,
    String tag_name
) {}

public record SearchUsersRow(
    int id,
    String name,
    @Nullable String email
) {}

public record UserAddress(String street, String city, String zip) {

    /**
     * ~keep board #196: r2dbc-postgresql has no codec for this composite -- an unregistered
     * `row.get(col, UserAddress.class)` is driver-codec-dependent and throws at runtime, the
     * same problem an enum column has. Parse the driver's text form instead.
     */
    public static UserAddress fromText(String text) {
        if (text == null) {
            return null;
        }
        java.util.List<String> f = parseCompositeFields(text);
        return new UserAddress(
            f.get(0),
            f.get(1),
            f.get(2)
        );
    }

    /**
     * ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
     * honoring its escaping rules: an empty unquoted field is SQL NULL (returned as `null`); a
     * field needing quoting (containing a comma, paren, quote, backslash, or leading/trailing
     * space, or the empty string) is wrapped in double quotes with `"` and `\` backslash-escaped
     * inside; every other field is unquoted and taken literally. A nested composite's own
     * "(x,y)" text form always contains parens, so it always comes back quoted here, ready for
     * that type's own `fromText` to parse recursively.
     */
    private static java.util.List<String> parseCompositeFields(String text) {
        java.util.List<String> fields = new java.util.ArrayList<>();
        String inner = text.substring(1, text.length() - 1);
        int i = 0;
        int n = inner.length();
        while (true) {
            StringBuilder field = new StringBuilder();
            boolean isNull = false;
            if (i < n && inner.charAt(i) == '"') {
                i++;
                while (i < n) {
                    char c = inner.charAt(i);
                    if (c == '\\' && i + 1 < n) {
                        field.append(inner.charAt(i + 1));
                        i += 2;
                    } else if (c == '"' && i + 1 < n && inner.charAt(i + 1) == '"') {
                        field.append('"');
                        i += 2;
                    } else if (c == '"') {
                        i++;
                        break;
                    } else {
                        field.append(c);
                        i++;
                    }
                }
            } else {
                int start = i;
                while (i < n && inner.charAt(i) != ',') {
                    i++;
                }
                field.append(inner, start, i);
                isNull = field.length() == 0;
            }
            fields.add(isNull ? null : field.toString());
            if (i < n && inner.charAt(i) == ',') {
                i++;
                continue;
            }
            break;
        }
        return fields;
    }
}

public record GetUserProfileRow(
    int id,
    @Nullable UserStatus secondary_status,
    @Nullable UserAddress address
) {}

    private static void bindNullable(Statement stmt, int index, Object value, Class<?> type) {
        if (value == null) {
            stmt.bindNull(index, type);
        } else {
            stmt.bind(index, value);
        }
    }

public static Mono<CreateOrderRow> createOrder(ConnectionFactory cf, int user_id, @Nonnull java.math.BigDecimal total, @Nullable String notes) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at");
            stmt.bind(0, user_id);
            stmt.bind(1, total);
            bindNullable(stmt, 2, notes, String.class);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.map((row, meta) ->
                    new CreateOrderRow(
                        row.get("id", Integer.class),
                        row.get("user_id", Integer.class),
                        row.get("total", java.math.BigDecimal.class),
                        row.get("notes", String.class),
                        row.get("created_at", java.time.OffsetDateTime.class)
                    ))))
                .switchIfEmpty(Mono.error(new java.util.NoSuchElementException("createOrder: no rows returned")));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Flux<GetOrdersByUserRow> getOrdersByUser(ConnectionFactory cf, int user_id) {
    return Flux.usingWhen(
        cf.create(),
        conn -> {
            var stmt = conn.createStatement("SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC");
            stmt.bind(0, user_id);
            return Flux.from(stmt.execute())
                .flatMap(result -> result.map((row, meta) ->
                    new GetOrdersByUserRow(
                        row.get("id", Integer.class),
                        row.get("total", java.math.BigDecimal.class),
                        row.get("notes", String.class),
                        row.get("created_at", java.time.OffsetDateTime.class)
                    )));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<GetOrderTotalRow> getOrderTotal(ConnectionFactory cf, int user_id) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1");
            stmt.bind(0, user_id);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.map((row, meta) ->
                    new GetOrderTotalRow(
                        row.get("total_sum", java.math.BigDecimal.class)
                    ))))
                .switchIfEmpty(Mono.error(new java.util.NoSuchElementException("getOrderTotal: no rows returned")));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<GetOrderWeightTotalRow> getOrderWeightTotal(ConnectionFactory cf, int user_id) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = $1");
            stmt.bind(0, user_id);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.map((row, meta) ->
                    new GetOrderWeightTotalRow(
                        row.get("weight_total", Double.class)
                    ))))
                .switchIfEmpty(Mono.error(new java.util.NoSuchElementException("getOrderWeightTotal: no rows returned")));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<Long> deleteOrdersByUser(ConnectionFactory cf, int user_id) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("DELETE FROM orders WHERE user_id = $1");
            stmt.bind(0, user_id);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.getRowsUpdated()));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<GetUserByIdRow> getUserById(ConnectionFactory cf, int id) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("SELECT id, name, email, status, created_at FROM users WHERE id = $1");
            stmt.bind(0, id);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.map((row, meta) ->
                    new GetUserByIdRow(
                        row.get("id", Integer.class),
                        row.get("name", String.class),
                        row.get("email", String.class),
                        UserStatus.fromValue(row.get("status", String.class)),
                        row.get("created_at", java.time.OffsetDateTime.class)
                    ))))
                .switchIfEmpty(Mono.error(new java.util.NoSuchElementException("getUserById: no rows returned")));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Flux<ListActiveUsersRow> listActiveUsers(ConnectionFactory cf, @Nonnull UserStatus status) {
    return Flux.usingWhen(
        cf.create(),
        conn -> {
            var stmt = conn.createStatement("SELECT id, name, email FROM users WHERE status = $1::user_status");
            stmt.bind(0, status.getValue());
            return Flux.from(stmt.execute())
                .flatMap(result -> result.map((row, meta) ->
                    new ListActiveUsersRow(
                        row.get("id", Integer.class),
                        row.get("name", String.class),
                        row.get("email", String.class)
                    )));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<CreateUserRow> createUser(ConnectionFactory cf, @Nonnull String name, @Nullable String email, @Nonnull UserStatus status) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("INSERT INTO users (name, email, status) VALUES ($1, $2, $3::user_status) RETURNING id, name, email, status, created_at");
            stmt.bind(0, name);
            bindNullable(stmt, 1, email, String.class);
            stmt.bind(2, status.getValue());
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.map((row, meta) ->
                    new CreateUserRow(
                        row.get("id", Integer.class),
                        row.get("name", String.class),
                        row.get("email", String.class),
                        UserStatus.fromValue(row.get("status", String.class)),
                        row.get("created_at", java.time.OffsetDateTime.class)
                    ))))
                .switchIfEmpty(Mono.error(new java.util.NoSuchElementException("createUser: no rows returned")));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<Void> updateUserEmail(ConnectionFactory cf, @Nonnull String email, int id) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("UPDATE users SET email = $1 WHERE id = $2");
            stmt.bind(0, email);
            stmt.bind(1, id);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.getRowsUpdated()))
                .then();
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<Void> deleteUser(ConnectionFactory cf, int id) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("DELETE FROM users WHERE id = $1");
            stmt.bind(0, id);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.getRowsUpdated()))
                .then();
        },
        conn -> Mono.from(conn.close())
    );
}

public static Flux<GetUserOrdersRow> getUserOrders(ConnectionFactory cf, @Nonnull UserStatus status) {
    return Flux.usingWhen(
        cf.create(),
        conn -> {
            var stmt = conn.createStatement("SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = $1::user_status");
            stmt.bind(0, status.getValue());
            return Flux.from(stmt.execute())
                .flatMap(result -> result.map((row, meta) ->
                    new GetUserOrdersRow(
                        row.get("id", Integer.class),
                        row.get("name", String.class),
                        row.get("total", java.math.BigDecimal.class),
                        row.get("notes", String.class)
                    )));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<CountUsersByStatusRow> countUsersByStatus(ConnectionFactory cf, @Nonnull UserStatus status) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1::user_status");
            stmt.bind(0, status.getValue());
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.map((row, meta) ->
                    new CountUsersByStatusRow(
                        UserStatus.fromValue(row.get("status", String.class)),
                        row.get("user_count", Long.class)
                    ))))
                .switchIfEmpty(Mono.error(new java.util.NoSuchElementException("countUsersByStatus: no rows returned")));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Flux<GetUserWithTagsRow> getUserWithTags(ConnectionFactory cf, int id) {
    return Flux.usingWhen(
        cf.create(),
        conn -> {
            var stmt = conn.createStatement("SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = $1");
            stmt.bind(0, id);
            return Flux.from(stmt.execute())
                .flatMap(result -> result.map((row, meta) ->
                    new GetUserWithTagsRow(
                        row.get("id", Integer.class),
                        row.get("name", String.class),
                        row.get("tag_name", String.class)
                    )));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Flux<SearchUsersRow> searchUsers(ConnectionFactory cf, @Nonnull String name) {
    return Flux.usingWhen(
        cf.create(),
        conn -> {
            var stmt = conn.createStatement("SELECT id, name, email FROM users WHERE name LIKE $1");
            stmt.bind(0, name);
            return Flux.from(stmt.execute())
                .flatMap(result -> result.map((row, meta) ->
                    new SearchUsersRow(
                        row.get("id", Integer.class),
                        row.get("name", String.class),
                        row.get("email", String.class)
                    )));
        },
        conn -> Mono.from(conn.close())
    );
}

public static Mono<GetUserProfileRow> getUserProfile(ConnectionFactory cf, int id) {
    return Mono.usingWhen(
        Mono.from(cf.create()),
        conn -> {
            var stmt = conn.createStatement("SELECT id, secondary_status, address FROM users WHERE id = $1");
            stmt.bind(0, id);
            return Mono.from(stmt.execute())
                .flatMap(result -> Mono.from(result.map((row, meta) ->
                    new GetUserProfileRow(
                        row.get("id", Integer.class),
                        UserStatus.fromValue(row.get("secondary_status", String.class)),
                        UserAddress.fromText(row.get("address", String.class))
                    ))))
                .switchIfEmpty(Mono.error(new java.util.NoSuchElementException("getUserProfile: no rows returned")));
        },
        conn -> Mono.from(conn.close())
    );
}

}

