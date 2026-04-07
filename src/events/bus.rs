use std::any::{Any, TypeId};
use std::collections::HashMap;

use super::queue::EventQueue;

// ---------------------------------------------------------------------------
// Type-erased queue trait
// ---------------------------------------------------------------------------

trait AnyQueue: Any {
    fn clear_boxed(&mut self);
}

impl<E: 'static> AnyQueue for EventQueue<E> {
    fn clear_boxed(&mut self) {
        self.clear();
    }
}

// Implement the upcast so we can recover `&EventQueue<E>` from `Box<dyn Any>`.
// `dyn AnyQueue` is stored, but we need `downcast_ref` which is on `dyn Any`.
// We achieve this by storing `Box<dyn AnyQueue>` and adding a blanket upcast.
// Actually we store `Box<dyn Any>` and separately iterate via `AnyQueue` —
// simplest approach: store `Box<dyn AnyQueue>` and add an `as_any` helper.

trait AnyQueueExt: AnyQueue {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<E: 'static> AnyQueueExt for EventQueue<E> {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// EventBus
// ---------------------------------------------------------------------------

pub struct EventBus {
    queues: HashMap<TypeId, Box<dyn AnyQueueExt>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            queues: HashMap::new(),
        }
    }

    /// Emit an event — creates the queue for this type on first use.
    pub fn emit<E: 'static>(&mut self, event: E) {
        self.get_or_create::<E>().push(event);
    }

    /// Read all events of type `E` this frame as a slice.
    /// Returns an empty slice if no events of this type have been emitted.
    pub fn read<E: 'static>(&self) -> &[E] {
        match self.queues.get(&TypeId::of::<E>()) {
            Some(boxed) => boxed
                .as_any()
                .downcast_ref::<EventQueue<E>>()
                .map(|q| q.as_slice())
                .unwrap_or(&[]),
            None => &[],
        }
    }

    /// Returns the mutable queue for type `E` (for draining / clearing).
    pub fn queue_mut<E: 'static>(&mut self) -> &mut EventQueue<E> {
        self.get_or_create::<E>()
    }

    /// Clear ALL event queues — call this at the end of each frame.
    pub fn clear_all(&mut self) {
        for queue in self.queues.values_mut() {
            queue.clear_boxed();
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn get_or_create<E: 'static>(&mut self) -> &mut EventQueue<E> {
        self.queues
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(EventQueue::<E>::new()))
            .as_any_mut()
            .downcast_mut::<EventQueue<E>>()
            .expect("EventBus: type map corrupted — TypeId collision is impossible in safe Rust")
    }
}
