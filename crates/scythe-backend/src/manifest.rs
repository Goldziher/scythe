use std::path::Path;

use ahash::AHashMap;
use serde::Deserialize;

use crate::errors::BackendError;
use crate::naming::NamingConfig;

/// Top-level backend manifest parsed from `manifest.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendManifest {
    pub backend: BackendMeta,
    pub types: TypeMappings,
    pub naming: NamingConfig,
    pub imports: Option<ImportConfig>,
}

/// Metadata about the backend.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendMeta {
    pub name: String,
    pub language: String,
    pub file_extension: String,
    pub engine: String,
    pub description: Option<String>,
}

/// Mappings from neutral types to language-specific types.
#[derive(Debug, Clone, Deserialize)]
pub struct TypeMappings {
    /// Scalar type mappings: neutral name -> language type.
    pub scalars: AHashMap<String, String>,
    /// Container type patterns: container name -> pattern with `{T}` placeholder.
    pub containers: AHashMap<String, String>,
    /// Container patterns used only where the target language accepts a type
    /// in a *comment*, not in a native type position -- a PHPStan/Psalm
    /// docblock being the only such position today.
    ///
    /// A language whose native syntax can express the element type needs
    /// nothing here: any container this table omits falls back to
    /// [`Self::containers`], so the two positions render identically and
    /// every non-PHP manifest is unaffected by this table existing.
    ///
    /// PHP is the case that forces the split. `public array<string> $tags` is
    /// a parse error -- PHP has no generics in a native type position -- so
    /// `[types.containers]` must map `array` to a bare `array`. That is a real
    /// loss of information, because `/** @var list<string> */` is both legal
    /// and, at PHPStan level 9, necessary: a bare `array` is
    /// `array<mixed, mixed>` there. This table is where the element type
    /// survives.
    #[serde(default)]
    pub docblock_containers: AHashMap<String, String>,
}

/// Import rules for generated code.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportConfig {
    /// Maps a type prefix to the import statement needed.
    pub rules: AHashMap<String, String>,
}

/// Load and parse a backend manifest from a TOML file.
pub fn load_manifest(path: &Path) -> Result<BackendManifest, BackendError> {
    let content = std::fs::read_to_string(path).map_err(BackendError::Io)?;
    toml::from_str(&content).map_err(|e| BackendError::ManifestError(e.to_string()))
}

/// A *partial* backend manifest, merged over a backend's compiled-in manifest
/// when a `[[sql.gen]]` target sets `manifest = "..."`.
///
/// # Merge granularity
///
/// The overlay is applied by [`BackendManifest::apply_overlay`] with two
/// different granularities, chosen per section:
///
/// * **Map-valued tables** (`[types.scalars]`, `[types.containers]`,
///   `[imports.rules]`) merge **per leaf key**. Each key the overlay mentions
///   replaces exactly that entry; every key it does not mention keeps its
///   compiled-in value. This is what makes the file an overlay rather than a
///   replacement: retargeting `int64` does not require restating the other
///   thirty-odd scalar mappings.
/// * **Scalar-valued keys** (every field of `[naming]`) replace the
///   compiled-in value **whole**. A key is either present (replaces) or
///   absent (inherits); there is no sub-value merging.
///
/// # What may be added versus only replaced
///
/// `[types.scalars]` and `[types.containers]` are **replace-only**: a key that
/// does not already exist in the compiled-in manifest is rejected. Neutral
/// type names (`int32`, `datetime_tz`, `array`, ...) are a fixed vocabulary
/// produced by scythe's type inference, so a key outside it is a typo, and a
/// silently-accepted typo would leave the original mapping in place and
/// generate code the user did not ask for.
///
/// `[imports.rules]` is **merge-and-add**: its keys are prefixes of the
/// *generated language* types, which necessarily change when a scalar is
/// retargeted (mapping `decimal` to a different crate's type requires a new
/// import rule keyed on that crate's prefix). Restricting it to existing keys
/// would make scalar overrides unusable.
///
/// # What is not overridable
///
/// There is deliberately no `[backend]` section. `name`, `language`,
/// `file_extension`, and `engine` are identity, not configuration: manifest
/// selection is a pure function of `(backend, engine)` (#82), and letting an
/// override rewrite the engine would reintroduce exactly the cross-engine
/// mismatch that made the old working-directory lookup unsafe.
/// `#[serde(deny_unknown_fields)]` turns a `[backend]` table — or any other
/// misspelled section or field — into a parse error naming the offending key.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestOverlay {
    /// Per-leaf-key overrides for `[types.scalars]` / `[types.containers]`.
    #[serde(default)]
    pub types: Option<TypeMappingsOverlay>,
    /// Whole-value overrides for individual `[naming]` fields.
    #[serde(default)]
    pub naming: Option<NamingOverlay>,
    /// Per-leaf-key additions and overrides for `[imports.rules]`.
    #[serde(default)]
    pub imports: Option<ImportConfigOverlay>,
}

