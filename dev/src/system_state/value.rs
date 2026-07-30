//! Private type erasure for values stored in a system state.
//!
//! [`SystemState`](super::state::SystemState) must hold unrelated Rust types in
//! one fixed-layout collection while preserving ordinary ownership semantics.
//! Rust collections require a single statically sized element type, so concrete
//! payloads are stored behind a private trait object and exposed to the state
//! implementation through [`StateValue`].
//!
//! # Ownership model
//!
//! Creating a `StateValue` consumes its payload. Borrowed downcasts return
//! references into that same allocation, and a consuming downcast returns
//! ownership of the original concrete value. None of these operations call
//! [`Clone`]. A clone occurs only when `StateValue::clone` is explicitly
//! invoked, in which case the concrete payload's `Clone` implementation is
//! used.
//!
//! This guarantees that moving common scientific containers such as `Vec<T>`,
//! owned tensors, and memory-map handles into and out of a state preserves
//! their backing allocations. Rust may still move the small top-level owner
//! value itself. A caller that requires address stability for that top-level
//! value can store `Box<T>` or `Pin<Box<T>>` as the payload type.
//!
//! # Visibility
//!
//! Type erasure and boxing are implementation details. This module is
//! crate-private so end users interact only with typed methods on
//! `SystemState`, such as `set`, `get`, `get_mut`, and `take`.
//!
//! # Threading
//!
//! Stored values must implement [`Send`] so a complete state or SSTS chunk can
//! transfer ownership to a writer thread. [`Sync`] is intentionally not
//! required: the workflow moves payload ownership between stages rather than
//! sharing mutable state concurrently.

use std::any::{Any, TypeId, type_name};
use std::fmt;

/// Object-safe behavior required from every erased payload.
///
/// `Any` supplies runtime type identity, while the explicit conversion methods
/// avoid relying on trait-object upcasting. `clone_box` is the standard
/// cloneable-trait-object pattern: dynamic dispatch reaches the concrete
/// payload's `Clone` implementation and returns a newly owned erased value.
trait ErasedValue: Any + Send {
    /// Deep-clones the concrete payload into a new erased allocation.
    fn clone_box(&self) -> Box<dyn ErasedValue>;

    /// Views the concrete payload through `Any` for a borrowed downcast.
    fn as_any(&self) -> &dyn Any;

    /// Views the concrete payload through `Any` for a mutable downcast.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Converts the erased owner into an `Any` owner for a consuming downcast.
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;

    /// Returns the fully qualified Rust name of the concrete payload type.
    fn concrete_type_name(&self) -> &'static str;
}

impl<T> ErasedValue for T
where
    T: Any + Clone + Send,
{
    fn clone_box(&self) -> Box<dyn ErasedValue> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }

    fn concrete_type_name(&self) -> &'static str {
        type_name::<T>()
    }
}

/// An owned, cloneable, type-erased system-state payload.
///
/// The wrapper has one stable representation regardless of the concrete
/// payload type. Its methods are crate-private because public callers should
/// use the typed dictionary operations provided by `SystemState`.
pub(crate) struct StateValue {
    inner: Box<dyn ErasedValue>,
}

impl StateValue {
    /// Erases and takes ownership of a concrete payload.
    ///
    /// This operation does not clone `value`. The `Clone` bound exists solely
    /// so an enclosing `SystemState` can honor an explicit clone request.
    pub(crate) fn new<T>(value: T) -> Self
    where
        T: Any + Clone + Send,
    {
        Self {
            inner: Box::new(value),
        }
    }

    /// Returns the runtime identifier of the stored concrete type.
    pub(crate) fn type_id(&self) -> TypeId {
        self.inner.as_any().type_id()
    }

    /// Returns the fully qualified Rust name of the stored concrete type.
    ///
    /// Type names are intended for diagnostics only. Persisted formats must
    /// use stable codec tags from `StateSpec`, because Rust type-name spelling
    /// is not a compatibility contract.
    pub(crate) fn type_name(&self) -> &'static str {
        self.inner.concrete_type_name()
    }

    /// Reports whether the payload has concrete type `T`.
    pub(crate) fn is<T>(&self) -> bool
    where
        T: Any,
    {
        self.type_id() == TypeId::of::<T>()
    }

    /// Borrows the payload as `T` when the concrete type matches.
    pub(crate) fn downcast_ref<T>(&self) -> Option<&T>
    where
        T: Any,
    {
        self.inner.as_any().downcast_ref::<T>()
    }

    /// Mutably borrows the payload as `T` when the concrete type matches.
    pub(crate) fn downcast_mut<T>(&mut self) -> Option<&mut T>
    where
        T: Any,
    {
        self.inner.as_any_mut().downcast_mut::<T>()
    }

    /// Returns ownership of the payload as `T` when the concrete type matches.
    ///
    /// On a type mismatch, the original `StateValue` is returned unchanged so
    /// callers can restore it to its state slot. A successful extraction does
    /// not invoke `Clone`; it consumes the internal box and moves out `T`.
    pub(crate) fn downcast<T>(self) -> Result<T, Self>
    where
        T: Any + Send,
    {
        if !self.is::<T>() {
            return Err(self);
        }

        // The TypeId comparison above and `into_any` are both implemented by
        // the same concrete `ErasedValue` object. Their agreement is an
        // internal invariant, so a failed downcast here would indicate a bug
        // in this module rather than malformed user data.
        match self.inner.into_any().downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(_) => unreachable!("matching TypeId failed its Any downcast"),
        }
    }
}

impl Clone for StateValue {
    /// Deep-clones the stored concrete payload.
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone_box(),
        }
    }
}

impl fmt::Debug for StateValue {
    /// Formats type information without recursively printing scientific data.
    ///
    /// Large tensors and lattices can make derived debug output prohibitively
    /// expensive. The payload remains inspectable through a typed borrow when
    /// a caller intentionally wants its contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateValue")
            .field("type_name", &self.type_name())
            .finish_non_exhaustive()
    }
}
