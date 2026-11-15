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
//! Contacts land at the decision (never at MayBegin, so a resting finger makes nothing). A
//! scroll's two contacts never lift before `MIN_DOWN` (`Down::could_tap`): libinput reads two
//! contacts that go down and up inside its tap timeout without moving as a two-finger tap, a
//! right click (measured 2026-11-15: a flick's last reverse step re-landed contacts that had just
//! landed). An ending, cancelled or no longer allowed scroll lifts at `MIN_DOWN`, one out of room
//! stops at the edge until then, and a new scroll over a pending lift takes the contacts over. A
//! swipe's lift at once, since libinput only taps with up to three fingers and a held swipe would
//! add stationary time to a flick. A cancelled swipe returns to where it landed before it lifts, so
//! the compositor sees no travel and stays on its Space. `policy` says what may be down at all;
//! `layout` has the geometry and the libinput zones it keeps clear of.

mod layout;
mod policy;

pub use policy::Policy;

use crate::config::Ms;
use crate::gestures::{Intent, Motion};
use crate::sensor::units::{Mm, MmPoint};
use crate::time::Mono;
use serde::{Deserialize, Serialize};

/// How long a scroll's fingers stay down so apps see a stroke.
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

/// What is down: the intent it landed for, when, where it landed and where its centre is now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Down {
    intent: Intent,
    landed: Mono,
    origin: MmPoint,
    /// Where the fingers have taken the contacts.
    at: MmPoint,
    /// Where the contacts were last written: `at`, or for a swipe the mean of `at` and the
    /// position before it (`shown_after`).
    shown: MmPoint,
}

/// The pad between frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Pad {
    #[default]
    Up,
    Down(Down),
    /// A scroll ended but its contacts stay where they are until `until`.
    Lifting {
        down: Down,
        until: Mono,
    },
}

impl Down {
    /// When lifting these contacts is no longer a tap to libinput.
    fn safe_from(&self) -> Mono {
        self.landed.after(MIN_DOWN.duration())
    }

    /// Whether lifting these contacts at `now` could read as a tap: a scroll's two contacts
    /// younger than `MIN_DOWN`. A swipe's four never tap.
    fn could_tap(&self, now: Mono) -> bool {
        matches!(self.intent, Intent::Scroll(_)) && now < self.safe_from()
    }

    /// A pending lift of these contacts, at the earliest moment it is not a tap.
    fn lifting(self) -> Pad {
        Pad::Lifting {
            until: self.safe_from(),
            down: self,
        }
    }
}

