//! The socket's tests. The decisions are pure and tested as steps over events (`step`, `packet`,
//! `aggregate`); `wiring` and `passes` keep the few tests that need real sockets, to prove the thread feeds
//! the events and carries out the effects.

mod aggregate;
mod packet;
mod passes;
mod step;
mod support;
mod wiring;

use super::FromSocket;
use super::model::{ClientId, SocketEffect, SocketEvent, Subscribers};
use latchkey::guard::Claim;
use palmrest_protocol::ToHost;
use std::time::Instant;

/// The person at the seat in the pure tests.
const USER: u32 = 1000;
const SHELL: Claim = Claim::Shell { user: USER };
const OTHER: Claim = Claim::Other;

fn id(n: u64) -> ClientId {
    ClientId(n)
}

/// `events` fed in order at `now`: the model after them and every effect they asked for.
fn feed(
    subs: Subscribers,
    events: impl IntoIterator<Item = SocketEvent>,
    now: Instant,
) -> (Subscribers, Vec<SocketEffect>) {
    events
        .into_iter()
        .fold((subs, Vec::new()), |(subs, mut asked), event| {
            let (next, effects) = subs.step(event, now);
            asked.extend(effects);
            (next, asked)
        })
}

fn accepted(n: u64, uid: u32, claim: Claim) -> SocketEvent {
    SocketEvent::Accepted {
        id: id(n),
        uid,
        claim,
    }
}

/// A model where `USER` has the seat and `members` (number, claim) are connected as that user.
fn seated(members: &[(u64, Claim)]) -> Subscribers {
    let joins = members.iter().map(|&(n, claim)| accepted(n, USER, claim));
    feed(Subscribers::new(Some(USER)), joins, Instant::now()).0
}

/// The end of a pass: the model after it and what the reader is told.
fn tick(subs: Subscribers) -> (Subscribers, Vec<FromSocket>) {
    let (subs, effects) = subs.step(SocketEvent::Tick, Instant::now());
    (subs, notes(&effects))
}

fn notes(effects: &[SocketEffect]) -> Vec<FromSocket> {
    effects
        .iter()
        .filter_map(|e| match e {
            SocketEffect::Note(note) => Some(note.clone()),
            _ => None,
        })
        .collect()
}

/// Who was sent what, in order.
fn sends(effects: &[SocketEffect]) -> Vec<(u64, ToHost)> {
    effects
        .iter()
        .filter_map(|e| match e {
            SocketEffect::Send { id, msg } => Some((id.0, msg.clone())),
            _ => None,
        })
        .collect()
}

fn logged(effects: &[SocketEffect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, SocketEffect::Log(_)))
        .count()
}
