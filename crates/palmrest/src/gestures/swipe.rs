//! G5 for a remapped two-finger swipe: sideways travel of the recogniser's `SpaceSwipe` fires
//! the mapped action once per threshold.
//!
//! When both two-finger swipes are the workspace defaults the swipe is live on the virtual
//! touchpad and nothing here runs. When the person mapped them to something else (keys, a
//! command, a shell line) this fires it. Travel counts from the start of the stroke, including
//! the motion that accumulated before the recogniser decided; `THRESHOLD` of it fires once in
//! the direction the fingers moved and starts the count again, so a long drag keeps going after
//! the cooldown.

use super::stroke::{Event, Intent, Moment};
use crate::config::{Ms, Um};
use crate::sensor::lattice::{grid, units};
use crate::sensor::units::Mm;
use crate::time::Mono;
use palmrest_protocol::Axis;

/// Horizontal travel that fires: 200 grid units. The same number as `bands::SIDE_BAND_LEFT`, but
/// not the same fact (a distance to travel against a width of surface), so each keeps its own.
const THRESHOLD: Um = Um(7692);
/// The next swipe counts only this long after the last fired.
const COOLDOWN: Ms = Ms(500);

/// Which way the fingers moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerDirection {
    Left,
    Right,
}

/// The discrete swipe between reports.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SwipeFire {
    /// Sideways travel since the start of the stroke or the last threshold, in millimetres.
    travel: f64,
    last_fire: Option<Mono>,
}

impl SwipeFire {
    /// Consumes one phase of a stroke; returns the direction when the swipe fires.
    pub fn step(self, e: &Event, now: Mono) -> (Self, Option<FingerDirection>) {
        match e.moment {
            Moment::Began {
                intent: Intent::SpaceSwipe,
                lead,
            } => self.travelled(lead.x, 0.0, now),
            Moment::Changed {
                intent: Intent::SpaceSwipe,
                motion,
                ..
            } => self.travelled(motion.x, self.travel, now),
            Moment::Ended { .. } | Moment::Cancelled { .. } => (
                Self {
                    travel: 0.0,
                    ..self
                },
                None,
            ),
            Moment::Announced
            | Moment::Began {
                intent: Intent::Scroll(_),
                ..
            }
            | Moment::Changed {
                intent: Intent::Scroll(_),
                ..
            } => (self, None),
        }
    }

    /// `by` more travel on top of `from`: fires when the threshold is passed.
    fn travelled(self, by: Mm, from: f64, now: Mono) -> (Self, Option<FingerDirection>) {
        let travel = from + by.0;
        let reached = grid(Mm(travel), Axis::X).abs() >= units(THRESHOLD, Axis::X).max(1);
        if !reached {
            return (Self { travel, ..self }, None);
        }
        let cooled = self
            .last_fire
            .is_none_or(|t| now.since(t) >= COOLDOWN.duration());
        let dir = if travel > 0.0 {
            FingerDirection::Right
        } else {
            FingerDirection::Left
        };
        (
            Self {
                travel: 0.0,
                last_fire: if cooled { Some(now) } else { self.last_fire },
            },
            cooled.then_some(dir),
        )
    }
}
