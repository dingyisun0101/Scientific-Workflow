//! Immutable state schemas loaded from JSON templates.
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
//! `SystemStateSchema` is a small cloneable handle around an immutable, reference-
//! counted layout. Cloning it never duplicates field names or lookup tables.
//! Field lookup uses a hash map, while iteration preserves JSON declaration
//! order through the field slice.
//!
//! # Construction boundary
//!
//! Public callers load the first specification from a JSON template path using
//! [`SystemStateSchema::load_json_template`]. A crate-private byte parser applies the identical
//! validation path for persistence readers that recover an embedded template
//! from the sole dataset metadata file. Keeping that parser crate-private
//! preserves the public file-template initialization contract.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::error::StateError;
use super::field::StateFieldSchema;
use super::state::SystemState;
use super::time::StateTime;

/// A validated, shareable SystemState layout.
///
/// `SystemStateSchema` owns an [`Arc`] to immutable metadata, making `Clone` a cheap
/// reference-count increment. Every state derived from a specification shares
/// the exact field order and name lookup table.
#[derive(Clone, Debug)]
pub struct SystemStateSchema {
    inner: Arc<StateLayout>,
}

impl SystemStateSchema {
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
    pub fn load_json_template(path: &Path) -> Result<Self, StateError> {
        let source = path.to_path_buf();
        let bytes = fs::read(&source).map_err(|error| StateError::TemplateRead {
            path: source.clone(),
            source: error,
        })?;

        Self::parse(source, &bytes)
    }

    /// Parses and validates a specification from an in-memory JSON document.
    ///
    /// This is the internal reconstruction boundary used by persistence
    /// readers. `source` identifies the containing metadata file for provenance
    /// and errors; it need not be a standalone state-template path. Parsing
    /// uses the same strict Serde representation and semantic validation as
    /// [`SystemStateSchema::load_json_template`].
    ///
    /// The method is crate-private so application code cannot bypass the
    /// required public initialization from a JSON template file.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::TemplateParse`] for invalid JSON structure or
    /// syntax and the same semantic template variants documented by
    /// [`SystemStateSchema::load_json_template`]. The input bytes are borrowed only for this call.
    pub(crate) fn parse(source: PathBuf, bytes: &[u8]) -> Result<Self, StateError> {
        let template: StateTemplate =
            serde_json::from_slice(bytes).map_err(|error| StateError::TemplateParse {
                path: source.clone(),
                source: error,
            })?;

        Self::from_template(source, template)
    }

    /// Validates a centrally parsed JSON value without a second file read.
    pub(crate) fn from_json_template_value(
        source_path: &Path,
        document: &serde_json::Value,
    ) -> Result<Self, StateError> {
        let template =
            StateTemplate::deserialize(document).map_err(|source| StateError::TemplateParse {
                path: source_path.to_path_buf(),
                source,
            })?;
        Self::from_template(source_path.to_path_buf(), template)
    }

    /// Creates an empty SystemState that shares this specification.
    ///
    /// Every declared field exists in the returned state's layout, while every
    /// payload slot starts empty. Cloning the specification is constant-time
    /// and does not duplicate layout data.
    pub fn create_empty_state(&self, time: StateTime) -> SystemState {
        SystemState::new(self.clone(), time)
    }

    /// Resolves a declared field name to its payload-slot index.
    ///
    /// This is crate-private because compact indices are an implementation
    /// detail. Public callers address fields by name or inspect [`StateFieldSchema`].
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

            let field = StateFieldSchema::new(index, name, declaration.description.as_deref());
            by_name.insert(field.owned_name(), index);
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

/// Advanced inspection and tooling operations for a validated state schema.
///
/// Import this trait from [`crate::state::advanced`] when schema metadata is
/// needed. Ordinary state construction requires only the inherent basic API.
pub trait StateSchemaAccess {
    /// Reports whether two schema handles share one immutable allocation.
    ///
    /// This is an identity comparison, not structural equality. Independently
    /// loaded but textually identical schemas do not share an instance.
    fn shares_schema_instance(&self, other: &SystemStateSchema) -> bool;

    /// Returns the path from which this schema was loaded.
    fn template_path(&self) -> &Path;

    /// Returns field descriptions in deterministic template order.
    fn field_schemas(&self) -> &[StateFieldSchema];

    /// Looks up one field description by normalized name.
    fn field_schema(&self, name: &str) -> Option<&StateFieldSchema>;

    /// Reports whether the schema declares `name`.
    fn contains_field(&self, name: &str) -> bool;

    /// Returns the number of declared fields.
    fn len(&self) -> usize;

    /// Reports whether the schema declares no fields.
    fn is_empty(&self) -> bool;

    /// Converts this schema to the strict, pretty-printed JSON template format.
    fn to_json_template(&self) -> Result<String, serde_json::Error>;
}

impl StateSchemaAccess for SystemStateSchema {
    fn shares_schema_instance(&self, other: &SystemStateSchema) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn template_path(&self) -> &Path {
        &self.inner.source
    }

    fn field_schemas(&self) -> &[StateFieldSchema] {
        &self.inner.fields
    }

    fn field_schema(&self, name: &str) -> Option<&StateFieldSchema> {
        let index = self.inner.by_name.get(name)?;
        self.inner.fields.get(*index)
    }

    fn contains_field(&self, name: &str) -> bool {
        self.inner.by_name.contains_key(name)
    }

    fn len(&self) -> usize {
        self.inner.fields.len()
    }

    fn is_empty(&self) -> bool {
        self.inner.fields.is_empty()
    }

    fn to_json_template(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&StateTemplateRef {
            fields: self.field_schemas(),
        })
    }
}

/// Immutable metadata shared by every clone of a [`SystemStateSchema`].
#[derive(Debug)]
struct StateLayout {
    /// Original template path retained for provenance and diagnostics.
    source: PathBuf,
    /// Validated fields in deterministic template order.
    fields: Vec<StateFieldSchema>,
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
    fields: &'a [StateFieldSchema],
}
