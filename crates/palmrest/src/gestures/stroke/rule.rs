//! The axis-lock rule and its numbers (11.3.9, 12.5): which intent a stroke's accumulated
//! motion decides, if any.

use super::Intent;
use crate::config::{Fraction, Ms, Um};
use crate::sensor::lattice::units;
use palmrest_protocol::Axis;
use std::time::Duration;

/// Accumulated motion an axis needs before it can lock, per axis, in grid units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Threshold {
    pub x: i32,
    pub y: i32,
}

impl Threshold {
    /// The same number of grid units on both axes.
    pub const fn both(units: i32) -> Self {
        Self { x: units, y: units }
    }
}

/// Accumulated sideways motion before the X axis can lock.
pub(crate) const LOCK_THRESHOLD_X: Um = Um(1538);
/// Accumulated up or down motion before the Y axis can lock.
pub(crate) const LOCK_THRESHOLD_Y: Um = Um(571);
/// How much further one axis must go than the other to lock (1.5).
const LOCK_RATIO: Fraction = Fraction(1500);
/// A scroll's delta arriving this long after its previous motion unlocks it.
const LOCK_TIMEOUT: Ms = Ms(250);

/// The rule's tunables in the units the arithmetic uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub threshold: Threshold,
    pub ratio: f64,
    pub timeout: Duration,
}

impl Params {
    /// The shipped rule, with the P5 threshold multiplier applied while the mouse is moving.
    pub fn standard(threshold_multiplier: f64) -> Self {
        Self {
            threshold: Threshold {
                x: scaled(LOCK_THRESHOLD_X, Axis::X, threshold_multiplier),
                y: scaled(LOCK_THRESHOLD_Y, Axis::Y, threshold_multiplier),
            },
            ratio: LOCK_RATIO.ratio(),
            timeout: LOCK_TIMEOUT.duration(),
        }
    }
}

/// A threshold on the grid times the P5 multiplier, rounded as the reference feel rounds it.
fn scaled(um: Um, axis: Axis, multiplier: f64) -> i32 {
    (f64::from(units(um, axis)) * multiplier).round() as i32
}

/// The axis the accumulated motion `(x, y)` locks, X first and then Y.
fn axis_of((x, y): (i32, i32), p: &Params) -> Option<Axis> {
    let (ax, ay) = (f64::from(x.abs()), f64::from(y.abs()));
    let (tx, ty) = (
        f64::from(p.threshold.x.max(1)),
        f64::from(p.threshold.y.max(1)),
    );
    if ax >= tx && ax >= ay * p.ratio {
        Some(Axis::X)
    } else if ay >= ty && ay >= ax * p.ratio {
        Some(Axis::Y)
    } else {
        None
    }
}

/// The intent the accumulated motion decides for `fingers` contacts: sideways motion of one
/// finger scrolls, of two fingers swipes between Spaces; up and down scrolls.
pub fn intent_of(motion: (i32, i32), fingers: usize, p: &Params) -> Option<Intent> {
    axis_of(motion, p).map(|axis| match (axis, fingers) {
        (Axis::X, 2..) => Intent::SpaceSwipe,
        (axis, _) => Intent::Scroll(axis),
    })
}
