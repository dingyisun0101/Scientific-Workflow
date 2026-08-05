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
//! `set` consumes a payload without cloning it. Initial insertion returns
//! `None`, replacement returns ownership of the previous same-typed payload,
//! and rejection returns ownership of the unchanged incoming payload through
//! [`SetError`]. `take` moves a stored payload back to the caller. These
//! operations preserve the backing allocations of ordinary scientific owners
//! such as `Vec<T>` and tensor containers.
//!
//! Explicitly cloning a `SystemState` is different: it shares the immutable
//! specification but deep-clones every populated payload. Persistence should
//! borrow a live state during synchronous serialization rather than invoke
//! this expensive clone.
//!
//! # Mutation
//!
//! The owning simulation can replace, borrow mutably, extract, or clear every
//! payload. It can also replace the complete time point with `set_time` or
//! advance it transactionally with `advance`. The [`StateSpec`] is deliberately
//! immutable because changing field order or identity would invalidate the
//! state's slot layout and every downstream schema assumption.
//!
//! # Type safety
//!
//! Typed access uses Rust's exact runtime [`TypeId`](std::any::TypeId). A type
//! mismatch reports both type names. A failed consuming `take` restores the
//! original erased payload to its slot before returning the error, so an
//! incorrect type request cannot discard scientific data.
//!
//! # Serialization capability
//!
//! New payloads must implement Serde [`Serialize`]. A crate-private accessor
//! exposes that existing implementation as a borrowed erased trait object for
//! the storage encoder. `SystemState` itself does not select JSON, frame
//! records, or perform IO.

use std::any::{Any, type_name};
use std::fmt;

use serde::Serialize;

use super::error::{SetError, StateError};
use super::spec::{FieldSpec, StateSpec};
use super::value::StateValue;

