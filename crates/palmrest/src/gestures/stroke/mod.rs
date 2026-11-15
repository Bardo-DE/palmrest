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
pub(crate) use rule::{LOCK_THRESHOLD_X, LOCK_THRESHOLD_Y};
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

/// One phase of one gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    pub gesture: GestureId,
    pub fingers: u8,
    pub moment: Moment,
}

/// What a gesture's phase carries: an intent exists from Began on, so only the phases that can
/// come before the decision (Ended, Cancelled) hold it as an option.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Moment {
    /// MayBegin: a set of one or two fingers landed.
    Announced,
    /// The intent was decided. `lead` is the motion that accumulated before the decision and
    /// was dropped from the stroke; its direction says which way the stroke starts.
    Began { intent: Intent, lead: Motion },
    /// The stroke moved: `motion` on the axes the intent follows, `dt_s` seconds after its
    /// previous motion.
    Changed {
        intent: Intent,
        motion: Motion,
        dt_s: f64,
    },
    /// The contact set ended or changed, or a scroll's lock timed out.
    Ended { intent: Option<Intent> },
    /// A click or a palm cancelled the gesture.
    Cancelled { intent: Option<Intent> },
}

impl Event {
    /// The phase the shell stream names this event by.
    pub fn phase(&self) -> Phase {
        match self.moment {
            Moment::Announced => Phase::MayBegin,
            Moment::Began { .. } => Phase::Began,
            Moment::Changed { .. } => Phase::Changed,
            Moment::Ended { .. } => Phase::Ended,
            Moment::Cancelled { .. } => Phase::Cancelled,
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
