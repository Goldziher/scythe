<?php

declare(strict_types=1);

require_once __DIR__ . '/generated/queries.php';

function get_mysql_url(): string
{
    $url = getenv('MYSQL_URL');
    if ($url === false || $url === '') {
        fwrite(STDERR, "ERROR: MYSQL_URL environment variable is not set\n");
        exit(1);
    }
    return $url;
}

function parse_mysql_url(string $url): array
{
    $parts = parse_url($url);
    if ($parts === false) {
        fwrite(STDERR, "ERROR: Invalid MYSQL_URL format\n");
        exit(1);
    }
    return [
        'host' => $parts['host'] ?? 'localhost',
        'port' => $parts['port'] ?? 3306,
        'dbname' => ltrim($parts['path'] ?? '/scythe_test', '/'),
        'user' => $parts['user'] ?? 'root',
        'password' => $parts['pass'] ?? '',
    ];
}

function create_pdo(string $url): PDO
{
    $params = parse_mysql_url($url);
    $dsn = sprintf(
        'mysql:host=%s;port=%d;dbname=%s',
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
    $pdo->exec("DROP TABLE IF EXISTS user_tags");
    $pdo->exec("DROP TABLE IF EXISTS tags");
    $pdo->exec("DROP TABLE IF EXISTS orders");
    $pdo->exec("DROP TABLE IF EXISTS users");
    $schema_path = __DIR__ . '/../sql/mysql/schema.sql';
    $schema_sql = file_get_contents($schema_path);
    if ($schema_sql === false) {
        throw new RuntimeException("Failed to read schema file: {$schema_path}");
    }
    // MySQL requires executing statements one at a time
    $statements = array_filter(array_map('trim', explode(';', $schema_sql)));
    foreach ($statements as $statement) {
        if ($statement !== '') {
            $pdo->exec($statement);
        }
    }
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

function test_create_user(PDO $pdo): int
{
    createUser($pdo, "Alice", "alice@example.com", UsersStatus::active);
    $user = getLastInsertUser($pdo);
    assert_not_null($user, "CreateUser: getLastInsertUser returned null");
    assert_equal("Alice", $user->name, "CreateUser name");
    assert_equal("alice@example.com", $user->email, "CreateUser email");
    echo "PASS: CreateUser\n";
    return $user->id;
}

function test_get_user_by_id(PDO $pdo, int $user_id): void
{
    $user = getUserById($pdo, $user_id);
    assert_not_null($user, "GetUserById returned null for id={$user_id}");
    assert_equal("Alice", $user->name, "GetUserById name");
    assert_equal($user_id, $user->id, "GetUserById id");
    echo "PASS: GetUserById\n";
}

function test_list_active_users(PDO $pdo): void
{
    $users = listActiveUsers($pdo, UsersStatus::active);
    assert_true(count($users) >= 1, "Expected at least 1 active user, got " . count($users));
    $names = array_map(fn($u) => $u->name, $users);
    assert_true(in_array("Alice", $names, true), "Expected 'Alice' in active users");
    echo "PASS: ListActiveUsers\n";
}

function test_create_order(PDO $pdo, int $user_id): int
{
    createOrder($pdo, $user_id, "49.99", "Test order");
    $order = getLastInsertOrder($pdo);
    assert_not_null($order, "CreateOrder: getLastInsertOrder returned null");
    assert_equal($user_id, $order->user_id, "CreateOrder user_id");
    assert_equal("Test order", $order->notes, "CreateOrder notes");
    echo "PASS: CreateOrder\n";
    return $order->id;
}

function test_get_orders_by_user(PDO $pdo, int $user_id): void
{
    $orders = getOrdersByUser($pdo, $user_id);
    assert_true(count($orders) >= 1, "Expected at least 1 order, got " . count($orders));
    assert_equal("Test order", $orders[0]->notes, "GetOrdersByUser notes");
    echo "PASS: GetOrdersByUser\n";
}

function test_delete_user(PDO $pdo, int $user_id): void
{
    // Delete orders first due to FK constraint
    deleteOrdersByUser($pdo, $user_id);
    deleteUser($pdo, $user_id);
    $user = getUserById($pdo, $user_id);
    assert_true($user === null || $user === false, "Expected user to be deleted, but it still exists");
    echo "PASS: DeleteUser\n";
}

try {
    $mysql_url = get_mysql_url();
    $pdo = create_pdo($mysql_url);

    setup_schema($pdo);

    $user_id = test_create_user($pdo);
    test_get_user_by_id($pdo, $user_id);
    test_list_active_users($pdo);
    $order_id = test_create_order($pdo, $user_id);
    test_get_orders_by_user($pdo, $user_id);
    test_delete_user($pdo, $user_id);

    echo "\nALL TESTS PASSED\n";
    exit(0);
} catch (Throwable $e) {
    fwrite(STDERR, "FAIL: " . $e->getMessage() . "\n");
    fwrite(STDERR, $e->getTraceAsString() . "\n");
    exit(1);
}
