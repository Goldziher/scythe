<?php
// scythe:provenance v=0.16.1 backend=php-amphp engine=postgresql schema=sch2:c247390d575b8f71 queries=q1:b6aca93cc722fe32 options=opt1:cbf29ce484222325

declare(strict_types=1);

namespace App\Generated;

final class RecordNotFoundException extends \RuntimeException {}

final class ScytheCompositeText {
    /**
     * ~keep Splits a PostgreSQL composite's text form ("(a,b,c)") into its raw field tokens,
     * honoring its escaping rules: an empty unquoted field is SQL NULL (returned as null); a
     * field needing quoting (containing a comma, paren, quote, backslash, or leading/trailing
     * space, or the empty string) is wrapped in double quotes; every other field is unquoted and
     * taken literally. A nested composite's own "(x,y)" text form always contains parens, so it
     * always comes back quoted here, ready for that type's own fromText to parse recursively.
     *
     * Inside a quoted field record_out writes a literal '"' as '""' and a literal '\' as '\\'.
     * Both spellings must be accepted: reading '""' as "closing quote, then a new field" both
     * truncates the value and desynchronizes every field after it. Verified against
     * PostgreSQL 16 -- ROW('he said "hi"', 'back\slash', NULL) renders as
     * ("he said ""hi""","back\\slash",).
     *
     * @return array<int, string|null>
     */
    public static function parseCompositeFields(string $text): array {
        $fields = [];
        $inner = substr($text, 1, strlen($text) - 2);
        $i = 0;
        $n = strlen($inner);
        while (true) {
            $field = '';
            $isNull = false;
            if ($i < $n && $inner[$i] === '"') {
                $i++;
                while ($i < $n) {
                    $c = $inner[$i];
                    if ($c === '\\' && $i + 1 < $n) {
                        $field .= $inner[$i + 1];
                        $i += 2;
                    } elseif ($c === '"' && $i + 1 < $n && $inner[$i + 1] === '"') {
                        $field .= '"';
                        $i += 2;
                    } elseif ($c === '"') {
                        $i++;
                        break;
                    } else {
                        $field .= $c;
                        $i++;
                    }
                }
            } else {
                $start = $i;
                while ($i < $n && $inner[$i] !== ',') {
                    $i++;
                }
                $field = substr($inner, $start, $i - $start);
                $isNull = $field === '';
            }
            $fields[] = $isNull ? null : $field;
            if ($i < $n && $inner[$i] === ',') {
                $i++;
                continue;
            }
            break;
        }
        return $fields;
    }
}


enum UserStatus: string {
    case ACTIVE = "active";
    case INACTIVE = "inactive";
    case BANNED = "banned";
}

readonly class GetUserAsJsonRowPayload {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public ?UserStatus $secondary_status,
        public ?UserAddress $address,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromJson(array $value): self {
        return new self(
            id: (int) $value['id'],
            name: (string) $value['name'],
            email: $value['email'] !== null ? (string) $value['email'] : null,
            status: UserStatus::from($value['status']),
            secondary_status: $value['secondary_status'] !== null ? UserStatus::from($value['secondary_status']) : null,
            address: $value['address'] !== null ? UserAddress::fromJson($value['address']) : null,
            created_at: new \DateTimeImmutable($value['created_at']),
        );
    }
}

readonly class GetUsersAsJsonRowPayload {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public ?UserStatus $secondary_status,
        public ?UserAddress $address,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromJson(array $value): self {
        return new self(
            id: (int) $value['id'],
            name: (string) $value['name'],
            email: $value['email'] !== null ? (string) $value['email'] : null,
            status: UserStatus::from($value['status']),
            secondary_status: $value['secondary_status'] !== null ? UserStatus::from($value['secondary_status']) : null,
            address: $value['address'] !== null ? UserAddress::fromJson($value['address']) : null,
            created_at: new \DateTimeImmutable($value['created_at']),
        );
    }
}

readonly class GetUserOrdersAsJsonRowPayload {
    public function __construct(
        public int $id,
        public int $user_id,
        public float $total,
        public ?float $weight_kg,
        public ?string $notes,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromJson(array $value): self {
        return new self(
            id: (int) $value['id'],
            user_id: (int) $value['user_id'],
            total: (float) $value['total'],
            weight_kg: $value['weight_kg'] !== null ? (float) $value['weight_kg'] : null,
            notes: $value['notes'] !== null ? (string) $value['notes'] : null,
            created_at: new \DateTimeImmutable($value['created_at']),
        );
    }
}

readonly class CreateOrderRow {
    public function __construct(
        public int $id,
        public int $user_id,
        public string $total,
        public ?string $notes,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            user_id: (int) $row['user_id'],
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

readonly class GetOrderWeightTotalRow {
    public function __construct(
        public ?float $weight_total,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            weight_total: $row['weight_total'] !== null ? (float) $row['weight_total'] : null,
        );
    }
}

readonly class GetUserByIdRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: UserStatus::from($row['status']),
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class ListActiveUsersRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
        );
    }
}