/// Overlay half of [`TypeMappings`]. Every map is replace-only; see
/// [`ManifestOverlay`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeMappingsOverlay {
    #[serde(default)]
    pub scalars: AHashMap<String, String>,
    #[serde(default)]
    pub containers: AHashMap<String, String>,
    /// Overrides for [`TypeMappings::docblock_containers`].
    ///
    /// The accepted vocabulary is the *container* vocabulary, not whatever
    /// subset the backend happens to have overridden for docblocks: because an
    /// absent key falls back to `[types.containers]`, every container name is
    /// a meaningful docblock key whether or not the compiled-in manifest
    /// spells it out. Validating against `docblock_containers` alone would
    /// reject `range` on a PHP manifest that only overrides `array`, which is
    /// a legitimate thing to want.
    #[serde(default)]
    pub docblock_containers: AHashMap<String, String>,
}

/// Overlay half of [`NamingConfig`]. Every field replaces its compiled-in
/// counterpart whole; omitted fields inherit.
///
/// The field list is enumerated by hand rather than derived from
/// [`NamingConfig`], and that is deliberate: it is an allowlist, not a
/// mirror. A field added to `NamingConfig` later is *not* overridable until
/// someone adds it here on purpose — which is the desired default for any
/// field that is internal to codegen (e.g. one carrying `#[serde(skip)]`),
/// since exposing it would let an override reach machinery that never
/// contracted to be user-configurable. `deny_unknown_fields` makes the gap
/// loud: naming such a field in an override is a parse error, not a silent
/// no-op. Treat an omission here as a decision, not an oversight.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingOverlay {
    #[serde(default)]
    pub struct_case: Option<String>,
    #[serde(default)]
    pub fn_case: Option<String>,
    #[serde(default)]
    pub enum_variant_case: Option<String>,
    #[serde(default)]
    pub row_suffix: Option<String>,
}

/// Overlay half of [`ImportConfig`]. Merge-and-add; see [`ManifestOverlay`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportConfigOverlay {
    #[serde(default)]
    pub rules: AHashMap<String, String>,
}

/// Parse a [`ManifestOverlay`] from TOML source.
///
/// The error is the raw `toml` message, which for an unknown key already
/// names the offending key and the accepted alternatives. Callers are
/// expected to prefix it with the backend name and the resolved override
/// path, which this function does not know.
pub fn parse_overlay(content: &str) -> Result<ManifestOverlay, BackendError> {
    toml::from_str(content).map_err(|e| BackendError::ManifestError(e.to_string()))
}

