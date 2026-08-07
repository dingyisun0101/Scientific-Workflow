//! Fixed-layout, heterogeneous state values at one scientific time point.
//!
//! [`SystemState`] is the public typed access boundary over the private
//! [`StateValue`](super::value::StateValue) erasure layer. Every state owns its
//! payloads, while all states derived from one [`SystemStateSchema`] share immutable
//! field metadata and name lookup tables.
//!
//! # Layout invariant
//!
//! The slot vector always has exactly one entry per field declared by the
//! specification. A slot may be empty, but fields cannot be added, removed, or
//! reordered after template loading. The first successful insertion binds a
//! slot to that payload's concrete Rust type. Taking or clearing the payload
//! retains this type contract, and [`SystemState::clone_structure_without_payloads`] carries all contracts
//! into the derived blank state without cloning payloads.
//!
//! # Ownership and cloning
//!
//! `set` consumes a payload without cloning it. Initial insertion returns
//! `None`, replacement returns ownership of the previous same-typed payload,
//! and rejection returns ownership of the unchanged incoming payload through
//! [`PayloadInsertError`]. `take` moves a stored payload back to the caller. These
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
//! payload. [`SystemState::borrow_payloads`] and [`SystemState::borrow_payloads_mut`] grant
//! coordinated access to distinct heterogeneous fields through type and name
//! tuples, allowing one validated borrow to surround an entire scientific
//! kernel. The state can also replace the complete time point with `set_time`
//! or advance it transactionally with `advance`.
//!
//! # Type safety
//!
//! Typed access uses Rust's exact runtime [`TypeId`](std::any::TypeId). A type
//! mismatch reports both type names. `take` validates the retained slot type
//! before removing its owner, so an incorrect request cannot temporarily empty
//! or discard scientific data.
//!
//! # Serialization capability
//!
//! New payloads must implement Serde [`Serialize`]. A crate-private accessor
//! exposes that existing implementation as a borrowed erased trait object for
//! the storage encoder. `SystemState` itself does not select JSON, frame
//! records, or perform IO.

use std::any::{Any, TypeId, type_name};
use std::fmt;

use serde::Serialize;

use super::error::{PayloadInsertError, StateError};
use super::schema::{StateFieldSchema, SystemStateSchema};
use super::value::StateValue;

/// The temporal coordinate associated with one [`SystemState`].
///
/// `index` is always present and provides deterministic ordering, chunk
/// boundaries, and checkpoint identity. `physical` optionally records a
/// finite domain time such as seconds or model time. Time-axis units belong to
/// stream metadata so they are not repeated in every state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimulationTime {
    index: u64,
    physical: Option<f64>,
}

impl SimulationTime {
    /// Creates an index-only time point.
    pub const fn from_step(index: u64) -> Self {
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
    pub fn from_step_and_physical_time(index: u64, physical: f64) -> Option<Self> {
        physical.is_finite().then_some(Self {
            index,
            physical: Some(physical),
        })
    }

    /// Returns the deterministic integer index.
    pub const fn step(self) -> u64 {
        self.index
    }

    /// Returns the optional physical coordinate.
    pub const fn physical_time(self) -> Option<f64> {
        self.physical
    }
}

/// A heterogeneous collection of payloads describing one system time point.
///
/// Fields are declared by a JSON-derived [`SystemStateSchema`]. Values are addressed
/// by those field names but stored in compact optional slots. The type-erased
/// representation remains private; callers always insert, borrow, mutate, and
/// extract concrete Rust types.
pub struct SystemState {
    spec: SystemStateSchema,
    time: SimulationTime,
    slots: Vec<StateSlot>,
}

/// One fixed state field's retained type contract and optional payload.
///
/// A JSON template creates an unbound slot. The first accepted payload records
/// its exact runtime type independently from the optional owner, allowing an
/// emptied slot and every state derived from it to reject accidental retyping.
/// This structure is private because callers interact only with concrete types
/// through [`SystemState`].
#[derive(Clone)]
struct StateSlot {
    definition: Option<ValueType>,
    value: Option<StateValue>,
}

impl StateSlot {
    /// Creates a payload-empty slot without a concrete type contract.
    const fn unbound() -> Self {
        Self {
            definition: None,
            value: None,
        }
    }

