<?php

declare(strict_types=1);

require_once __DIR__ . '/generated/queries.php';

use App\Generated\Queries;
use App\Generated\CreateUserRow;
use App\Generated\GetUserByIdRow;
use App\Generated\ListActiveUsersRow;
use App\Generated\CreateOrderRow;
use App\Generated\GetOrdersByUserRow;

function get_database_url(): string
{
    $url = getenv('SNOWFLAKE_URL');
    if ($url === false || $url === '') {
        fwrite(STDERR, "ERROR: SNOWFLAKE_URL environment variable is not set\n");
        exit(1);
    }
    return $url;
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
    $user = Queries::createUser($pdo, "Alice", "alice@example.com");
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
    $users = iterator_to_array(Queries::listActiveUsers($pdo));
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

function test_delete_user(PDO $pdo, int $user_id): void
{
    // Delete orders first due to FK constraint
    Queries::deleteOrdersByUser($pdo, $user_id);
    Queries::deleteUser($pdo, $user_id);
    $user = Queries::getUserById($pdo, $user_id);
    assert_true($user === null || $user === false, "Expected user to be deleted, but it still exists");
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
    test_delete_user($pdo, $user_id);

    echo "\nALL TESTS PASSED\n";
    exit(0);
} catch (Throwable $e) {
    fwrite(STDERR, "FAIL: " . $e->getMessage() . "\n");
    fwrite(STDERR, $e->getTraceAsString() . "\n");
    exit(1);
}