impl Pad {
    /// A stroke decided its intent: land its contacts, on the side opposite `lead`'s motion
    /// (lifting a pending row first, so the tool count never changes under a client's feet). A
    /// scroll over a pending scroll's lift takes its contacts over instead, where they are, so
    /// they never go up as a tap.
    pub fn land(self, intent: Intent, lead: Motion, now: Mono) -> (Self, Vec<PadFrame>) {
        if let Pad::Lifting { down, .. } = self
            && layout::shape(down.intent).fingers == layout::shape(intent).fingers
        {
            return (Pad::Down(Down { intent, ..down }), vec![]);
        }
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
                origin: at,
                at,
                shown: at,
            }),
            lifted.into_iter().chain([land]).collect(),
        )
    }

    /// Moves the contacts of a stroke of `intent` by `by`, in pad millimetres. A scroll that
    /// runs out of room lands again on the opposite side, unless its contacts are younger than
    /// `MIN_DOWN`: then they stop at the edge like a swipe's. Nothing moves when nothing of that
    /// intent is down.
    pub fn moved(self, intent: Intent, by: Motion, now: Mono) -> (Self, Vec<PadFrame>) {
        match self {
            Pad::Down(down) if down.intent == intent => {
                match layout::advance(intent, down.at, by) {
                    layout::Next::Within(at) => {
                        let shown = shown_after(intent, down.at, at);
                        (
                            Pad::Down(Down { at, shown, ..down }),
                            vec![PadFrame::Move { at: point(shown) }],
                        )
                    }
                    // Young contacts that lifted without moving would be a tap to libinput,
                    // and two fingers tapping is a right click: they wait at the edge instead.
                    layout::Next::OutOfRoom if down.could_tap(now) => {
                        let at = layout::held(intent, down.at, by);
                        let moved = (at != down.at).then(|| PadFrame::Move { at: point(at) });
                        (
                            Pad::Down(Down {
                                at,
                                shown: at,
                                ..down
                            }),
                            moved.into_iter().collect(),
                        )
                    }
                    layout::Next::OutOfRoom => self.land(intent, by, now),
                }
            }
            other => (other, vec![]),
        }
    }

    /// The stroke Ended. A swipe lifts now. A scroll lifts now, or at `landed + MIN_DOWN` if it
    /// is younger than that: one older has outlived the tap timeout whatever it moved.
    pub fn end(self, now: Mono) -> (Self, Vec<PadFrame>) {
        match self {
            Pad::Down(down) if down.could_tap(now) => (down.lifting(), vec![]),
            // A swipe's row catches up with the fingers before it lifts, so the compositor
            // reads the whole travel and the last report's speed.
            Pad::Down(down) => {
                let catch_up =
                    (down.shown != down.at).then(|| PadFrame::Move { at: point(down.at) });
                (
                    Pad::Up,
                    catch_up.into_iter().chain([PadFrame::Lift]).collect(),
                )
            }
            other => (other, vec![]),
        }
    }

    /// The stroke was Cancelled (a click or a palm). A scroll ends as it would have. A swipe
    /// first moves the row back to where it landed, in one frame, and then lifts, so the
    /// compositor reads no travel and returns to the Space it was on.
    pub fn cancel(self, now: Mono) -> (Self, Vec<PadFrame>) {
        match self {
            swipe @ Pad::Down(Down {
                intent: Intent::SpaceSwipe,
                ..
            }) => swipe.withdraw(now),
            other => other.end(now),
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

    /// Takes away whatever the policy no longer allows: a host took the pointer from a scroll,
    /// or the session locked on a swipe.
    pub fn enforce(self, policy: Policy, now: Mono) -> (Self, Vec<PadFrame>) {
        match self.intent() {
            Some(intent) if !policy.allows(intent) => self.withdraw(now),
            _ => (self, vec![]),
        }
    }

    /// Takes the contacts away: now, or for a scroll that could still tap at `MIN_DOWN`, still
    /// (a pending lift stays pending). A swipe is first returned to where it landed, so the
    /// compositor puts the Space back instead of committing the one it was moving to.
    fn withdraw(self, now: Mono) -> (Self, Vec<PadFrame>) {
        match self {
            Pad::Down(down) if down.could_tap(now) => (down.lifting(), vec![]),
            lifting @ Pad::Lifting { .. } => (lifting, vec![]),
            Pad::Down(Down {
                intent: Intent::SpaceSwipe,
                origin,
                shown,
                ..
            }) => {
                let back = (shown != origin).then(|| PadFrame::Move { at: point(origin) });
                (Pad::Up, back.into_iter().chain([PadFrame::Lift]).collect())
            }
            other => other.lift_now(),
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
            Pad::Lifting { down, .. } => Some(down.intent),
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

/// Where a stroke's contacts are written when the fingers moved from `before` to `after`. The
/// Magic Mouse reports a step about twice the size of its neighbours every few reports at a
/// steady rate (measured 2026-11-15: both fingers at once, device time unchanged), which the
/// Space strip shows as a stutter; a swipe writes the mean of the two latest positions, which
/// halves such a step for half a report of lag. A scroll goes through unchanged: it felt right on
/// the device without it.
fn shown_after(intent: Intent, before: MmPoint, after: MmPoint) -> MmPoint {
    match intent {
        Intent::SpaceSwipe => MmPoint {
            x: Mm((before.x.0 + after.x.0) / 2.0),
            y: Mm((before.y.0 + after.y.0) / 2.0),
        },
        Intent::Scroll(_) => after,
    }
}

#[cfg(test)]
mod tests;
