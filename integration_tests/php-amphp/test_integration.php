<?php

declare(strict_types=1);

if (file_exists(__DIR__ . '/vendor/autoload.php')) {
    require_once __DIR__ . '/vendor/autoload.php';
}
require_once __DIR__ . '/generated/queries.php';

use App\Generated\Queries;
use App\Generated\RecordNotFoundException;
use App\Generated\UserStatus;
use App\Generated\CreateUserRow;
use App\Generated\GetUserByIdRow;
use App\Generated\ListActiveUsersRow;
use App\Generated\CreateOrderRow;
use App\Generated\GetOrdersByUserRow;

function get_database_url(): string
{
    $url = getenv('DATABASE_URL');
    if ($url === false || $url === '') {
        fwrite(STDERR, "ERROR: DATABASE_URL environment variable is not set\n");
        exit(1);
    }
    return $url;
}

/**
 * Splits a SQL script into statements on top-level ';' only -- unlike a
 * naive `explode(';', $sql)`, this tracks single- and double-quoted spans,
 * PostgreSQL dollar-quoted bodies, and '--' line comments (an apostrophe in
 * a comment must not open a phantom string -- board #224 follow-up) so a
 * ';' inside a string literal, a `$$ ... $$` function body, or a comment
 * does not split the statement in half. '/* ... *' + '/' block comments are
 * not handled -- no schema under integration_tests/sql/ uses them today.
 *
 * @return array<string>
 */
function split_sql_statements(string $sql): array
{
    $statements = [];
    $current = '';
    $inSingle = false;
    $inDouble = false;
    $inLineComment = false;
    $dollarTag = null;
    $length = strlen($sql);
    $i = 0;
    while ($i < $length) {
        $ch = $sql[$i];
        if ($inLineComment) {
            $current .= $ch;
            if ($ch === "\n") {
                $inLineComment = false;
            }
            $i++;
            continue;
        }
        if ($dollarTag !== null) {
            $current .= $ch;
            if ($ch === '$' && substr($sql, $i, strlen($dollarTag)) === $dollarTag) {
                $current .= substr($dollarTag, 1);
                $i += strlen($dollarTag);
                $dollarTag = null;
                continue;
            }
            $i++;
            continue;
        }
        if ($inSingle) {
            $current .= $ch;
            if ($ch === "'") {
                $inSingle = false;
            }
            $i++;
            continue;
        }
        if ($inDouble) {
            $current .= $ch;
            if ($ch === '"') {
                $inDouble = false;
            }
            $i++;
            continue;
        }
        if ($ch === "'") {
            $inSingle = true;
            $current .= $ch;
            $i++;
            continue;
        }
        if ($ch === '"') {
            $inDouble = true;
            $current .= $ch;
            $i++;
            continue;
        }
        if ($ch === '-' && ($sql[$i + 1] ?? '') === '-') {
            $inLineComment = true;
            $current .= $ch;
            $i++;
            continue;
        }
        if ($ch === '$' && preg_match('/\G\$[A-Za-z0-9_]*\$/', $sql, $matches, 0, $i) === 1) {
            $dollarTag = $matches[0];
            $current .= $dollarTag;
            $i += strlen($dollarTag);
            continue;
        }
        if ($ch === ';') {
            $statements[] = $current;
            $current = '';
            $i++;
            continue;
        }
        $current .= $ch;
        $i++;
    }
    if (trim($current) !== '') {
        $statements[] = $current;
    }
    return array_values(array_filter(array_map('trim', $statements), static fn (string $stmt): bool => $stmt !== ''));
}

function parse_database_url(string $url): array
{
    $parts = parse_url($url);
    if ($parts === false) {
        fwrite(STDERR, "ERROR: Invalid DATABASE_URL format\n");
        exit(1);
    }
    return [
        'host' => $parts['host'] ?? 'localhost',
        'port' => $parts['port'] ?? 5432,
        'dbname' => ltrim($parts['path'] ?? '/scythe_test', '/'),
        'user' => $parts['user'] ?? 'scythe',
        'password' => $parts['pass'] ?? 'scythe',
    ];
}

