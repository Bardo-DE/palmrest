//! The socket thread's loop: one pass polls the wake-up, the listener and every subscriber, then
//! gathers the facts that need a syscall (who has the seat, what each connection claims, who is
//! on the other end), feeds them to the pure `Subscribers::step` as events and carries out the
//! effects it returns. Every step is bounded, so no subscriber can keep the others, or the
//! reader's stream, waiting. Nothing here decides.

use super::conn::{Conn, Read};
use super::limits::{ACCEPTS_PER_PASS, PACKETS_PER_PASS, SEAT_RECHECK};
use super::model::{ClientId, SocketEffect, SocketEvent, Subscribers};
use super::{FromSocket, ToSocket};
use latchkey::guard::{AcceptFailure, SeatFile, Vouch, accept_failure};
use palmrest_protocol as protocol;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fd::OwnedFd;
use rustix::io::Errno;
use rustix::net::SocketFlags;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

/// Why the loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exit {
    /// The reader dropped its end of the channel: the daemon is shutting down.
    ReaderGone,
    /// Waiting for events failed for good: the socket can no longer be served.
    PollFailed,
}

/// What a poll found: whether the wake-up and the listener are ready, and the subscribers that
/// have something to read or have hung up, by id.
struct Ready {
    listener: bool,
    wake: bool,
    clients: Vec<ClientId>,
}

pub(super) struct Server {
    listener: OwnedFd,
    wake: Arc<OwnedFd>,
    rx: Receiver<ToSocket>,
    notes: Sender<FromSocket>,
    seat: SeatFile,
    /// Says which connection is the shell.
    vouch: Box<dyn Vouch>,
    subs: Subscribers,
    conns: BTreeMap<ClientId, Conn>,
    next_id: u64,
    /// While set, the listener is not polled until then: `accept` is failing for want of
    /// descriptors.
    paused_until: Option<Instant>,
    /// When the seat file was last read.
    looked: Instant,
}

impl Server {
    pub(super) fn new(
        listener: OwnedFd,
        wake: Arc<OwnedFd>,
        rx: Receiver<ToSocket>,
        notes: Sender<FromSocket>,
        seat: SeatFile,
        vouch: Box<dyn Vouch>,
    ) -> Self {
        Self {
            listener,
            wake,
            rx,
            notes,
            subs: Subscribers::new(seat.active_uid()),
            seat,
            vouch,
            conns: BTreeMap::new(),
            next_id: 0,
            paused_until: None,
            looked: Instant::now(),
        }
    }

    pub(super) fn run(mut self) -> Exit {
        loop {
            if let Some(exit) = self.pass() {
                return exit;
            }
        }
    }

