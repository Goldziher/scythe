//! Spec rendering shared by the Elixir backends.
//!
//! Elixir writes a parameter's type twice, in two positions that do not mean
//! the same thing. In a `@type`/`defstruct` field position a bare module
//! alias is a reference to that module, so an enum column's `UserStatus`
//! renders correctly as `UserStatus.t()`. In a `@spec` *argument* position
//! the same bare alias is a **literal atom** type -- `:"Elixir.UserStatus"`
//! -- which no caller can ever satisfy, because what actually crosses the
//! call boundary is the runtime string the database stores (`"active"`).
//!
//! Only `elixir-ecto` handled that distinction; the other five backends put
//! `full_type` straight into the `@spec` and produced a signature Dialyzer
//! rejects for every correct call. Keeping the rule here rather than in one
//! backend that the others reach across into means there is one derivation
//! of it, which is the property the whole `*_common.rs` family exists for.

use crate::backend_trait::ResolvedParam;

/// Render a bound query parameter's `@spec` argument type.
///
/// An enum parameter is spelled `String.t()` -- the type of the value a
/// caller actually passes -- rather than its module alias, which in argument
/// position would be a literal-atom type nothing can satisfy. Every other
/// parameter keeps the `full_type` the manifest resolved for it (#202).
pub(crate) fn elixir_param_spec_type(param: &ResolvedParam) -> String {
    if param.neutral_type.starts_with("enum::") {
        "String.t()".to_string()
    } else {
        param.full_type.clone()
    }
}
