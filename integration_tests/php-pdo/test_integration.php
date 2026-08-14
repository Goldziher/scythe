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

function create_pdo(string $url): PDO
{
    $params = parse_database_url($url);
    $dsn = sprintf(
        'pgsql:host=%s;port=%d;dbname=%s',
        $params['host'],
        $params['port'],
        $params['dbname']
    );
    $pdo = new PDO($dsn, $params['user'], $params['password'], [
        PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
        PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC,
    ]);
    return $pdo;
}

function setup_schema(PDO $pdo): void
{
    $pdo->exec("DROP TABLE IF EXISTS user_tags CASCADE");
    $pdo->exec("DROP TABLE IF EXISTS tags CASCADE");
    $pdo->exec("DROP TABLE IF EXISTS orders CASCADE");
    $pdo->exec("DROP TABLE IF EXISTS users CASCADE");
    $pdo->exec("DROP TYPE IF EXISTS user_status CASCADE");
    $pdo->exec("DROP TYPE IF EXISTS user_address CASCADE");
    $schema_path = __DIR__ . '/../sql/pg/schema.sql';
    $schema_sql = file_get_contents($schema_path);
    if ($schema_sql === false) {
        throw new RuntimeException("Failed to read schema file: {$schema_path}");
    }
    $pdo->exec($schema_sql);
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



function test_create_user(PDO $pdo): int
{
    $user = Queries::createUser($pdo, "Alice", "alice@example.com", UserStatus::ACTIVE);
    assert_not_null($user, "CreateUser returned null");
    assert_equal("Alice", $user->name, "CreateUser name");
    assert_equal("alice@example.com", $user->email, "CreateUser email");
    echo "PASS: CreateUser\n";
    return $user->id;
}

function test_get_user_by_id(PDO $pdo, int $user_id): void
{
    $user = Queries::getUserById($pdo, $user_id);
    assert_not_null($user, "GetUserById returned null for id={$user_id}");
    assert_equal("Alice", $user->name, "GetUserById name");
    assert_equal($user_id, $user->id, "GetUserById id");
    echo "PASS: GetUserById\n";
}

function test_list_active_users(PDO $pdo): void
{
    $users = iterator_to_array(Queries::listActiveUsers($pdo, UserStatus::ACTIVE));
    assert_true(count($users) >= 1, "Expected at least 1 active user, got " . count($users));
    $names = array_map(fn($u) => $u->name, $users);
    assert_true(in_array("Alice", $names, true), "Expected 'Alice' in active users");
    echo "PASS: ListActiveUsers\n";
}

function test_create_order(PDO $pdo, int $user_id): int
{
    $order = Queries::createOrder($pdo, $user_id, "49.99", "Test order");
    assert_not_null($order, "CreateOrder returned null");
    assert_equal($user_id, $order->user_id, "CreateOrder user_id");
    assert_equal("Test order", $order->notes, "CreateOrder notes");
    echo "PASS: CreateOrder\n";
    return $order->id;
}

function test_get_orders_by_user(PDO $pdo, int $user_id): void
{
    $orders = iterator_to_array(Queries::getOrdersByUser($pdo, $user_id));
    assert_true(count($orders) >= 1, "Expected at least 1 order, got " . count($orders));
    assert_equal("Test order", $orders[0]->notes, "GetOrdersByUser notes");
    echo "PASS: GetOrdersByUser\n";
}
function seed_user_profile_row(PDO $pdo, string $sql): int
{
    $stmt = $pdo->query($sql);
    return (int) $stmt->fetchColumn();
}

function test_get_user_profile(PDO $pdo): void
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

function test_delete_user(PDO $pdo, int $user_id): void
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
    $pdo = create_pdo($database_url);

    setup_schema($pdo);

    $user_id = test_create_user($pdo);
    test_get_user_by_id($pdo, $user_id);
    test_list_active_users($pdo);
    $order_id = test_create_order($pdo, $user_id);
    test_get_orders_by_user($pdo, $user_id);
    test_get_user_profile($pdo);
    test_delete_user($pdo, $user_id);

    echo "\nALL TESTS PASSED\n";
    exit(0);
} catch (Throwable $e) {
    fwrite(STDERR, "FAIL: " . $e->getMessage() . "\n");
    fwrite(STDERR, $e->getTraceAsString() . "\n");
    exit(1);
}
