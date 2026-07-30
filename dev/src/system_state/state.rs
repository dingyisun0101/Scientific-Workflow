//! Fixed-layout, heterogeneous state values at one scientific time point.
//!
//! [`SystemState`] is the public typed access boundary over the private
//! [`StateValue`](super::value::StateValue) erasure layer. Every state owns its
//! payloads, while all states derived from one [`StateSpec`] share immutable
//! field metadata and name lookup tables.
//!
//! # Layout invariant
//!
//! The payload vector always has exactly one slot per field declared by the
//! specification. A slot may be empty, but fields cannot be added, removed, or
//! reordered after template loading. Consequently, name lookup resolves once
//! to a stable integer index and all states with the same specification have
//! identical structural shape.
//!
//! # Ownership and cloning
//!
//! `set` consumes a payload, and `take` returns that same owned payload without
//! calling `Clone`. Moving a complete state into an SSTS or writer similarly
//! moves ownership. Explicitly cloning a `SystemState` is different: it shares
//! the immutable specification but deep-clones every populated payload.
//!
//! # Type safety
//!
//! Typed access uses Rust's exact runtime [`TypeId`](std::any::TypeId). A type
//! mismatch reports both type names. A failed consuming `take` restores the
//! original erased payload to its slot before returning the error, so an
//! incorrect type request cannot discard scientific data.

use std::any::{Any, type_name};
use std::fmt;

use super::error::StateError;
use super::spec::{FieldSpec, StateSpec};
use super::value::StateValue;

/// The temporal coordinate associated with one [`SystemState`].
///
/// `index` is always present and provides deterministic ordering, chunk
/// boundaries, and checkpoint identity. `physical` optionally records a
/// finite domain time such as seconds or model time. Time-axis units belong to
/// SSTS metadata so they are not repeated in every state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimePoint {
    index: u64,
    physical: Option<f64>,
}

impl TimePoint {
    /// Creates an index-only time point.
    pub const fn new(index: u64) -> Self {
        Self {
            index,
            physical: None,
        }
    }

    /// Creates a time point with an optional finite physical coordinate.
    ///
    /// Returns `None` for `NaN` or either infinity. Negative finite values are
    /// accepted because some scientific coordinate systems use an origin after
    /// the beginning of a simulation or observation.
    pub fn from_physical(index: u64, physical: f64) -> Option<Self> {
        physical.is_finite().then_some(Self {
            index,
            physical: Some(physical),
        })
    }

    /// Returns the deterministic integer index.
    pub const fn index(self) -> u64 {
        self.index
    }

    /// Returns the optional physical coordinate.
    pub const fn physical(self) -> Option<f64> {
        self.physical
    }
}

/// A heterogeneous collection of payloads describing one system time point.
///
/// Fields are declared by a JSON-derived [`StateSpec`]. Values are addressed
/// by those field names but stored in compact optional slots. The type-erased
/// representation remains private; callers always insert, borrow, mutate, and
/// extract concrete Rust types.
pub struct SystemState {
    spec: StateSpec,
    time: TimePoint,
    values: Vec<Option<StateValue>>,
}

impl SystemState {
    /// Creates an empty state from a validated specification.
    ///
    /// This constructor is crate-private so an external caller cannot create a
    /// state without first loading a template. [`StateSpec::empty`] is the
    /// public initial construction path.
    pub(crate) fn new(spec: StateSpec, time: TimePoint) -> Self {
        let values = (0..spec.len()).map(|_| None).collect();
        Self { spec, time, values }
    }

    /// Creates another empty state with the same shared specification.
    ///
    /// No payload is cloned. Only the immutable specification handle is
    /// cloned, which increments an internal `Arc` reference count.
    pub fn empty(&self, time: TimePoint) -> Self {
        Self::new(self.spec.clone(), time)
    }

    /// Returns this state's temporal coordinate.
    pub const fn time(&self) -> TimePoint {
        self.time
    }

    /// Returns the shared immutable field specification.
    pub const fn spec(&self) -> &StateSpec {
        &self.spec
    }

    /// Returns the number of fields declared by the state specification.
    ///
    /// This count is structural and includes empty payload slots.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Reports whether the state specification declares no fields.
    ///
    /// This is consistent with [`SystemState::len`]. To test whether a
    /// non-empty layout currently carries no payloads, use
    /// [`SystemState::is_blank`].
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Returns the number of slots that currently contain payloads.
    pub fn loaded(&self) -> usize {
        self.values.iter().filter(|value| value.is_some()).count()
    }

    /// Reports whether every declared payload slot is empty.
    pub fn is_blank(&self) -> bool {
        self.values.iter().all(Option::is_none)
    }

    /// Returns field specifications in deterministic template order.
    pub fn fields(&self) -> &[FieldSpec] {
        self.spec.fields()
    }

    /// Reports whether a declared field currently contains a payload.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] when `key` was not declared by the
    /// JSON template.
    pub fn has(&self, key: &str) -> Result<bool, StateError> {
        let index = self.spec.index_of(key)?;
        Ok(self.values[index].is_some())
    }