    /// Creates a payload-empty slot retaining this slot's type contract.
    const fn empty_like(&self) -> Self {
        Self {
            definition: self.definition,
            value: None,
        }
    }
}

/// Copyable runtime identity retained after a slot's payload is removed.
#[derive(Clone, Copy)]
struct ValueType {
    id: TypeId,
    name: &'static str,
}

impl ValueType {
    /// Captures the exact runtime identity and diagnostic name of `T`.
    fn of<T>() -> Self
    where
        T: Any,
    {
        Self {
            id: TypeId::of::<T>(),
            name: type_name::<T>(),
        }
    }

    /// Reports whether this definition names the exact concrete type `T`.
    fn is<T>(self) -> bool
    where
        T: Any,
    {
        self.id == TypeId::of::<T>()
    }
}

impl SystemState {
    /// Creates an empty state from a validated specification.
    ///
    /// This constructor is crate-private so an external caller cannot create a
    /// state without first loading a template. [`SystemStateSchema::create_empty_state`] is the
    /// public initial construction path.
    pub(crate) fn new(spec: SystemStateSchema, time: SimulationTime) -> Self {
        let slots = (0..spec.len()).map(|_| StateSlot::unbound()).collect();
        Self { spec, time, slots }
    }

    /// Creates another empty state with the same specification and field types.
    ///
    /// No payload is cloned. The immutable specification handle is shared, and
    /// each assembly-established concrete type contract is copied into an empty
    /// slot. A later [`SystemState::insert_payload`] must therefore use the same type even
    /// though the derived state begins without payloads.
    pub fn clone_structure_without_payloads(&self, time: SimulationTime) -> Self {
        Self {
            spec: self.spec.clone(),
            time,
            slots: self.slots.iter().map(StateSlot::empty_like).collect(),
        }
    }

    /// Returns this state's temporal coordinate.
    pub const fn simulation_time(&self) -> SimulationTime {
        self.time
    }

    /// Replaces this state's complete temporal coordinate.
    ///
    /// The previous [`SimulationTime`] is returned by value. Both coordinates are
    /// small `Copy` values, so replacement performs no heap allocation and
    /// does not inspect, move, or clone any scientific payload.
    ///
    /// Replacing the complete value rather than exposing its individual fields
    /// ensures that a physical coordinate can enter a state only through the
    /// finite-value validation performed by
    /// [`SimulationTime::from_step_and_physical_time`].
    ///
    /// # Collection invariants
    ///
    /// A state stored inside a time-ordered collection must not be passed as
    /// `&mut SystemState` to external callers: changing its time could violate
    /// collection ordering. The owning simulation may freely call this method
    /// before submitting a state or encoded sample.
    pub fn replace_simulation_time(&mut self, time: SimulationTime) -> SimulationTime {
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
    /// On success, the new [`SimulationTime`] is stored and returned. All validation
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
    pub fn advance_simulation_time(
        &mut self,
        physical_delta: Option<f64>,
    ) -> Result<SimulationTime, StateError> {
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

        let next = SimulationTime {
            index: next_index,
            physical: next_physical,
        };
        self.time = next;
        Ok(next)
    }

    /// Returns the shared immutable field specification.
    pub const fn schema(&self) -> &SystemStateSchema {
        &self.spec
    }

    /// Returns the number of fields declared by the state specification.
    ///
    /// This count is structural and includes empty payload slots.
    pub fn declared_field_count(&self) -> usize {
        self.slots.len()
    }

    /// Reports whether the state specification declares no fields.
    ///
    /// This is consistent with [`SystemState::declared_field_count`]. To test whether a
    /// non-empty layout currently carries no payloads, use
    /// [`SystemState::has_no_payloads`].
    pub fn has_no_declared_fields(&self) -> bool {
        self.slots.is_empty()
    }

    /// Returns the number of slots that currently contain payloads.
    pub fn populated_field_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
    }

    /// Reports whether every declared payload slot is empty.
    pub fn has_no_payloads(&self) -> bool {
        self.slots.iter().all(|slot| slot.value.is_none())
    }

    /// Returns field specifications in deterministic template order.
    pub fn field_schemas(&self) -> &[StateFieldSchema] {
        self.spec.field_schemas()
    }

    /// Reports whether a declared field currently contains a payload.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] when `key` was not declared by the
    /// JSON template.
    pub fn contains_payload(&self, key: &str) -> Result<bool, StateError> {
        let index = self.spec.index_of(key)?;
        Ok(self.slots[index].value.is_some())
    }

