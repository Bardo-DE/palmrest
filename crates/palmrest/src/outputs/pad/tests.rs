//! The pad session over short sequences: where it lands, how it follows, when it lifts.

use super::*;
use crate::sensor::units::Mm;
use palmrest_protocol::{Axis, LockState, Over};

const DOWN: Intent = Intent::Scroll(Axis::Y);
const UP_RIGHT: Intent = Intent::Scroll(Axis::X);
const SWIPE: Intent = Intent::SpaceSwipe;

fn lead(x: f64, y: f64) -> Motion {
    Motion { x: Mm(x), y: Mm(y) }
}

/// The x of every contact of a landing frame, in millimetres, and its y.
fn contacts(f: &PadFrame) -> (Vec<f64>, f64) {
    let PadFrame::Land {
        fingers,
        at,
        spacing,
    } = *f
    else {
        panic!("not a landing: {f:?}");
    };
    let n = i32::from(fingers);
    let xs = (0..n)
        .map(|i| f64::from(at.x + (2 * i - (n - 1)) * spacing / 2) / f64::from(UNITS_PER_MM))
        .collect();
    (xs, f64::from(at.y) / f64::from(UNITS_PER_MM))
}

fn landed(intent: Intent, lead: Motion) -> PadFrame {
    Pad::Up.land(intent, lead, Mono(0)).1[0]
}

#[test]
fn a_scroll_lands_two_contacts_20_mm_apart_opposite_the_motion() {
    let (xs, y) = contacts(&landed(DOWN, lead(0.0, 1.0)));
    assert_eq!(xs, [40.0, 60.0]);
    assert_eq!(y, 5.0, "moving down: near the top");
    let (_, y) = contacts(&landed(DOWN, lead(0.0, -1.0)));
    assert_eq!(y, 55.0, "moving up: as low as it may land");
    let (xs, _) = contacts(&landed(UP_RIGHT, lead(1.0, 0.0)));
    assert_eq!(xs, [10.0, 30.0], "moving right: near the left, 10 mm clear");
    let (xs, _) = contacts(&landed(UP_RIGHT, lead(-1.0, 0.0)));
    assert_eq!(xs, [70.0, 90.0]);
}

#[test]
fn a_swipe_lands_four_contacts_12_mm_apart_opposite_the_motion() {
    let (xs, y) = contacts(&landed(SWIPE, lead(1.0, 0.3)));
    assert_eq!(xs, [10.0, 22.0, 34.0, 46.0]);
    assert_eq!(y, 35.0);
    let (xs, _) = contacts(&landed(SWIPE, lead(-1.0, 0.0)));
    assert_eq!(xs, [54.0, 66.0, 78.0, 90.0]);
}

#[test]
fn every_landing_keeps_clear_of_libinputs_zones() {
    for intent in [DOWN, UP_RIGHT, SWIPE] {
        for lead in [
            lead(1.0, 1.0),
            lead(-1.0, 1.0),
            lead(1.0, -1.0),
            lead(-1.0, -1.0),
        ] {
            let (xs, y) = contacts(&landed(intent, lead));
            assert!(xs.iter().all(|x| (10.0..=90.0).contains(x)), "{xs:?}");
            assert!((5.0..=55.0).contains(&y), "{y}");
        }
    }
}

