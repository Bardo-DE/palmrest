//! The lifecycle as pure steps: states and events in, the next state and effects out.

use super::super::model::{Cuts, Effect, Event, Life, Why};
use super::super::step::step;
use crate::daemon::discover::Backoff;
use crate::time::Mono;
use std::time::Duration;

const ZERO: Cuts = Cuts(0);

fn cut() -> Why {
    Why::Cut("the service closed the connection".into())
}

fn other() -> Why {
    Why::Other("no acl yet".into())
}

fn waiting(retry_at: Mono, backoff: Backoff, cuts: Cuts) -> Life {
    Life::Waiting {
        retry_at,
        backoff,
        cuts,
    }
}

/// The effects that are not log lines, in order.
fn doings(effects: &[Effect]) -> Vec<&Effect> {
    effects
        .iter()
        .filter(|e| !matches!(e, Effect::Log(_)))
        .collect()
}

fn says(effects: &[Effect], words: &str) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Log(line) if line.contains(words)))
}

#[test]
fn the_first_build_is_due_at_once_and_waits_for_its_result() {
    let (life, effects) = step(Life::start(Mono(5)), Event::Due(Mono(5)));
    assert_eq!(life, Life::start(Mono(5)));
    assert_eq!(effects, [Effect::Create { at: Mono(5) }]);
}

#[test]
fn a_retry_is_not_tried_before_its_time() {
    let life = waiting(Mono::from_millis(1000), Backoff::UINPUT, ZERO);
    let (next, effects) = step(life, Event::Due(Mono::from_millis(999)));
    assert_eq!((next, effects), (life, vec![]));
}

#[test]
fn a_build_that_works_keeps_the_cuts_until_a_frame_goes_through() {
    let life = waiting(Mono(0), Backoff::UINPUT, Cuts(2));
    let (ready, _) = step(life, Event::Created);
    assert_eq!(ready, Life::Ready { cuts: Cuts(2) });
    let (ready, effects) = step(ready, Event::Written);
    assert_eq!((ready, effects), (Life::Ready { cuts: ZERO }, vec![]));
}

#[test]
fn a_failed_build_waits_the_next_backoff_delay() {
    let (second, backoff) = Backoff::UINPUT.step();
    let at = Mono::from_millis(10);
    let (life, effects) = step(
        waiting(Mono(0), Backoff::UINPUT, ZERO),
        Event::CreateFailed { why: other(), at },
    );
    assert_eq!(life, waiting(at.after(second), backoff, ZERO));
    assert!(says(&effects, "not ready: no acl yet; retrying in 1s"));
    assert!(doings(&effects).is_empty());
}

#[test]
fn a_failed_write_drops_the_emitter_and_reports_the_loss() {
    let (life, effects) = step(Life::Ready { cuts: ZERO }, Event::WriteFailed(other()));
    assert_eq!(life, Life::Lost { cuts: ZERO });
    assert_eq!(effects, [Effect::DropEmitter, Effect::Lost]);
}

#[test]
fn only_cuts_count_towards_giving_up() {
    // One cut is the loss itself and three rebuilds follow; the fourth in a row ends it.
    let mut life = Life::Ready { cuts: ZERO };
    for n in 1..=3u8 {
        let (next, effects) = step(life, Event::WriteFailed(cut()));
        assert_eq!(next, Life::Lost { cuts: Cuts(n) });
        assert!(doings(&effects).contains(&&Effect::Lost));
        life = Life::Ready { cuts: Cuts(n) };
    }
    let (next, effects) = step(life, Event::WriteFailed(cut()));
    assert_eq!(next, Life::Dead);
    assert!(says(&effects, "virtual input stopped"));
    assert_eq!(doings(&effects), [&Effect::DropEmitter, &Effect::Lost]);

    let (next, _) = step(Life::Ready { cuts: Cuts(3) }, Event::WriteFailed(other()));
    assert_eq!(
        next,
        Life::Lost { cuts: Cuts(3) },
        "other failures never give up"
    );
}

#[test]
fn a_build_cut_for_the_fourth_time_gives_up_without_a_loss() {
    let (life, effects) = step(
        waiting(Mono(0), Backoff::UINPUT, Cuts(3)),
        Event::CreateFailed {
            why: cut(),
            at: Mono(1),
        },
    );
    assert_eq!(life, Life::Dead);
    assert!(says(&effects, "virtual input stopped"));
    assert!(doings(&effects).is_empty());
}

#[test]
fn a_loss_starts_the_wait_at_the_next_poll() {
    let now = Mono::from_millis(50);
    let (delay, backoff) = Backoff::UINPUT.step();
    let (life, effects) = step(Life::Lost { cuts: Cuts(1) }, Event::Due(now));
    assert_eq!(life, waiting(now.after(delay), backoff, Cuts(1)));
    assert!(says(&effects, "lost; rebuilding in 1s"));
    assert!(doings(&effects).is_empty(), "not built before the delay");
    assert_eq!(delay, Duration::from_secs(1));
}

#[test]
fn a_change_of_shape_restarts_from_any_state_and_builds_at_once() {
    let now = Mono::from_millis(7);
    let states = [
        Life::Ready { cuts: Cuts(2) },
        Life::Lost { cuts: ZERO },
        waiting(Mono::from_millis(9_999), Backoff::UINPUT, Cuts(3)),
        Life::Dead,
    ];
    for life in states {
        let (next, effects) = step(life, Event::ShapeChanged(now));
        assert_eq!(next, Life::start(now), "{life:?}");
        assert_eq!(
            effects,
            [Effect::DropEmitter, Effect::Create { at: now }],
            "{life:?}"
        );
    }
}

#[test]
fn a_dead_pad_ignores_everything_but_a_change_of_shape() {
    for event in [
        Event::Due(Mono::from_millis(99_999)),
        Event::Written,
        Event::WriteFailed(cut()),
    ] {
        assert_eq!(step(Life::Dead, event), (Life::Dead, vec![]));
    }
}
