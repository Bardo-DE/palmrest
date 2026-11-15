//! The recogniser over short report sequences.

use super::*;
use crate::touch::{Point, TouchId};
use std::time::Duration;

const PARAMS: Params = Params {
    threshold: Threshold::both(40),
    ratio: 1.5,
    timeout: Duration::from_millis(250),
};

fn sample(id: u8, x: i32, y: i32) -> Sample {
    Sample {
        id: TouchId(id),
        at: Point::new(x, y),
    }
}

fn one(x: i32, y: i32) -> Vec<Sample> {
    vec![sample(1, x, y)]
}

fn two(x: i32, y: i32) -> Vec<Sample> {
    vec![sample(1, x - 300, y), sample(2, x + 300, y)]
}

fn run(rec: Recogniser, frames: &[(u64, Vec<Sample>)]) -> (Recogniser, Vec<Event>) {
    frames
        .iter()
        .fold((rec, vec![]), |(rec, events), (ms, contacts)| {
            let (rec, more) = rec.step(contacts, Mono::from_millis(*ms), &PARAMS, Hold::Open);
            (rec, events.into_iter().chain(more).collect())
        })
}

fn phases(events: &[Event]) -> Vec<Phase> {
    events.iter().map(|e| e.phase).collect()
}

#[test]
fn locking_drops_the_pending_motion_and_hands_it_on_as_the_lead() {
    let frames: Vec<_> = (0..10)
        .map(|i| (10 + i * 8, one(0, i as i32 * 10)))
        .collect();
    let (rec, events) = run(Recogniser::default(), &frames);
    assert_eq!(rec.intent(), Some(Intent::Scroll(Axis::Y)));
    // Frames 1..=4 accumulate 40 and lock; frames 5..=9 scroll 10 each.
    assert_eq!(
        phases(&events),
        [vec![Phase::MayBegin, Phase::Began], vec![Phase::Changed; 5]].concat()
    );
    assert_eq!(events[1].motion.y, lattice::mm(40, Axis::Y), "the lead");
    for e in &events[2..] {
        assert_eq!(e.motion.y, lattice::mm(10, Axis::Y));
        assert_eq!(e.motion.x, Mm(0.0), "a scroll follows its axis only");
        assert_eq!(e.intent, Some(Intent::Scroll(Axis::Y)));
    }
}

#[test]
fn two_fingers_sideways_are_a_space_swipe_and_two_fingers_up_and_down_scroll() {
    let sideways: Vec<_> = (0..12).map(|i| (i * 8, two(i as i32 * 10, 0))).collect();
    let (rec, events) = run(Recogniser::default(), &sideways);
    assert_eq!(rec.intent(), Some(Intent::SpaceSwipe));
    let began = events.iter().find(|e| e.phase == Phase::Began).unwrap();
    assert_eq!((began.intent, began.fingers), (Some(Intent::SpaceSwipe), 2));
    assert!(began.motion.x.0 > 0.0, "the lead says which way it starts");
    let changed: Vec<_> = events
        .iter()
        .filter(|e| e.phase == Phase::Changed)
        .collect();
    assert!(!changed.is_empty());
    assert!(changed.iter().all(|e| e.motion.x.0 > 0.0));

    let down: Vec<_> = (0..12).map(|i| (i * 8, two(0, i as i32 * 10))).collect();
    let (rec, _) = run(Recogniser::default(), &down);
    assert_eq!(rec.intent(), Some(Intent::Scroll(Axis::Y)));
}

#[test]
fn one_finger_sideways_scrolls() {
    let frames: Vec<_> = (0..12).map(|i| (i * 8, one(i as i32 * 10, 0))).collect();
    let (rec, _) = run(Recogniser::default(), &frames);
    assert_eq!(rec.intent(), Some(Intent::Scroll(Axis::X)));
}

