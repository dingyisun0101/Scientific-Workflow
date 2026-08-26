//! Immutable descriptions of fields declared by a state schema.

use serde::Serialize;

/// One validated field in a state template.
///
/// A field description is immutable after template loading. Its position is
/// the corresponding payload slot in every state sharing the schema.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StateFieldSchema {
    #[serde(skip)]
    index: usize,
    name: Box<str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<Box<str>>,
}

impl StateFieldSchema {
    pub(super) fn new(index: usize, name: &str, description: Option<&str>) -> Self {
        Self {
            index,
            name: name.trim().into(),
            description: description
                .map(str::trim)
                .filter(|description| !description.is_empty())
                .map(Into::into),
        }
    }

    /// Returns the zero-based payload-slot position assigned by template order.
    pub fn position(&self) -> usize {
        self.index
    }

    /// Returns the field name used by typed state accessors.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional natural-language payload description.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub(super) fn owned_name(&self) -> Box<str> {
        self.name.clone()
    }
}
