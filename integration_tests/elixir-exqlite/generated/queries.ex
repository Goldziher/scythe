# scythe:provenance v=0.16.0 backend=elixir-exqlite engine=sqlite schema=sch1:588fb635332179bc queries=q1:f7199f36438b6396 options=opt1:cbf29ce484222325
defmodule GetOrdersByUserRow do
  @moduledoc "Row type for GetOrdersByUser queries."

  @type t :: %__MODULE__{
    id: integer(),
    total: float(),
    notes: String.t() | nil,
    created_at: String.t()
  }
  defstruct [:id, :total, :notes, :created_at]
end

defmodule GetOrderTotalRow do
  @moduledoc "Row type for GetOrderTotal queries."

  @type t :: %__MODULE__{
    total_sum: float() | nil
  }
  defstruct [:total_sum]
end

defmodule GetUserByIdRow do
  @moduledoc "Row type for GetUserById queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    status: String.t(),
    created_at: String.t()
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

@spec create_order(Exqlite.Sqlite3.db(), integer(), float(), String.t() | nil) :: :ok | {:error, term()}
def create_order(conn, user_id, total, notes) do
  sql = "INSERT INTO orders (user_id, total, notes) VALUES (?, ?, ?)"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [user_id, total, notes]) do
      :ok ->
        case Exqlite.Sqlite3.step(conn, stmt) do
          :done ->
            Exqlite.Sqlite3.release(conn, stmt)
            :ok
          {:error, err} ->
            Exqlite.Sqlite3.release(conn, stmt)
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec get_orders_by_user(Exqlite.Sqlite3.db(), integer()) :: {:ok, [%GetOrdersByUserRow{}]} | {:error, term()}
def get_orders_by_user(conn, user_id) do
  sql = "SELECT id, total, notes, created_at FROM orders WHERE user_id = ? ORDER BY created_at DESC"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [user_id]) do
      :ok ->
        result = Exqlite.Sqlite3.fetch_all(conn, stmt)
        Exqlite.Sqlite3.release(conn, stmt)
        case result do
          {:ok, rows} ->
            results = Enum.map(rows, fn row ->
              [id, total, notes, created_at] = row
              %GetOrdersByUserRow{id: id, total: total, notes: notes, created_at: created_at}
            end)
            {:ok, results}
          {:error, err} ->
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec get_order_total(Exqlite.Sqlite3.db(), integer()) :: {:ok, %GetOrderTotalRow{}} | {:error, :not_found} | {:error, term()}
def get_order_total(conn, user_id) do
  sql = "SELECT SUM(total) AS total_sum FROM orders WHERE user_id = ?"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [user_id]) do
      :ok ->
        rows = Exqlite.Sqlite3.fetch_all(conn, stmt)
        Exqlite.Sqlite3.release(conn, stmt)
        case rows do
          {:ok, [[_|_] = row]} ->
            [total_sum] = row
            {:ok, %GetOrderTotalRow{total_sum: total_sum}}
          {:ok, []} ->
            {:error, :not_found}
          {:error, err} ->
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec delete_orders_by_user(Exqlite.Sqlite3.db(), integer()) :: {:ok, non_neg_integer()} | {:error, term()}
def delete_orders_by_user(conn, user_id) do
  sql = "DELETE FROM orders WHERE user_id = ?"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [user_id]) do
      :ok ->
        case Exqlite.Sqlite3.step(conn, stmt) do
          :done ->
            case Exqlite.Sqlite3.changes(conn) do
              {:ok, changes} ->
                Exqlite.Sqlite3.release(conn, stmt)
                {:ok, changes}
              {:error, err} ->
                Exqlite.Sqlite3.release(conn, stmt)
                {:error, err}
            end
          {:error, err} ->
            Exqlite.Sqlite3.release(conn, stmt)
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec get_user_by_id(Exqlite.Sqlite3.db(), integer()) :: {:ok, %GetUserByIdRow{}} | {:error, :not_found} | {:error, term()}
def get_user_by_id(conn, id) do
  sql = "SELECT id, name, email, status, created_at FROM users WHERE id = ?"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [id]) do
      :ok ->
        rows = Exqlite.Sqlite3.fetch_all(conn, stmt)
        Exqlite.Sqlite3.release(conn, stmt)
        case rows do
          {:ok, [[_|_] = row]} ->
            [id, name, email, status, created_at] = row
            {:ok, %GetUserByIdRow{id: id, name: name, email: email, status: status, created_at: created_at}}
          {:ok, []} ->
            {:error, :not_found}
          {:error, err} ->
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec list_active_users(Exqlite.Sqlite3.db(), String.t()) :: {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn, status) do
  sql = "SELECT id, name, email FROM users WHERE status = ?"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [status]) do
      :ok ->
        result = Exqlite.Sqlite3.fetch_all(conn, stmt)
        Exqlite.Sqlite3.release(conn, stmt)
        case result do
          {:ok, rows} ->
            results = Enum.map(rows, fn row ->
              [id, name, email] = row
              %ListActiveUsersRow{id: id, name: name, email: email}
            end)
            {:ok, results}
          {:error, err} ->
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec create_user(Exqlite.Sqlite3.db(), String.t(), String.t() | nil, String.t()) :: :ok | {:error, term()}
def create_user(conn, name, email, status) do
  sql = "INSERT INTO users (name, email, status) VALUES (?, ?, ?)"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [name, email, status]) do
      :ok ->
        case Exqlite.Sqlite3.step(conn, stmt) do
          :done ->
            Exqlite.Sqlite3.release(conn, stmt)
            :ok
          {:error, err} ->
            Exqlite.Sqlite3.release(conn, stmt)
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(Exqlite.Sqlite3.db(), String.t(), integer()) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  sql = "UPDATE users SET email = ? WHERE id = ?"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [email, id]) do
      :ok ->
        case Exqlite.Sqlite3.step(conn, stmt) do
          :done ->
            Exqlite.Sqlite3.release(conn, stmt)
            :ok
          {:error, err} ->
            Exqlite.Sqlite3.release(conn, stmt)
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec delete_user(Exqlite.Sqlite3.db(), integer()) :: :ok | {:error, term()}
def delete_user(conn, id) do
  sql = "DELETE FROM users WHERE id = ?"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [id]) do
      :ok ->
        case Exqlite.Sqlite3.step(conn, stmt) do
          :done ->
            Exqlite.Sqlite3.release(conn, stmt)
            :ok
          {:error, err} ->
            Exqlite.Sqlite3.release(conn, stmt)
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

@spec search_users(Exqlite.Sqlite3.db(), String.t()) :: {:ok, [%SearchUsersRow{}]} | {:error, term()}
def search_users(conn, name) do
  sql = "SELECT id, name, email FROM users WHERE name LIKE ?"
  with {:ok, stmt} <- Exqlite.Sqlite3.prepare(conn, sql) do
    case Exqlite.Sqlite3.bind(stmt, [name]) do
      :ok ->
        result = Exqlite.Sqlite3.fetch_all(conn, stmt)
        Exqlite.Sqlite3.release(conn, stmt)
        case result do
          {:ok, rows} ->
            results = Enum.map(rows, fn row ->
              [id, name, email] = row
              %SearchUsersRow{id: id, name: name, email: email}
            end)
            {:ok, results}
          {:error, err} ->
            {:error, err}
        end
      {:error, err} ->
        Exqlite.Sqlite3.release(conn, stmt)
        {:error, err}
    end
  else
    {:error, err} -> {:error, err}
  end
end

end
