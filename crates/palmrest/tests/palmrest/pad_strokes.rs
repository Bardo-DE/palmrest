//! The virtual touchpad as the tracker drives it: strokes of known millimetres in, pad frames
//! out. What apps see through libinput is these frames, so the properties are the ones libinput
//! needs: the fingers move as far as the real ones, land where a stroke fits, and never land in
//! a zone libinput ignores.

use crate::support::Script;
use crate::support::mm::{drive, feed_all, outs_of, stroke};
use palmrest::action::{Action, Gesture};
use palmrest::config::Config;
use palmrest::outputs::events::{
    BTN_TOOL_DOUBLETAP, BTN_TOOL_FINGER, BTN_TOOL_QUADTAP, BTN_TOOL_TRIPLETAP, EV_KEY, PadEncoder,
};
use palmrest::outputs::pad::{PadFrame, UNITS_PER_MM};
use palmrest::{Mono, Out, Tracker};
use palmrest_protocol::{Axis, LockState, Over, ToHost, Workspace};

fn pad(outs: &[Out]) -> Vec<PadFrame> {
    outs.iter()
        .filter_map(|o| match o {
            Out::Pad(f) => Some(*f),
            _ => None,
        })
        .collect()
}

fn mm(units: i32) -> f64 {
    f64::from(units) / f64::from(UNITS_PER_MM)
}

/// Every landing: how many contacts, where the centre is (mm) and the x of each contact (mm).
fn landings(frames: &[PadFrame]) -> Vec<(u8, (f64, f64), Vec<f64>)> {
    frames
        .iter()
        .filter_map(|f| match *f {
            PadFrame::Land {
                fingers,
                at,
                spacing,
            } => {
                let n = i32::from(fingers);
                let xs = (0..n)
                    .map(|i| mm(at.x + (2 * i - (n - 1)) * spacing / 2))
                    .collect();
                Some((fingers, (mm(at.x), mm(at.y)), xs))
            }
            _ => None,
        })
        .collect()
}

fn moves(frames: &[PadFrame]) -> Vec<(f64, f64)> {
    frames
        .iter()
        .filter_map(|f| match f {
            PadFrame::Move { at } => Some((mm(at.x), mm(at.y))),
            _ => None,
        })
        .collect()
}

fn lifts(frames: &[PadFrame]) -> usize {
    frames.iter().filter(|f| **f == PadFrame::Lift).count()
}

/// Drives a fresh tracker with an unlocked session through `script`.
fn run(cfg: Config, script: &Script) -> Vec<Out> {
    let tracker = Tracker::new(cfg).with_session_lock(LockState::Unlocked);
    outs_of(&drive(tracker, &script.records))
}

/// The shipped settings with both two-finger swipes mapped the other way round: not the
/// workspace defaults, so the swipe fires its action instead of running live.
fn remapped() -> Config {
    let mut cfg = Config::default();
    cfg.gestures.gesture_action_map = cfg
        .gestures
        .gesture_action_map
        .with(
            Gesture::TwoFingerSwipeRight,
            Action::Workspace(Workspace::Next),
        )
        .with(
            Gesture::TwoFingerSwipeLeft,
            Action::Workspace(Workspace::Prev),
        );
    cfg
}

// ---- scrolling ----------------------------------------------------------------------------