    /// Reports whether a populated field contains the exact Rust type `T`.
    ///
    /// An empty declared field returns `false`.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] when `key` was not declared by the
    /// JSON template.
    pub fn payload_has_type<T>(&self, key: &str) -> Result<bool, StateError>
    where
        T: Any,
    {
        let index = self.spec.index_of(key)?;
        Ok(self.slots[index]
            .value
            .as_ref()
            .is_some_and(StateValue::is::<T>))
    }

    /// Sets or replaces a payload while preserving ownership on every outcome.
    ///
    /// `payload` moves into this operation and is never cloned:
    ///
    /// - a never-populated declared slot binds itself to `T`, receives the
    ///   payload, and returns `Ok(None)`;
    /// - a slot bound to exactly `T` receives it and returns the displaced
    ///   payload as `Ok(Some(previous))`, or `Ok(None)` when currently empty;
    /// - an undeclared key returns `Err(PayloadInsertError<T>)` containing the unchanged
    ///   incoming payload;
    /// - a slot bound to another concrete type remains unchanged and returns
    ///   the incoming payload in `PayloadInsertError<T>`, even when its payload is empty.
    ///
    /// Returning a previous payload is deliberate assignment behavior. A
    /// caller that does not need that owner should discard it explicitly:
    ///
    /// ```no_run
    /// # use scientific_workflow::system_state::{SystemStateSchema, SimulationTime};
    /// # fn example(spec: &SystemStateSchema) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut state = spec.create_empty_state(SimulationTime::from_step(0));
    /// drop(state.insert_payload("population", vec![1_u64, 2, 3])?);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Type-contract validation occurs before the slot is changed.
    /// Consequently, rejection cannot discard or temporarily remove an
    /// existing scientific value, and `take` or `clear` cannot reopen a field
    /// for a different type.
    ///
    /// # Errors
    ///
    /// Returns [`PayloadInsertError`] containing:
    ///
    /// - [`StateError::UnknownField`] when `key` is undeclared;
    /// - [`StateError::TypeMismatch`] when the slot is bound to a different
    ///   concrete Rust type.
    ///
    /// In both cases [`PayloadInsertError::into_parts`] recovers the unchanged incoming
    /// `T` without cloning it.
    pub fn insert_payload<T>(
        &mut self,
        key: &str,
        payload: T,
    ) -> Result<Option<T>, PayloadInsertError<T>>
    where
        T: Serialize + Clone + Send + 'static,
    {
        let index = match self.spec.index_of(key) {
            Ok(index) => index,
            Err(error) => return Err(PayloadInsertError::new(error, payload)),
        };

        let slot = &mut self.slots[index];
        match slot.definition {
            Some(definition) if !definition.is::<T>() => {
                return Err(PayloadInsertError::new(
                    StateError::TypeMismatch {
                        field: key.to_owned(),
                        expected: type_name::<T>(),
                        actual: definition.name,
                    },
                    payload,
                ));
            }
            Some(_) => {}
            None => slot.definition = Some(ValueType::of::<T>()),
        }

        let previous = slot.value.replace(StateValue::new(payload));
        match previous {
            None => Ok(None),
            Some(previous) => match previous.downcast::<T>() {
                Ok(previous) => Ok(Some(previous)),
                Err(_) => unreachable!("a type-bound StateValue failed its consuming downcast"),
            },
        }
    }

    /// Borrows a populated field as the exact Rust type `T`.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] for an undeclared key,
    /// [`StateError::MissingPayload`] for an empty slot, or
    /// [`StateError::TypeMismatch`] when the stored concrete type differs from
    /// `T`.
    pub fn payload<T>(&self, key: &str) -> Result<&T, StateError>
    where
        T: Any,
    {
        let index = self.spec.index_of(key)?;
        self.validate_slot::<T>(index, key)?;
        Ok(self.slots[index]
            .value
            .as_ref()
            .and_then(StateValue::downcast_ref::<T>)
            .expect("a validated state slot must contain its bound concrete type"))
    }