readonly class CreateUserRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
        public UserStatus $status,
        public \DateTimeImmutable $created_at,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
            status: UserStatus::from($row['status']),
            created_at: new \DateTimeImmutable($row['created_at']),
        );
    }
}

readonly class GetUserOrdersRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $total,
        public ?string $notes,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            total: $row['total'] !== null ? (string) $row['total'] : null,
            notes: $row['notes'] !== null ? (string) $row['notes'] : null,
        );
    }
}

readonly class CountUsersByStatusRow {
    public function __construct(
        public UserStatus $status,
        public int $user_count,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            status: UserStatus::from($row['status']),
            user_count: (int) $row['user_count'],
        );
    }
}

readonly class GetUserWithTagsRow {
    public function __construct(
        public int $id,
        public string $name,
        public string $tag_name,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            tag_name: (string) $row['tag_name'],
        );
    }
}

readonly class SearchUsersRow {
    public function __construct(
        public int $id,
        public string $name,
        public ?string $email,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            name: (string) $row['name'],
            email: $row['email'] !== null ? (string) $row['email'] : null,
        );
    }
}

readonly class UserAddress {
    public function __construct(
        public string $street,
        public string $city,
        public string $zip,
    ) {}

    public static function fromText(?string $text): ?self {
        if ($text === null) {
            return null;
        }
        $f = ScytheCompositeText::parseCompositeFields($text);
        return new self(
            (string) $f[0],
            (string) $f[1],
            (string) $f[2],
        );
    }

    public function toPgText(): string {
        return '(' . implode(',', [self::encodeCompositeField($this->street), self::encodeCompositeField($this->city), self::encodeCompositeField($this->zip)]) . ')';
    }

    private static function encodeCompositeField(mixed $value): string {
        if ($value === null) {
            return '';
        }
        if (is_object($value) && method_exists($value, 'toPgText')) {
            $raw = $value->toPgText();
        } elseif ($value instanceof \BackedEnum) {
            $raw = (string) $value->value;
        } elseif ($value instanceof \DateTimeInterface) {
            $raw = $value->format('Y-m-d H:i:sP');
        } else {
            $raw = (string) $value;
        }
        if ($raw !== '' && strpbrk($raw, ',()"\\') === false && $raw === trim($raw)) {
            return $raw;
        }
        return '"' . str_replace(['\\', '"'], ['\\\\', '""'], $raw) . '"';
    }

    public static function fromJson(array $value): self {
        return new self(
            (string) $value['street'],
            (string) $value['city'],
            (string) $value['zip'],
        );
    }
}

readonly class GetUserProfileRow {
    public function __construct(
        public int $id,
        public ?UserStatus $secondary_status,
        public ?UserAddress $address,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            id: (int) $row['id'],
            secondary_status: $row['secondary_status'] !== null ? UserStatus::from($row['secondary_status']) : null,
            address: UserAddress::fromText($row['address']),
        );
    }
}

readonly class RoundTripUserAddressRow {
    public function __construct(
        public ?UserAddress $address,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            address: UserAddress::fromText($row['address']),
        );
    }
}

readonly class GetUserAsJsonRow {
    public function __construct(
        public ?GetUserAsJsonRowPayload $payload,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            payload: $row['payload'] !== null ? GetUserAsJsonRowPayload::fromJson(json_decode($row['payload'], true, 512, \JSON_THROW_ON_ERROR)) : null,
        );
    }
}

readonly class GetUsersAsJsonRow {
    public function __construct(
        /** @var ?array<GetUsersAsJsonRowPayload> */
        public ?array $payload,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            payload: $row['payload'] !== null ? array_map(static fn (array $item): GetUsersAsJsonRowPayload => GetUsersAsJsonRowPayload::fromJson($item), json_decode($row['payload'], true, 512, \JSON_THROW_ON_ERROR)) : null,
        );
    }
}

