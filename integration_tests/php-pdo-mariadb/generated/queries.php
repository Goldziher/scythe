<?php
// scythe:provenance v=0.16.1 backend=php-pdo engine=mariadb schema=sch1:262bec5a0954c973 queries=q1:2f37bd0f0a685c79 options=opt1:cbf29ce484222325

declare(strict_types=1);

namespace App\Generated;

final class RecordNotFoundException extends \RuntimeException {}


enum UsersStatus: string {
    case ACTIVE = "active";
    case INACTIVE = "inactive";
    case BANNED = "banned";
}

readonly class CreateOrderRow {
    public function __construct(
        public int $id,
        public string $user_id,
        public string $total,
        public ?string $notes,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            user_id: (string) $row['user_id'],
            total: (string) $row['total'],
            notes: $row['notes'] !== null ? (string) $row['notes'] : null,
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class GetOrdersByUserRow {
    public function __construct(
        public int $id,
        public string $total,
        public ?string $notes,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            total: (string) $row['total'],
            notes: $row['notes'] !== null ? (string) $row['notes'] : null,
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class GetOrderTotalRow {
    public function __construct(
        public ?string $total_sum,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            total_sum: $row['total_sum'] !== null ? (string) $row['total_sum'] : null,
        );
    }
}

readonly class GetUserByIdRow {
    public function __construct(
        public string $id,
        public string $name,
        public ?string $email,
        public UsersStatus $status,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (string) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: UsersStatus::from($row['status']),
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class ListActiveUsersRow {
    public function __construct(
        public string $id,
        public string $name,
        public ?string $email,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (string) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
        );
    }
}

readonly class CreateUserRow {
    public function __construct(
        public string $id,
        public string $name,
        public ?string $email,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (string) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
        );
    }
}

readonly class SearchUsersRow {
    public function __construct(
        public string $id,
        public string $name,
        public ?string $email,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (string) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
        );
    }
}

final class Queries {

    /**
     * @param \PDO $pdo
     * @param string $user_id
     * @param string $total
     * @param ?string $notes
     * @return CreateOrderRow
     * @throws RecordNotFoundException
     */
    public static function createOrder(\PDO $pdo, string $user_id, string $total, ?string $notes): CreateOrderRow {
        $stmt = $pdo->prepare('INSERT INTO orders (user_id, total, notes) VALUES (:p1, :p2, :p3) RETURNING id, user_id, total, notes, created_at');
        $stmt->execute(["p1" => $user_id, "p2" => $total, "p3" => $notes]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('createOrder: no row found');
        }
        return CreateOrderRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param string $user_id
     * @return \Generator<int, GetOrdersByUserRow, mixed, void>
     */
    public static function getOrdersByUser(\PDO $pdo, string $user_id): \Generator {
        $stmt = $pdo->prepare('SELECT id, total, notes, created_at FROM orders WHERE user_id = :p1 ORDER BY created_at DESC');
        $stmt->execute(["p1" => $user_id]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield GetOrdersByUserRow::fromRow($row);
        }
    }

    /**
     * @param \PDO $pdo
     * @param string $user_id
     * @return GetOrderTotalRow
     * @throws RecordNotFoundException
     */
    public static function getOrderTotal(\PDO $pdo, string $user_id): GetOrderTotalRow {
        $stmt = $pdo->prepare('SELECT SUM(total) AS total_sum FROM orders WHERE user_id = :p1');
        $stmt->execute(["p1" => $user_id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getOrderTotal: no row found');
        }
        return GetOrderTotalRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param string $user_id
     * @return int
     */
    public static function deleteOrdersByUser(\PDO $pdo, string $user_id): int {
        $stmt = $pdo->prepare('DELETE FROM orders WHERE user_id = :p1');
        $stmt->execute(["p1" => $user_id]);
        return $stmt->rowCount();
    }

    /**
     * @param \PDO $pdo
     * @param string $id
     * @return GetUserByIdRow
     * @throws RecordNotFoundException
     */
    public static function getUserById(\PDO $pdo, string $id): GetUserByIdRow {
        $stmt = $pdo->prepare('SELECT id, name, email, status, created_at FROM users WHERE id = :p1');
        $stmt->execute(["p1" => $id]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('getUserById: no row found');
        }
        return GetUserByIdRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param UsersStatus $status
     * @return \Generator<int, ListActiveUsersRow, mixed, void>
     */
    public static function listActiveUsers(\PDO $pdo, UsersStatus $status): \Generator {
        $stmt = $pdo->prepare('SELECT id, name, email FROM users WHERE status = :p1');
        $stmt->execute(["p1" => $status->value]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield ListActiveUsersRow::fromRow($row);
        }
    }

    /**
     * @param \PDO $pdo
     * @param string $name
     * @param ?string $email
     * @param UsersStatus $status
     * @return CreateUserRow
     * @throws RecordNotFoundException
     */
    public static function createUser(\PDO $pdo, string $name, ?string $email, UsersStatus $status): CreateUserRow {
        $stmt = $pdo->prepare('INSERT INTO users (name, email, status) VALUES (:p1, :p2, :p3) RETURNING id, name, email');
        $stmt->execute(["p1" => $name, "p2" => $email, "p3" => $status->value]);
        $row = $stmt->fetch(\PDO::FETCH_ASSOC);
        if ($row === false) {
            throw new RecordNotFoundException('createUser: no row found');
        }
        return CreateUserRow::fromRow($row);
    }

    /**
     * @param \PDO $pdo
     * @param string $email
     * @param string $id
     * @return void
     */
    public static function updateUserEmail(\PDO $pdo, string $email, string $id): void {
        $stmt = $pdo->prepare('UPDATE users SET email = :p1 WHERE id = :p2');
        $stmt->execute(["p1" => $email, "p2" => $id]);
    }

    /**
     * @param \PDO $pdo
     * @param string $id
     * @return void
     */
    public static function deleteUser(\PDO $pdo, string $id): void {
        $stmt = $pdo->prepare('DELETE FROM users WHERE id = :p1 RETURNING id');
        $stmt->execute(["p1" => $id]);
    }

    /**
     * @param \PDO $pdo
     * @param string $name
     * @return \Generator<int, SearchUsersRow, mixed, void>
     */
    public static function searchUsers(\PDO $pdo, string $name): \Generator {
        $stmt = $pdo->prepare('SELECT id, name, email FROM users WHERE name LIKE :p1');
        $stmt->execute(["p1" => $name]);
        while ($row = $stmt->fetch(\PDO::FETCH_ASSOC)) {
            yield SearchUsersRow::fromRow($row);
        }
    }

}