    /// Mutably borrows a populated field as the exact Rust type `T`.
    ///
    /// Mutation occurs in place and does not clone the payload.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] for an undeclared key,
    /// [`StateError::MissingPayload`] for an empty slot, or
    /// [`StateError::TypeMismatch`] when the stored concrete type differs from
    /// `T`.
    pub fn payload_mut<T>(&mut self, key: &str) -> Result<&mut T, StateError>
    where
        T: Any,
    {
        let index = self.spec.index_of(key)?;
        self.validate_slot::<T>(index, key)?;
        Ok(self.slots[index]
            .value
            .as_mut()
            .and_then(StateValue::downcast_mut::<T>)
            .expect("a validated state slot must contain its bound concrete type"))
    }

    /// Borrows several distinct populated fields as concrete immutable types.
    ///
    /// `Q` is a tuple of expected payload types, while `keys` is the equally
    /// sized tuple of field names. Tuple positions correspond exactly. The
    /// supported arities are two through eight; single-field callers should use
    /// [`SystemState::payload`]. The sealed tuple implementation is internal and
    /// requires no user-defined selector, query object, or macro invocation.
    ///
    /// # Errors
    ///
    /// Validation proceeds from left to right and completes before references
    /// are returned. The method reports an unknown field, repeated field,
    /// retained type mismatch, or missing payload through [`StateError`]. An
    /// error leaves every slot unchanged.
    pub fn borrow_payloads<'state, Q>(
        &'state self,
        keys: Q::Keys<'_>,
    ) -> Result<Q::Refs<'state>, StateError>
    where
        Q: PayloadTuple,
    {
        Q::borrow(self, keys)
    }

    /// Borrows several distinct populated fields as concrete mutable types.
    ///
    /// `Q` is a tuple of expected payload types, while `keys` is the equally
    /// sized tuple of field names. All names, duplicate indices, retained type
    /// contracts, and payload presence are validated before any mutable
    /// reference is produced. Payloads remain owned by this state and are not
    /// cloned, moved, serialized, locked, or temporarily removed.
    ///
    /// One call should normally surround a complete coupled kernel or sweep so
    /// name lookup and dynamic type validation occur once outside its inner
    /// loop. Supported arities are two through eight; single-field callers
    /// should use [`SystemState::payload_mut`].
    ///
    /// # Errors
    ///
    /// Returns the same deterministic validation errors as
    /// [`SystemState::borrow_payloads`]. A failure leaves the state unchanged and grants
    /// no partial borrow.
    pub fn borrow_payloads_mut<'state, Q>(
        &'state mut self,
        keys: Q::Keys<'_>,
    ) -> Result<Q::RefsMut<'state>, StateError>
    where
        Q: PayloadTuple,
    {
        Q::borrow_mut(self, keys)
    }

    /// Removes and returns the payload from a declared field.
    ///
    /// A successful call moves the original concrete `T` out of its internal
    /// box and leaves the field slot empty while retaining its type contract.
    /// It does not invoke `Clone`. Type and presence validation occurs before
    /// the payload owner is removed.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::UnknownField`] for an undeclared key,
    /// [`StateError::MissingPayload`] for an empty slot, or
    /// [`StateError::TypeMismatch`] when the stored concrete type differs from
    /// `T`.
    pub fn take_payload<T>(&mut self, key: &str) -> Result<T, StateError>
    where
        T: Any + Send,
    {
        let index = self.spec.index_of(key)?;
        self.validate_slot::<T>(index, key)?;
        let value = self.slots[index]
            .value
            .take()
            .expect("a validated state slot must contain a payload");
        match value.downcast::<T>() {
            Ok(payload) => Ok(payload),
            Err(_) => unreachable!("a type-bound StateValue failed its consuming downcast"),
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
    pub fn clear_payload(&mut self, key: &str) -> Result<bool, StateError> {
        let index = self.spec.index_of(key)?;
        Ok(self.slots[index].value.take().is_some())
    }

    /// Drops every payload while retaining layout, type contracts, and time.
    pub fn clear_all_payloads(&mut self) {
        self.slots.iter_mut().for_each(|slot| slot.value = None);
    }

    /// Returns a populated erased value for a typed immutable accessor.
    fn value(&self, key: &str) -> Result<&StateValue, StateError> {
        let index = self.spec.index_of(key)?;
        self.slots[index]
            .value
            .as_ref()
            .ok_or_else(|| StateError::MissingPayload {
                field: key.to_owned(),
            })
    }

    /// Validates one resolved slot against an expected concrete type and value.
    fn validate_slot<T>(&self, index: usize, key: &str) -> Result<(), StateError>
    where
        T: Any,
    {
        let slot = &self.slots[index];
        if let Some(definition) = slot.definition
            && !definition.is::<T>()
        {
            return Err(StateError::TypeMismatch {
                field: key.to_owned(),
                expected: type_name::<T>(),
                actual: definition.name,
            });
        }

        if slot.value.is_none() {
            return Err(StateError::MissingPayload {
                field: key.to_owned(),
            });
        }
        Ok(())
    }

    /// Resolves and validates a fixed-size tuple of distinct field names.
    fn resolve_distinct<const N: usize>(&self, keys: [&str; N]) -> Result<[usize; N], StateError> {
        let mut indices = [0; N];
        for (position, key) in keys.iter().enumerate() {
            let index = self.spec.index_of(key)?;
            if indices[..position].contains(&index) {
                return Err(StateError::RepeatedPayloadBorrow {
                    field: (*key).to_owned(),
                });
            }
            indices[position] = index;
        }
        Ok(indices)
    }

    /// Safely separates already validated distinct slot indices.
    fn disjoint_slots_mut<const N: usize>(&mut self, indices: [usize; N]) -> [&mut StateSlot; N] {
        let mut positions: [(usize, usize); N] =
            std::array::from_fn(|position| (position, indices[position]));
        positions.sort_unstable_by_key(|(_, index)| *index);

        let mut remaining = self.slots.as_mut_slice();
        let mut base = 0;
        let mut selected: [Option<&mut StateSlot>; N] = std::array::from_fn(|_| None);
        for (original_position, index) in positions {
            let relative = index - base;
            let (_, at_index) = remaining.split_at_mut(relative);
            let (slot, tail) = at_index
                .split_first_mut()
                .expect("resolved state slot index must be in bounds");
            selected[original_position] = Some(slot);
            remaining = tail;
            base = index + 1;
        }

        selected
            .map(|slot| slot.expect("one disjoint slot must be returned for every requested index"))
    }

    /// Borrows one populated payload through erased Serde serialization.
    ///
    /// This crate-private method is the complete format-agnostic boundary used
    /// by the storage encoder. It performs the same declared-field and
    /// populated-slot validation as [`SystemState::payload`], but it neither
    /// downcasts nor exposes the private [`StateValue`] wrapper.
    ///
    /// The returned object refers directly to the stored concrete payload. No
    /// clone, allocation, encoding, or ownership transfer occurs here.
    #[allow(
        dead_code,
        reason = "reserved for storage::JsonStateRecordEncoder, which is implemented in the next module stage"
    )]
    pub(crate) fn serializable(
        &self,
        key: &str,
    ) -> Result<&dyn erased_serde::Serialize, StateError> {
        Ok(self.value(key)?.serializable())
    }
}

