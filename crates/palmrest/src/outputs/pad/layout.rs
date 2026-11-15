//! Where the virtual fingers land and how far they may travel, per intent: pure geometry in
//! millimetres on the pad, `+x` rightwards and `+y` toward the user.
//!
//! libinput reads the pad as a real touchpad, so the contacts keep out of the places it
//! ignores: its left and right edge palm zones at landing, and the bottom thumb zone of a
//! clickpad.

use super::{PAD_X_MAX, PAD_Y_MAX, UNITS_PER_MM};
use crate::gestures::{Intent, Motion};
use crate::sensor::units::{Mm, MmPoint};
use palmrest_protocol::Axis;

/// libinput's edge palm zones: no contact lands this close to the left or right edge.
const SIDE_ZONE_MM: f64 = 10.0;
/// No contact lands lower than this (the bottom thumb zone of a clickpad starts below it).
const LAND_Y_MAX_MM: f64 = 55.0;
/// Contacts stay at least this far from every edge while moving.
const MOVE_MARGIN_MM: f64 = 5.0;
/// The contacts sit at this height when the stroke does not move along y.
const REST_Y_MM: f64 = 35.0;

/// The contacts one intent puts down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub fingers: u8,
    /// The gap between neighbouring contacts, in millimetres.
    pub spacing_mm: f64,
}

/// The contacts of `intent`: a scroll is two fingers 20 mm apart, a Spaces swipe four fingers
/// in a row 12 mm apart.
pub fn shape(intent: Intent) -> Shape {
    match intent {
        Intent::Scroll(_) => Shape {
            fingers: 2,
            spacing_mm: 20.0,
        },
        Intent::SpaceSwipe => Shape {
            fingers: 4,
            spacing_mm: 12.0,
        },
    }
}

/// An inclusive stretch of one axis, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    lo: f64,
    hi: f64,
}

impl Span {
    fn contains(self, v: f64) -> bool {
        (self.lo..=self.hi).contains(&v)
    }

    fn clamp(self, v: f64) -> f64 {
        v.clamp(self.lo, self.hi)
    }
}

fn width_mm() -> f64 {
    f64::from(PAD_X_MAX) / f64::from(UNITS_PER_MM)
}

fn height_mm() -> f64 {
    f64::from(PAD_Y_MAX) / f64::from(UNITS_PER_MM)
}

/// The axis the intent's stroke moves along.
fn axis(intent: Intent) -> Axis {
    match intent {
        Intent::Scroll(axis) => axis,
        Intent::SpaceSwipe => Axis::X,
    }
}

/// How far the outer contacts sit from the centre of the row.
fn half_row_mm(intent: Intent) -> f64 {
    let s = shape(intent);
    f64::from(s.fingers - 1) * s.spacing_mm / 2.0
}

/// Where the centre may be while moving.
fn travel_span(intent: Intent) -> Span {
    match axis(intent) {
        Axis::X => Span {
            lo: MOVE_MARGIN_MM + half_row_mm(intent),
            hi: width_mm() - MOVE_MARGIN_MM - half_row_mm(intent),
        },
        Axis::Y => Span {
            lo: MOVE_MARGIN_MM,
            hi: height_mm() - MOVE_MARGIN_MM,
        },
    }
}

/// Where the centre may land.
fn landing_span(intent: Intent) -> Span {
    match axis(intent) {
        Axis::X => Span {
            lo: SIDE_ZONE_MM + half_row_mm(intent),
            hi: width_mm() - SIDE_ZONE_MM - half_row_mm(intent),
        },
        Axis::Y => Span {
            lo: MOVE_MARGIN_MM,
            hi: LAND_Y_MAX_MM.min(height_mm() - MOVE_MARGIN_MM),
        },
    }
}

/// The centre of the row for a stroke starting with `lead`: on the side opposite the motion
/// along the stroke's axis, so the stroke has the most room (moving down lands near the top,
/// moving right near the left); at rest across the other axis.
pub fn landing(intent: Intent, lead: Motion) -> MmPoint {
    let span = landing_span(intent);
    let start = |along: Mm| if along.0 >= 0.0 { span.lo } else { span.hi };
    match axis(intent) {
        Axis::X => MmPoint {
            x: Mm(start(lead.x)),
            y: Mm(REST_Y_MM),
        },
        Axis::Y => MmPoint {
            x: Mm(width_mm() / 2.0),
            y: Mm(start(lead.y)),
        },
    }
}

/// Where a move ends up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Next {
    Within(MmPoint),
    /// A scroll that would leave the pad: it must land again.
    OutOfRoom,
}

/// The centre after moving by `by`: along the stroke's axis only. A scroll that runs out of
/// room must land again; a swipe never does, it stops at the edge.
pub fn advance(intent: Intent, at: MmPoint, by: Motion) -> Next {
    let span = travel_span(intent);
    let (from, delta) = match axis(intent) {
        Axis::X => (at.x.0, by.x.0),
        Axis::Y => (at.y.0, by.y.0),
    };
    let want = from + delta;
    let along = match intent {
        Intent::SpaceSwipe => span.clamp(want),
        Intent::Scroll(_) if span.contains(want) => want,
        Intent::Scroll(_) => return Next::OutOfRoom,
    };
    Next::Within(match axis(intent) {
        Axis::X => MmPoint { x: Mm(along), ..at },
        Axis::Y => MmPoint { y: Mm(along), ..at },
    })
}
