//! Acceptance 12.8 items 4-7, and the replay half of 10, with the shipped configuration
//! (rejection on, touchpad output): the remapped swipe's trigger, double-taps, one-finger swipe,
//! rejection, suppression. The live Spaces swipe on the touchpad is in `pad_strokes`.

use palmrest::action::{Action, Gesture};
use palmrest::config::Config;
use palmrest::keys::{Key, KeyCombo};
use palmrest::replay::{Timing, feed, outs, replay};
use palmrest::stamp::StampClock;
use palmrest::{Out, Tracker};
use palmrest_protocol::{Axis, LockState, Over, Phase, TapKind, ToHost, Workspace};

/// The shipped settings with both two-finger swipes mapped the other way round: not the
/// workspace defaults, so the swipe is not live on the touchpad and fires its action instead.
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

fn run(name: &str) -> Vec<Out> {
    outs(&replay(
        &crate::support::load(name),
        Config::default(),
        Timing::Arrival,
    ))
}

/// Replays with a host holding the pointer from the start.
fn run_claimed(name: &str) -> Vec<Out> {
    let (t, _) = Tracker::new(Config::default()).claim(Over::Ours);
    run_from(t, name)
}

/// Replays the remapped settings.
fn run_remapped(name: &str) -> Vec<Out> {
    outs(&replay(
        &crate::support::load(name),
        remapped(),
        Timing::Arrival,
    ))
}

/// Replays the remapped settings with the session locked from the start.
fn run_locked(name: &str) -> Vec<Out> {
    run_from(
        Tracker::new(remapped()).with_session_lock(LockState::Locked),
        name,
    )
}

fn run_from(mut t: Tracker, name: &str) -> Vec<Out> {
    let mut clock = StampClock::new();
    let mut all = Vec::new();
    for r in &crate::support::load(name) {
        let (nt, nc, step) = feed(t, clock, r, Timing::Arrival);
        t = nt;
        clock = nc;
        all.extend(step.outs);
    }
    all
}

fn actions(outs: &[Out]) -> Vec<Action> {
    outs.iter()
        .filter_map(|o| {
            if let Out::Deferred(a) = o {
                Some(a.clone())
            } else {
                None
            }
        })
        .collect()
}

fn keys(outs: &[Out]) -> Vec<KeyCombo> {
    outs.iter()
        .filter_map(|o| {
            if let Out::Key(k) = o {
                Some(k.clone())
            } else {
                None
            }
        })
        .collect()
}

fn taps(outs: &[Out]) -> Vec<TapKind> {
    crate::support::host(outs)
        .into_iter()
        .filter_map(|h| {
            if let ToHost::Tap { kind, .. } = h {
                Some(kind)
            } else {
                None
            }
        })
        .collect()
}

// ---- 4. Swipe trigger ---------------------------------------------------------------------

#[test]
fn swipe_trigger() {
    // Fingers right: the mapping of `remapped`, which is the other way round from the default.
    const CASES: &[(&str, &[Action])] = &[
        ("swipe_200_right", &[Action::Workspace(Workspace::Next)]),
        ("swipe_200_left", &[Action::Workspace(Workspace::Prev)]),
        ("swipe_199", &[]),
        (
            "swipe_vertical_then_horizontal",
            &[Action::Workspace(Workspace::Next)],
        ),
        ("swipe_600_fast", &[Action::Workspace(Workspace::Next)]),
    ];
    for &(name, want) in CASES {
        assert_eq!(actions(&run_remapped(name)), want, "{name}");
    }
}

#[test]
fn the_default_mapping_fires_nothing_because_the_swipe_is_live_on_the_pad() {
    for name in ["swipe_200_right", "swipe_200_left", "swipe_600_fast"] {
        assert!(actions(&run(name)).is_empty(), "{name}");
    }
}

#[test]
fn vertical_then_horizontal_fires_only_after_the_further_200() {
    let steps = replay(
        &crate::support::load("swipe_vertical_then_horizontal"),
        remapped(),
        Timing::Arrival,
    );
    let fired_at: Vec<u64> = steps
        .iter()
        .filter(|s| !actions(&s.outs).is_empty())
        .map(|s| s.at.0 / 1_000_000)
        .collect();
    // 160 vertical (to 240 ms) lock Y, and the sideways strokes after it do not count until the
    // lock times out, 250 ms after the last vertical motion (492 ms). The sideways motion then
    // locks at 552 ms (40 units) and its 200 units complete 20 reports later.
    assert_eq!(fired_at, [792]);
}