/// Sealing boundary for the internally generated tuple implementations.
mod tuple_sealed {
    /// Prevents downstream crates from implementing the hidden tuple contract.
    pub trait Sealed {}
}

/// Internal type-level mapping used by [`SystemState::borrow_payloads`] and
/// [`SystemState::borrow_payloads_mut`].
///
/// This trait must be public because it appears in those generic methods'
/// signatures, but it is sealed, omitted from the prelude, and hidden from
/// generated documentation. Applications select an implementation simply by
/// writing a supported tuple type such as `(Position, Velocity)`.
#[doc(hidden)]
pub trait PayloadTuple: tuple_sealed::Sealed {
    /// Equally sized tuple of borrowed field names.
    type Keys<'key>;

    /// Equally sized tuple of immutable concrete payload references.
    type Refs<'state>
    where
        Self: 'state;

    /// Equally sized tuple of mutable concrete payload references.
    type RefsMut<'state>
    where
        Self: 'state;

    /// Resolves and immutably borrows one supported field tuple.
    #[doc(hidden)]
    fn borrow<'state, 'key>(
        state: &'state SystemState,
        keys: Self::Keys<'key>,
    ) -> Result<Self::Refs<'state>, StateError>;

    /// Resolves and mutably borrows one supported field tuple.
    #[doc(hidden)]
    fn borrow_mut<'state, 'key>(
        state: &'state mut SystemState,
        keys: Self::Keys<'key>,
    ) -> Result<Self::RefsMut<'state>, StateError>;
}

/// Substitutes one repeated generic identifier with a common tuple element.
macro_rules! substitute_type {
    ($_generic:ident => $replacement:ty) => {
        $replacement
    };
}