    /// Reports whether a populated field contains the exact Rust type `T`.
    ///
    /// An empty declared field returns `false`.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] when `key` was not declared by the
    /// JSON template.
    pub fn is<T>(&self, key: &str) -> Result<bool, StateError>
    where
        T: Any,
    {
        let index = self.spec.index_of(key)?;
        Ok(self.values[index].as_ref().is_some_and(StateValue::is::<T>))
    }

    /// Sets or replaces the payload in a declared field.
    ///
    /// `payload` moves into the state and is never cloned. If the slot was
    /// already populated, its previous value is dropped. Call [`take`](Self::take)
    /// first when the previous payload must be retained.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] when `key` was not declared by the
    /// JSON template. The payload is dropped with the returned error because
    /// this minimal API does not expose the internal erased-value wrapper.
    pub fn set<T>(&mut self, key: &str, payload: T) -> Result<(), StateError>
    where
        T: Any + Clone + Send,
    {
        let index = self.spec.index_of(key)?;
        self.values[index] = Some(StateValue::new(payload));
        Ok(())
    }

    /// Borrows a populated field as the exact Rust type `T`.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] for an undeclared key,
    /// [`StateError::MissingValue`] for an empty slot, or
    /// [`StateError::TypeMismatch`] when the stored concrete type differs from
    /// `T`.
    pub fn get<T>(&self, key: &str) -> Result<&T, StateError>
    where
        T: Any,
    {
        let value = self.value(key)?;
        let actual = value.type_name();

        value
            .downcast_ref::<T>()
            .ok_or_else(|| StateError::TypeMismatch {
                field: key.to_owned(),
                expected: type_name::<T>(),
                actual,
            })
    }

    /// Mutably borrows a populated field as the exact Rust type `T`.
    ///
    /// Mutation occurs in place and does not clone the payload.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] for an undeclared key,
    /// [`StateError::MissingValue`] for an empty slot, or
    /// [`StateError::TypeMismatch`] when the stored concrete type differs from
    /// `T`.
    pub fn get_mut<T>(&mut self, key: &str) -> Result<&mut T, StateError>
    where
        T: Any,
    {
        let value = self.value_mut(key)?;
        let actual = value.type_name();

        value
            .downcast_mut::<T>()
            .ok_or_else(|| StateError::TypeMismatch {
                field: key.to_owned(),
                expected: type_name::<T>(),
                actual,
            })
    }

    /// Removes and returns the payload from a declared field.
    ///
    /// A successful call moves the original concrete `T` out of its internal
    /// box and leaves the field slot empty. It does not invoke `Clone`. If `T`
    /// does not match, the original erased value is restored before the error
    /// is returned.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] for an undeclared key,
    /// [`StateError::MissingValue`] for an empty slot, or
    /// [`StateError::TypeMismatch`] when the stored concrete type differs from
    /// `T`.
    pub fn take<T>(&mut self, key: &str) -> Result<T, StateError>
    where
        T: Any + Send,
    {
        let index = self.spec.index_of(key)?;
        let value = self.values[index]
            .take()
            .ok_or_else(|| StateError::MissingValue {
                field: key.to_owned(),
            })?;
        let actual = value.type_name();

        match value.downcast::<T>() {
            Ok(payload) => Ok(payload),
            Err(value) => {
                self.values[index] = Some(value);
                Err(StateError::TypeMismatch {
                    field: key.to_owned(),
                    expected: type_name::<T>(),
                    actual,
                })
            }
        }
    }

    /// Drops the payload stored in one declared field.
    ///
    /// Returns `true` when a payload was present and dropped, or `false` when
    /// the declared slot was already empty.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] when `key` was not declared by the
    /// JSON template.
    pub fn clear(&mut self, key: &str) -> Result<bool, StateError> {
        let index = self.spec.index_of(key)?;
        Ok(self.values[index].take().is_some())
    }

    /// Drops every payload while retaining the shared layout and time point.
    pub fn clear_all(&mut self) {
        self.values.iter_mut().for_each(|value| *value = None);
    }

    /// Returns a populated erased value for a typed immutable accessor.
    fn value(&self, key: &str) -> Result<&StateValue, StateError> {
        let index = self.spec.index_of(key)?;
        self.values[index]
            .as_ref()
            .ok_or_else(|| StateError::MissingValue {
                field: key.to_owned(),
            })
    }

    /// Returns a populated erased value for a typed mutable accessor.
    fn value_mut(&mut self, key: &str) -> Result<&mut StateValue, StateError> {
        let index = self.spec.index_of(key)?;
        self.values[index]
            .as_mut()
            .ok_or_else(|| StateError::MissingValue {
                field: key.to_owned(),
            })
    }
}

impl Clone for SystemState {
    /// Shares the immutable specification and deep-clones populated payloads.
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            time: self.time,
            values: self.values.clone(),
        }
    }
}

impl fmt::Debug for SystemState {
    /// Formats structural metadata without formatting scientific payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SystemState")
            .field("time", &self.time)
            .field("source", &self.spec.source())
            .field("fields", &self.len())
            .field("loaded", &self.loaded())
            .finish_non_exhaustive()
    }
}
