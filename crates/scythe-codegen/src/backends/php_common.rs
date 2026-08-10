//! Docblock rendering shared by the PHP backends.
//!
//! PHP is the one target where a type has to be written twice. The native
//! position -- a promoted property, a parameter, a return type -- accepts no
//! generic syntax, so `[types.containers]` maps `array` to a bare `array`;
//! `public array<string> $tags` is a parse error, not a style question. A
//! PHPStan docblock has the opposite constraint: at level 9 a bare `array` is
//! `array<mixed, mixed>` and every read out of it is an error, so the element
//! type has to appear *somewhere*.
//!
//! The two strings come from the same manifest and the same resolver: the
//! native one from `[types.containers]`, the docblock one from
//! `[types.docblock_containers]` falling back to `[types.containers]`. Nothing
//! here decides what a PHP array is called -- ask the manifest, twice.

use std::fmt::Write;

use scythe_backend::manifest::BackendManifest;
use scythe_backend::types::resolve_docblock_type;

use scythe_core::errors::{ErrorCode, ScytheError};

use crate::backend_trait::{ResolvedColumn, ResolvedParam};

/// The docblock type for a value already rendered as `native` in a native
/// position, or `None` when the two are the same string.
///
/// `None` is the common case, and it is the reason this returns an `Option`
/// rather than a string: emitting `/** @var string */` above `public string
/// $name` is noise that says nothing the native type did not already say, and
/// PHPStan gains nothing from it. Only the positions where the manifest can
/// say more than PHP's syntax allows get a docblock.
fn narrower_docblock_type(
    neutral: &str,
    nullable: bool,
    native: &str,
    name: &str,
    manifest: &BackendManifest,
) -> Result<Option<String>, ScytheError> {
    let rendered = resolve_docblock_type(neutral, manifest, nullable).map_err(|e| {
        ScytheError::new(
            ErrorCode::InternalError,
            format!("docblock type resolution failed for '{name}': {e}"),
        )
    })?;

    // Compared against the *emitted* native string rather than against a
    // freshly resolved one on purpose: that makes this a cross-check of what
    // actually reached the file, so a native type produced some other way
    // (an override, a hand-built `ResolvedColumn`) still gets a docblock when
    // it differs and still gets none when it does not.
    if rendered == native {
        return Ok(None);
    }
    Ok(Some(rendered.into_owned()))
}

/// Write one promoted constructor property, preceded by a `/** @var ... */`
/// line when the docblock type is narrower than the native one.
pub(crate) fn write_promoted_property(
    out: &mut String,
    column: &ResolvedColumn,
    manifest: &BackendManifest,
) -> Result<(), ScytheError> {
    if let Some(docblock) = narrower_docblock_type(
        &column.neutral_type,
        column.nullable,
        &column.full_type,
        &column.name,
        manifest,
    )? {
        let _ = writeln!(out, "        /** @var {} */", docblock);
    }
    let _ = writeln!(out, "        public {} ${},", column.full_type, column.field_name);
    Ok(())
}

/// The type to render in a parameter's `@param` tag: the docblock type where
/// it says more, the native type otherwise.
pub(crate) fn param_docblock_type(param: &ResolvedParam, manifest: &BackendManifest) -> Result<String, ScytheError> {
    let narrower = narrower_docblock_type(
        &param.neutral_type,
        param.nullable,
        &param.full_type,
        &param.name,
        manifest,
    )?;
    Ok(narrower.unwrap_or_else(|| param.full_type.clone()))
}