function create_connection(string $url): \Amp\Postgres\PostgresLink
{
    $params = parse_database_url($url);
    $dsn = sprintf(
        "host=%s port=%d dbname=%s user=%s password=%s",
        $params['host'], $params['port'], $params['dbname'],
        $params['user'], $params['password']
    );
    $config = \Amp\Postgres\PostgresConfig::fromString($dsn);
    return new \Amp\Postgres\PostgresConnectionPool($config);
}

function setup_schema($pdo): void
{
    $pdo->query("DROP TABLE IF EXISTS user_tags CASCADE");
    $pdo->query("DROP TABLE IF EXISTS tags CASCADE");
    $pdo->query("DROP TABLE IF EXISTS orders CASCADE");
    $pdo->query("DROP TABLE IF EXISTS users CASCADE");
    $pdo->query("DROP TYPE IF EXISTS user_status CASCADE");
    $pdo->query("DROP TYPE IF EXISTS user_address CASCADE");
    $schema_path = __DIR__ . '/../sql/pg/schema.sql';
    $schema_sql = file_get_contents($schema_path);
    if ($schema_sql === false) {
        throw new RuntimeException("Failed to read schema file: {$schema_path}");
    }
    $pdo->query($schema_sql);
}

function assert_equal(mixed $expected, mixed $actual, string $message): void
{
    if ($expected !== $actual) {
        throw new RuntimeException(
            "Assertion failed: {$message} (expected " . var_export($expected, true)
            . ", got " . var_export($actual, true) . ")"
        );
    }
}

function assert_not_null(mixed $value, string $message): void
{
    if ($value === null) {
        throw new RuntimeException("Assertion failed: {$message} (got null)");
    }
}

function assert_true(bool $value, string $message): void
{
    if (!$value) {
        throw new RuntimeException("Assertion failed: {$message}");
    }
}

function assert_null(mixed $value, string $message): void
{
    if ($value !== null) {
        throw new RuntimeException("Assertion failed: {$message} (expected null, got " . var_export($value, true) . ")");
    }
}



function test_create_user($pdo): int
{
    $user = Queries::createUser($pdo, "Alice", "alice@example.com", UserStatus::ACTIVE);
    assert_not_null($user, "CreateUser returned null");
    assert_equal("Alice", $user->name, "CreateUser name");
    assert_equal("alice@example.com", $user->email, "CreateUser email");
    echo "PASS: CreateUser\n";
    return $user->id;
}

function test_get_user_by_id($pdo, int $user_id): void
{
    $user = Queries::getUserById($pdo, $user_id);
    assert_not_null($user, "GetUserById returned null for id={$user_id}");
    assert_equal("Alice", $user->name, "GetUserById name");
    assert_equal($user_id, $user->id, "GetUserById id");
    echo "PASS: GetUserById\n";
}

function test_list_active_users($pdo): void
{
    $users = iterator_to_array(Queries::listActiveUsers($pdo, UserStatus::ACTIVE));
    assert_true(count($users) >= 1, "Expected at least 1 active user, got " . count($users));
    $names = array_map(fn($u) => $u->name, $users);
    assert_true(in_array("Alice", $names, true), "Expected 'Alice' in active users");
    echo "PASS: ListActiveUsers\n";
}

function test_create_order($pdo, int $user_id): int
{
    $order = Queries::createOrder($pdo, $user_id, "49.99", "Test order");
    assert_not_null($order, "CreateOrder returned null");
    assert_equal($user_id, $order->user_id, "CreateOrder user_id");
    assert_equal("Test order", $order->notes, "CreateOrder notes");
    echo "PASS: CreateOrder\n";
    return $order->id;
}

function test_get_orders_by_user($pdo, int $user_id, int $order_id): void
{
    $orders = iterator_to_array(Queries::getOrdersByUser($pdo, $user_id));
    assert_true(count($orders) >= 1, "Expected at least 1 order, got " . count($orders));
    assert_equal("Test order", $orders[0]->notes, "GetOrdersByUser notes");
    $found_ids = array_map(fn($o) => $o->id, $orders);
    assert_true(in_array($order_id, $found_ids, true), "Expected order $order_id in results, got " . implode(", ", $found_ids));
    echo "PASS: GetOrdersByUser\n";
}
function seed_user_profile_row($pdo, string $sql): int
{
    $result = $pdo->prepare($sql)->execute([]);
    foreach ($result as $row) {
        return (int) $row['id'];
    }
    throw new RuntimeException("seed_user_profile_row: no row returned");
}