    /// One pass: wait, look, accept, deliver, read, publish. `Some` when the loop must end.
    pub(super) fn pass(&mut self) -> Option<Exit> {
        self.paused_until = self.paused_until.filter(|&until| until > Instant::now());
        let ready = match self.wait() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("palmrest: socket poll failed: {e}");
                return Some(Exit::PollFailed);
            }
        };
        self.watch_seat();
        self.look_at_claims();
        if ready.listener {
            self.accept();
        }
        if ready.wake && !self.drain() {
            return Some(Exit::ReaderGone);
        }
        ready.clients.into_iter().for_each(|id| self.receive(id));
        self.feed(SocketEvent::Tick);
        None
    }

    fn wait(&self) -> Result<Ready, Errno> {
        let mut fds = vec![PollFd::new(&*self.wake, PollFlags::IN)];
        let listener_slot = self.paused_until.is_none().then(|| {
            fds.push(PollFd::new(&self.listener, PollFlags::IN));
            fds.len() - 1
        });
        let first_client = fds.len();
        fds.extend(
            self.conns
                .values()
                .map(|c| PollFd::new(&c.fd, PollFlags::IN)),
        );
        // Never asleep for longer than the seat recheck: a user switch with no traffic is noticed.
        let timeout = timespec(self.paused_until.map_or(SEAT_RECHECK, |until| {
            until
                .saturating_duration_since(Instant::now())
                .min(SEAT_RECHECK)
        }));
        match poll(&mut fds, Some(&timeout)) {
            Ok(_) | Err(Errno::INTR) => {}
            Err(e) => return Err(e),
        }
        Ok(Ready {
            listener: listener_slot.is_some_and(|slot| readable(&fds[slot])),
            wake: readable(&fds[0]),
            clients: self
                .conns
                .keys()
                .zip(&fds[first_client..])
                .filter(|(_, f)| readable(f))
                .map(|(&id, _)| id)
                .collect(),
        })
    }

    /// Accepts what is waiting, a bounded number per pass.
    fn accept(&mut self) {
        for _ in 0..ACCEPTS_PER_PASS {
            let accepted = rustix::net::accept_with(
                &self.listener,
                SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            );
            match accepted {
                Ok(fd) => self.admit(fd),
                Err(errno) => {
                    self.feed(SocketEvent::AcceptFailed { errno });
                    if accept_failure(errno) != AcceptFailure::Retry {
                        return;
                    }
                }
            }
        }
    }

    /// Takes a connection in: asks who is on the other end and who has the seat, and lets the
    /// model decide whether it stays.
    pub(super) fn admit(&mut self, fd: OwnedFd) {
        let Ok(cred) = rustix::net::sockopt::socket_peercred(&fd) else {
            return;
        };
        // Asked of every connection, whoever has the seat now: the shell may connect a moment
        // before logind names its user the seat's owner.
        let origin = self.vouch.origin(&fd);
        self.look_at_seat();
        let id = ClientId(self.next_id);
        self.next_id += 1;
        let conn = Conn::new(fd, origin);
        let claim = conn.claim();
        self.conns.insert(id, conn);
        self.feed(SocketEvent::Accepted {
            id,
            uid: cred.uid.as_raw(),
            claim,
        });
    }

    /// Reads the seat file when it is due.
    fn watch_seat(&mut self) {
        if Instant::now().saturating_duration_since(self.looked) >= SEAT_RECHECK {
            self.look_at_seat();
        }
    }

    /// Reads the seat file now. Returns who has the seat.
    pub(super) fn look_at_seat(&mut self) -> Option<u32> {
        let uid = self.seat.active_uid();
        self.looked = Instant::now();
        self.feed(SocketEvent::SeatSeen { uid });
        uid
    }

    /// What every connection claims now: a shell whose process has exited claims nothing.
    fn look_at_claims(&mut self) {
        let claims: Vec<_> = self.conns.iter().map(|(&id, c)| (id, c.claim())).collect();
        if !claims.is_empty() {
            self.feed(SocketEvent::ClaimsSeen(claims));
        }
    }

    /// Hands over everything the reader queued; false when the reader has gone (shutdown).
    fn drain(&mut self) -> bool {
        let mut buf = [0u8; 8];
        let _ = rustix::io::read(&*self.wake, &mut buf);
        loop {
            match self.rx.try_recv() {
                Ok(message) => self.feed(SocketEvent::FromReader(message)),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    /// Reads what subscriber `id` sent, at most `PACKETS_PER_PASS` packets; the rest is read next
    /// pass, as `poll` reports it. The subscriber may have left since the poll, so it is looked up
    /// by id and a missing one is skipped.
    fn receive(&mut self, id: ClientId) {
        let mut buf = [0u8; protocol::MAX_PACKET + 1];
        for _ in 0..PACKETS_PER_PASS {
            let Some(conn) = self.conns.get(&id) else {
                return;
            };
            match conn.read(&mut buf) {
                Read::Packet(heard) => self.feed(SocketEvent::Packet { id, heard }),
                Read::Retry => {}
                Read::Empty => return,
                Read::Closed => {
                    self.hang_up(id);
                    return;
                }
            }
        }
    }

    fn hang_up(&mut self, id: ClientId) {
        self.conns.remove(&id);
        self.feed(SocketEvent::Hangup { id });
    }

    /// Steps the model by `event` and carries out what it asks.
    fn feed(&mut self, event: SocketEvent) {
        let (subs, effects) = std::mem::take(&mut self.subs).step(event, Instant::now());
        self.subs = subs;
        effects.into_iter().for_each(|effect| self.perform(effect));
    }

    fn perform(&mut self, effect: SocketEffect) {
        match effect {
            SocketEffect::Send { id, msg } => {
                if !self.conns.get(&id).is_none_or(|c| c.send(&msg)) {
                    self.hang_up(id);
                }
            }
            SocketEffect::Close { id } => {
                self.conns.remove(&id);
            }
            SocketEffect::Note(note) => {
                let _ = self.notes.send(note);
            }
            SocketEffect::PauseAccept { until } => self.paused_until = Some(until),
            SocketEffect::Log(line) => eprintln!("palmrest: {line}"),
        }
    }

    /// The subscribers connected now (for the pass-level tests).
    #[cfg(test)]
    pub(super) fn subscribers(&self) -> usize {
        self.conns.len()
    }
}

/// Whether poll found a descriptor ready.
fn readable(fd: &PollFd<'_>) -> bool {
    !fd.revents().is_empty()
}

fn timespec(d: Duration) -> Timespec {
    Timespec {
        tv_sec: d.as_secs().try_into().unwrap_or(1),
        tv_nsec: i64::from(d.subsec_nanos()),
    }
}
