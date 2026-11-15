//! O2: the strokes as fingers on a virtual touchpad (12.3.9).
//!
//! Foreign clients get momentum and their own feel from libinput, so the contacts must look like
//! a real touchpad's and never like anything else. Two intents put fingers down:
//!
//! - a **scroll**: two contacts 20 mm apart moving together along the stroke's axis, landing on
//!   the side opposite the motion so a whole mouse stroke fits without landing again;
//! - a **Spaces swipe**: four contacts in a row 12 mm apart following the fingers' x, which the
//!   compositor reads as a four-finger swipe. It never lands again mid-swipe: at the edge it
//!   stops.
//!
//! Contacts land at the decision (never at MayBegin, so a resting finger makes nothing), and are
//! held down until `MIN_DOWN` so libinput's tap timeout has passed. `layout` has the geometry
//! and the libinput zones it keeps clear of.

mod layout;

use crate::config::Ms;
use crate::gestures::{Intent, Motion};
use crate::sensor::units::MmPoint;
use crate::time::Mono;
use serde::{Deserialize, Serialize};

/// How long the fingers stay down so apps see a stroke.
const MIN_DOWN: Ms = Ms(200);

/// Pad units per millimetre (resolution of ABS_MT_POSITION_*).
pub const UNITS_PER_MM: i32 = 40;
/// The pad's extent in units (100 x 70 mm).
pub const PAD_X_MAX: i32 = 4000;
pub const PAD_Y_MAX: i32 = 2800;

/// The centre of the contact row, in pad units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadPoint {
    pub x: i32,
    pub y: i32,
}

/// The pad's centre.
pub const CENTRE: PadPoint = PadPoint {
    x: PAD_X_MAX / 2,
    y: PAD_Y_MAX / 2,
};

/// One frame for the virtual touchpad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum PadFrame {
    /// `fingers` contacts land around `at`, `spacing` units apart horizontally.
    Land {
        fingers: u8,
        at: PadPoint,
        spacing: i32,
    },
    /// Every contact moves so the row's centre is at `at`.
    Move { at: PadPoint },
    /// Every contact lifts.
    Lift,
}

/// What is down: the intent it landed for, when, and where its centre is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Down {
    intent: Intent,
    landed: Mono,
    at: MmPoint,
}

/// The pad between frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Pad {
    #[default]
    Up,
    Down(Down),
    /// The stroke ended but the lift waits until `until`.
    Lifting {
        intent: Intent,
        until: Mono,
    },
}

impl Pad {
    /// A stroke decided its intent: land its contacts, on the side opposite `lead`'s motion
    /// (lifting a pending row first, so the tool count never changes under a client's feet).
    pub fn land(self, intent: Intent, lead: Motion, now: Mono) -> (Self, Vec<PadFrame>) {
        let (_, lifted) = self.lift_now();
        let at = layout::landing(intent, lead);
        let shape = layout::shape(intent);
        let land = PadFrame::Land {
            fingers: shape.fingers,
            at: point(at),
            spacing: units(shape.spacing_mm),
        };
        (
            Pad::Down(Down {
                intent,
                landed: now,
                at,
            }),
            lifted.into_iter().chain([land]).collect(),
        )
    }

    /// Moves the contacts of a stroke of `intent` by `by`, in pad millimetres. A scroll that
    /// runs out of room lands again on the opposite side; a swipe stops at the edge. Nothing
    /// moves when nothing of that intent is down.
    pub fn moved(self, intent: Intent, by: Motion, now: Mono) -> (Self, Vec<PadFrame>) {
        match self {
            Pad::Down(down) if down.intent == intent => {
                match layout::advance(intent, down.at, by) {
                    layout::Next::Within(at) => (
                        Pad::Down(Down { at, ..down }),
                        vec![PadFrame::Move { at: point(at) }],
                    ),
                    layout::Next::OutOfRoom => self.land(intent, by, now),
                }
            }
            other => (other, vec![]),
        }
    }

    /// The stroke Ended or was Cancelled: lift now, or at `landed + MIN_DOWN` if the stroke is
    /// younger than that. A stroke older than that has outlived the tap timeout whatever it
    /// moved.
    pub fn end(self, now: Mono) -> (Self, Vec<PadFrame>) {
        match self {
            Pad::Down(down) => {
                let until = down.landed.after(MIN_DOWN.duration());
                if now < until {
                    (
                        Pad::Lifting {
                            intent: down.intent,
                            until,
                        },
                        vec![],
                    )
                } else {
                    (Pad::Up, vec![PadFrame::Lift])
                }
            }
            other => (other, vec![]),
        }
    }

    /// Performs a delayed lift that has come due.
    pub fn tick(self, now: Mono) -> (Self, Vec<PadFrame>) {
        match self {
            Pad::Lifting { until, .. } if now >= until => (Pad::Up, vec![PadFrame::Lift]),
            other => (other, vec![]),
        }
    }

    /// Lifts whatever is down, immediately (disconnect).
    pub fn lift_now(self) -> (Self, Vec<PadFrame>) {
        match self {
            Pad::Up => (Pad::Up, vec![]),
            _ => (Pad::Up, vec![PadFrame::Lift]),
        }
    }

    /// Lifts a scroll's contacts immediately and leaves a swipe alone: a host took the pointer,
    /// and the swipe is the compositor's, whoever holds the pointer.
    pub fn lift_scroll(self) -> (Self, Vec<PadFrame>) {
        match self.intent() {
            Some(Intent::Scroll(_)) => self.lift_now(),
            _ => (self, vec![]),
        }
    }

    /// Lifts a swipe's contacts immediately and leaves a scroll alone: the session locked.
    pub fn lift_swipe(self) -> (Self, Vec<PadFrame>) {
        match self.intent() {
            Some(Intent::SpaceSwipe) => self.lift_now(),
            _ => (self, vec![]),
        }
    }

    /// When `tick` next has work.
    pub fn deadline(&self) -> Option<Mono> {
        match self {
            Pad::Lifting { until, .. } => Some(*until),
            _ => None,
        }
    }

    /// The intent of the contacts that are down or waiting to lift.
    fn intent(&self) -> Option<Intent> {
        match self {
            Pad::Up => None,
            Pad::Down(down) => Some(down.intent),
            Pad::Lifting { intent, .. } => Some(*intent),
        }
    }
}

/// Millimetres on the pad in whole pad units.
fn units(mm: f64) -> i32 {
    (mm * f64::from(UNITS_PER_MM)).round() as i32
}

fn point(at: MmPoint) -> PadPoint {
    PadPoint {
        x: units(at.x.0),
        y: units(at.y.0),
    }
}

#[cfg(test)]
mod tests;