/// The temporal coordinate associated with one [`SystemState`].
///
/// `index` is always present and provides deterministic ordering, chunk
/// boundaries, and checkpoint identity. `physical` optionally records a
/// finite domain time such as seconds or model time. Time-axis units belong to
/// stream metadata so they are not repeated in every state.
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

    /// Creates a time point with a finite physical coordinate.
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

    /// Replaces this state's complete temporal coordinate.
    ///
    /// The previous [`TimePoint`] is returned by value. Both coordinates are
    /// small `Copy` values, so replacement performs no heap allocation and
    /// does not inspect, move, or clone any scientific payload.
    ///
    /// Replacing the complete value rather than exposing its individual fields
    /// ensures that a physical coordinate can enter a state only through the
    /// finite-value validation performed by [`TimePoint::from_physical`].
    ///
    /// # Collection invariants
    ///
    /// A state stored inside a time-ordered collection must not be passed as
    /// `&mut SystemState` to external callers: changing its time could violate
    /// collection ordering. The owning simulation may freely call this method
    /// before submitting a state or encoded sample.
    pub fn set_time(&mut self, time: TimePoint) -> TimePoint {
        std::mem::replace(&mut self.time, time)
    }

    /// Advances the integer index by one and optionally advances physical time.
    ///
    /// Passing `None` increments only the authoritative integer index and
    /// preserves the current optional physical coordinate. Passing
    /// `Some(delta)` additionally requires an existing physical coordinate,
    /// a finite `delta`, and a finite sum. Negative and zero finite deltas are
    /// valid because integer index—not physical time—defines record ordering.
    ///
    /// On success, the new [`TimePoint`] is stored and returned. All validation
    /// occurs before assignment, so every error leaves the original time point
    /// unchanged.
    ///
    /// # Errors
    ///
    /// Returns:
    ///
    /// - [`StateError::TimeIndexOverflow`] when the current index is
    ///   `u64::MAX`;
    /// - [`StateError::MissingPhysicalTime`] when a delta is supplied but the
    ///   current state has no physical coordinate;
    /// - [`StateError::InvalidPhysicalAdvance`] when the delta or resulting
    ///   coordinate is not finite.
    pub fn advance(&mut self, physical_delta: Option<f64>) -> Result<TimePoint, StateError> {
        let next_index = self
            .time
            .index
            .checked_add(1)
            .ok_or(StateError::TimeIndexOverflow {
                index: self.time.index,
            })?;

        let next_physical = match (self.time.physical, physical_delta) {
            (physical, None) => physical,
            (None, Some(_)) => {
                return Err(StateError::MissingPhysicalTime {
                    index: self.time.index,
                });
            }
            (Some(current), Some(delta)) => {
                let next = current + delta;
                if !delta.is_finite() || !next.is_finite() {
                    return Err(StateError::InvalidPhysicalAdvance { current, delta });
                }
                Some(next)
            }
        };

        let next = TimePoint {
            index: next_index,
            physical: next_physical,
        };
        self.time = next;
        Ok(next)
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

    /// Sets or replaces a payload while preserving ownership on every outcome.
    ///
    /// `payload` moves into this operation and is never cloned:
    ///
    /// - an empty declared slot receives it and returns `Ok(None)`;
    /// - a slot containing exactly `T` receives it and returns the displaced
    ///   payload as `Ok(Some(previous))`;
    /// - an undeclared key returns `Err(SetError<T>)` containing the unchanged
    ///   incoming payload;
    /// - a slot containing another concrete type remains unchanged and returns
    ///   the incoming payload in `SetError<T>`.
    ///
    /// Returning a previous payload is deliberate assignment behavior. A
    /// caller that does not need that owner should discard it explicitly:
    ///
    /// ```no_run
    /// # use scientific_workflow::system_state::{StateSpec, TimePoint};
    /// # fn example(spec: &StateSpec) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut state = spec.empty(TimePoint::new(0));
    /// drop(state.set("population", vec![1_u64, 2, 3])?);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Same-type validation occurs before the occupied slot is changed.
    /// Consequently, rejection cannot discard or temporarily remove the
    /// existing scientific value.
    ///
    /// # Errors
    ///
    /// Returns [`SetError`] containing:
    ///
    /// - [`StateError::UnknownField`] when `key` is undeclared;
    /// - [`StateError::TypeMismatch`] when an occupied slot contains a
    ///   different concrete Rust type.
    ///
    /// In both cases [`SetError::into_parts`] recovers the unchanged incoming
    /// `T` without cloning it.
    pub fn set<T>(&mut self, key: &str, payload: T) -> Result<Option<T>, SetError<T>>
    where
        T: Serialize + Clone + Send + 'static,
    {
        let index = match self.spec.index_of(key) {
            Ok(index) => index,
            Err(error) => return Err(SetError::new(error, payload)),
        };

        let Some(previous) = self.values[index].as_ref() else {
            self.values[index] = Some(StateValue::new(payload));
            return Ok(None);
        };

        if !previous.is::<T>() {
            return Err(SetError::new(
                StateError::TypeMismatch {
                    field: key.to_owned(),
                    expected: type_name::<T>(),
                    actual: previous.type_name(),
                },
                payload,
            ));
        }

        let previous = self.values[index]
            .replace(StateValue::new(payload))
            .expect("the occupied slot was validated immediately before replacement");

        match previous.downcast::<T>() {
            Ok(previous) => Ok(Some(previous)),
            Err(_) => unreachable!("a matching StateValue failed its consuming downcast"),
        }
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

    /// Borrows one populated payload through erased Serde serialization.
    ///
    /// This crate-private method is the complete format-agnostic boundary used
    /// by the storage encoder. It performs the same declared-field and
    /// populated-slot validation as [`SystemState::get`], but it neither
    /// downcasts nor exposes the private [`StateValue`] wrapper.
    ///
    /// The returned object refers directly to the stored concrete payload. No
    /// clone, allocation, encoding, or ownership transfer occurs here.
    #[allow(
        dead_code,
        reason = "reserved for storage::JsonEncoder, which is implemented in the next module stage"
    )]
    pub(crate) fn serializable(
        &self,
        key: &str,
    ) -> Result<&dyn erased_serde::Serialize, StateError> {
        Ok(self.value(key)?.serializable())
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
