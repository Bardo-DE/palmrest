//! Millimetres to the distance the virtual touchpad's fingers move, for every client outside
//! the shell. libinput reads that pad as a real two-finger touchpad and owns the scroll feel
//! (its scaling, its acceleration, its momentum), so the fingers move the same physical
//! distance as the real fingers: 1:1 in millimetres, with no acceleration curve.
//!
//! The one exception is the `scroll.speed` setting, a flat multiplier on that distance. The
//! default speed is exactly 1.0, and every 20 steps double it, so each step changes the
//! distance by the same proportion:
//!
//! ```text
//! multiplier(speed) = 2 ^ ((speed - 22) / 20)        speed in 0..=63
//! ```
//!
//! so speed 0 is 0.47, 22 is 1.0, 42 is 2.0 and 63 is 4.1.

use crate::config::Count;
use crate::sensor::units::Mm;

/// The speed that is exactly 1:1 (the default of `scroll.speed`).
const REFERENCE_SPEED: u16 = 22;
/// Speed steps that double the distance.
const STEPS_PER_DOUBLING: f64 = 20.0;

/// The multiplier `scroll.speed` makes on the pad's finger distance.
pub fn multiplier(speed: Count) -> f64 {
    let steps = f64::from(speed.0) - f64::from(REFERENCE_SPEED);
    (steps / STEPS_PER_DOUBLING).exp2()
}

/// The pad distance for `finger` millimetres of finger travel.
pub fn travel(finger: Mm, speed: Count) -> Mm {
    Mm(finger.0 * multiplier(speed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn the_default_speed_is_one_to_one() {
        let speed = Config::default().scroll.speed;
        assert_eq!(multiplier(speed), 1.0);
        assert_eq!(travel(Mm(12.5), speed), Mm(12.5));
        assert_eq!(travel(Mm(-3.0), speed), Mm(-3.0));
    }

    #[test]
    fn the_multiplier_grows_with_the_speed_without_a_jump() {
        let all: Vec<f64> = (0..=63).map(|s| multiplier(Count(s))).collect();
        assert!(all.windows(2).all(|w| w[1] > w[0]), "{all:?}");
        assert!(
            all.windows(2).all(|w| w[1] / w[0] < 2.0),
            "no jumps: {all:?}"
        );
        assert!((multiplier(Count(42)) - 2.0).abs() < 1e-12);
        assert!(all[0] > 0.4 && all[63] < 4.5, "{all:?}");
    }
}
