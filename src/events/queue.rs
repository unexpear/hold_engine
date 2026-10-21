pub struct EventQueue<E> {
    events: Vec<E>,
}

impl<E> Default for EventQueue<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> EventQueue<E> {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn push(&mut self, event: E) {
        self.events.push(event);
    }

    pub fn iter(&self) -> std::slice::Iter<'_, E> {
        self.events.iter()
    }

    pub fn drain(&mut self) -> std::vec::Drain<'_, E> {
        self.events.drain(..)
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Expose the inner slice for zero-copy reads.
    pub fn as_slice(&self) -> &[E] {
        &self.events
    }
}
