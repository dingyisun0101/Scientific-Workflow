//! Immutable field specifications loaded from a JSON state template.
//!
//! A scientific program establishes its SystemState layout before constructing
//! states. This module loads that layout, validates field declarations, assigns
//! compact deterministic field indices, and shares the resulting metadata
//! among all derived states.
//!
//! # Template format
//!
//! The accepted JSON document has one ordered `fields` array:
//!
//! ```json
//! {
//!   "fields": [
//!     {
//!       "name": "population",
//!       "description": "Population count for each simulated region"
//!     },
//!     {"name": "space"}
//!   ]
//! }
//! ```
//!
//! Array order is significant. It assigns each field a zero-based index used
//! by the compact payload-slot vector in `SystemState`. Names and present
//! descriptions are trimmed. Missing, null, empty, and whitespace-only
//! descriptions all normalize to no description. Unknown JSON properties are
//! rejected so misspelled template configuration cannot be silently ignored.
//! Payload types and storage encodings deliberately do not belong here.
//!
//! # Sharing and performance
//!
//! `StateSpec` is a small cloneable handle around an immutable, reference-
//! counted layout. Cloning it never duplicates field names or lookup tables.
//! Field lookup uses a hash map, while iteration preserves JSON declaration
//! order through the field slice.
//!
//! # Construction boundary
//!
//! Public callers load the first specification from a JSON template path using
//! [`StateSpec::load`]. A crate-private byte parser applies the identical
//! validation path for persistence readers that recover an embedded template
//! from the sole dataset metadata file. Keeping that parser crate-private
//! preserves the public file-template initialization contract.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::error::StateError;
use super::state::{SystemState, TimePoint};

/// One validated field in a state template.
///
/// A field specification is immutable after template loading. Its index is the
/// position of the corresponding payload slot in every `SystemState` created
/// from the same [`StateSpec`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FieldSpec {
    #[serde(skip)]
    index: usize,
    name: Box<str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<Box<str>>,
}

impl FieldSpec {
    /// Constructs one normalized field definition.
    fn new(index: usize, name: &str, description: Option<&str>) -> Self {
        Self {
            index,
            name: name.trim().into(),
            description: description
                .map(str::trim)
                .filter(|description| !description.is_empty())
                .map(Into::into),
        }
    }

    /// Returns the zero-based payload-slot index assigned by template order.
    pub fn index(&self) -> usize {
        self.index
    }

    /// Returns the field name used by typed SystemState accessors.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional natural-language description of the payload.
    ///
    /// Descriptions are documentation only. They do not identify a Rust type,
    /// select a decoder, or affect typed access through `SystemState`.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// A validated, shareable SystemState layout.
///
/// `StateSpec` owns an [`Arc`] to immutable metadata, making `Clone` a cheap
/// reference-count increment. Every state derived from a specification shares
/// the exact field order and name lookup table.
#[derive(Clone, Debug)]
pub struct StateSpec {
    inner: Arc<StateLayout>,
}

impl StateSpec {
    /// Loads and validates a state specification from a JSON template.
    ///
    /// The file is read as bytes and parsed directly, avoiding an intermediate
    /// UTF-8 `String` allocation. The returned specification retains the input
    /// path for diagnostics and provenance but does not canonicalize it or keep
    /// the file open.
    ///
    /// # Errors
    ///
    /// Returns:
    ///
    /// - [`StateError::TemplateRead`] when the file cannot be read;
    /// - [`StateError::TemplateParse`] when JSON syntax or structure is invalid;
    /// - [`StateError::EmptyFieldName`] for an empty normalized field name;
    /// - [`StateError::DuplicateField`] for repeated normalized names.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, StateError> {
        let source = path.as_ref().to_path_buf();
        let bytes = fs::read(&source).map_err(|error| StateError::TemplateRead {
            path: source.clone(),
            source: error,
        })?;

        Self::parse(source, &bytes)
    }

    /// Parses and validates a specification from an in-memory JSON document.
    ///
    /// This is the internal reconstruction boundary for a future persistence
    /// reader. `source` identifies the containing metadata file for provenance
    /// and errors; it need not be a standalone state-template path. Parsing
    /// uses the same strict Serde representation and semantic validation as
    /// [`StateSpec::load`].
    ///
    /// The method is crate-private so application code cannot bypass the
    /// required public initialization from a JSON template file.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::TemplateParse`] for invalid JSON structure or
    /// syntax and the same semantic template variants documented by
    /// [`StateSpec::load`]. The input bytes are borrowed only for this call.
    pub(crate) fn parse(source: PathBuf, bytes: &[u8]) -> Result<Self, StateError> {
        let template: StateTemplate =
            serde_json::from_slice(bytes).map_err(|error| StateError::TemplateParse {
                path: source.clone(),
                source: error,
            })?;

        Self::from_template(source, template)
    }

