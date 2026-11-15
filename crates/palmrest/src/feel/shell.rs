//! Millimetres to px for the shell's own surfaces (`ToHost::Scroll`): the acceleration curve,
//! the `scroll.speed` base ratio and the wheel detent.
//!
//! Computed on the grid's integer units so the values are those of the reference feel; the
//! operation order is fixed, because a reordered float product moves a result by one ulp.

use super::curve::sample_gain;
use crate::config::{Count, Px};
use crate::sensor::lattice::grid;
use crate::sensor::units::Mm;
use palmrest_protocol::Axis;

/// Raw units per detent are `(64 - speed) * ACCEL_BASE`.
const ACCEL_BASE: u32 = 7;
/// Hi-res wheel units per detent (Linux `REL_WHEEL_HI_RES`).
const HI_RES_PER_DETENT: f64 = 120.0;
/// The smallest raw-per-hi-res step, so an extreme speed cannot divide by nearly zero.
const RAW_PER_HI_RES_MIN: f64 = 0.05;
/// Pixels scrolled by one wheel step at the base ratio.
const DETENT_PX: Px = Px(60);

/// Raw units per hi-res unit at this speed and gain.
fn raw_per_hi_res(speed: Count, gain: f64) -> f64 {
    let num = f64::from((64 - u32::from(speed.0.min(63))) * ACCEL_BASE);
    (num / (HI_RES_PER_DETENT * gain)).max(RAW_PER_HI_RES_MIN)
}

/// The part of a sample's hi-res units that the stroke has already counted: the exact stream
/// carries the raw remainder between samples, valued at the rate it was carried at, so a change of
/// rate never drops or repeats a fraction of a unit. It starts empty at every decision.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Carry {
    raw_remainder: f64,
    carried_units: f64,
}

/// The px, in finger direction, for `mm` of finger travel along `axis`, `dt_s` seconds after
/// the previous motion: the speed's base ratio times the curve's gain at the finger's speed.
pub fn px(carry: Carry, mm: Mm, axis: Axis, speed: Count, dt_s: f64) -> (Carry, f64) {
    let raw = grid(mm, axis);
    let per_hi_res = raw_per_hi_res(speed, sample_gain(raw, dt_s));
    let raw_remainder = carry.raw_remainder + f64::from(raw);
    let whole = (raw_remainder / per_hi_res).trunc();
    let raw_remainder = raw_remainder - whole * per_hi_res;
    let carried_units = raw_remainder / per_hi_res;
    let exact_units = whole + carried_units - carry.carried_units;
    let carry = Carry {
        raw_remainder,
        carried_units,
    };
    (
        carry,
        exact_units * f64::from(DETENT_PX.0) / HI_RES_PER_DETENT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::lattice::mm;

    fn px_of(carry: Carry, units: i32, dt_s: f64) -> (Carry, f64) {
        px(carry, mm(units, Axis::Y), Axis::Y, Count(22), dt_s)
    }

    #[test]
    fn base_ratio_is_sixty_px_per_294_units() {
        assert!((raw_per_hi_res(Count(22), 1.0) - 294.0 / 120.0).abs() < 1e-12);
        assert!((px_of(Carry::default(), 294, 0.0).1 - 60.0).abs() < 1e-9);
        assert!((px_of(Carry::default(), 1, 0.0).1 - 0.2041).abs() < 1e-4);
        assert_eq!(raw_per_hi_res(Count(63), 4.6), RAW_PER_HI_RES_MIN);
    }

    #[test]
    fn the_remainder_carries_between_samples() {
        // 5 raw units at 2.45 raw per hi-res unit is 2 whole units and 0.04 of the next; the
        // stream is exact all the same.
        let rate = raw_per_hi_res(Count(22), 1.0);
        let (carry, a) = px_of(Carry::default(), 5, 0.0);
        let (_, b) = px_of(carry, 1, 0.0);
        let per_unit = 60.0 / 120.0;
        assert!((a - 5.0 / rate * per_unit).abs() < 1e-12);
        assert!((b - 1.0 / rate * per_unit).abs() < 1e-12);
    }

    #[test]
    fn a_fast_sample_scrolls_further_than_a_slow_one() {
        let slow = px_of(Carry::default(), 60, 0.1).1;
        let fast = px_of(Carry::default(), 60, 0.008).1;
        assert!(fast > slow * 2.0, "{fast} against {slow}");
    }

    #[test]
    fn the_direction_of_the_finger_is_the_sign_of_the_px() {
        let x = |units| {
            px(
                Carry::default(),
                mm(units, Axis::X),
                Axis::X,
                Count(22),
                0.01,
            )
            .1
        };
        assert!(x(-12) < 0.0);
        assert!(x(12) > 0.0);
    }
}
