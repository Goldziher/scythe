defmodule CreateOrderRow do
  @moduledoc "Row type for CreateOrder queries."

  @type t :: %__MODULE__{
    id: integer(),
    user_id: integer(),
    total: Decimal.t(),
    notes: String.t() | nil,
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :user_id, :total, :notes, :created_at]
end

defmodule GetOrdersByUserRow do
  @moduledoc "Row type for GetOrdersByUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    total: Decimal.t(),
    notes: String.t() | nil,
    created_at: NaiveDateTime.t()
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

defmodule GetUserByIdRow do
  @moduledoc "Row type for GetUserById queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    active: boolean(),
    external_id: String.t() | nil,
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :name, :email, :active, :external_id, :created_at]
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
    active: boolean(),
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :name, :email, :active, :created_at]
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

defmodule Scythe.Queries do

@spec create_order(pid(), integer(), integer(), Decimal.t(), String.t() | nil) :: {:ok, %CreateOrderRow{}} | {:error, :not_found} | {:error, term()}
def create_order(conn, id, user_id, total, notes) do
  case Tds.query(conn, "INSERT INTO orders (id, user_id, total, notes)
OUTPUT INSERTED.id, INSERTED.user_id, INSERTED.total, INSERTED.notes, INSERTED.created_at
VALUES (@p1, @p2, @p3, @p4)", [id, user_id, total, notes]) do
    {:ok, %{rows: [row | _]}} ->
      [id, user_id, total, notes, created_at] = row
      {:ok, %CreateOrderRow{id: id, user_id: user_id, total: total, notes: notes, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec get_orders_by_user(pid(), integer()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
def get_orders_by_user(conn, user_id) do
  case Tds.query(conn, "SELECT id, total, notes, created_at FROM orders WHERE user_id = @p1 ORDER BY created_at DESC", [user_id]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, total, notes, created_at] = row
        %GetOrdersByUserRow{id: id, total: total, notes: notes, created_at: created_at}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec get_order_total(pid(), integer()) :: {:ok, %GetOrderTotalRow{}} | {:error, :not_found} | {:error, term()}
def get_order_total(conn, user_id) do
  case Tds.query(conn, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = @p1", [user_id]) do
    {:ok, %{rows: [row | _]}} ->
      [total_sum] = row
      {:ok, %GetOrderTotalRow{total_sum: total_sum}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec delete_orders_by_user(pid(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_orders_by_user(conn, user_id) do
  case Tds.query(conn, "DELETE FROM orders WHERE user_id = @p1", [user_id]) do
    {:ok, %{num_rows: n}} -> {:ok, n}
    {:error, err} -> {:error, err}
  end
end

@spec get_user_by_id(pid(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, :not_found} | {:error, term()}
def get_user_by_id(conn, id) do
  case Tds.query(conn, "SELECT id, name, email, active, external_id, created_at FROM users WHERE id = @p1", [id]) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, active, external_id, created_at] = row
      {:ok, %GetUserByIdRow{id: id, name: name, email: email, active: active, external_id: external_id, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec list_active_users(pid()) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn) do
  case Tds.query(conn, "SELECT id, name, email FROM users WHERE active = CAST(1 AS BIT)", []) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %ListActiveUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec create_user(pid(), integer(), String.t(), String.t() | nil, boolean()) :: {:ok, %CreateUserRow{}} | {:error, :not_found} | {:error, term()}
def create_user(conn, id, name, email, active) do
  case Tds.query(conn, "INSERT INTO users (id, name, email, active)
OUTPUT INSERTED.id, INSERTED.name, INSERTED.email, INSERTED.active, INSERTED.created_at
VALUES (@p1, @p2, @p3, @p4)", [id, name, email, active]) do
    {:ok, %{rows: [row | _]}} ->
      [id, name, email, active, created_at] = row
      {:ok, %CreateUserRow{id: id, name: name, email: email, active: active, created_at: created_at}}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(pid(), String.t(), integer()) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  case Tds.query(conn, "UPDATE users SET email = @p1 WHERE id = @p2", [email, id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec delete_user(pid(), integer()) :: :ok | {:error, term()}
def delete_user(conn, id) do
  case Tds.query(conn, "DELETE FROM users WHERE id = @p1", [id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end

@spec search_users(pid(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
def search_users(conn, name) do
  case Tds.query(conn, "SELECT id, name, email FROM users WHERE name LIKE @p1", [name]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %SearchUsersRow{id: id, name: name, email: email}
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

end
