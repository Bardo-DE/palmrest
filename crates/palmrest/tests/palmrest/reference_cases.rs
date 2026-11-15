//! Acceptance 12.8 item 3: the reference gesture cases, against the stroke recogniser and the
//! discrete swipe with the case's own parameters. The cases and their expected values are frozen
//! from the retired Python daemon's test suite; points are in its unsigned reading.

use palmrest::Mono;
use palmrest::action::{Action, Gesture, plan};
use palmrest::config::{Config, Count};
use palmrest::feel::shell::{self, Carry};
use palmrest::gestures::stroke::Threshold;
use palmrest::gestures::{
    FingerDirection, Hold, Intent, Params, Recogniser, Sample, StrokeEvent, SwipeFire,
};
use palmrest_protocol::{Axis, HostAction, Phase, Workspace};
use std::time::Duration;

// ---- the case helpers ---------------------------------------------------------------------

/// `touch(x, y, id)` in the unsigned reading, as a sample in the kernel frame.
fn touch(x: i32, y: i32, id: u8) -> Sample {
    let t = crate::support::unsigned_finger(id, x, y);
    Sample { id: t.id, at: t.at }
}

/// `ScrollAxisLock(speed=32, lock_threshold=40)`.
const PARAMS: Params = Params {
    threshold: Threshold::both(40),
    ratio: 1.5,
    timeout: Duration::from_millis(250),
};

/// Every phase the recogniser reports for the reports `(ms, fingers)`, from a fresh one.
fn recognise(frames: &[(u64, Vec<Sample>)]) -> (Recogniser, Vec<(Mono, StrokeEvent)>) {
    frames.iter().fold(
        (Recogniser::default(), vec![]),
        |(rec, all), (ms, fingers)| {
            let now = Mono::from_millis(*ms);
            let (rec, events) = rec.step(fingers, now, &PARAMS, Hold::Open);
            let all = all.into_iter().chain(events.into_iter().map(|e| (now, e)));
            (rec, all.collect())
        },
    )
}

fn timed(
    n: i32,
    start_ms: u64,
    step_ms: u64,
    f: impl Fn(i32) -> Vec<Sample>,
) -> Vec<(u64, Vec<Sample>)> {
    (0..n)
        .map(|i| (start_ms + step_ms * i as u64, f(i)))
        .collect()
}

/// The summed scroll in millimetres, horizontal and vertical.
fn scrolled(events: &[(Mono, StrokeEvent)]) -> (f64, f64) {
    events
        .iter()
        .filter(|(_, e)| e.phase == Phase::Changed && matches!(e.intent, Some(Intent::Scroll(_))))
        .fold((0.0, 0.0), |(h, v), (_, e)| {
            (h + e.motion.x.0, v + e.motion.y.0)
        })
}

// ---- ScrollAxisLock ---------------------------------------------------------------------

/// What a drag must do: scroll one axis only, or swipe between Spaces.
#[derive(Clone, Copy, Debug)]
enum Does {
    ScrollsVertically,
    ScrollsHorizontally,
    SwipesSpaces,
}

