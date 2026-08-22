# scythe:provenance v=0.18.1 backend=elixir-ecto engine=postgresql schema=sch2:59e0edaa3ac94824 queries=q1:861cdfc5df3ece62 options=opt1:cbf29ce484222325
defmodule UserStatus do
  @moduledoc "Enum type for user_status."

  @type t :: String.t()

  @spec active() :: String.t()
  def active(), do: "active"
  @spec inactive() :: String.t()
  def inactive(), do: "inactive"
  @spec banned() :: String.t()
  def banned(), do: "banned"
  @spec values() :: [String.t()]
  def values, do: ["active", "inactive", "banned"]
end

defmodule CreateOrderRow do
  @moduledoc "Row type for CreateOrder queries."

  @type t :: %__MODULE__{
    id: integer(),
    user_id: integer(),
    total: Decimal.t(),
    notes: String.t() | nil,
    created_at: DateTime.t()
  }
  defstruct [:id, :user_id, :total, :notes, :created_at]
end

defmodule GetOrdersByUserRow do
  @moduledoc "Row type for GetOrdersByUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    total: Decimal.t(),
    notes: String.t() | nil,
    created_at: DateTime.t()
  }
  defstruct [:id, :total, :notes, :created_at]
end

defmodule GetOrderTotalRow do
  @moduledoc "Row type for GetOrderTotal queries."

  @type t :: %__MODULE__{
    total_sum: Decimal.t() | nil
  }
  defstruct [:total_sum]
end

defmodule GetOrderWeightTotalRow do
  @moduledoc "Row type for GetOrderWeightTotal queries."

  @type t :: %__MODULE__{
    weight_total: float() | nil
  }
  defstruct [:weight_total]
end

defmodule GetUserByIdRow do
  @moduledoc "Row type for GetUserById queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    status: UserStatus.t(),
    created_at: DateTime.t()
  }
  defstruct [:id, :name, :email, :status, :created_at]
end

defmodule ListActiveUsersRow do
  @moduledoc "Row type for ListActiveUsers queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil
  }
  defstruct [:id, :name, :email]
end

defmodule CreateUserRow do
  @moduledoc "Row type for CreateUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    status: UserStatus.t(),
    created_at: DateTime.t()
  }
  defstruct [:id, :name, :email, :status, :created_at]
end

defmodule GetUserOrdersRow do
  @moduledoc "Row type for GetUserOrders queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    total: Decimal.t() | nil,
    notes: String.t() | nil
  }
  defstruct [:id, :name, :total, :notes]
end

defmodule CountUsersByStatusRow do
  @moduledoc "Row type for CountUsersByStatus queries."

  @type t :: %__MODULE__{
    status: UserStatus.t(),
    user_count: integer()
  }
  defstruct [:status, :user_count]
end

defmodule GetUserWithTagsRow do
  @moduledoc "Row type for GetUserWithTags queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    tag_name: String.t()
  }
  defstruct [:id, :name, :tag_name]
end

defmodule SearchUsersRow do
  @moduledoc "Row type for SearchUsers queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil
  }
  defstruct [:id, :name, :email]
end

defmodule UserAddress do
  @moduledoc "Composite type for user_address."

  @type t :: %__MODULE__{
    street: term(),
    city: term(),
    zip: term()
  }

  defstruct [:street, :city, :zip]

  def from_tuple(nil), do: nil

  def from_tuple({street, city, zip}) do
    %__MODULE__{
      street: street,
      city: city,
      zip: zip,
    }
  end

  def to_tuple(%__MODULE__{} = value) do
    {value.street, value.city, value.zip}
  end
end

defmodule GetUserProfileRow do
  @moduledoc "Row type for GetUserProfile queries."

  @type t :: %__MODULE__{
    id: integer(),
    secondary_status: UserStatus | nil.t(),
    address: UserAddress | nil
  }
  defstruct [:id, :secondary_status, :address]
end

defmodule RoundTripUserAddressRow do
  @moduledoc "Row type for RoundTripUserAddress queries."

  @type t :: %__MODULE__{
    address: UserAddress | nil
  }
  defstruct [:address]
end

defmodule GetUserAsJsonRow do
  @moduledoc "Row type for GetUserAsJson queries."

  @type t :: %__MODULE__{
    payload: map() | nil
  }
  defstruct [:payload]
end

defmodule GetUsersAsJsonRow do
  @moduledoc "Row type for GetUsersAsJson queries."

  @type t :: %__MODULE__{
    payload: list(map()) | nil
  }
  defstruct [:payload]
