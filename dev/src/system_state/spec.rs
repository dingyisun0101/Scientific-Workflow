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
//!     {"name": "population", "type": "vec.u64"},
//!     {"name": "space", "type": "example.lattice.v1"}
//!   ]
//! }
//! ```
//!
//! Array order is significant. It assigns each field a zero-based index used
//! by the compact payload-slot vector in `SystemState`. Names and type tags are
//! trimmed before validation and storage. Unknown JSON properties are rejected
//! so misspelled template configuration cannot be silently ignored.
//!
//! # Type tags
//!
//! A field's `type` is a stable serialization tag, not a Rust type name.
//! Runtime Rust types are still checked by `SystemState` through `TypeId`.
//! Connecting stable tags to codecs is a later storage-layer responsibility.
//!
//! # Sharing and performance
//!
//! `StateSpec` is a small cloneable handle around an immutable, reference-
//! counted layout. Cloning it never duplicates field names or lookup tables.
//! Field lookup uses a hash map, while iteration preserves JSON declaration
//! order through the field slice.

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
    #[serde(rename = "type")]
    type_tag: Box<str>,
}

impl FieldSpec {
    /// Constructs one normalized field definition.
    fn new(index: usize, name: &str, type_tag: &str) -> Self {
        Self {
            index,
            name: name.trim().into(),
            type_tag: type_tag.trim().into(),
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

    /// Returns the stable codec type tag declared by the template.
    ///
    /// The tag identifies serialized meaning across processes and versions. It
    /// must not be interpreted as `std::any::type_name::<T>()`.
    pub fn type_tag(&self) -> &str {
        &self.type_tag
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
    /// - [`StateError::DuplicateField`] for repeated normalized names;
    /// - [`StateError::EmptyTypeTag`] for an empty normalized type tag.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, StateError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| StateError::TemplateRead {
            path: path.to_path_buf(),
            source,
        })?;
        let template: StateTemplate =
            serde_json::from_slice(&bytes).map_err(|source| StateError::TemplateParse {
                path: path.to_path_buf(),
                source,
            })?;

        Self::from_template(path.to_path_buf(), template)
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
    /// field names or type tags.
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
    /// Empty templates are structurally valid. They can represent a
    /// time-bearing event stream whose payload schema will be extended in a
    /// later template revision.
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

            let type_tag = declaration.type_tag.trim();
            if type_tag.is_empty() {
                return Err(StateError::EmptyTypeTag {
                    field: name.to_owned(),
                });
            }

            if by_name.contains_key(name) {
                return Err(StateError::DuplicateField {
                    field: name.to_owned(),
                });
            }

            let field = FieldSpec::new(index, name, type_tag);
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
    /// Stable serialization tag; `type` is renamed because it is a Rust
    /// keyword.
    #[serde(rename = "type")]
    type_tag: String,
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