/// Generates the sealed heterogeneous borrow contract for one tuple arity.
///
/// Public callers see only `SystemState::borrow_payloads[_mut]`; this macro centralizes
/// validation order, exact downcasts, and tuple construction so every supported
/// arity has identical semantics.
macro_rules! impl_state_tuple {
    ($(($type:ident, $key:ident, $slot:ident, $index:tt)),+ $(,)?) => {
        impl<$($type),+> tuple_sealed::Sealed for ($($type,)+)
        where
            $($type: Any,)+
        {
        }

        impl<$($type),+> PayloadTuple for ($($type,)+)
        where
            $($type: Any,)+
        {
            type Keys<'key> = ($(substitute_type!($type => &'key str),)+);
            type Refs<'state> = ($(&'state $type,)+) where Self: 'state;
            type RefsMut<'state> = ($(&'state mut $type,)+) where Self: 'state;

            fn borrow<'state, 'key>(
                state: &'state SystemState,
                keys: Self::Keys<'key>,
            ) -> Result<Self::Refs<'state>, StateError> {
                let ($($key,)+) = keys;
                let indices = state.resolve_distinct([$($key,)+])?;
                $(state.validate_slot::<$type>(indices[$index], $key)?;)+

                Ok(($(
                    state.slots[indices[$index]]
                        .value
                        .as_ref()
                        .and_then(StateValue::downcast_ref::<$type>)
                        .expect("a preflighted state slot must contain its bound concrete type"),
                )+))
            }

            fn borrow_mut<'state, 'key>(
                state: &'state mut SystemState,
                keys: Self::Keys<'key>,
            ) -> Result<Self::RefsMut<'state>, StateError> {
                let ($($key,)+) = keys;
                let indices = state.resolve_distinct([$($key,)+])?;
                $(state.validate_slot::<$type>(indices[$index], $key)?;)+
                let [$($slot,)+] = state.disjoint_slots_mut(indices);

                Ok(($(
                    $slot
                        .value
                        .as_mut()
                        .and_then(StateValue::downcast_mut::<$type>)
                        .expect("a preflighted state slot must contain its bound concrete type"),
                )+))
            }
        }
    };
}

impl_state_tuple!((A, key_a, slot_a, 0), (B, key_b, slot_b, 1));
impl_state_tuple!(
    (A, key_a, slot_a, 0),
    (B, key_b, slot_b, 1),
    (C, key_c, slot_c, 2),
);
impl_state_tuple!(
    (A, key_a, slot_a, 0),
    (B, key_b, slot_b, 1),
    (C, key_c, slot_c, 2),
    (D, key_d, slot_d, 3),
);
impl_state_tuple!(
    (A, key_a, slot_a, 0),
    (B, key_b, slot_b, 1),
    (C, key_c, slot_c, 2),
    (D, key_d, slot_d, 3),
    (E, key_e, slot_e, 4),
);
impl_state_tuple!(
    (A, key_a, slot_a, 0),
    (B, key_b, slot_b, 1),
    (C, key_c, slot_c, 2),
    (D, key_d, slot_d, 3),
    (E, key_e, slot_e, 4),
    (F, key_f, slot_f, 5),
);
impl_state_tuple!(
    (A, key_a, slot_a, 0),
    (B, key_b, slot_b, 1),
    (C, key_c, slot_c, 2),
    (D, key_d, slot_d, 3),
    (E, key_e, slot_e, 4),
    (F, key_f, slot_f, 5),
    (G, key_g, slot_g, 6),
);
impl_state_tuple!(
    (A, key_a, slot_a, 0),
    (B, key_b, slot_b, 1),
    (C, key_c, slot_c, 2),
    (D, key_d, slot_d, 3),
    (E, key_e, slot_e, 4),
    (F, key_f, slot_f, 5),
    (G, key_g, slot_g, 6),
    (H, key_h, slot_h, 7),
);

impl Clone for SystemState {
    /// Shares the immutable specification and deep-clones populated payloads.
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            time: self.time,
            slots: self.slots.clone(),
        }
    }
}

impl fmt::Debug for SystemState {
    /// Formats structural metadata without formatting scientific payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SystemState")
            .field("time", &self.time)
            .field("source", &self.spec.template_path())
            .field("fields", &self.declared_field_count())
            .field("loaded", &self.populated_field_count())
            .finish_non_exhaustive()
    }
}
