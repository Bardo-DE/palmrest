//! The pure transition over the subscribers: `(Subscribers, SocketEvent, now) -> (Subscribers,
//! effects)`. The decisions are here: whom to admit, what a peer is, what a seat change closes,
//! whom a reader's message goes to, and when the reader is told. The pieces by concern are
//! `admit`, `packet`, `deliver` and `publish`.

use super::model::{ClientId, SocketEffect, SocketEvent, Subscribers};
use super::seat::Peer;
use latchkey::guard::{ACCEPT_PAUSE, AcceptFailure, Claim, accept_failure};
use rustix::io::Errno;
use std::collections::BTreeMap;
use std::time::Instant;

impl Subscribers {
    pub(super) fn step(self, event: SocketEvent, now: Instant) -> (Self, Vec<SocketEffect>) {
        match event {
            SocketEvent::Accepted { id, uid, claim } => self.admit(id, uid, claim, now),
            SocketEvent::Packet { id, heard } => self.hear(id, heard),
            SocketEvent::Hangup { id } => (self.without(id), vec![]),
            SocketEvent::SeatSeen { uid } => self.seat_seen(uid),
            SocketEvent::ClaimsSeen(claims) => (self.with_claims(claims), vec![]),
            SocketEvent::FromReader(message) => self.from_reader(message),
            SocketEvent::AcceptFailed { errno } => (self, accept_failed(errno, now)),
            SocketEvent::Tick => self.publish(),
        }
    }

    fn without(mut self, id: ClientId) -> Self {
        self.members.remove(&id);
        self
    }

    fn with_claims(mut self, claims: Vec<(ClientId, Claim)>) -> Self {
        for (id, claim) in claims {
            if let Some(member) = self.members.get_mut(&id) {
                member.claim = claim;
            }
        }
        self
    }

    /// When another user has the seat, every connection that is not theirs is closed, and with it
    /// whatever it said about the lock. Another person's settings are not this one's.
    fn seat_seen(self, uid: Option<u32>) -> (Self, Vec<SocketEffect>) {
        if uid == self.active {
            return (self, vec![]);
        }
        let (kept, left): (BTreeMap<_, _>, BTreeMap<_, _>) = self
            .members
            .into_iter()
            .partition(|(_, member)| member.peer(uid) != Peer::Observer);
        let closes = left.keys().map(|&id| SocketEffect::Close { id }).collect();
        let next = Self {
            members: kept,
            active: uid,
            config: Some(None),
            ..self
        };
        (next, closes)
    }
}

/// A failure that retrying cannot fix (no descriptors left) leaves the connection pending, so the
/// listener is left alone for a while instead of waking the loop in a spin.
fn accept_failed(errno: Errno, now: Instant) -> Vec<SocketEffect> {
    match accept_failure(errno) {
        AcceptFailure::NothingPending | AcceptFailure::Retry => vec![],
        AcceptFailure::Pause => vec![
            SocketEffect::Log(format!(
                "accept failed: {errno}; not accepting for a moment"
            )),
            SocketEffect::PauseAccept {
                until: now + ACCEPT_PAUSE,
            },
        ],
    }
}