    /// Creates an empty SystemState that shares this specification.
    ///
    /// Every declared field exists in the returned state's layout, while every
    /// payload slot starts empty. Cloning the specification is constant-time
    /// and does not duplicate layout data.
    pub fn empty(&self, time: TimePoint) -> SystemState {
        SystemState::new(self.clone(), time)
    }

    /// Converts this specification into a pretty-printed JSON template.
    ///
    /// The generated document has the same strict `fields` structure accepted
    /// by [`StateSpec::load`]. Runtime-only field indices and the source path
    /// are omitted: field indices are reconstructed from array order, and the
    /// destination path becomes the source when the JSON is loaded again.
    ///
    /// Serialization borrows the immutable field slice and does not clone
    /// field names or descriptions. Absent descriptions are omitted.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`serde_json::Error`] if JSON serialization
    /// fails.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&StateTemplateRef {
            fields: self.fields(),
        })
    }

    /// Returns the path from which this specification was loaded.
    ///
    /// The path is retained exactly as supplied to [`StateSpec::load`]. It may
    /// be relative and is not guaranteed to remain accessible after loading.
    pub fn source(&self) -> &Path {
        &self.inner.source
    }

    /// Returns field definitions in deterministic template order.
    pub fn fields(&self) -> &[FieldSpec] {
        &self.inner.fields
    }

    /// Returns the number of declared fields.
    pub fn len(&self) -> usize {
        self.inner.fields.len()
    }

    /// Reports whether the template declares no fields.
    ///
    /// Empty templates are structurally valid and can represent time-bearing
    /// event records without scientific payloads.
    pub fn is_empty(&self) -> bool {
        self.inner.fields.is_empty()
    }

    /// Looks up a field definition by its normalized name.
    pub fn get(&self, name: &str) -> Option<&FieldSpec> {
        let index = self.inner.by_name.get(name)?;
        self.inner.fields.get(*index)
    }

    /// Reports whether the template declares `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.inner.by_name.contains_key(name)
    }

    /// Reports whether two specification handles share one immutable layout.
    ///
    /// This is an identity comparison, not structural equality. Two templates
    /// loaded independently may declare identical fields but still return
    /// `false`; states derived by cloning one `StateSpec` return `true` without
    /// comparing field names, descriptions, source paths, or lookup maps.
    ///
    /// Identity is useful when building a homogeneous collection of states.
    /// Once a collection accepts only states sharing its canonical layout,
    /// later indexing and serialization can rely on one field order without
    /// repeating structural comparisons.
    pub fn shares_layout(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Resolves a declared field name to its payload-slot index.
    ///
    /// This is crate-private because compact indices are an implementation
    /// detail. Public callers address fields by name or inspect [`FieldSpec`].
    pub(crate) fn index_of(&self, name: &str) -> Result<usize, StateError> {
        self.inner
            .by_name
            .get(name)
            .copied()
            .ok_or_else(|| StateError::UnknownField {
                field: name.to_owned(),
            })
    }

    /// Validates a parsed template and constructs its shared lookup layout.
    fn from_template(source: PathBuf, template: StateTemplate) -> Result<Self, StateError> {
        let mut fields = Vec::with_capacity(template.fields.len());
        let mut by_name = HashMap::with_capacity(template.fields.len());

        for (index, declaration) in template.fields.into_iter().enumerate() {
            let name = declaration.name.trim();
            if name.is_empty() {
                return Err(StateError::EmptyFieldName { index });
            }

            if by_name.contains_key(name) {
                return Err(StateError::DuplicateField {
                    field: name.to_owned(),
                });
            }

            let field = FieldSpec::new(index, name, declaration.description.as_deref());
            by_name.insert(field.name.clone(), index);
            fields.push(field);
        }

        Ok(Self {
            inner: Arc::new(StateLayout {
                source,
                fields,
                by_name,
            }),
        })
    }
}

/// Immutable metadata shared by every clone of a [`StateSpec`].
#[derive(Debug)]
struct StateLayout {
    /// Original template path retained for provenance and diagnostics.
    source: PathBuf,
    /// Validated fields in deterministic template order.
    fields: Vec<FieldSpec>,
    /// Normalized field name to compact payload-slot index.
    by_name: HashMap<Box<str>, usize>,
}

/// Serde-only representation of the top-level JSON template.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateTemplate {
    /// Ordered field declarations.
    fields: Vec<FieldDeclaration>,
}

/// Serde-only representation of one JSON field declaration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldDeclaration {
    /// Human-facing dictionary key.
    name: String,
    /// Optional human-facing payload documentation.
    #[serde(default)]
    description: Option<String>,
}

/// Borrowed serialization view of a validated state specification.
///
/// Keeping this separate from [`StateTemplate`] prevents deserialization-only
/// owned strings from being allocated when converting an existing
/// specification back to JSON.
#[derive(Serialize)]
struct StateTemplateRef<'a> {
    /// Fields borrowed in deterministic template order.
    fields: &'a [FieldSpec],
}
