defmodule UsersStatus do
  @moduledoc "Enum type for users_status."

  @type t :: String.t()
  def active(), do: "active"
  def inactive(), do: "inactive"
  def banned(), do: "banned"
  def values, do: ["active", "inactive", "banned"]
end

defmodule GetLastInsertOrderRow do
  @moduledoc "Row type for GetLastInsertOrder queries."

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
    status: UsersStatus,
    created_at: NaiveDateTime.t()
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

defmodule GetLastInsertUserRow do
  @moduledoc "Row type for GetLastInsertUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    status: UsersStatus,
    created_at: NaiveDateTime.t()
  }
  defstruct [:id, :name, :email, :status, :created_at]
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
  @moduledoc "Auto-generated query functions."

  alias Decimal, as: Decimal

  @spec create_order(pid(), integer(), Decimal.t(), String.t()) :: :ok | {:error, term()}
  def create_order(conn, user_id, total, notes) do
    case MyXQL.query(conn, "INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)", [user_id, total, notes]) do
      {:ok, _} -> :ok
      {:error, err} -> {:error, err}
    end
  end

  @spec get_last_insert_order(pid()) :: {:ok, %GetLastInsertOrderRow{}} | {:error, term()}
  def get_last_insert_order(conn) do
    case MyXQL.query(conn, "SELECT id, user_id, total, notes, created_at FROM orders WHERE id = LAST_INSERT_ID()", []) do
      {:ok, %MyXQL.Result{rows: [row]}} ->
        [id, user_id, total, notes, created_at] = row
        {:ok, %GetLastInsertOrderRow{id: id, user_id: user_id, total: total, notes: notes, created_at: created_at}}
      {:ok, %MyXQL.Result{rows: []}} -> {:error, :not_found}
      {:error, err} -> {:error, err}
    end
  end

  @spec get_orders_by_user(pid(), integer()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
  def get_orders_by_user(conn, user_id) do
    case MyXQL.query(conn, "SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC", [user_id]) do
      {:ok, %MyXQL.Result{rows: rows}} ->
        results = Enum.map(rows, fn row ->
          [id, total, notes, created_at] = row
          %GetOrdersByUserRow{id: id, total: total, notes: notes, created_at: created_at}
        end)
        {:ok, results}
      {:error, err} -> {:error, err}
    end
  end

  @spec get_order_total(pid(), integer()) :: {:ok, %GetOrderTotalRow{}} | {:error, term()}
  def get_order_total(conn, user_id) do
    case MyXQL.query(conn, "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?", [user_id]) do
      {:ok, %MyXQL.Result{rows: [row]}} ->
        [total_sum] = row
        {:ok, %GetOrderTotalRow{total_sum: total_sum}}
      {:ok, %MyXQL.Result{rows: []}} -> {:error, :not_found}
      {:error, err} -> {:error, err}
    end
  end

  @spec delete_orders_by_user(pid(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
  def delete_orders_by_user(conn, user_id) do
    case MyXQL.query(conn, "DELETE FROM orders WHERE user_id = ?", [user_id]) do
      {:ok, %MyXQL.Result{num_rows: n}} -> {:ok, n}
      {:error, err} -> {:error, err}
    end
  end

  @spec get_user_by_id(pid(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, term()}
  def get_user_by_id(conn, id) do
    case MyXQL.query(conn, "SELECT id, name, email, status, created_at FROM users WHERE id = ?", [id]) do
      {:ok, %MyXQL.Result{rows: [row]}} ->
        [id, name, email, status, created_at] = row
        {:ok, %GetUserByIdRow{id: id, name: name, email: email, status: status, created_at: created_at}}
      {:ok, %MyXQL.Result{rows: []}} -> {:error, :not_found}
      {:error, err} -> {:error, err}
    end
  end

  @spec list_active_users(pid(), UsersStatus) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
  def list_active_users(conn, status) do
    case MyXQL.query(conn, "SELECT id, name, email FROM users WHERE status = ?", [status]) do
      {:ok, %MyXQL.Result{rows: rows}} ->
        results = Enum.map(rows, fn row ->
          [id, name, email] = row
          %ListActiveUsersRow{id: id, name: name, email: email}
        end)
        {:ok, results}
      {:error, err} -> {:error, err}
    end
  end

  @spec create_user(pid(), String.t(), String.t(), UsersStatus) :: :ok | {:error, term()}
  def create_user(conn, name, email, status) do
    case MyXQL.query(conn, "INSERT INTO users (name, email, status) VALUES (?, ?, ?)", [name, email, status]) do
      {:ok, _} -> :ok
      {:error, err} -> {:error, err}
    end
  end

  @spec get_last_insert_user(pid()) :: {:ok, %GetLastInsertUserRow{}} | {:error, term()}
  def get_last_insert_user(conn) do
    case MyXQL.query(conn, "SELECT id, name, email, status, created_at FROM users WHERE id = LAST_INSERT_ID()", []) do
      {:ok, %MyXQL.Result{rows: [row]}} ->
        [id, name, email, status, created_at] = row
        {:ok, %GetLastInsertUserRow{id: id, name: name, email: email, status: status, created_at: created_at}}
      {:ok, %MyXQL.Result{rows: []}} -> {:error, :not_found}
      {:error, err} -> {:error, err}
    end
  end

  @spec update_user_email(pid(), String.t(), integer()) :: :ok | {:error, term()}
  def update_user_email(conn, email, id) do
    case MyXQL.query(conn, "UPDATE users SET email = ? WHERE id = ?", [email, id]) do
      {:ok, _} -> :ok
      {:error, err} -> {:error, err}
    end
  end

  @spec delete_user(pid(), integer()) :: :ok | {:error, term()}
  def delete_user(conn, id) do
    case MyXQL.query(conn, "DELETE FROM users WHERE id = ?", [id]) do
      {:ok, _} -> :ok
      {:error, err} -> {:error, err}
    end
  end

  @spec search_users(pid(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
  def search_users(conn, name) do
    case MyXQL.query(conn, "SELECT id, name, email FROM users WHERE name LIKE ?", [name]) do
      {:ok, %MyXQL.Result{rows: rows}} ->
        results = Enum.map(rows, fn row ->
          [id, name, email] = row
          %SearchUsersRow{id: id, name: name, email: email}
        end)
        {:ok, results}
      {:error, err} -> {:error, err}
    end
  end
end
