<?php

declare(strict_types=1);

require_once __DIR__ . '/generated/queries.php';

use App\Generated\Queries;
use App\Generated\UserStatus;
use App\Generated\CreateUserRow;
use App\Generated\GetUserByIdRow;
use App\Generated\ListActiveUsersRow;
use App\Generated\CreateOrderRow;
use App\Generated\GetOrdersByUserRow;

use Amp\Postgres\PostgresConnectionPool;
use Amp\Postgres\PostgresConfig;

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

function create_pool(string $url): PostgresConnectionPool
{
    $params = parse_database_url($url);
    $config = PostgresConfig::fromString(sprintf(
        'host=%s port=%d dbname=%s user=%s password=%s',
        $params['host'],
        $params['port'],
        $params['dbname'],
        $params['user'],
        $params['password']
    ));
    return new PostgresConnectionPool($config);
}

function setup_schema(PostgresConnectionPool $pool): void
{
    $pool->query("DROP TABLE IF EXISTS user_tags CASCADE");
    $pool->query("DROP TABLE IF EXISTS tags CASCADE");
    $pool->query("DROP TABLE IF EXISTS orders CASCADE");
    $pool->query("DROP TABLE IF EXISTS users CASCADE");
    $pool->query("DROP TYPE IF EXISTS user_status CASCADE");
    $schema_path = __DIR__ . '/../sql/pg/schema.sql';
    $schema_sql = file_get_contents($schema_path);
    if ($schema_sql === false) {
        throw new RuntimeException("Failed to read schema file: {$schema_path}");
    }
    $pool->query($schema_sql);
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

function test_create_user(PostgresConnectionPool $pool): int
{
    $user = Queries::createUser($pool, "Alice", "alice@example.com", UserStatus::active);
    assert_not_null($user, "CreateUser returned null");
    assert_equal("Alice", $user->name, "CreateUser name");
    assert_equal("alice@example.com", $user->email, "CreateUser email");
    echo "PASS: CreateUser\n";
    return $user->id;
}

function test_get_user_by_id(PostgresConnectionPool $pool, int $user_id): void
{
    $user = Queries::getUserById($pool, $user_id);
    assert_not_null($user, "GetUserById returned null for id={$user_id}");
    assert_equal("Alice", $user->name, "GetUserById name");
    assert_equal($user_id, $user->id, "GetUserById id");
    echo "PASS: GetUserById\n";
}

function test_list_active_users(PostgresConnectionPool $pool): void
{
    $users = iterator_to_array(Queries::listActiveUsers($pool, UserStatus::active));
    assert_true(count($users) >= 1, "Expected at least 1 active user, got " . count($users));
    $names = array_map(fn($u) => $u->name, $users);
    assert_true(in_array("Alice", $names, true), "Expected 'Alice' in active users");
    echo "PASS: ListActiveUsers\n";
}

function test_create_order(PostgresConnectionPool $pool, int $user_id): int
{
    $order = Queries::createOrder($pool, $user_id, "49.99", "Test order");
    assert_not_null($order, "CreateOrder returned null");
    assert_equal($user_id, $order->user_id, "CreateOrder user_id");
    assert_equal("Test order", $order->notes, "CreateOrder notes");
    echo "PASS: CreateOrder\n";
    return $order->id;
}

function test_get_orders_by_user(PostgresConnectionPool $pool, int $user_id): void
{
    $orders = iterator_to_array(Queries::getOrdersByUser($pool, $user_id));
    assert_true(count($orders) >= 1, "Expected at least 1 order, got " . count($orders));
    assert_equal("Test order", $orders[0]->notes, "GetOrdersByUser notes");
    echo "PASS: GetOrdersByUser\n";
}

function test_delete_user(PostgresConnectionPool $pool, int $user_id): void
{
    // Delete orders first due to FK constraint
    Queries::deleteOrdersByUser($pool, $user_id);
    Queries::deleteUser($pool, $user_id);
    $user = Queries::getUserById($pool, $user_id);
    assert_true($user === null, "Expected user to be deleted, but it still exists");
    echo "PASS: DeleteUser\n";
}

try {
    $database_url = get_database_url();
    $pool = create_pool($database_url);

    setup_schema($pool);

    $user_id = test_create_user($pool);
    test_get_user_by_id($pool, $user_id);
    test_list_active_users($pool);
    $order_id = test_create_order($pool, $user_id);
    test_get_orders_by_user($pool, $user_id);
    test_delete_user($pool, $user_id);

    echo "\nALL TESTS PASSED\n";
    exit(0);
} catch (Throwable $e) {
    fwrite(STDERR, "FAIL: " . $e->getMessage() . "\n");
    fwrite(STDERR, $e->getTraceAsString() . "\n");
    exit(1);
}
