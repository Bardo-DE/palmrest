//! The stroke recogniser: G1/G2/G5's one reading of the contacts.
//!
//! It consumes each report's rejected and filtered contact samples once. It owns the contact
//! set, the one centroid-delta computation, the axis decision and the scroll-or-swipe intent,
//! and it emits what the stroke does as phases (`Event`) carrying physical motion in
//! millimetres per axis, so every output reads the same stroke and applies its own feel.
//!
//! One or two fingers moving up or down, or one finger moving sideways, is a `Scroll` on the
//! locked axis. Two fingers moving sideways is a `SpaceSwipe`.

mod follow;
mod life;
mod lock;
mod rule;
#[cfg(test)]
mod tests;

use super::{Hold, Sample};
use crate::sensor::lattice;
use crate::sensor::units::Mm;
use crate::time::Mono;
use life::Life;
use lock::Lock;
use palmrest_protocol::{Axis, GestureId, Phase};
pub use rule::{Params, Threshold};

/// What a decided stroke is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Scroll on the locked axis.
    Scroll(Axis),
    /// Two fingers moving sideways: the Spaces swipe.
    SpaceSwipe,
}

/// Physical motion of the fingers' centroid, in millimetres, `+x` rightwards and `+y` toward
/// the user.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Motion {
    pub x: Mm,
    pub y: Mm,
}

impl Motion {
    /// A motion given in grid units.
    fn of((dx, dy): (i32, i32)) -> Self {
        Self {
            x: lattice::mm(dx, Axis::X),
            y: lattice::mm(dy, Axis::Y),
        }
    }

    /// The same motion with each axis passed through `f`.
    pub fn map(self, f: impl Fn(Mm) -> Mm) -> Self {
        Self {
            x: f(self.x),
            y: f(self.y),
        }
    }
}

/// One phase of a stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    pub gesture: GestureId,
    pub phase: Phase,
    pub fingers: u8,
    /// What the stroke is for: known from Began on, and carried by its Changed, Ended and
    /// Cancelled; none before the decision.
    pub intent: Option<Intent>,
    /// Began: the motion that accumulated before the decision and was dropped from the stroke,
    /// whose direction says which way the stroke starts. Changed: this report's motion on the
    /// axes the intent follows. Zero otherwise.
    pub motion: Motion,
    /// Changed: seconds since the stroke's previous motion.
    pub dt_s: f64,
}

impl Event {
    fn bare(gesture: GestureId, fingers: u8, phase: Phase, intent: Option<Intent>) -> Self {
        Self {
            gesture,
            phase,
            fingers,
            intent,
            motion: Motion::default(),
            dt_s: 0.0,
        }
    }
}

/// The recogniser between reports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recogniser {
    lock: Lock,
    life: Life,
}

impl Recogniser {
    /// The decided intent after the last report.
    pub fn intent(&self) -> Option<Intent> {
        self.lock.intent()
    }

    /// Consumes one report's contacts.
    pub fn step(
        self,
        contacts: &[Sample],
        now: Mono,
        p: &Params,
        hold: Hold,
    ) -> (Self, Vec<Event>) {
        let (lock, signals) = self.lock.step(contacts, now, p, hold);
        let (life, events) = self.life.apply(signals);
        (Self { lock, life }, events)
    }

    /// Cancels the running stroke (P3 palm, P4 click): the fingers stay followed, and the next
    /// motion decides afresh.
    pub fn cancel(self) -> (Self, Vec<Event>) {
        let (life, events) = self.life.cancel();
        let lock = if events.is_empty() {
            self.lock
        } else {
            self.lock.cancel()
        };
        (Self { lock, life }, events)
    }

    /// Cancels and forgets everything (device gone).
    pub fn disconnect(self) -> (Self, Vec<Event>) {
        let (life, events) = self.life.disconnect();
        (
            Self {
                lock: Lock::default(),
                life,
            },
            events,
        )
    }
}
