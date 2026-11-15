//! The pad session over short sequences: where it lands, how it follows, when it lifts.

use super::*;
use crate::sensor::units::Mm;
use palmrest_protocol::Axis;

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

#[test]
fn a_swipe_follows_x_only_and_stops_at_the_edge() {
    let (pad, _) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    let (pad, frames) = pad.moved(SWIPE, lead(10.0, 7.0), Mono::from_millis(50));
    let at = PadPoint {
        x: (28.0 + 10.0) as i32 * 40,
        y: 35 * 40,
    };
    assert_eq!(frames, [PadFrame::Move { at }], "x moved, y did not");
    let (pad, frames) = pad.moved(SWIPE, lead(500.0, 0.0), Mono::from_millis(60));
    assert_eq!(
        frames,
        [PadFrame::Move {
            at: PadPoint {
                x: 77 * 40,
                y: 35 * 40
            }
        }],
        "the outer contact stops 5 mm from the edge and nothing lands again"
    );
    assert!(matches!(pad, Pad::Down(_)));
}

#[test]
fn a_short_stroke_holds_its_lift_past_the_tap_timeout() {
    for intent in [DOWN, SWIPE] {
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

#[test]
fn a_new_landing_lifts_a_pending_one_first_so_the_tool_count_never_changes_in_place() {
    let (pad, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    let (pad, _) = pad.end(Mono::from_millis(10));
    let (_, frames) = pad.land(SWIPE, lead(1.0, 0.0), Mono::from_millis(20));
    assert_eq!(frames[0], PadFrame::Lift);
    assert!(matches!(frames[1], PadFrame::Land { fingers: 4, .. }));
}

#[test]
fn a_claim_lifts_a_scroll_and_leaves_a_swipe() {
    let (scroll, _) = Pad::Up.land(DOWN, lead(0.0, 1.0), Mono(0));
    assert_eq!(scroll.lift_scroll(), (Pad::Up, vec![PadFrame::Lift]));
    let (swipe, _) = Pad::Up.land(SWIPE, lead(1.0, 0.0), Mono(0));
    assert_eq!(swipe.lift_scroll(), (swipe, vec![]));
    assert_eq!(swipe.lift_swipe(), (Pad::Up, vec![PadFrame::Lift]));
    assert_eq!(scroll.lift_swipe(), (scroll, vec![]));
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