/// Levenshtein distance, used only to suggest a near-miss key in an
/// unknown-key error. Iterative two-row variant: it keeps two rows of
/// `b.len() + 1` cells rather than the full `a * b` matrix, which matters
/// because it runs once per candidate key in the target map.
fn edit_distance(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut current = vec![0usize; b_chars.len() + 1];

    for (i, a_char) in a.chars().enumerate() {
        current[0] = i + 1;
        for (j, &b_char) in b_chars.iter().enumerate() {
            let substitution_cost = usize::from(a_char != b_char);
            current[j + 1] = (prev[j] + substitution_cost).min(prev[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut prev, &mut current);
    }

    prev[b_chars.len()]
}

/// The closest key in `candidates` to `key`, if any is within a small edit
/// distance. The threshold scales with key length so short names like `date`
/// do not collect spurious suggestions.
fn closest_key<'a>(key: &str, candidates: impl Iterator<Item = &'a String>) -> Option<&'a String> {
    let threshold = if key.len() <= 4 { 1 } else { 2 };
    candidates
        .map(|candidate| (edit_distance(key, candidate), candidate))
        .filter(|(distance, _)| *distance <= threshold)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

/// Reject any `overlay` key outside `vocabulary`.
///
/// Split out from [`merge_replace_only`] because one table's accepted
/// vocabulary is not its own key set: `[types.docblock_containers]` falls back
/// to `[types.containers]`, so it accepts every container name, including the
/// ones it does not itself spell out.
fn reject_unknown_keys(
    overlay: &AHashMap<String, String>,
    vocabulary: &[String],
    section: &str,
) -> Result<(), BackendError> {
    // Sorted so a manifest with several bad keys reports the same one on
    // every run; hash-map iteration order is not stable across processes.
    let mut unknown: Vec<&String> = overlay
        .keys()
        .filter(|key| !vocabulary.iter().any(|known| known == *key))
        .collect();
    unknown.sort();

    let Some(key) = unknown.first() else {
        return Ok(());
    };

    let suggestion = match closest_key(key, vocabulary.iter()) {
        Some(near) => format!(" (did you mean '{near}'?)"),
        None => String::new(),
    };
    Err(BackendError::ManifestError(format!(
        "unknown [{section}] key '{key}'{suggestion}; \
         this table may only override mappings the backend already defines"
    )))
}

/// Merge `overlay` entries into `target`, rejecting keys that `target` does
/// not already define. `section` names the TOML table for the error message
/// (e.g. `"types.scalars"`).
fn merge_replace_only(
    target: &mut AHashMap<String, String>,
    overlay: &AHashMap<String, String>,
    section: &str,
) -> Result<(), BackendError> {
    let vocabulary: Vec<String> = target.keys().cloned().collect();
    reject_unknown_keys(overlay, &vocabulary, section)?;

    for (key, value) in overlay {
        target.insert(key.clone(), value.clone());
    }
    Ok(())
}

impl BackendManifest {
    /// Merge a partial [`ManifestOverlay`] into this manifest in place.
    ///
    /// See [`ManifestOverlay`] for the merge granularity and for which
    /// sections accept new keys. On error the manifest may be partially
    /// merged, so callers must treat a failure as fatal rather than
    /// continuing with the backend — which is what `scythe generate` does.
    pub fn apply_overlay(&mut self, overlay: &ManifestOverlay) -> Result<(), BackendError> {
        if let Some(ref types) = overlay.types {
            merge_replace_only(&mut self.types.scalars, &types.scalars, "types.scalars")?;
            merge_replace_only(&mut self.types.containers, &types.containers, "types.containers")?;

            // Validated against both tables, then written into the docblock
            // one -- see `TypeMappingsOverlay::docblock_containers`. The
            // vocabulary is collected before the mutable borrow so the union
            // can include keys the docblock table does not yet have.
            let vocabulary: Vec<String> = self
                .types
                .containers
                .keys()
                .chain(self.types.docblock_containers.keys())
                .cloned()
                .collect();
            reject_unknown_keys(&types.docblock_containers, &vocabulary, "types.docblock_containers")?;
            for (key, value) in &types.docblock_containers {
                self.types.docblock_containers.insert(key.clone(), value.clone());
            }
        }

        if let Some(ref naming) = overlay.naming {
            if let Some(ref value) = naming.struct_case {
                self.naming.struct_case = value.clone();
            }
            if let Some(ref value) = naming.fn_case {
                self.naming.fn_case = value.clone();
            }
            if let Some(ref value) = naming.enum_variant_case {
                self.naming.enum_variant_case = value.clone();
            }
            if let Some(ref value) = naming.row_suffix {
                self.naming.row_suffix = value.clone();
            }
        }

        if let Some(ref imports) = overlay.imports {
            let rules = &mut self
                .imports
                .get_or_insert_with(|| ImportConfig { rules: AHashMap::new() })
                .rules;
            for (key, value) in &imports.rules {
                rules.insert(key.clone(), value.clone());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_manifest_from_string() {
        let toml_str = include_str!("../test-manifests/rust-sqlx.toml");
        let manifest: BackendManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.backend.name, "rust-sqlx");
        assert_eq!(manifest.backend.language, "rust");
        assert_eq!(manifest.backend.file_extension, "rs");
        assert_eq!(manifest.types.scalars["int32"], "i32");
        assert_eq!(manifest.types.containers["array"], "Vec<{T}>");
        assert_eq!(manifest.naming.struct_case, "PascalCase");
        assert_eq!(manifest.naming.row_suffix, "Row");
    }

    #[test]
    fn test_load_tokio_postgres_manifest() {
        let toml_str = include_str!("../test-manifests/rust-tokio-postgres.toml");
        let manifest: BackendManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.backend.name, "rust-tokio-postgres");
        assert_eq!(manifest.backend.language, "rust");
        assert_eq!(manifest.backend.file_extension, "rs");
        assert_eq!(manifest.backend.engine, "postgresql");
        assert_eq!(manifest.types.scalars["int32"], "i32");
        assert_eq!(manifest.types.scalars["inet"], "std::net::IpAddr");
        assert_eq!(manifest.types.scalars["time_tz"], "chrono::NaiveTime");
        assert_eq!(manifest.types.scalars["interval"], "String");
        assert_eq!(manifest.types.containers["array"], "Vec<{T}>");
        assert_eq!(manifest.types.containers["json_typed"], "{T}");
        assert_eq!(manifest.types.containers["range"], "String");
        assert_eq!(manifest.naming.struct_case, "PascalCase");
        assert_eq!(manifest.naming.row_suffix, "Row");
        let imports = manifest.imports.unwrap();
        assert_eq!(imports.rules["std::net::"], "use std::net::IpAddr;");
    }

    fn base_manifest() -> BackendManifest {
        toml::from_str(include_str!("../test-manifests/rust-sqlx.toml")).unwrap()
    }

    /// Map-valued tables merge per leaf key: the mentioned key is replaced and
    /// every unmentioned key keeps its compiled-in value. This is the whole
    /// point of the overlay — retargeting one scalar must not require
    /// restating the other thirty.
    #[test]
    fn apply_overlay_replaces_only_the_named_scalar() {
        let mut manifest = base_manifest();
        let overlay: ManifestOverlay = toml::from_str("[types.scalars]\nint32 = \"MyInt\"\n").unwrap();

        manifest.apply_overlay(&overlay).unwrap();

        assert_eq!(manifest.types.scalars["int32"], "MyInt");
        assert_eq!(manifest.types.scalars["int64"], "i64", "unmentioned keys must survive");
        assert_eq!(manifest.types.scalars["string"], "String");
    }

    /// Containers merge with the same per-leaf-key granularity as scalars.
    #[test]
    fn apply_overlay_replaces_only_the_named_container() {
        let mut manifest = base_manifest();
        let overlay: ManifestOverlay = toml::from_str("[types.containers]\narray = \"MyVec<{T}>\"\n").unwrap();

        manifest.apply_overlay(&overlay).unwrap();

        assert_eq!(manifest.types.containers["array"], "MyVec<{T}>");
        assert_eq!(manifest.types.containers["nullable"], "Option<{T}>");
    }

    /// A manifest with no `[types.docblock_containers]` table -- which is
    /// every manifest but the PHP ones -- parses, and gets an empty table
    /// rather than a missing-field error.
    #[test]
    fn docblock_containers_defaults_to_empty_when_the_table_is_absent() {
        let manifest = base_manifest();
        assert!(manifest.types.docblock_containers.is_empty());
    }

    #[test]
    fn apply_overlay_replaces_only_the_named_docblock_container() {
        let mut manifest = base_manifest();
        manifest
            .types
            .docblock_containers
            .insert("array".to_string(), "list<{T}>".to_string());
        let overlay: ManifestOverlay =
            toml::from_str("[types.docblock_containers]\narray = \"non-empty-list<{T}>\"\n").unwrap();

        manifest.apply_overlay(&overlay).unwrap();

        assert_eq!(manifest.types.docblock_containers["array"], "non-empty-list<{T}>");
        assert_eq!(
            manifest.types.containers["array"], "Vec<{T}>",
            "the docblock table must not write through to the native one"
        );
    }

    /// The vocabulary is the *container* vocabulary, not the docblock table's
    /// own keys: an absent key falls back to `[types.containers]`, so every
    /// container name is a meaningful docblock key.
    #[test]
    fn apply_overlay_accepts_a_docblock_key_only_the_native_table_declares() {
        let mut manifest = base_manifest();
        assert!(manifest.types.docblock_containers.is_empty());
        let overlay: ManifestOverlay =
            toml::from_str("[types.docblock_containers]\nrange = \"RangeOf<{T}>\"\n").unwrap();

        manifest.apply_overlay(&overlay).unwrap();

        assert_eq!(manifest.types.docblock_containers["range"], "RangeOf<{T}>");
    }

    #[test]
    fn apply_overlay_rejects_a_docblock_key_no_container_table_declares() {
        let mut manifest = base_manifest();
        let overlay: ManifestOverlay =
            toml::from_str("[types.docblock_containers]\nnotacontainer = \"X<{T}>\"\n").unwrap();

        let error = manifest.apply_overlay(&overlay).unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains("types.docblock_containers") && message.contains("notacontainer"),
            "error must name the table and the offending key, got: {message}"
        );
    }

    /// `[naming]` fields replace whole values; omitted fields inherit.
    #[test]
    fn apply_overlay_replaces_named_fields_and_inherits_the_rest() {
        let mut manifest = base_manifest();
        let overlay: ManifestOverlay = toml::from_str("[naming]\nrow_suffix = \"Record\"\n").unwrap();

        manifest.apply_overlay(&overlay).unwrap();

        assert_eq!(manifest.naming.row_suffix, "Record");
        assert_eq!(
            manifest.naming.struct_case, "PascalCase",
            "an omitted naming field must inherit"
        );
    }

    /// `[imports.rules]` is merge-and-add: its keys are prefixes of the
    /// *generated* language types, so retargeting a scalar necessarily
    /// requires a rule keyed on a prefix the compiled-in manifest never had.
    #[test]
    fn apply_overlay_adds_new_import_rules() {
        let mut manifest = base_manifest();
        let overlay: ManifestOverlay = toml::from_str("[imports.rules]\n\"mycrate::\" = \"use mycrate;\"\n").unwrap();

        manifest.apply_overlay(&overlay).unwrap();

        let rules = &manifest.imports.as_ref().unwrap().rules;
        assert_eq!(rules["mycrate::"], "use mycrate;");
        assert_eq!(rules["chrono::"], "use chrono;", "existing rules must survive");
    }

    /// Scalars are replace-only: an unrecognised neutral type name is a typo,
    /// and accepting it silently would leave the intended key at its
    /// compiled-in value.
    #[test]
    fn apply_overlay_rejects_unknown_scalar_key_with_a_suggestion() {
        let mut manifest = base_manifest();
        let overlay: ManifestOverlay = toml::from_str("[types.scalars]\nint_32 = \"MyInt\"\n").unwrap();

        let error = manifest.apply_overlay(&overlay).unwrap_err().to_string();

        assert!(error.contains("int_32"), "error must name the offending key: {error}");
        assert!(error.contains("types.scalars"), "error must name the section: {error}");
        assert!(
            error.contains("int32"),
            "error should suggest the near-miss key: {error}"
        );
    }

    /// Containers are replace-only for the same reason as scalars.
    #[test]
    fn apply_overlay_rejects_unknown_container_key() {
        let mut manifest = base_manifest();
        let overlay: ManifestOverlay = toml::from_str("[types.containers]\nslice = \"Box<[{T}]>\"\n").unwrap();

        let error = manifest.apply_overlay(&overlay).unwrap_err().to_string();

        assert!(error.contains("slice"), "error must name the offending key: {error}");
        assert!(
            error.contains("types.containers"),
            "error must name the section: {error}"
        );
    }

    /// `deny_unknown_fields` is what turns a misspelled section into a loud
    /// parse failure instead of a silently ignored table.
    #[test]
    fn parse_overlay_rejects_unknown_sections() {
        // Dropping the `types.` prefix is the likeliest real mistake: the
        // section name itself is spelled correctly, so only the closed field
        // set catches it.
        let error = parse_overlay("[scalars]\nint32 = \"MyInt\"\n").unwrap_err().to_string();
        assert!(error.contains("scalars"), "error must name the offending key: {error}");
        assert!(
            error.contains("types"),
            "error should list the accepted sections: {error}"
        );
    }

    /// `[backend]` is identity, not configuration: manifest selection stays a
    /// pure function of `(backend, engine)` (#82), so an overlay may not
    /// rewrite the engine or the file extension.
    #[test]
    fn parse_overlay_rejects_the_backend_section() {
        let error = parse_overlay("[backend]\nengine = \"mysql\"\n")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("backend"),
            "error must name the rejected section: {error}"
        );
    }

    /// A misspelled `[naming]` *field* is caught too, not just a misspelled
    /// section.
    #[test]
    fn parse_overlay_rejects_unknown_naming_fields() {
        let error = parse_overlay("[naming]\nrow_prefix = \"X\"\n").unwrap_err().to_string();
        assert!(
            error.contains("row_prefix"),
            "error must name the offending key: {error}"
        );
    }

    /// A non-string mapping value is a type error, not a coercion.
    #[test]
    fn parse_overlay_rejects_non_string_mappings() {
        assert!(parse_overlay("[types.scalars]\nint32 = 42\n").is_err());
    }

    /// An empty overlay is legal and is a no-op.
    #[test]
    fn apply_overlay_of_an_empty_overlay_changes_nothing() {
        let mut manifest = base_manifest();
        let before = manifest.types.scalars.clone();

        manifest.apply_overlay(&parse_overlay("").unwrap()).unwrap();

        assert_eq!(manifest.types.scalars, before);
    }
}