#[test]
fn a_45_mm_vertical_stroke_lands_once_and_moves_the_pad_as_far_as_the_fingers() {
    // (fingers, reports, ms apart): slow and fast, one finger and two.
    const CASES: &[(u8, usize, f64)] = &[(1, 90, 30.0), (2, 90, 30.0), (1, 45, 8.0), (2, 45, 8.0)];
    let travelled: Vec<f64> = CASES
        .iter()
        .map(|&(fingers, steps, ms)| {
            let frames = pad(&run(
                Config::default(),
                &stroke(fingers, Axis::Y, 45.0, steps, ms),
            ));
            let lands = landings(&frames);
            assert_eq!(lands.len(), 1, "{fingers} fingers, {ms} ms: one landing");
            assert_eq!(lands[0].0, 2, "a scroll lands two contacts");
            assert_eq!(lifts(&frames), 1, "and lifts once, at the end");
            assert_eq!(frames.last(), Some(&PadFrame::Lift));
            let (_, land_y) = lands[0].1;
            let last = moves(&frames).last().copied().expect("the stroke moves");
            last.1 - land_y
        })
        .collect();
    // 45 mm less the 1 mm the lock drops: 1:1, whatever the speed (no acceleration curve).
    for t in &travelled {
        assert!((t - 44.0).abs() < 0.1, "{travelled:?}");
    }
}

#[test]
fn a_stroke_up_lands_low_and_a_stroke_across_lands_at_the_side_it_comes_from() {
    let up = pad(&run(
        Config::default(),
        &stroke(2, Axis::Y, -40.0, 40, 12.0),
    ));
    let (_, (x, y), _) = landings(&up)[0].clone();
    assert_eq!(
        (x, y),
        (50.0, 55.0),
        "moving up: as low as a landing may be"
    );
    assert!(moves(&up).last().unwrap().1 < y - 38.0);

    let right = pad(&run(Config::default(), &stroke(1, Axis::X, 30.0, 30, 12.0)));
    let (fingers, (x, _), xs) = landings(&right)[0].clone();
    assert_eq!((fingers, x), (2, 20.0), "moving right: near the left");
    assert!(xs.iter().all(|x| *x >= 10.0));
    let left = pad(&run(
        Config::default(),
        &stroke(1, Axis::X, -30.0, 30, 12.0),
    ));
    let (_, (x, _), _) = landings(&left)[0].clone();
    assert_eq!(x, 80.0, "moving left: near the right");
}

#[test]
fn a_claim_silences_the_scroll_pad_but_not_the_shell_stream() {
    let script = stroke(2, Axis::Y, 30.0, 40, 12.0);
    let (claimed, _) = Tracker::new(Config::default()).claim(Over::Ours);
    let outs = outs_of(&drive(claimed, &script.records));
    assert!(pad(&outs).is_empty());
    assert!(
        outs.iter().any(|o| matches!(
            o,
            Out::Host(ToHost::Scroll { dy_px, .. }) if *dy_px != 0.0
        )),
        "the shell still scrolls"
    );
}

// ---- the live Spaces swipe ----------------------------------------------------------------

#[test]
fn a_two_finger_horizontal_stroke_is_a_four_contact_swipe_that_follows_x_one_to_one() {
    let script = stroke(2, Axis::X, 45.0, 45, 12.0);
    let frames = pad(&run(Config::default(), &script));
    let lands = landings(&frames);
    assert_eq!(lands.len(), 1, "one landing, never a re-land: {frames:?}");
    let (fingers, (x0, y0), _) = lands[0].clone();
    assert_eq!(fingers, 4);
    assert_eq!(frames.last(), Some(&PadFrame::Lift));
    assert_eq!(lifts(&frames), 1);
    let path = moves(&frames);
    assert!(
        path.iter().all(|&(_, y)| y == y0),
        "the pad never moves vertically"
    );
    assert!(
        path.windows(2).all(|w| w[1].0 >= w[0].0),
        "following the fingers right"
    );
    // 45 mm less the 2 mm the lock drops.
    let travelled = path.last().unwrap().0 - x0;
    assert!((travelled - 43.0).abs() < 0.1, "{travelled}");
}

#[test]
fn the_swipe_ignores_vertical_motion_and_stops_at_the_edge_instead_of_landing_again() {
    // 90 mm across the mouse is more than the pad has room for from the left landing.
    let script = stroke(2, Axis::X, 90.0, 90, 12.0);
    let frames = pad(&run(Config::default(), &script));
    assert_eq!(landings(&frames).len(), 1);
    let (_, (_, y0), _) = landings(&frames)[0].clone();
    let path = moves(&frames);
    let end = path.last().unwrap();
    assert_eq!(
        end.0, 77.0,
        "the centre stops 5 mm short of the edge, 18 mm of row on each side"
    );
    assert!(path.iter().all(|&(_, y)| y == y0));
}

