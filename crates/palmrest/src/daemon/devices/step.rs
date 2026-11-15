//! The lifecycle's transitions: `(Life, Event) -> (Life, effects)`, with the backoff and the cuts.

use super::model::{Cuts, Effect, Event, Life, Why};
use crate::daemon::discover::Backoff;
use crate::time::Mono;
use std::iter::once;

pub(super) fn step(life: Life, event: Event) -> (Life, Vec<Effect>) {
    match (life, event) {
        (Life::Ready { .. }, Event::Written) => (
            Life::Ready {
                cuts: Cuts::default(),
            },
            vec![],
        ),
        (Life::Ready { cuts }, Event::WriteFailed(why)) => lose(cuts, &why),
        (Life::Lost { cuts }, Event::Due(now)) => begin_wait(cuts, now),
        (Life::Waiting { retry_at, .. }, Event::Due(now)) if now >= retry_at => {
            (life, vec![Effect::Create { at: now }])
        }
        (Life::Waiting { cuts, .. }, Event::Created) => (Life::Ready { cuts }, vec![]),
        (Life::Waiting { backoff, cuts, .. }, Event::CreateFailed { why, at }) => {
            fail_build(&why, at, backoff, cuts)
        }
        (_, Event::ShapeChanged(now)) => restart(now),
        (life, _) => (life, vec![]),
    }
}

/// A write failed: the devices are dropped, and rebuilt unless the service has cut palmrest off
/// too often.
fn lose(cuts: Cuts, why: &Why) -> (Life, Vec<Effect>) {
    let cuts = why.tally(cuts);
    if cuts.spent() {
        (
            Life::Dead,
            vec![Effect::DropEmitter, give_up(why), Effect::Lost],
        )
    } else {
        (Life::Lost { cuts }, vec![Effect::DropEmitter, Effect::Lost])
    }
}

/// The wait after a loss. A rebuilt device is not tried before the first backoff delay, so a
/// device that is created but cannot be written is rebuilt once a second at most.
fn begin_wait(cuts: Cuts, now: Mono) -> (Life, Vec<Effect>) {
    let (delay, backoff) = Backoff::UINPUT.step();
    let waiting = Life::Waiting {
        retry_at: now.after(delay),
        backoff,
        cuts,
    };
    let (life, rest) = step(waiting, Event::Due(now));
    let said = Effect::Log(format!(
        "virtual input lost; rebuilding in {}s",
        delay.as_secs()
    ));
    (life, once(said).chain(rest).collect())
}

/// A rebuild failed: wait and try again, unless the service has cut palmrest off too often.
fn fail_build(why: &Why, at: Mono, backoff: Backoff, cuts: Cuts) -> (Life, Vec<Effect>) {
    let cuts = why.tally(cuts);
    if cuts.spent() {
        return (Life::Dead, vec![give_up(why)]);
    }
    let (delay, backoff) = backoff.step();
    let said = Effect::Log(format!(
        "virtual input not ready: {}; retrying in {}s",
        why.text(),
        delay.as_secs()
    ));
    let waiting = Life::Waiting {
        retry_at: at.after(delay),
        backoff,
        cuts,
    };
    (waiting, vec![said])
}

/// The settings changed what the devices must be: drop the old ones and build at once.
fn restart(now: Mono) -> (Life, Vec<Effect>) {
    let (life, rest) = step(Life::start(now), Event::Due(now));
    (life, once(Effect::DropEmitter).chain(rest).collect())
}

fn give_up(why: &Why) -> Effect {
    Effect::Log(format!(
        "virtual input stopped: {}. It starts again after a change of settings or a restart of \
         palmrest.",
        why.text()
    ))
}

/// The first build of a run: due at once.
pub(super) fn first(now: Mono) -> (Life, Vec<Effect>) {
    step(Life::start(now), Event::Due(now))
}