#[test]
fn the_axis_lock_decides_the_dominant_axis_only() {
    // (name, the fingers at step i, what the drag does)
    type Case = (&'static str, fn(i32) -> Vec<Sample>, Does);
    const AXIS_CASES: &[Case] = &[
        (
            "one finger vertical",
            |i| vec![touch(1000, 1000 + i * 6, 1)],
            Does::ScrollsVertically,
        ),
        (
            "one finger horizontal",
            |i| vec![touch(1000 + i * 6, 1000, 1)],
            Does::ScrollsHorizontally,
        ),
        (
            "two fingers horizontal",
            |i| vec![touch(900 + i * 6, 1000, 1), touch(1300 + i * 6, 1000, 2)],
            Does::SwipesSpaces,
        ),
        (
            "two fingers vertical",
            |i| vec![touch(900, 1000 + i * 6, 1), touch(1300, 1000 + i * 6, 2)],
            Does::ScrollsVertically,
        ),
    ];
    for &(name, fingers, want) in AXIS_CASES {
        let (rec, events) = recognise(&timed(40, 10_000, 8, fingers));
        let (h, v) = scrolled(&events);
        let intent = rec.intent();
        let held = match want {
            Does::ScrollsVertically => v != 0.0 && h == 0.0,
            Does::ScrollsHorizontally => h != 0.0 && v == 0.0,
            Does::SwipesSpaces => (h, v) == (0.0, 0.0) && intent == Some(Intent::SpaceSwipe),
        };
        assert!(held, "{name}: {want:?}, got ({h}, {v}) and {intent:?}");
    }
}

#[test]
fn three_fingers_do_nothing() {
    let (rec, events) = recognise(&timed(40, 10_000, 8, |i| {
        (1..=3)
            .map(|k| touch(500 + 400 * i32::from(k), 1000 + i * 6, k))
            .collect()
    }));
    assert_eq!(scrolled(&events), (0.0, 0.0));
    assert_eq!(rec.intent(), None);
}

#[test]
fn finger_count_change_does_not_jump() {
    let mut frames = timed(20, 10_000, 8, |i| vec![touch(1000, 1000 + i * 6, 1)]);
    frames.push((20_000, vec![touch(1000, 1120, 1), touch(3000, 3000, 2)]));
    let (_, events) = recognise(&frames);
    assert!(
        events
            .iter()
            .filter(|(at, _)| *at == Mono::from_millis(20_000))
            .all(|(_, e)| e.phase != Phase::Changed),
        "a new contact set moves nothing"
    );
}

#[test]
fn scroll_steps_are_finer_than_a_detent_tenth() {
    let (_, events) = recognise(&timed(40, 10_000, 8, |i| {
        vec![touch(1000, 1000 + i * 6, 1)]
    }));
    let steps: Vec<f64> = events
        .iter()
        .filter(|(_, e)| e.phase == Phase::Changed)
        .scan(Carry::default(), |carry, (_, e)| {
            let (next, px) = shell::px(*carry, e.motion.y, Axis::Y, Count(32), e.dt_s);
            *carry = next;
            Some(px)
        })
        .filter(|px| *px != 0.0)
        .collect();
    assert!(!steps.is_empty(), "expected high-resolution scroll");
    assert!(
        steps.iter().any(|v| v.abs() < 6.0),
        "a step below 6 px (a tenth of a 60 px detent)"
    );
}

// ---- DesktopSwipe -------------------------------------------------------------------------

/// The directions the discrete swipe fires over `frames`.
fn swipe_run(frames: &[(u64, Vec<Sample>)]) -> Vec<FingerDirection> {
    let (_, events) = recognise(frames);
    events
        .iter()
        .scan(SwipeFire::default(), |swipe, (now, e)| {
            let (next, fired) = swipe.step(e, *now);
            *swipe = next;
            Some(fired)
        })
        .flatten()
        .collect()
}

#[test]
fn two_finger_swipe_right_fires_once_and_the_workspace_follows_the_fingers() {
    let mut frames = timed(40, 10_000, 8, |i| {
        vec![touch(900 + i * 10, 1000, 1), touch(1300 + i * 10, 1000, 2)]
    });
    frames.push((11_000, vec![]));
    assert_eq!(swipe_run(&frames), [FingerDirection::Right]);
    // Fingers right: the Space on the left; fingers left: the one on the right. The mapping
    // does not depend on the desktop: the shell is asked for the step.
    let map = Config::default().gestures.gesture_action_map;
    assert_eq!(
        plan(map.action_for(Gesture::TwoFingerSwipeRight)),
        Some(HostAction::Workspace(Workspace::Prev))
    );
    assert_eq!(
        plan(map.action_for(Gesture::TwoFingerSwipeLeft)),
        Some(HostAction::Workspace(Workspace::Next))
    );
    assert_eq!(
        plan(&Action::Command(vec!["a".to_owned()])),
        Some(HostAction::Command {
            argv: vec!["a".to_owned()]
        })
    );
}

#[test]
fn long_two_finger_drag_fires_again_after_cooldown() {
    let frames = timed(120, 10_000, 10, |i| {
        vec![touch(500 + i * 10, 1000, 1), touch(900 + i * 10, 1000, 2)]
    });
    assert_eq!(
        swipe_run(&frames),
        [FingerDirection::Right, FingerDirection::Right]
    );
}

#[test]
fn these_drags_do_not_fire_a_swipe() {
    // (name, the fingers at step i)
    type Case = (&'static str, fn(i32) -> Vec<Sample>);
    const NO_SWITCH_CASES: &[Case] = &[
        ("two-finger vertical drag", |i| {
            vec![
                touch(900 + i * 3, 1000 + i * 10, 1),
                touch(1300 + i * 3, 1000 + i * 10, 2),
            ]
        }),
        ("one finger never swipes", |i| {
            vec![touch(500 + i * 20, 1000, 1)]
        }),
    ];
    for &(name, fingers) in NO_SWITCH_CASES {
        assert!(
            swipe_run(&timed(60, 10_000, 8, fingers)).is_empty(),
            "{name}: the unit fired"
        );
    }
}