#[test]
fn a_two_finger_swipe_never_scrolls_sideways_and_lands_four_contacts() {
    let all = run("swipe_600_fast");
    assert!(
        crate::support::scrolls(&all)
            .iter()
            .all(|s| s.2 != Some(Axis::X))
    );
    let lands: Vec<_> = all
        .iter()
        .filter_map(|o| match o {
            Out::Pad(palmrest::outputs::pad::PadFrame::Land { fingers, .. }) => Some(*fingers),
            _ => None,
        })
        .collect();
    assert_eq!(lands, [4], "one swipe, four contacts, no scroll pair");
}

// ---- 5. Double-taps -----------------------------------------------------------------------

#[test]
fn two_finger_double_tap() {
    const CASES: &[(&str, &[Action])] = &[
        ("tap_two_double", &[Action::Workspace(Workspace::Overview)]),
        ("tap_two_gap400", &[]),
        ("tap_two_moving45", &[]),
        ("tap_two_click", &[]),
    ];
    for &(name, want) in CASES {
        assert_eq!(actions(&run(name)), want, "{name}");
    }
}

#[test]
fn one_finger_double_tap_zooms_only_under_a_claim() {
    let free = run("tap_one_double");
    assert!(
        taps(&free).is_empty() && actions(&free).is_empty() && keys(&free).is_empty(),
        "{free:?}"
    );
    assert_eq!(taps(&run_claimed("tap_one_double")), [TapKind::One]);
}

#[test]
fn taps_never_scroll() {
    for name in ["tap_two_double", "tap_one_double"] {
        let all = run(name);
        assert!(
            crate::support::scrolls(&all)
                .iter()
                .all(|s| matches!(s.0, Phase::MayBegin | Phase::Ended)),
            "{name}"
        );
        assert!(
            !all.iter().any(|o| matches!(o, Out::Pad(_))),
            "{name}: no touchpad contact for a tap"
        );
    }
}

// ---- 6. One-finger swipe ------------------------------------------------------------------

#[test]
fn one_finger_swipe() {
    let fast = run("g4_fast");
    assert_eq!(
        keys(&fast),
        [KeyCombo::alt(Key::LEFT)],
        "fingers moved right: Back"
    );
    assert!(
        crate::support::scrolled(&fast).0 > 0.0,
        "the horizontal scroll is still emitted"
    );
    let slow = run("g4_slow");
    assert!(keys(&slow).is_empty());
    assert!(crate::support::scrolled(&slow).0 > 0.0);
    assert!(
        keys(&run_claimed("g4_fast")).is_empty(),
        "no G4 keys while a host holds the pointer"
    );
}

#[test]
fn a_locked_session_runs_no_gesture_effect() {
    for name in ["swipe_200_right", "tap_two_double", "g4_fast"] {
        let free = run_remapped(name);
        assert!(
            !actions(&free).is_empty() || !keys(&free).is_empty(),
            "{name}: the unlocked run fires something"
        );
        let locked = run_locked(name);
        assert!(
            actions(&locked).is_empty() && keys(&locked).is_empty(),
            "{name}: {locked:?}"
        );
    }
    assert!(
        crate::support::scrolled(&run_locked("g4_fast")).0 > 0.0,
        "scrolling goes on"
    );
}

// ---- 7. Rejection -------------------------------------------------------------------------

#[test]
fn a_resting_side_contact_is_ignored() {
    let all = run("rest_side_band");
    let fingers = crate::support::scrolls(&all).iter().map(|s| s.1).max();
    assert_eq!(fingers, Some(1), "only the moving finger counts");
}

#[test]
fn a_click_cancels_the_scroll_and_holds_gestures_off() {
    let all = run("click_during_scroll");
    let phases: Vec<Phase> = crate::support::scrolls(&all).iter().map(|s| s.0).collect();
    let cancel = phases
        .iter()
        .position(|p| *p == Phase::Cancelled)
        .expect("cancelled at the press");
    assert!(
        phases[cancel + 1..].iter().all(|p| *p != Phase::Changed),
        "no scroll while the button is held: {phases:?}"
    );
    assert!(phases.iter().filter(|p| **p == Phase::Began).count() == 1);
    let pad: Vec<&Out> = all.iter().filter(|o| matches!(o, Out::Pad(_))).collect();
    assert!(matches!(
        pad.last(),
        Some(Out::Pad(palmrest::outputs::pad::PadFrame::Lift))
    ));
}