function test_get_user_profile($pdo): void
{
    // Test: GetUserProfile (board #197/#204) -- a nullable enum and a nullable
    // composite column, each observed both present and as SQL NULL, plus a
    // composite field containing a double quote and a comma to prove
    // UserAddress::fromText handles record_out's doubled-quote escaping
    // (board #204) rather than truncating on it.
    $present_id = seed_user_profile_row($pdo,
        "INSERT INTO users (name, email, status, secondary_status, address) " .
        "VALUES ('Carol', 'carol@example.com', 'active', 'inactive', " .
        "ROW('1 Main St', 'Springfield', '12345')) RETURNING id");
    $absent_id = seed_user_profile_row($pdo,
        "INSERT INTO users (name, email, status, secondary_status, address) " .
        "VALUES ('Dave', 'dave@example.com', 'active', NULL, NULL) RETURNING id");
    $quoted_id = seed_user_profile_row($pdo,
        "INSERT INTO users (name, email, status, secondary_status, address) " .
        "VALUES ('Eve', 'eve@example.com', 'active', 'inactive', " .
        "ROW('12 \"Main\", Apt 3', 'Berlin', '10115')) RETURNING id");

    $profile = Queries::getUserProfile($pdo, $present_id);
    assert_true($profile->secondary_status === UserStatus::INACTIVE, "GetUserProfile secondary_status present");
    assert_not_null($profile->address, "GetUserProfile address should be present");
    assert_equal("1 Main St", $profile->address->street, "GetUserProfile address.street");
    assert_equal("Springfield", $profile->address->city, "GetUserProfile address.city");
    assert_equal("12345", $profile->address->zip, "GetUserProfile address.zip");

    $null_profile = Queries::getUserProfile($pdo, $absent_id);
    assert_true($null_profile->secondary_status === null, "GetUserProfile secondary_status null");
    assert_true($null_profile->address === null, "GetUserProfile address null");

    $quoted_profile = Queries::getUserProfile($pdo, $quoted_id);
    assert_not_null($quoted_profile->address, "GetUserProfile quoted address should be present");
    assert_equal('12 "Main", Apt 3', $quoted_profile->address->street, "GetUserProfile quoted address.street");
    assert_equal("Berlin", $quoted_profile->address->city, "GetUserProfile quoted address.city");
    assert_equal("10115", $quoted_profile->address->zip, "GetUserProfile quoted address.zip");

    Queries::deleteUser($pdo, $present_id);
    Queries::deleteUser($pdo, $absent_id);
    Queries::deleteUser($pdo, $quoted_id);

    echo "PASS: GetUserProfile\n";
}

function test_round_trip_user_address($pdo): void
{
    $address = new UserAddress('12 "Main", Apt \\3', '', '10115');
    $present = Queries::roundTripUserAddress($pdo, $address);
    assert_not_null($present->address, "RoundTripUserAddress escaped composite");
    assert_equal($address->street, $present->address->street, "RoundTripUserAddress street");
    assert_equal($address->city, $present->address->city, "RoundTripUserAddress empty city");
    assert_equal($address->zip, $present->address->zip, "RoundTripUserAddress zip");
    $absent = Queries::roundTripUserAddress($pdo, null);
    assert_true($absent->address === null, "RoundTripUserAddress whole-composite NULL");
    echo "PASS: RoundTripUserAddress\n";
}