#[test]
fn a_pause_unlocks_on_the_next_delta() {
    let frames = vec![
        (0, one(0, 0)),
        (8, one(0, 50)),
        (16, one(0, 60)),
        (400, one(0, 70)),
    ];
    let (rec, events) = run(Recogniser::default(), &frames);
    assert_eq!(rec.intent(), None);
    let tail: Vec<Phase> = phases(&events).into_iter().rev().take(2).collect();
    assert_eq!(
        tail,
        [Phase::MayBegin, Phase::Ended],
        "the old stroke closed, a new one opened"
    );
}

#[test]
fn zero_delta_does_not_refresh_motion_time() {
    let frames = vec![
        (0, one(0, 0)),
        (8, one(0, 50)),
        (200, one(0, 50)),
        (300, one(0, 60)),
    ];
    let (_, events) = run(Recogniser::default(), &frames);
    assert_eq!(phases(&events).last(), Some(&Phase::MayBegin));
}

#[test]
fn a_held_click_accumulates_nothing() {
    let (rec, _) = Recogniser::default().step(&one(0, 0), Mono(0), &PARAMS, Hold::Open);
    let (rec, events) = rec.step(&one(0, 100), Mono::from_millis(8), &PARAMS, Hold::Held);
    assert!(events.is_empty());
    assert_eq!(rec.intent(), None);
    let (_, events) = rec.step(&one(0, 110), Mono::from_millis(16), &PARAMS, Hold::Open);
    assert!(events.is_empty(), "only the 10 units after the hold count");
}

#[test]
fn three_contacts_end_the_stroke_once() {
    let three: Vec<Sample> = (1..=3).map(|i| sample(i, 0, 0)).collect();
    let frames = vec![(0, one(0, 0)), (8, three.clone()), (16, three)];
    let (_, events) = run(Recogniser::default(), &frames);
    assert_eq!(phases(&events), [Phase::MayBegin, Phase::Ended]);
}

#[test]
fn every_may_begin_is_closed_exactly_once() {
    let strokes: Vec<(u64, Vec<Sample>)> = (0..12)
        .map(|i| (i * 8, one(0, i as i32 * 10)))
        .chain([(100, vec![])])
        .chain((0..12).map(|i| (200 + i * 8, two(i as i32 * 10, 0))))
        .chain([(400, vec![])])
        .collect();
    let (_, events) = run(Recogniser::default(), &strokes);
    let opens = events.iter().filter(|e| e.phase == Phase::MayBegin).count();
    let closes = events
        .iter()
        .filter(|e| matches!(e.phase, Phase::Ended | Phase::Cancelled))
        .count();
    assert_eq!((opens, closes), (2, 2));
}

#[test]
fn a_cancel_keeps_the_fingers_and_the_next_decision_opens_a_new_gesture() {
    let down: Vec<_> = (0..12).map(|i| (i * 8, one(0, i as i32 * 10))).collect();
    let (rec, events) = run(Recogniser::default(), &down);
    let first = events[0].gesture;
    let (rec, cancelled) = rec.cancel();
    assert_eq!(phases(&cancelled), [Phase::Cancelled]);
    assert_eq!(cancelled[0].gesture, first);
    assert_eq!(rec.intent(), None);
    let (_, after) = rec.cancel();
    assert!(after.is_empty(), "a quiet stroke has nothing to cancel");

    let more: Vec<_> = (12..24).map(|i| (i * 8, one(0, i as i32 * 10))).collect();
    let (rec, _) = run(Recogniser::default(), &down);
    let (rec, _) = rec.cancel();
    let (_, events) = run(rec, &more);
    assert_eq!(phases(&events)[..2], [Phase::MayBegin, Phase::Began]);
    assert_ne!(events[0].gesture, first);
}

#[test]
fn a_disconnect_cancels_and_forgets() {
    let down: Vec<_> = (0..12).map(|i| (i * 8, one(0, i as i32 * 10))).collect();
    let (rec, _) = run(Recogniser::default(), &down);
    let (rec, events) = rec.disconnect();
    assert_eq!(phases(&events), [Phase::Cancelled]);
    assert_eq!(rec.intent(), None);
    let (_, after) = rec.step(&[], Mono::from_millis(200), &PARAMS, Hold::Open);
    assert!(after.is_empty(), "nothing is open to close");
}
