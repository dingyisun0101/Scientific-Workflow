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
//! 4. Mutate time through [`SystemState::set_time`] or
//!    [`SystemState::advance`].
//! 5. Create later blank states with [`SystemState::empty`].
//!
//! The template fixes field names, field order, and optional human-facing
//! descriptions. It contains no Rust type or storage codec information.
//! Individual payload slots may be empty, but callers cannot add, remove, or
//! reorder fields after the template is loaded.
//!
//! Every inserted payload implements Serde `Serialize`, `Clone`, `Send`, and
//! `'static`. Serialization is supplied by the payload type itself; this
//! module only retains a private borrowed erased view for the future storage
//! encoder. It does not select JSON framing or perform IO.
//!
//! # Ownership
//!
//! [`SystemState::set`] consumes a concrete payload without cloning it. An
//! insertion into an empty slot returns `None`; replacement returns the
//! previous payload as `Some(T)`, preserving its ownership instead of dropping
//! it. A rejected insertion returns [`SetError<T>`], from which the unchanged
//! incoming payload can be recovered.
//!
//! [`SystemState::take`] moves a stored payload back to the caller. Together,
//! `set` and `take` allow large scientific allocations to cross the state
//! boundary without copying their contents. Explicitly cloning a
//! [`SystemState`] is intentionally different: it creates a new erased box and
//! invokes each populated payload's `Clone` implementation. Clone depth is
//! therefore defined by the concrete payload type.
//!
//! The public insertion contract deliberately makes replacement visible:
//!
//! ```no_run
//! use scientific_workflow::system_state::{StateSpec, TimePoint};
//!
//! # fn example(spec: &StateSpec) -> Result<(), Box<dyn std::error::Error>> {
//! let mut state = spec.empty(TimePoint::new(0));
//!
//! let previous = state.set("population", vec![1_u64, 2, 3])?;
//! assert!(previous.is_none());
//!
//! let previous = state.set("population", vec![4_u64, 5, 6])?;
//! assert_eq!(previous, Some(vec![1, 2, 3]));
//!
//! let time = state.advance(None)?;
//! assert_eq!(time.index(), 1);
//! # Ok(())
//! # }
//! ```
//!
//! Ignoring a successful replacement result would drop the displaced payload.
//! Callers should bind or explicitly drop the returned `Option<T>` so that
//! ownership disposal is intentional.
//!
//! # Encapsulation
//!
//! Runtime type erasure and boxing are private implementation details.
//! Downstream crates interact only with concrete types through generic state
//! methods. Template parsing representations, compact field indices, and
//! name-to-slot lookup tables are likewise hidden behind the public types
//! re-exported below.
//!
//! Mutable payload access intentionally borrows one field at a time. Callers
//! must end one `get_mut` borrow before requesting another; tightly coupled
//! data that requires simultaneous mutation should be stored in one aggregate
//! payload.

mod error;
mod spec;
mod state;
mod value;

pub use error::{SetError, StateError};
pub use spec::{FieldSpec, StateSpec};
pub use state::{SystemState, TimePoint};