function test_nested_json($pdo): void
{
    $user_id = seed_user_profile_row($pdo,
        "INSERT INTO users (name, email, status, secondary_status, address) " .
        "VALUES ('Nested', 'nested@example.com', 'active', 'inactive', " .
        "ROW('5 JSON Way', 'Berlin', '10115')) RETURNING id");

    $single = Queries::getUserAsJson($pdo, $user_id)->payload;
    assert_equal("Nested", $single->name, "GetUserAsJson name");
    assert_true($single->status === UserStatus::ACTIVE, "GetUserAsJson enum");
    assert_true($single->secondary_status === UserStatus::INACTIVE, "GetUserAsJson nullable enum");
    assert_not_null($single->address, "GetUserAsJson composite");
    assert_equal("Berlin", $single->address->city, "GetUserAsJson composite field");
    assert_true($single->created_at instanceof DateTimeImmutable, "GetUserAsJson datetime");

    $many = Queries::getUsersAsJson($pdo)->payload;
    assert_true(count($many) >= 1, "GetUsersAsJson rows");
    $matches = array_values(array_filter(
        $many,
        static fn ($candidate): bool => $candidate->id === $user_id
    ));
    assert_equal(1, count($matches), "GetUsersAsJson seeded row count");
    $row = $matches[0];
    assert_equal("Nested", $row->name, "GetUsersAsJson aggregate name");
    assert_true($row->status === UserStatus::ACTIVE, "GetUsersAsJson aggregate enum");
    assert_true($row->secondary_status === UserStatus::INACTIVE, "GetUsersAsJson aggregate nullable enum");
    assert_not_null($row->address, "GetUsersAsJson aggregate composite");
    assert_equal("5 JSON Way", $row->address->street, "GetUsersAsJson aggregate address.street");
    assert_equal("Berlin", $row->address->city, "GetUsersAsJson aggregate address.city");
    assert_equal("10115", $row->address->zip, "GetUsersAsJson aggregate address.zip");
    assert_true($row->created_at instanceof DateTimeImmutable, "GetUsersAsJson aggregate datetime");

    $orders = Queries::getUserOrdersAsJson($pdo, $user_id)->payload;
    assert_equal(1, count($orders), "GetUserOrdersAsJson null element count");
    assert_null($orders[0], "GetUserOrdersAsJson nullable element");

    $pdo->query("ALTER TABLE orders ALTER COLUMN total TYPE NUMERIC(30, 15)");
    Queries::createOrder($pdo, $user_id, "123456789012345.123456789012345", "Nested numeric precision");
    $orders = Queries::getUserOrdersAsJson($pdo, $user_id)->payload;
    assert_equal(1, count($orders), "GetUserOrdersAsJson numeric row count");
    assert_true(is_float($orders[0]->total), "GetUserOrdersAsJson decimal is an honest PHP float");
    assert_equal(123456789012345.12, $orders[0]->total, "GetUserOrdersAsJson follows PHP float precision");

    Queries::deleteOrdersByUser($pdo, $user_id);
    Queries::deleteUser($pdo, $user_id);
    echo "PASS: NestedJson\n";
}

function test_empty_nested_json($pdo): void
{
    $pdo->query("DELETE FROM users");
    $empty = Queries::getUsersAsJson($pdo);
    assert_null($empty->payload, "GetUsersAsJson empty aggregate");
    echo "PASS: EmptyNestedJson\n";
}

function test_delete_user($pdo, int $user_id): void
{
    // Delete orders first due to FK constraint
    Queries::deleteOrdersByUser($pdo, $user_id);
    Queries::deleteUser($pdo, $user_id);
    // getUserById is `:one`, so a missing row throws RecordNotFoundException rather than
    // returning null.
    try {
        Queries::getUserById($pdo, $user_id);
        throw new RuntimeException("Expected getUserById to throw RecordNotFoundException, but it returned a row");
    } catch (RecordNotFoundException) {
        // expected: the user was deleted
    }
    echo "PASS: DeleteUser\n";
}

try {
    $database_url = get_database_url();
    $pdo = create_connection($database_url);

    setup_schema($pdo);

    $user_id = test_create_user($pdo);
    test_get_user_by_id($pdo, $user_id);
    test_list_active_users($pdo);
    $order_id = test_create_order($pdo, $user_id);
    test_get_orders_by_user($pdo, $user_id, $order_id);
    test_get_user_profile($pdo);
    test_round_trip_user_address($pdo);
    test_nested_json($pdo);
    test_delete_user($pdo, $user_id);
    test_empty_nested_json($pdo);

    echo "\nALL TESTS PASSED\n";
    exit(0);
} catch (Throwable $e) {
    fwrite(STDERR, "FAIL: " . $e->getMessage() . "\n");
    fwrite(STDERR, $e->getTraceAsString() . "\n");
    exit(1);
}
