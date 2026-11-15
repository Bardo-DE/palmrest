//! The axis lock as a value: accumulate the centroid's motion until it decides an intent,
//! then report what the stroke moves.
//!
//! Settled behaviour (11.3.9): accumulate centroid deltas until one axis passes the threshold
//! and dominates by the ratio; lock it and drop the accumulated motion so locking never jumps
//! (the dropped motion is handed on as the lead); a delta arriving at least the timeout after
//! the previous motion unlocks.

use super::follow::Follow;
use super::rule::{Params, intent_of};
use super::{Intent, Motion};
use crate::gestures::{Hold, Sample};
use crate::time::Mono;
use palmrest_protocol::Axis;

/// What one report did to the lock, in order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    /// Fewer than one or more than two contacts: the stroke is over.
    Reset,
    /// A new contact set landed (or the set changed).
    Start { fingers: u8 },
    /// The lock timed out on this report's delta.
    Unlocked,
    /// The intent was decided; `lead` is the motion accumulated before it.
    Locked { intent: Intent, lead: Motion },
    /// The stroke moved, `dt_s` seconds after its previous motion.
    Moved { motion: Motion, dt_s: f64 },
}

/// The decision between reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    /// Not decided: this much motion has accumulated, in grid units.
    Free {
        x: i32,
        y: i32,
    },
    Locked(Intent),
}

const FREE: Decision = Decision::Free { x: 0, y: 0 };

/// The lock between reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Lock {
    follow: Follow,
    decision: Decision,
    last_motion: Option<Mono>,
}

impl Default for Lock {
    fn default() -> Self {
        Self {
            follow: Follow::default(),
            decision: FREE,
            last_motion: None,
        }
    }
}

impl Lock {
    /// The decided intent, if any.
    pub fn intent(&self) -> Option<Intent> {
        match self.decision {
            Decision::Locked(intent) => Some(intent),
            Decision::Free { .. } => None,
        }
    }

    /// Drops the decision but keeps following the same contacts: a click or a palm cancelled the
    /// stroke, and the next motion must decide afresh.
    pub fn cancel(self) -> Self {
        Self {
            decision: FREE,
            ..self
        }
    }

    /// Consumes one report's contacts.
    pub fn step(
        self,
        contacts: &[Sample],
        now: Mono,
        p: &Params,
        hold: Hold,
    ) -> (Self, Vec<Signal>) {
        match contacts.len() {
            1 | 2 if self.follow.is(contacts) => self.follow_on(contacts, now, p, hold),
            n @ (1 | 2) => (
                Self {
                    follow: Follow::land(contacts),
                    decision: FREE,
                    last_motion: Some(now),
                },
                vec![Signal::Start { fingers: n as u8 }],
            ),
            _ => (Self::default(), vec![Signal::Reset]),
        }
    }

    fn follow_on(
        self,
        contacts: &[Sample],
        now: Mono,
        p: &Params,
        hold: Hold,
    ) -> (Self, Vec<Signal>) {
        let (follow, delta) = self.follow.delta(contacts);
        let s = Self { follow, ..self };
        if delta == (0, 0) || hold == Hold::Held {
            (s, vec![])
        } else {
            s.motion(delta, now, p)
        }
    }

    fn motion(self, delta: (i32, i32), now: Mono, p: &Params) -> (Self, Vec<Signal>) {
        let timed_out = self
            .last_motion
            .is_some_and(|t| now.float_since(t) >= p.timeout.as_secs_f64());
        match self.decision {
            Decision::Free { .. } => self.accumulate(delta, now, p, vec![]),
            Decision::Locked(_) if timed_out => {
                self.cancel()
                    .accumulate(delta, now, p, vec![Signal::Unlocked])
            }
            Decision::Locked(intent) => self.moved(intent, delta, now),
        }
    }

    /// A locked stroke moved by `delta`: report what its intent follows of it.
    fn moved(self, intent: Intent, delta: (i32, i32), now: Mono) -> (Self, Vec<Signal>) {
        let along = along(intent, delta);
        if along == (0, 0) {
            return (self, vec![]);
        }
        let dt_s = self.last_motion.map_or(0.0, |t| now.float_since(t));
        let signal = Signal::Moved {
            motion: Motion::of(along),
            dt_s,
        };
        (
            Self {
                last_motion: Some(now),
                ..self
            },
            vec![signal],
        )
    }

    fn accumulate(
        self,
        delta: (i32, i32),
        now: Mono,
        p: &Params,
        before: Vec<Signal>,
    ) -> (Self, Vec<Signal>) {
        let (px, py) = match self.decision {
            Decision::Free { x, y } => (x, y),
            Decision::Locked(_) => (0, 0),
        };
        let sum = (px + delta.0, py + delta.1);
        let decided = intent_of(sum, self.follow.len(), p);
        let (decision, signals) = match decided {
            Some(intent) => (
                Decision::Locked(intent),
                vec![Signal::Locked {
                    intent,
                    lead: Motion::of(along(intent, sum)),
                }],
            ),
            None => (Decision::Free { x: sum.0, y: sum.1 }, vec![]),
        };
        (
            Self {
                decision,
                last_motion: Some(now),
                ..self
            },
            before.into_iter().chain(signals).collect(),
        )
    }
}

/// The part of a grid-unit motion an intent follows: a scroll its axis only, a swipe both.
fn along(intent: Intent, (dx, dy): (i32, i32)) -> (i32, i32) {
    match intent {
        Intent::Scroll(Axis::X) => (dx, 0),
        Intent::Scroll(Axis::Y) => (0, dy),
        Intent::SpaceSwipe => (dx, dy),
    }
}
