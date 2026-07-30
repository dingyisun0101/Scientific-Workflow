//! Template-defined, heterogeneous scientific system states.
//!
//! This module is the complete public boundary for describing one scientific
//! system at a particular time point. A program first loads a JSON template
//! into [`StateSpec`], constructs its initial blank [`SystemState`], and then
//! moves concrete Rust payloads into and out of the declared fields.
//!
//! # Public workflow
//!
//! 1. Load and validate a template with [`StateSpec::load`].
//! 2. Create the initial state with [`StateSpec::empty`].
//! 3. Insert, borrow, mutate, or extract payloads through [`SystemState`].
//! 4. Create later blank states with [`SystemState::empty`].
//!
//! The template fixes field names, field order, and stable serialization type
//! tags. Individual payload slots may be empty, but callers cannot add,
//! remove, or reorder fields after the template is loaded.
//!
//! # Ownership
//!
//! [`SystemState::set`] consumes a concrete payload, while
//! [`SystemState::take`] returns ownership of that payload. These operations
//! do not invoke `Clone`, which allows large scientific allocations to move
//! through the state boundary without copying their contents. Explicitly
//! cloning a [`SystemState`] is intentionally different: populated payloads
//! are deeply cloned so the resulting states can be mutated independently.
//!
//! # Encapsulation
//!
//! Runtime type erasure and boxing are private implementation details.
//! Downstream crates interact only with concrete types through generic state
//! methods. Template parsing representations, compact field indices, and
//! name-to-slot lookup tables are likewise hidden behind the public types
//! re-exported below.

mod error;
mod spec;
mod state;
mod value;

pub use error::StateError;
pub use spec::{FieldSpec, StateSpec};
pub use state::{SystemState, TimePoint};
