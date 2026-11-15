//! The subscribers as data: who is connected, what each said, what the reader was last told, and
//! the events and effects of the pure step over them. No descriptor lives here: a subscriber is
//! an id, and the facts that needed a syscall (its uid, what it claims, who has the seat) arrive
//! inside events.

use super::limits::Complaints;
use super::seat::{Peer, peer_of};
use super::{FromSocket, ToSocket};
use latchkey::guard::Claim;
use palmrest_protocol::{DeviceInfo, Filter, LockState, Over, ToDaemon, ToHost};
use rustix::io::Errno;
use std::collections::BTreeMap;
use std::time::Instant;

/// A subscriber's identity for as long as it is connected. Positions in a list change when a
/// subscriber leaves, so nothing that outlives one pass holds a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ClientId(pub(super) u64);

/// One connected subscriber.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Subscriber {
    /// The uid the kernel vouches for, taken when it connected.
    pub(super) uid: u32,
    /// What it claims, as of the last look.
    pub(super) claim: Claim,
    pub(super) filter: Filter,
    pub(super) over: Over,
    /// What it last said about the session's lock; `None` until it says.
    pub(super) lock: Option<LockState>,
}

impl Subscriber {
    pub(super) fn new(uid: u32, claim: Claim) -> Self {
        Self {
            uid,
            claim,
            filter: Filter::All,
            over: Over::NotOurs,
            lock: None,
        }
    }

    /// What this subscriber is while `active` has the seat.
    pub(super) fn peer(&self, active: Option<u32>) -> Peer {
        peer_of(self.uid, active, self.claim)
    }

    /// Whether the pointer is over this subscriber's surface, as far as it is believed: only the
    /// shell of the person at the seat.
    pub(super) fn pointer(&self, active: Option<u32>) -> Over {
        match self.peer(active) {
            Peer::Shell => self.over,
            Peer::Plain | Peer::Observer => Over::NotOurs,
        }
    }

    /// What this subscriber says about the lock, as far as it is believed: anyone served may say
    /// locked; only the shell may say unlocked.
    pub(super) fn session(&self, active: Option<u32>) -> Option<LockState> {
        match (self.lock, self.peer(active)) {
            (Some(LockState::Unlocked), Peer::Plain | Peer::Observer) => None,
            (said, _) => said,
        }
    }
}

/// What the reader was last told about the subscribers' joint state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Told {
    pub(super) pointer: Over,
    pub(super) lock: LockState,
}

/// Everything the socket thread decides from: the subscribers and what is known about the seat,
/// the mouse and the reader.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Subscribers {
    pub(super) members: BTreeMap<ClientId, Subscriber>,
    /// Who has the seat as of the last look.
    pub(super) active: Option<u32>,
    pub(super) device: Option<DeviceInfo>,
    pub(super) complaints: Complaints,
    pub(super) told: Told,
    /// The person's settings to tell the reader at the next `Tick` (`None` inside: the layer is
    /// gone).
    pub(super) config: Option<Option<String>>,
}

impl Subscribers {
    pub(super) fn new(active: Option<u32>) -> Self {
        Self {
            members: BTreeMap::new(),
            active,
            device: None,
            complaints: Complaints::default(),
            // Locked until a subscriber says otherwise: the tracker starts locked too, so the two
            // agree and nothing is announced for a state nobody changed.
            told: Told {
                pointer: Over::NotOurs,
                lock: LockState::Locked,
            },
            config: None,
        }
    }
}

impl Default for Subscribers {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Something that happened, with the facts the thread gathered for it.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum SocketEvent {
    /// A connection arrived: its id, the uid the kernel vouches for and what it claims.
    Accepted {
        id: ClientId,
        uid: u32,
        claim: Claim,
    },
    /// A packet from a subscriber: its message, or why it is none.
    Packet {
        id: ClientId,
        heard: Result<ToDaemon, String>,
    },
    /// A subscriber closed, or could not be written to or read from.
    Hangup {
        id: ClientId,
    },
    /// The seat file was read.
    SeatSeen {
        uid: Option<u32>,
    },
    /// What each connection claims now.
    ClaimsSeen(Vec<(ClientId, Claim)>),
    FromReader(ToSocket),
    /// `accept` failed.
    AcceptFailed {
        errno: Errno,
    },
    /// The end of a pass: tell the reader what changed.
    Tick,
}

/// What the thread must do.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum SocketEffect {
    Send {
        id: ClientId,
        msg: ToHost,
    },
    Close {
        id: ClientId,
    },
    /// Tell the reader.
    Note(FromSocket),
    /// Leave the listener alone until `until`.
    PauseAccept {
        until: Instant,
    },
    /// One line for the log.
    Log(String),
}
