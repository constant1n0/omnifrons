//! The `Clock` port: the current time, abstracted so application-layer
//! logic (`LaunchGate`) can be tested against a fixed instant rather than
//! the real wall clock.

use std::time::SystemTime;

/// A source of the current time.
pub trait Clock {
    /// The current time.
    fn now(&self) -> SystemTime;
}
