# Hand-written stand-in for `Jamdb.Oracle.Query`, which the `elixir-jamdb` backend constructs
# to execute a statement through the DBConnection pool (`DBConnection.execute(conn,
# %Jamdb.Oracle.Query{statement: ...}, params)` -- see `src/backends/elixir_jamdb.rs`).
#
# See `myxql_result.ex` in this directory for why a struct construction needs a real stub where
# a plain remote call does not: `%Jamdb.Oracle.Query{...}` expands `__struct__/1` at compile
# time, and that expansion fails hard ("cannot expand struct Jamdb.Oracle.Query") when the module
# was never compiled -- confirmed against real `elixirc` output.
#
# The field list matches what jamdb_oracle 0.5.12 declares, read off a live `IO.inspect` of the
# query DBConnection.execute/3 hands back: statement, name, batch.
defmodule Jamdb.Oracle.Query do
  defstruct [:statement, :name, :batch]
end
