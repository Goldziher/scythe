# Hand-written stand-in for `Tds.Parameter`, which the `elixir-tds` backend
# constructs to bind named parameters (`%Tds.Parameter{name: ..., value: ...,
# type: ...}` -- see `src/backends/elixir_tds.rs`).
#
# See `myxql_result.ex` in this directory for why a struct construction needs
# a real stub where a plain remote call does not: `%Tds.Parameter{...}`
# expands `__struct__/1` at compile time, and that expansion fails hard
# ("cannot expand struct Tds.Parameter") when the module was never compiled --
# confirmed against real `elixirc` output.
defmodule Tds.Parameter do
  defstruct [:name, :value, :type]
end
