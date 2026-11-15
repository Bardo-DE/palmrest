//! The scroll feel: every transfer function from the fingers' millimetres to an output, one
//! function per sink. The recogniser (`gestures::stroke`) says what the fingers did in
//! millimetres; nothing outside this module turns that into pixels or pad distance.
//!
//! - `shell`: millimetres to the px the shell's own surfaces scroll (`ToHost::Scroll`).
//! - `pad`: millimetres to the distance the virtual touchpad's fingers move, for every other
//!   client, whose scroll feel libinput owns.

pub mod curve;
pub mod pad;
pub mod shell;