#[test]
fn the_swipe_runs_under_a_claim_and_a_claim_mid_swipe_does_not_lift_it() {
    let script = stroke(2, Axis::X, 45.0, 45, 12.0);
    let (claimed, _) = Tracker::new(Config::default()).claim(Over::Ours);
    let under_claim = pad(&outs_of(&drive(claimed, &script.records)));
    let free = pad(&run(Config::default(), &script));
    assert_eq!(
        under_claim, free,
        "the swipe is the compositor's, whoever holds the pointer"
    );

    let (half, rest) = script.records.split_at(25);
    let (t, before) = feed_all(Tracker::new(Config::default()), half);
    let (t, on_claim) = t.claim(Over::Ours);
    assert!(on_claim.is_empty(), "a claim does not lift a swipe");
    let (t, on_release) = t.claim(Over::NotOurs);
    assert!(on_release.is_empty());
    let after = drive(t, rest);
    let all = [outs_of(&before), outs_of(&after)].concat();
    assert_eq!(pad(&all), free);
}

#[test]
fn a_locked_session_has_no_swipe() {
    let script = stroke(2, Axis::X, 45.0, 45, 12.0);
    let locked = Tracker::new(Config::default()).with_session_lock(LockState::Locked);
    let outs = outs_of(&drive(locked, &script.records));
    assert!(pad(&outs).is_empty());
    assert!(
        outs.iter()
            .any(|o| matches!(o, Out::Host(ToHost::Scroll { .. }))),
        "the shell stream goes on"
    );
}

#[test]
fn a_session_that_locks_mid_swipe_lifts_it() {
    let script = stroke(2, Axis::X, 45.0, 45, 12.0);
    let (half, _) = script.records.split_at(25);
    let (t, before) = feed_all(Tracker::new(Config::default()), half);
    assert!(
        pad(&outs_of(&before))
            .iter()
            .any(|f| matches!(f, PadFrame::Land { .. }))
    );
    let (_, after) = feed_all(
        t.with_session_lock(LockState::Locked),
        &script.records[25..26],
    );
    assert_eq!(pad(&outs_of(&after)), [PadFrame::Lift]);
}

#[test]
fn a_remapped_swipe_fires_once_per_threshold_with_the_cooldown_and_never_touches_the_pad() {
    let script = stroke(2, Axis::X, 90.0, 90, 12.0);
    let timed = drive(
        Tracker::new(remapped()).with_session_lock(LockState::Unlocked),
        &script.records,
    );
    assert!(
        pad(&outs_of(&timed)).is_empty(),
        "a fired action is not a live swipe"
    );
    let fired: Vec<Mono> = timed
        .iter()
        .filter(|(_, o)| *o == Out::Deferred(Action::Workspace(Workspace::Next)))
        .map(|(at, _)| *at)
        .collect();
    assert!(
        fired.len() >= 2,
        "a long drag goes on after the cooldown: {fired:?}"
    );
    assert!(
        fired.windows(2).all(|w| w[1].0 - w[0].0 >= 500_000_000),
        "at least the cooldown apart: {fired:?}"
    );
    assert!(
        fired.len() <= 4,
        "once per 200 units of travel, not per report"
    );
}

// ---- both together ------------------------------------------------------------------------