#[test]
fn a_scroll_follows_one_to_one_and_a_stroke_fits_without_landing_again() {
    let (mut pad, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    for i in 1..=45 {
        let (next, frames) = pad.moved(DOWN, lead(0.0, 1.0), Mono::from_millis(i * 10));
        pad = next;
        assert!(matches!(frames[..], [PadFrame::Move { .. }]), "{frames:?}");
    }
    let (_, frames) = pad.end(Mono::from_millis(500));
    assert_eq!(frames, [PadFrame::Lift]);
    let (_, frames) = pad.moved(DOWN, lead(0.0, 1.0), Mono::from_millis(460));
    assert_eq!(
        frames,
        [PadFrame::Move {
            at: PadPoint {
                x: 2000,
                y: 5 * 40 + 46 * 40
            }
        }],
        "46 mm from the top: 1 mm of finger is 1 mm of pad"
    );
}

#[test]
fn a_scroll_out_of_room_lands_again_on_the_opposite_side() {
    let (pad, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (_, frames) = pad.moved(DOWN, lead(0.0, 61.0), Mono::from_millis(300));
    let relanded = contacts(&frames[1]);
    assert_eq!(frames[0], PadFrame::Lift);
    assert_eq!(relanded.1, 5.0, "still moving down: back near the top");
}

/// Measured 2026-11-15: at the end of a flick the fingers eased back, the contacts that had
/// just landed again ran out of room at once and lifted without moving, and libinput read a
/// two-finger tap (a right click). Young contacts stop at the edge instead.
#[test]
fn young_contacts_never_lift_to_land_again() {
    let (pad, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (pad, frames) = pad.moved(DOWN, lead(0.0, -3.0), Mono::from_millis(20));
    assert_eq!(
        frames,
        [],
        "already at the top edge: nothing moves and nothing lifts"
    );
    let (pad, frames) = pad.moved(DOWN, lead(0.0, 2.0), Mono::from_millis(30));
    assert_eq!(
        frames,
        [PadFrame::Move {
            at: PadPoint { x: 2000, y: 280 }
        }]
    );
    let (_, frames) = pad.moved(DOWN, lead(0.0, -9.0), Mono::from_millis(250));
    assert_eq!(
        frames[0],
        PadFrame::Lift,
        "past the tap timeout it lands again as before"
    );
}

#[test]
fn a_swipe_follows_x_only_and_stops_at_the_edge() {
    let row = |x_mm: f64| PadFrame::Move {
        at: PadPoint {
            x: (x_mm * 40.0) as i32,
            y: 35 * 40,
        },
    };
    let (pad, _) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    let (pad, frames) = pad.moved(SWIPE, lead(10.0, 7.0), Mono::from_millis(50));
    assert_eq!(
        frames,
        [row(33.0)],
        "x moved (halfway, smoothed), y did not"
    );
    let (pad, frames) = pad.moved(SWIPE, lead(500.0, 0.0), Mono::from_millis(60));
    assert_eq!(
        frames,
        [row(57.5)],
        "the fingers stop 5 mm from the edge (77 mm) and nothing lands again"
    );
    assert!(matches!(pad, Pad::Down(_)));
    let (_, frames) = pad.end(Mono::from_millis(70));
    assert_eq!(
        frames,
        [row(77.0), PadFrame::Lift],
        "the row catches up with the fingers before it lifts"
    );
}

#[test]
fn a_swipe_halves_a_lone_double_step_and_keeps_its_whole_travel() {
    // The Magic Mouse's steady 2 mm steps with one 4 mm step among them.
    let steps = [2.0, 2.0, 4.0, 2.0, 2.0];
    let (pad, _) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    let (pad, xs) = steps.iter().fold((pad, vec![28.0]), |(pad, mut xs), &dx| {
        let (pad, frames) = pad.moved(SWIPE, lead(dx, 0.0), Mono::from_millis(10));
        xs.push(contact_x(&frames[0]));
        (pad, xs)
    });
    let shown: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
    let biggest = shown.iter().copied().fold(0.0, f64::max);
    assert!(biggest <= 3.0, "the 4 mm step is spread out: {shown:?}");
    let (_, frames) = pad.end(Mono::from_millis(70));
    assert_eq!(
        contact_x(&frames[0]),
        28.0 + 12.0,
        "the whole 12 mm arrives"
    );
}

#[test]
fn a_short_scroll_holds_its_lift_past_the_tap_timeout() {
    for intent in [DOWN, UP_RIGHT] {
        let (pad, _) = Pad::Up.land(intent, lead(1.0, 1.0), Mono(0));
        let (pad, frames) = pad.end(Mono::from_millis(80));
        assert!(frames.is_empty());
        assert_eq!(pad.deadline(), Some(Mono::from_millis(200)));
        assert_eq!(pad.tick(Mono::from_millis(199)).1, vec![]);
        assert_eq!(
            pad.tick(Mono::from_millis(200)),
            (Pad::Up, vec![PadFrame::Lift])
        );
    }
}

/// Measured 2026-11-15: a 50 ms flick held still for the rest of `MIN_DOWN` gave the app no
/// speed to glide with. Contacts that moved past the tap slop can no longer tap, so a flick lifts
/// the moment it ends, still moving; a reverse step that runs out of room lands again at once.
#[test]
fn a_flick_that_moved_lifts_the_moment_it_ends() {
    let (pad, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (pad, _) = pad.moved(DOWN, lead(0.0, 1.5), Mono::from_millis(11));
    let (pad, _) = pad.moved(DOWN, lead(0.0, 1.5), Mono::from_millis(22));
    let (pad, _) = pad.moved(DOWN, lead(0.0, 3.0), Mono::from_millis(33));
    let (_, frames) = pad.end(Mono::from_millis(50));
    assert_eq!(frames, [PadFrame::Lift]);
    let (_, frames) = pad.moved(DOWN, lead(0.0, -20.0), Mono::from_millis(44));
    assert_eq!(frames[0], PadFrame::Lift, "6 mm moved: it may land again");
}

/// 2 mm or less from the landing could still be a tap to libinput: the lift waits.
#[test]
fn a_scroll_inside_the_tap_slop_still_waits() {
    let (pad, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (pad, _) = pad.moved(DOWN, lead(0.0, 1.0), Mono::from_millis(11));
    let (pad, _) = pad.moved(DOWN, lead(0.0, 1.0), Mono::from_millis(22));
    let (pad, frames) = pad.end(Mono::from_millis(50));
    assert_eq!(frames, []);
    assert_eq!(pad.deadline(), Some(Mono::from_millis(200)));
}

#[test]
fn a_swipe_lifts_the_moment_it_ends_however_young_it_is() {
    let (pad, _) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    let (pad, frames) = pad.end(Mono::from_millis(20));
    assert_eq!((pad, frames), (Pad::Up, vec![PadFrame::Lift]));
}

#[test]
fn a_new_landing_lifts_a_pending_one_first_so_the_tool_count_never_changes_in_place() {
    let (pad, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (pad, _) = pad.end(Mono::from_millis(10));
    let (_, frames) = pad.land(SWIPE, lead(1.0, 0.0), Mono::from_millis(20));
    assert_eq!(frames[0], PadFrame::Lift);
    assert!(matches!(frames[1], PadFrame::Land { fingers: 4, .. }));
}

/// Everything allowed, then the policy with one thing taken away.
const OPEN: Policy = Policy {
    claimed: Over::NotOurs,
    session: LockState::Unlocked,
    live_swipe: true,
};

#[test]
fn the_policy_lifts_what_it_no_longer_allows_and_nothing_else() {
    let claimed = Policy {
        claimed: Over::Ours,
        ..OPEN
    };
    let locked = Policy {
        session: LockState::Locked,
        ..OPEN
    };
    let late = Mono::from_millis(300);
    let (scroll, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (swipe, _) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    assert_eq!(
        scroll.enforce(claimed, late),
        (Pad::Up, vec![PadFrame::Lift])
    );
    assert_eq!(
        swipe.enforce(claimed, late),
        (swipe, vec![]),
        "a claim leaves a swipe"
    );
    assert_eq!(swipe.enforce(locked, late), (Pad::Up, vec![PadFrame::Lift]));
    assert_eq!(
        scroll.enforce(locked, late),
        (scroll, vec![]),
        "a lock leaves a scroll"
    );
    assert_eq!(scroll.enforce(OPEN, late), (scroll, vec![]));
    assert_eq!(Pad::Up.enforce(locked, late), (Pad::Up, vec![]));
}

/// A claim on a scroll younger than the tap timeout keeps its contacts still until then: lifted
/// at once they would be a two-finger tap, a right click.
#[test]
fn a_claim_on_a_young_scroll_lifts_it_at_the_tap_timeout() {
    let claimed = Policy {
        claimed: Over::Ours,
        ..OPEN
    };
    let (scroll, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (pad, frames) = scroll.enforce(claimed, Mono::from_millis(50));
    assert_eq!(frames, []);
    assert_eq!(pad.deadline(), Some(Mono::from_millis(200)));
    let (pad, frames) = pad.enforce(claimed, Mono::from_millis(60));
    assert_eq!(frames, [], "a pending lift stays pending");
    let (_, frames) = pad.moved(DOWN, lead(0.0, 5.0), Mono::from_millis(70));
    assert_eq!(frames, [], "and nothing moves it");
    assert_eq!(
        pad.tick(Mono::from_millis(200)),
        (Pad::Up, vec![PadFrame::Lift])
    );
}

/// A scroll that starts while the last one's young contacts wait to lift takes them over, so
/// they never go up as a tap; a swipe needs four contacts and lands its own.
#[test]
fn a_scroll_over_a_pending_lift_takes_its_contacts_over() {
    let (pad, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (pad, _) = pad.moved(DOWN, lead(0.0, 2.0), Mono::from_millis(20));
    let (pending, _) = pad.end(Mono::from_millis(40));
    let (pad, frames) = pending.land(UP_RIGHT, lead(-1.0, 0.0), Mono::from_millis(60));
    assert_eq!(frames, []);
    let (_, frames) = pad.moved(UP_RIGHT, lead(-3.0, 0.0), Mono::from_millis(70));
    assert_eq!(
        frames,
        [PadFrame::Move {
            at: PadPoint { x: 1880, y: 280 }
        }],
        "from where they were (50 mm, 7 mm), 3 mm to the left"
    );
}

#[test]
fn a_swipe_needs_the_live_mapping_and_an_unlocked_session_and_a_scroll_a_free_pointer() {
    let remapped = Policy {
        live_swipe: false,
        ..OPEN
    };
    assert!(OPEN.allows(SWIPE) && OPEN.allows(DOWN));
    assert!(!remapped.allows(SWIPE) && remapped.allows(DOWN));
    let claimed = Policy {
        claimed: Over::Ours,
        ..OPEN
    };
    assert!(!claimed.allows(DOWN) && !claimed.allows(UP_RIGHT) && claimed.allows(SWIPE));
}

#[test]
fn a_cancelled_swipe_returns_to_where_it_landed_before_it_lifts() {
    let (pad, landed) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    let PadFrame::Land { at: start, .. } = landed[0] else {
        panic!("not a landing: {landed:?}");
    };
    let (pad, _) = pad.moved(SWIPE, lead(20.0, 0.0), Mono::from_millis(50));
    let (pad, frames) = pad.cancel(Mono::from_millis(60));
    assert_eq!(
        frames,
        [PadFrame::Move { at: start }, PadFrame::Lift],
        "back to the landing x in one frame, then up"
    );
    assert_eq!(pad, Pad::Up);
}

#[test]
fn a_cancelled_swipe_that_never_moved_just_lifts_and_a_cancelled_scroll_ends_as_usual() {
    let (swipe, _) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    assert_eq!(
        swipe.cancel(Mono::from_millis(5)),
        (Pad::Up, vec![PadFrame::Lift])
    );
    let (scroll, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (moved, _) = scroll.moved(DOWN, lead(0.0, 5.0), Mono::from_millis(10));
    assert_eq!(
        moved.cancel(Mono::from_millis(20)),
        moved.end(Mono::from_millis(20))
    );
    assert_eq!(Pad::Up.cancel(Mono(0)), (Pad::Up, vec![]));
}

#[test]
fn only_the_intent_that_is_down_moves() {
    let (swipe, _) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    assert_eq!(swipe.moved(DOWN, lead(0.0, 1.0), Mono(1)), (swipe, vec![]));
    assert_eq!(
        Pad::Up.moved(DOWN, lead(0.0, 1.0), Mono(1)),
        (Pad::Up, vec![])
    );
}

/// The row's centre x in millimetres, from a Move frame.
fn contact_x(frame: &PadFrame) -> f64 {
    match frame {
        PadFrame::Move { at } => f64::from(at.x) / 40.0,
        other => panic!("not a move: {other:?}"),
    }
}