readonly class GetUserOrdersAsJsonRow {
    public function __construct(
        /** @var ?array<?GetUserOrdersAsJsonRowPayload> */
        public ?array $payload,
    ) {}

    public static function fromRow(array $row): self {
        return new self(
            payload: $row['payload'] !== null ? array_map(static fn (?array $item): ?GetUserOrdersAsJsonRowPayload => $item === null ? null : GetUserOrdersAsJsonRowPayload::fromJson($item), json_decode($row['payload'], true, 512, \JSON_THROW_ON_ERROR)) : null,
        );
    }
}

final class Queries {

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @param string $total
     * @param ?string $notes
     * @return CreateOrderRow
     * @throws RecordNotFoundException
     */
    public static function createOrder(\Amp\Sql\SqlExecutor $pool, int $user_id, string $total, ?string $notes): CreateOrderRow {
        $result = $pool->prepare('INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?) RETURNING id, user_id, total, notes, created_at')->execute([$user_id, $total, $notes]);
        foreach ($result as $row) {
            return CreateOrderRow::fromRow($row);
        }
        throw new RecordNotFoundException('createOrder: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @return \Generator<int, GetOrdersByUserRow, mixed, void>
     */
    public static function getOrdersByUser(\Amp\Sql\SqlExecutor $pool, int $user_id): \Generator {
        $result = $pool->prepare('SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC')->execute([$user_id]);
        foreach ($result as $row) {
            yield GetOrdersByUserRow::fromRow($row);
        }
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @return GetOrderTotalRow
     * @throws RecordNotFoundException
     */
    public static function getOrderTotal(\Amp\Sql\SqlExecutor $pool, int $user_id): GetOrderTotalRow {
        $result = $pool->prepare('SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?')->execute([$user_id]);
        foreach ($result as $row) {
            return GetOrderTotalRow::fromRow($row);
        }
        throw new RecordNotFoundException('getOrderTotal: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @return GetOrderWeightTotalRow
     * @throws RecordNotFoundException
     */
    public static function getOrderWeightTotal(\Amp\Sql\SqlExecutor $pool, int $user_id): GetOrderWeightTotalRow {
        $result = $pool->prepare('SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = ?')->execute([$user_id]);
        foreach ($result as $row) {
            return GetOrderWeightTotalRow::fromRow($row);
        }
        throw new RecordNotFoundException('getOrderWeightTotal: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $user_id
     * @return int
     */
    public static function deleteOrdersByUser(\Amp\Sql\SqlExecutor $pool, int $user_id): int {
        $result = $pool->prepare('DELETE FROM orders WHERE user_id = ?')->execute([$user_id]);
        return $result->getRowCount();
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $id
     * @return GetUserByIdRow
     * @throws RecordNotFoundException
     */
    public static function getUserById(\Amp\Sql\SqlExecutor $pool, int $id): GetUserByIdRow {
        $result = $pool->prepare('SELECT id, name, email, status, created_at FROM users WHERE id = ?')->execute([$id]);
        foreach ($result as $row) {
            return GetUserByIdRow::fromRow($row);
        }
        throw new RecordNotFoundException('getUserById: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param UserStatus $status
     * @return \Generator<int, ListActiveUsersRow, mixed, void>
     */
    public static function listActiveUsers(\Amp\Sql\SqlExecutor $pool, UserStatus $status): \Generator {
        $result = $pool->prepare('SELECT id, name, email FROM users WHERE status = ?')->execute([$status->value]);
        foreach ($result as $row) {
            yield ListActiveUsersRow::fromRow($row);
        }
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param string $name
     * @param ?string $email
     * @param UserStatus $status
     * @return CreateUserRow
     * @throws RecordNotFoundException
     */
    public static function createUser(\Amp\Sql\SqlExecutor $pool, string $name, ?string $email, UserStatus $status): CreateUserRow {
        $result = $pool->prepare('INSERT INTO users (name, email, status) VALUES (?, ?, ?) RETURNING id, name, email, status, created_at')->execute([$name, $email, $status->value]);
        foreach ($result as $row) {
            return CreateUserRow::fromRow($row);
        }
        throw new RecordNotFoundException('createUser: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param string $email
     * @param int $id
     * @return void
     */
    public static function updateUserEmail(\Amp\Sql\SqlExecutor $pool, string $email, int $id): void {
        $result = $pool->prepare('UPDATE users SET email = ? WHERE id = ?')->execute([$email, $id]);
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $id
     * @return void
     */
    public static function deleteUser(\Amp\Sql\SqlExecutor $pool, int $id): void {
        $result = $pool->prepare('DELETE FROM users WHERE id = ?')->execute([$id]);
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param UserStatus $status
     * @return \Generator<int, GetUserOrdersRow, mixed, void>
     */
    public static function getUserOrders(\Amp\Sql\SqlExecutor $pool, UserStatus $status): \Generator {
        $result = $pool->prepare('SELECT u.id, u.name, o.total, o.notes FROM users u LEFT JOIN orders o ON u.id = o.user_id WHERE u.status = ?')->execute([$status->value]);
        foreach ($result as $row) {
            yield GetUserOrdersRow::fromRow($row);
        }
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param UserStatus $status
     * @return CountUsersByStatusRow
     * @throws RecordNotFoundException
     */
    public static function countUsersByStatus(\Amp\Sql\SqlExecutor $pool, UserStatus $status): CountUsersByStatusRow {
        $result = $pool->prepare('SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = ?')->execute([$status->value]);
        foreach ($result as $row) {
            return CountUsersByStatusRow::fromRow($row);
        }
        throw new RecordNotFoundException('countUsersByStatus: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $id
     * @return \Generator<int, GetUserWithTagsRow, mixed, void>
     */
    public static function getUserWithTags(\Amp\Sql\SqlExecutor $pool, int $id): \Generator {
        $result = $pool->prepare('SELECT u.id, u.name, t.name AS tag_name FROM users u INNER JOIN user_tags ut ON u.id = ut.user_id INNER JOIN tags t ON ut.tag_id = t.id WHERE u.id = ?')->execute([$id]);
        foreach ($result as $row) {
            yield GetUserWithTagsRow::fromRow($row);
        }
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param string $name
     * @return \Generator<int, SearchUsersRow, mixed, void>
     */
    public static function searchUsers(\Amp\Sql\SqlExecutor $pool, string $name): \Generator {
        $result = $pool->prepare('SELECT id, name, email FROM users WHERE name LIKE ?')->execute([$name]);
        foreach ($result as $row) {
            yield SearchUsersRow::fromRow($row);
        }
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $id
     * @return GetUserProfileRow
     * @throws RecordNotFoundException
     */
    public static function getUserProfile(\Amp\Sql\SqlExecutor $pool, int $id): GetUserProfileRow {
        $result = $pool->prepare('SELECT id, secondary_status, address FROM users WHERE id = ?')->execute([$id]);
        foreach ($result as $row) {
            return GetUserProfileRow::fromRow($row);
        }
        throw new RecordNotFoundException('getUserProfile: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param ?UserAddress $address
     * @return RoundTripUserAddressRow
     * @throws RecordNotFoundException
     */
    public static function roundTripUserAddress(\Amp\Sql\SqlExecutor $pool, ?UserAddress $address): RoundTripUserAddressRow {
        $result = $pool->prepare('INSERT INTO users (name, status, address) VALUES (\'Composite Parameter Round Trip\', \'active\', (?::text::user_address)) RETURNING address')->execute([$address?->toPgText()]);
        foreach ($result as $row) {
            return RoundTripUserAddressRow::fromRow($row);
        }
        throw new RecordNotFoundException('roundTripUserAddress: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $id
     * @return GetUserAsJsonRow
     * @throws RecordNotFoundException
     */
    public static function getUserAsJson(\Amp\Sql\SqlExecutor $pool, int $id): GetUserAsJsonRow {
        $result = $pool->prepare('SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = ?')->execute([$id]);
        foreach ($result as $row) {
            return GetUserAsJsonRow::fromRow($row);
        }
        throw new RecordNotFoundException('getUserAsJson: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @return GetUsersAsJsonRow
     * @throws RecordNotFoundException
     */
    public static function getUsersAsJson(\Amp\Sql\SqlExecutor $pool): GetUsersAsJsonRow {
        $result = $pool->prepare('SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u')->execute([]);
        foreach ($result as $row) {
            return GetUsersAsJsonRow::fromRow($row);
        }
        throw new RecordNotFoundException('getUsersAsJson: no row found');
    }

    /**
     * @param \Amp\Sql\SqlExecutor $pool
     * @param int $id
     * @return GetUserOrdersAsJsonRow
     * @throws RecordNotFoundException
     */
    public static function getUserOrdersAsJson(\Amp\Sql\SqlExecutor $pool, int $id): GetUserOrdersAsJsonRow {
        $result = $pool->prepare('SELECT json_agg(o.* ORDER BY o.id) AS payload FROM users u LEFT JOIN orders o ON o.user_id = u.id WHERE u.id = ? GROUP BY u.id')->execute([$id]);
        foreach ($result as $row) {
            return GetUserOrdersAsJsonRow::fromRow($row);
        }
        throw new RecordNotFoundException('getUserOrdersAsJson: no row found');
    }

}
