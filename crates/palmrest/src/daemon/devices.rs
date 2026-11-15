//! The virtual devices of a daemon run, created lazily and retried while they are not ready.
//!
//! At boot the input service can arrive after the user session starts. The reader keeps running
//! meanwhile, so the socket stream to our own surfaces never waits on the virtual devices.
//!
//! When the service cuts palmrest off (closes the connection, refuses it, stops answering), the
//! devices are rebuilt after a pause, a few times; a service that keeps cutting us off ends the
//! virtual devices until the settings change, and the log says why.
//!
//! The lifecycle is pure (`model`, `step`): a state and the events that move it, with the effects
//! each move wants. `io` holds the emitter and the factory and carries the effects out.

mod io;
mod model;
mod step;

pub use io::{Devices, Factory};

#[cfg(test)]
mod tests;