#[test]
fn a_palm_cancels_the_scroll() {
    let phases: Vec<Phase> = crate::support::scrolls(&run("palm_during_scroll"))
        .iter()
        .map(|s| s.0)
        .collect();
    assert!(phases.contains(&Phase::Cancelled), "{phases:?}");
    let after = &phases[phases.iter().position(|p| *p == Phase::Cancelled).unwrap() + 1..];
    assert!(!after.contains(&Phase::Changed));
}

#[test]
fn a_moving_mouse_needs_twice_the_travel_to_scroll() {
    assert!(
        crate::support::scrolls(&run("mouse_moving"))
            .iter()
            .all(|s| s.0 != Phase::Began)
    );
}

// ---- phase bookkeeping, over every fixture ------------------------------------------------

#[test]
fn every_may_begin_is_closed_exactly_once() {
    for (name, records) in crate::support::scenarios::all() {
        let all = outs(&replay(&records, Config::default(), Timing::Arrival));
        let mut open: Option<palmrest_protocol::GestureId> = None;
        for h in crate::support::host(&all) {
            if let ToHost::Scroll { gesture, phase, .. } = h {
                match phase {
                    Phase::MayBegin => {
                        assert_eq!(open, None, "{name}: MayBegin while {open:?} is open");
                        open = Some(gesture);
                    }
                    Phase::Began | Phase::Changed => assert_eq!(open, Some(gesture), "{name}"),
                    Phase::Ended | Phase::Cancelled => {
                        assert_eq!(
                            open,
                            Some(gesture),
                            "{name}: closing a gesture that is not open"
                        );
                        open = None;
                    }
                }
            }
        }
    }
}

#[test]
fn touches_down_and_up_alternate() {
    for (name, records) in crate::support::scenarios::all() {
        let all = outs(&replay(&records, Config::default(), Timing::Arrival));
        let t: Vec<_> = crate::support::host(&all)
            .into_iter()
            .filter_map(|h| {
                if let ToHost::Touches { active } = h {
                    Some(active)
                } else {
                    None
                }
            })
            .collect();
        assert!(t.windows(2).all(|w| w[0] != w[1]), "{name}: {t:?}");
    }
}

// ---- 10 (replay half). Suppression -------------------------------------------------------

#[test]
fn a_claimed_pointer_silences_every_foreign_output() {
    for name in ["g4_fast", "feel_fast", "click_during_scroll"] {
        let free = run(name);
        assert!(
            free.iter().any(|o| matches!(o, Out::Pad(_))),
            "{name}: the touchpad is used when nobody claims"
        );
        let claimed = run_claimed(name);
        assert!(
            !claimed
                .iter()
                .any(|o| matches!(o, Out::Pad(_) | Out::Key(_))),
            "{name}"
        );
        assert_eq!(
            crate::support::scrolls(&claimed),
            crate::support::scrolls(&free),
            "{name}: the socket stream is unchanged"
        );
    }
}

#[test]
fn the_pad_lifts_once_when_the_scroll_ends_and_the_app_runs_the_glide() {
    let all = run("flick_two_vertical");
    let ended = all
        .iter()
        .rposition(|o| {
            matches!(
                o,
                Out::Host(ToHost::Scroll {
                    phase: Phase::Ended,
                    axis: Some(_),
                    ..
                })
            )
        })
        .expect("a locked scroll ends");
    let pad: Vec<&Out> = all[ended..]
        .iter()
        .filter(|o| matches!(o, Out::Pad(_)))
        .collect();
    assert_eq!(pad, [&Out::Pad(palmrest::outputs::pad::PadFrame::Lift)]);
}

#[test]
fn claiming_mid_stroke_lifts_the_touchpad() {
    let records = crate::support::load("g4_slow");
    let (mut t, mut clock) = (Tracker::new(Config::default()), StampClock::new());
    for r in &records[..31] {
        let (nt, nc, _) = feed(t, clock, r, Timing::Arrival);
        (t, clock) = (nt, nc);
    }
    let (_, lifted) = t.claim(Over::Ours);
    assert_eq!(lifted, [Out::Pad(palmrest::outputs::pad::PadFrame::Lift)]);
}