end

defmodule GetUserOrdersAsJsonRow do
  @moduledoc "Row type for GetUserOrdersAsJson queries."

  @type t :: %__MODULE__{
    payload: list(map()) | nil
  }
  defstruct [:payload]
end

defmodule Scythe.Queries do

@spec create_order(Ecto.Repo.t(), integer(), Decimal.t(), String.t() | nil) :: {:ok, %CreateOrderRow{}} | {:error, :not_found} | {:error, term()}
def create_order(repo, user_id, total, notes) do
  case Ecto.Adapters.SQL.query(repo, "INSERT INTO orders (user_id, total, notes) VALUES ($1, $2, $3) RETURNING id, user_id, total, notes, created_at", [user_id, total, notes], []) do
    {:ok, %{rows: [row | _]}} ->
      [id, user_id, total, notes, created_at] = row
      {:ok, %CreateOrderRow{id: id, user_id: user_id, total: total, notes: notes, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_orders_by_user(Ecto.Repo.t(), integer()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
def get_orders_by_user(repo, user_id) do
  case Ecto.Adapters.SQL.query(repo, "SELECT id, total, notes, created_at FROM orders WHERE user_id = $1 ORDER BY created_at DESC", [user_id], []) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, total, notes, created_at] = row
        %GetOrdersByUserRow{id: id, total: total, notes: notes, created_at: created_at}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec get_order_total(Ecto.Repo.t(), integer()) :: {:ok, %GetOrderTotalRow{}} | {:error, :not_found} | {:error, term()}
def get_order_total(repo, user_id) do
  case Ecto.Adapters.SQL.query(repo, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = $1", [user_id], []) do
    {:ok, %{rows: [row | _]}} ->
      [total_sum] = row
      {:ok, %GetOrderTotalRow{total_sum: total_sum}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_order_weight_total(Ecto.Repo.t(), integer()) :: {:ok, %GetOrderWeightTotalRow{}} | {:error, :not_found} | {:error, term()}
def get_order_weight_total(repo, user_id) do
  case Ecto.Adapters.SQL.query(repo, "SELECT SUM(weight_kg) AS weight_total FROM orders WHERE user_id = $1", [user_id], []) do
    {:ok, %{rows: [row | _]}} ->
      [weight_total] = row
      {:ok, %GetOrderWeightTotalRow{weight_total: weight_total}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec delete_orders_by_user(Ecto.Repo.t(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_orders_by_user(repo, user_id) do
  case Ecto.Adapters.SQL.query(repo, "DELETE FROM orders WHERE user_id = $1", [user_id], []) do
    {:ok, %{num_rows: n}} -> {:ok, n}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_by_id(Ecto.Repo.t(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, :not_found} | {:error, term()}
def get_user_by_id(repo, id) do
  case Ecto.Adapters.SQL.query(repo, "SELECT id, name, email, status, created_at FROM users WHERE id = $1", [id], []) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, status, created_at] = row
      {:ok, %GetUserByIdRow{id: id, name: name, email: email, status: status, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_active_users(Ecto.Repo.t(), String.t()) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(repo, status) do
  case Ecto.Adapters.SQL.query(repo, "SELECT id, name, email FROM users WHERE status = $1", [status], []) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %ListActiveUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec create_user(Ecto.Repo.t(), String.t(), String.t() | nil, String.t()) :: {:ok, %CreateUserRow{}} | {:error, :not_found} | {:error, term()}
def create_user(repo, name, email, status) do
  case Ecto.Adapters.SQL.query(repo, "INSERT INTO users (name, email, status) VALUES ($1, $2, $3) RETURNING id, name, email, status, created_at", [name, email, status], []) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, status, created_at] = row
      {:ok, %CreateUserRow{id: id, name: name, email: email, status: status, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(Ecto.Repo.t(), String.t(), integer()) :: :ok | {:error, term()}
def update_user_email(repo, email, id) do
  case Ecto.Adapters.SQL.query(repo, "UPDATE users SET email = $1 WHERE id = $2", [email, id], []) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec delete_user(Ecto.Repo.t(), integer()) :: :ok | {:error, term()}
def delete_user(repo, id) do
  case Ecto.Adapters.SQL.query(repo, "DELETE FROM users WHERE id = $1", [id], []) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec get_user_orders(Ecto.Repo.t(), String.t()) :: {:ok, [%GetUserOrdersRow{}]} | {:error, term()}
def get_user_orders(repo, status) do
  case Ecto.Adapters.SQL.query(repo, "SELECT u.id, u.name, o.total, o.notes
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.status = $1", [status], []) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, total, notes] = row
        %GetUserOrdersRow{id: id, name: name, total: total, notes: notes}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec count_users_by_status(Ecto.Repo.t(), String.t()) :: {:ok, %CountUsersByStatusRow{}} | {:error, :not_found} | {:error, term()}
def count_users_by_status(repo, status) do
  case Ecto.Adapters.SQL.query(repo, "SELECT status, COUNT(*) AS user_count FROM users GROUP BY status HAVING status = $1", [status], []) do
    {:ok, %{rows: [row | _]}} ->
      [status, user_count] = row
      {:ok, %CountUsersByStatusRow{status: status, user_count: user_count}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_with_tags(Ecto.Repo.t(), integer()) :: {:ok, [%GetUserWithTagsRow{}]} | {:error, term()}
def get_user_with_tags(repo, id) do
  case Ecto.Adapters.SQL.query(repo, "SELECT u.id, u.name, t.name AS tag_name
FROM users u
INNER JOIN user_tags ut ON u.id = ut.user_id
INNER JOIN tags t ON ut.tag_id = t.id
WHERE u.id = $1", [id], []) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, tag_name] = row
        %GetUserWithTagsRow{id: id, name: name, tag_name: tag_name}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec search_users(Ecto.Repo.t(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
def search_users(repo, name) do
  case Ecto.Adapters.SQL.query(repo, "SELECT id, name, email FROM users WHERE name LIKE $1", [name], []) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %SearchUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_profile(Ecto.Repo.t(), integer()) :: {:ok, %GetUserProfileRow{}} | {:error, :not_found} | {:error, term()}
def get_user_profile(repo, id) do
  case Ecto.Adapters.SQL.query(repo, "SELECT id, secondary_status, address FROM users WHERE id = $1", [id], []) do
    {:ok, %{rows: [row | _]}} ->
      [id, secondary_status, address] = row
      {:ok, %GetUserProfileRow{id: id, secondary_status: secondary_status, address: UserAddress.from_tuple(address)}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec round_trip_user_address(Ecto.Repo.t(), UserAddress | nil) :: {:ok, %RoundTripUserAddressRow{}} | {:error, :not_found} | {:error, term()}
def round_trip_user_address(repo, address) do
  case Ecto.Adapters.SQL.query(repo, "INSERT INTO users (name, status, address)
VALUES ('Composite Parameter Round Trip', 'active', ($1))
RETURNING address", [if(is_nil(address), do: nil, else: UserAddress.to_tuple(address))], []) do
    {:ok, %{rows: [row | _]}} ->
      [address] = row
      {:ok, %RoundTripUserAddressRow{address: UserAddress.from_tuple(address)}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_as_json(Ecto.Repo.t(), integer()) :: {:ok, %GetUserAsJsonRow{}} | {:error, :not_found} | {:error, term()}
def get_user_as_json(repo, id) do
  case Ecto.Adapters.SQL.query(repo, "SELECT row_to_json(u.*) AS payload FROM users u WHERE u.id = $1", [id], []) do
    {:ok, %{rows: [row | _]}} ->
      [payload] = row
      {:ok, %GetUserAsJsonRow{payload: payload}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_users_as_json(Ecto.Repo.t()) :: {:ok, %GetUsersAsJsonRow{}} | {:error, :not_found} | {:error, term()}
def get_users_as_json(repo) do
  case Ecto.Adapters.SQL.query(repo, "SELECT jsonb_agg(u.* ORDER BY u.id) AS payload FROM users u", [], []) do
    {:ok, %{rows: [row | _]}} ->
      [payload] = row
      {:ok, %GetUsersAsJsonRow{payload: payload}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_orders_as_json(Ecto.Repo.t(), integer()) :: {:ok, %GetUserOrdersAsJsonRow{}} | {:error, :not_found} | {:error, term()}
def get_user_orders_as_json(repo, id) do
  case Ecto.Adapters.SQL.query(repo, "SELECT json_agg(o.* ORDER BY o.id) AS payload
FROM users u
LEFT JOIN orders o ON o.user_id = u.id
WHERE u.id = $1
GROUP BY u.id", [id], []) do
    {:ok, %{rows: [row | _]}} ->
      [payload] = row
      {:ok, %GetUserOrdersAsJsonRow{payload: payload}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

end
