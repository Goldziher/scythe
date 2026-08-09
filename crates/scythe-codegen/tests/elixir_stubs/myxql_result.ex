# Hand-written stand-in for `MyXQL.Result`, which the `elixir-myxql` backend
# pattern-matches on (`%MyXQL.Result{rows: ..., num_rows: ...}` -- see
# `src/backends/elixir_myxql.rs`).
#
# Every other `elixir-*` backend matches on a plain map (`%{rows: ...}`), so
# `elixirc` needs nothing extra for them: an undefined *function* call
# degrades to a warning, not an error (see `validate_elixir_tools` in
# `src/validation.rs`). A struct reference is different -- Elixir's
# set-theoretic type checker treats a pattern match against an undefined
# struct as a hard compile error ("Type checking failed"), confirmed against
# real `elixirc` output, not assumed. Fetching the real `myxql` hex package
# here would need network access this harness must not depend on, and the
# fields this backend actually destructures are two, so a hand-written stub
# -- same hermetic-stub precedent as `tests/js_mode_stubs/driver-stubs.d.ts`
# and `tests/java_stubs/javax/annotation/*.java` -- costs far less than
# vendoring the real dependency.
defmodule MyXQL.Result do
  defstruct [:rows, :num_rows]
end
