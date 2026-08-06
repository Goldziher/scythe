```elixir title="Elixir (Postgrex)"
defmodule GetUserByIdRow do
  @moduledoc "Row type for GetUserById queries."

  @type t :: %__MODULE__{
    id: integer(),
    name: String.t(),
    email: String.t() | nil,
    status: UserStatus,
    created_at: DateTime.t()
  }
  defstruct [:id, :name, :email, :status, :created_at]
end

@spec get_user_by_id(pid(), integer()) ::
  {:ok, %GetUserByIdRow{}} | {:error, term()}
def get_user_by_id(conn, id) do
  case Postgrex.query(conn,
    "SELECT id, name, email, status, created_at "
    <> "FROM users WHERE id = $1", [id]) do
    {:ok, %{rows: [row]}} ->
      [id, name, email, status, created_at] = row
      {:ok, %GetUserByIdRow{
        id: id, name: name, email: email,
        status: status, created_at: created_at
      }}
    {:ok, %{rows: []}} -> {:error, :not_found}
    {:error, err} -> {:error, err}
  end
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

@spec list_active_users(pid(), UserStatus) ::
  {:ok, [%ListActiveUsersRow{}]} | {:error, term()}
def list_active_users(conn, status) do
  case Postgrex.query(conn,
    "SELECT id, name, email FROM users "
    <> "WHERE status = $1", [status]) do
    {:ok, %{rows: rows}} ->
      results = Enum.map(rows, fn row ->
        [id, name, email] = row
        %ListActiveUsersRow{
          id: id, name: name, email: email
        }
      end)
      {:ok, results}
    {:error, err} -> {:error, err}
  end
end

@spec update_user_email(
  pid(), String.t(), integer()
) :: :ok | {:error, term()}
def update_user_email(conn, email, id) do
  case Postgrex.query(conn,
    "UPDATE users SET email = $1 WHERE id = $2",
    [email, id]) do
    {:ok, _} -> :ok
    {:error, err} -> {:error, err}
  end
end
```
