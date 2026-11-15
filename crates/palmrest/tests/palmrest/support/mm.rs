//! Strokes of a known length in millimetres, and the tracker driven through them.

use super::{Script, finger};
use palmrest::recording::Record;
use palmrest::replay::{Timing, feed};
use palmrest::stamp::StampClock;
use palmrest::{Mono, Out, Tracker};
use palmrest_protocol::Axis;

/// Grid units per millimetre across and along the Magic Mouse 2.
const X_PER_MM: f64 = 26.0;
const Y_PER_MM: f64 = 70.0;
/// Fingers of one stroke are this far apart across the mouse, in grid units.
const FINGER_GAP: i32 = 400;

/// `fingers` fingers moving `mm` along `axis` (negative: the other way) in `steps` reports
/// `step_ms` apart from t = 1000 ms, then lifting. Every stroke starts where its first contact
/// is neither in a side band nor in the rear band, and stays in 12-bit range.
pub fn stroke(fingers: u8, axis: Axis, mm: f64, steps: usize, step_ms: f64) -> Script {
    let (per_mm, start) = match (axis, mm > 0.0) {
        (Axis::X, true) => (X_PER_MM, -850),
        (Axis::X, false) => (X_PER_MM, 200),
        (Axis::Y, true) => (Y_PER_MM, -1550),
        (Axis::Y, false) => (Y_PER_MM, 1400),
    };
    let travel = |i: usize| (mm * per_mm * i as f64 / steps as f64).round() as i32;
    let touches = move |i: usize| {
        (0..fingers)
            .map(|k| {
                let k = i32::from(k);
                match axis {
                    Axis::X => finger(k as u8 + 1, start + FINGER_GAP * k + travel(i), 0),
                    Axis::Y => finger(k as u8 + 1, -200 + FINGER_GAP * k, start + travel(i)),
                }
            })
            .collect()
    };
    Script::default()
        .frames(1000.0, step_ms, steps, touches)
        .lift(1000.0 + step_ms * (steps as f64 + 1.0))
}

/// Feeds `records` through `tracker`, ticking it at every deadline before each record, as the
/// reader thread does; the outputs with the instant of the report or tick that caused them.
pub fn feed_all(tracker: Tracker, records: &[Record]) -> (Tracker, Vec<(Mono, Out)>) {
    records.iter().fold((tracker, vec![]), |(t, all), r| {
        let (t, ticked) = tick_until(t, Some(r.at));
        let (t, _, step) = feed(t, StampClock::new(), r, Timing::Arrival);
        let all = all
            .into_iter()
            .chain(ticked)
            .chain(step.outs.into_iter().map(|o| (r.at, o)));
        (t, all.collect())
    })
}

/// `feed_all`, then ticks until nothing is left to do: the delayed lifts too.
pub fn drive(tracker: Tracker, records: &[Record]) -> Vec<(Mono, Out)> {
    let (tracker, fed) = feed_all(tracker, records);
    let (_, rest) = tick_until(tracker, None);
    fed.into_iter().chain(rest).collect()
}

fn tick_until(tracker: Tracker, until: Option<Mono>) -> (Tracker, Vec<(Mono, Out)>) {
    match tracker.deadline().filter(|d| until.is_none_or(|u| *d < u)) {
        None => (tracker, vec![]),
        Some(at) => {
            let (tracker, outs) = tracker.tick(at);
            let (tracker, rest) = tick_until(tracker, until);
            (
                tracker,
                outs.into_iter().map(|o| (at, o)).chain(rest).collect(),
            )
        }
    }
}

/// The outputs without their instants.
pub fn outs_of(timed: &[(Mono, Out)]) -> Vec<Out> {
    timed.iter().map(|(_, o)| o.clone()).collect()
}