#[test]
fn a_scroll_never_lands_four_contacts_and_a_swipe_never_lands_two() {
    let scrolls = [
        stroke(1, Axis::Y, 30.0, 30, 12.0),
        stroke(2, Axis::Y, -30.0, 30, 12.0),
        stroke(1, Axis::X, 30.0, 30, 12.0),
        stroke(1, Axis::X, -30.0, 30, 12.0),
    ];
    let swipes = [
        stroke(2, Axis::X, 30.0, 30, 12.0),
        stroke(2, Axis::X, -30.0, 30, 12.0),
    ];
    for s in &scrolls {
        let lands = landings(&pad(&run(Config::default(), s)));
        assert!(
            !lands.is_empty() && lands.iter().all(|l| l.0 == 2),
            "{lands:?}"
        );
    }
    for s in &swipes {
        let lands = landings(&pad(&run(Config::default(), s)));
        assert!(
            !lands.is_empty() && lands.iter().all(|l| l.0 == 4),
            "{lands:?}"
        );
    }
}

#[test]
fn every_landing_and_move_keeps_clear_of_the_zones_libinput_ignores() {
    let strokes = [
        stroke(1, Axis::Y, 45.0, 45, 12.0),
        stroke(2, Axis::Y, -40.0, 40, 12.0),
        stroke(1, Axis::X, 50.0, 50, 12.0),
        stroke(2, Axis::X, -50.0, 50, 12.0),
        stroke(2, Axis::X, 90.0, 90, 12.0),
    ];
    for s in &strokes {
        let frames = pad(&run(Config::default(), s));
        for (_, (_, y), xs) in landings(&frames) {
            assert!(
                xs.iter().all(|x| *x >= 10.0 && *x <= 90.0),
                "side zones: {xs:?}"
            );
            assert!(y <= 55.0, "bottom thumb zone: {y}");
        }
        for f in &frames {
            if let PadFrame::Move { at } = f {
                assert!(mm(at.y) >= 5.0 && mm(at.y) <= 65.0, "{at:?}");
                assert!(mm(at.x) >= 5.0 && mm(at.x) <= 95.0, "{at:?}");
            }
        }
        // The outer contacts of a swipe stay 5 mm clear of the edge while moving.
        if let Some(PadFrame::Land { fingers: 4, .. }) = frames.first() {
            assert!(
                moves(&frames)
                    .iter()
                    .all(|&(x, _)| (23.0..=77.0).contains(&x))
            );
        }
    }
}

#[test]
fn the_tool_changes_only_after_every_contact_lifted() {
    // A scroll, then a swipe 100 ms after: the pair is still waiting out its hold.
    let scroll = stroke(2, Axis::Y, 5.0, 4, 12.0);
    let end = scroll.end_ms();
    let swipe = stroke(2, Axis::X, 30.0, 30, 12.0);
    let shifted = swipe.records.iter().map(|r| palmrest::recording::Record {
        at: Mono(r.at.0 - 1_000_000_000 + ((end + 100.0) * 1e6) as u64),
        bytes: r.bytes.clone(),
    });
    let records: Vec<_> = scroll.records.iter().cloned().chain(shifted).collect();
    let frames = pad(&outs_of(&drive(Tracker::new(Config::default()), &records)));
    let tools: Vec<u16> = frames
        .iter()
        .scan(PadEncoder::default(), |enc, f| {
            let (next, events) = enc.encode(f);
            *enc = next;
            Some(events)
        })
        .flatten()
        .filter(|e| e.kind == EV_KEY && is_tool(e.code))
        .map(|e| if e.value == 1 { e.code } else { 0 })
        .collect();
    assert!(
        tools.contains(&BTN_TOOL_DOUBLETAP) && tools.contains(&BTN_TOOL_QUADTAP),
        "{tools:?}"
    );
    // On, off, on, off: a tool is never switched on while another is still on.
    assert!(
        tools.chunks(2).all(|c| c[0] != 0 && c.get(1) == Some(&0)),
        "{tools:?}"
    );
}

fn is_tool(code: u16) -> bool {
    [
        BTN_TOOL_FINGER,
        BTN_TOOL_DOUBLETAP,
        BTN_TOOL_TRIPLETAP,
        BTN_TOOL_QUADTAP,
    ]
    .contains(&code)
}
