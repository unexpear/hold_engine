mod bus;
mod queue;

pub use bus::EventBus;
pub use queue::EventQueue;

// ---------------------------------------------------------------------------
// Built-in engine events
// ---------------------------------------------------------------------------

/// Emitted when the window / surface is resized.
pub struct WindowResized {
    pub width: u32,
    pub height: u32,
}

/// Emitted when the application requests a clean shutdown.
pub struct AppExit;
