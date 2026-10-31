//! Properties (ADR 0003): named State where only the latest value matters.
//! `Property::set` marks dirty — nothing fires synchronously; [`Hub`]'s flush
//! step evaluates changes in a deterministic order (object insertion order,
//! then declaration order) and notifies observers as queued signal deliveries.

use core::any::Any;
use core::cell::{Cell, RefCell};

/// A cloneable, type-erased property payload for heterogeneous fanout.
pub trait CloneAny: Any {
    fn clone_any(&self) -> Box<dyn Any>;
}

impl<T: Clone + 'static> CloneAny for T {
    fn clone_any(&self) -> Box<dyn Any> {
        Box::new(self.clone())
    }
}

/// Named state with change tracking. Values must be `Clone` (observers each
/// receive a copy). Interior mutability is `RefCell` — single-threaded core.
pub struct Property<T> {
    value: RefCell<T>,
    dirty: Cell<bool>,
}

impl<T> Property<T> {
    pub const fn new(value: T) -> Self {
        Self {
            value: RefCell::new(value),
            dirty: Cell::new(false),
        }
    }

    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.value.borrow().clone()
    }

    /// Store the latest value and mark dirty. Change notification happens at
    /// the next [`Hub`](crate::signal::Hub) flush — never synchronously.
    pub fn set(&self, value: T) {
        *self.value.borrow_mut() = value;
        self.dirty.set(true);
    }

    #[doc(hidden)]
    pub fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }
}

/// Runtime hook the derive emits: reports each dirty property as
/// `(signal index, latest value)`, clearing the dirty marks.
pub trait PropertyInputs {
    fn take_dirty_changes(&mut self) -> Vec<(u32, Box<dyn CloneAny>)>;
}
