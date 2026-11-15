//! The socket thread one pass at a time on the test's thread: the budgets, the closing of
//! connections and the queue, which only real descriptors show.

use super::support::{Role, Scripted, TOUCHES, me, queued, seat_file, send};
use crate::daemon::socket::io::Server;
use crate::daemon::socket::limits::{MAX_CLIENTS, PACKETS_PER_PASS, QUEUE_MAX};
use crate::daemon::socket::{FromSocket, SocketHandle, ToSocket, bind};
use palmrest_protocol::{self as protocol, LockState, Over, ToDaemon, ToHost};
use rustix::event::{EventfdFlags, eventfd};
use rustix::fd::OwnedFd;
use rustix::net::{AddressFamily, RecvFlags, SocketFlags, SocketType, socketpair};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

/// A server driven one pass at a time on the test's thread, over a private listener.
struct Stepped {
    server: Server,
    handle: SocketHandle,
    claims: Receiver<FromSocket>,
    dir: PathBuf,
    roles: Scripted,
}

impl Stepped {
    /// A server where this test's user has the seat.
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("palmrest-{name}-{}", std::process::id()));
        let listener = bind(&dir.join("events.sock")).unwrap();
        let wake = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).unwrap());
        let (tx, rx) = mpsc::sync_channel(QUEUE_MAX);
        let (claims_tx, claims) = mpsc::channel();
        let roles = Scripted::default();
        Self {
            server: Server::new(
                listener,
                wake.clone(),
                rx,
                claims_tx,
                seat_file(&dir, Some(me())),
                Box::new(roles.clone()),
            ),
            handle: SocketHandle { tx, wake },
            claims,
            dir,
            roles,
        }
    }

    /// Connects the shell through a socket pair and returns its end.
    fn subscribe(&mut self) -> OwnedFd {
        self.connect(Role::Shell)
    }

    /// Connects a program that is `role` through a socket pair and returns its end.
    fn connect(&mut self, role: Role) -> OwnedFd {
        let (ours, theirs) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        self.roles.0.lock().unwrap().push_back(role);
        self.server.admit(ours);
        theirs
    }
}

impl Drop for Stepped {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_subscriber_closing_during_a_broadcast_is_dropped_not_indexed() {
    let mut s = Stepped::new("close-in-broadcast");
    let stays = s.subscribe();
    let leaves = s.subscribe();
    drop(leaves);
    // The closed peer is readable (hang-up) and fails the broadcast in the same pass.
    s.handle.send(ToSocket::Msg(TOUCHES));
    assert_eq!(s.server.pass(), None);
    assert_eq!(s.server.subscribers(), 1);
    assert_eq!(queued(&stays).last(), Some(&TOUCHES));
}

#[test]
fn subscribers_beyond_the_cap_are_closed_at_once() {
    let mut s = Stepped::new("cap");
    let peers: Vec<OwnedFd> = (0..MAX_CLIENTS + 3).map(|_| s.subscribe()).collect();
    assert_eq!(s.server.subscribers(), MAX_CLIENTS);
    let refused = &peers[MAX_CLIENTS];
    assert!(queued(refused).is_empty(), "no greeting for a refused one");
    let mut buf = [0u8; 8];
    assert_eq!(
        rustix::net::recv(refused, &mut buf, RecvFlags::DONTWAIT).map(|(n, _)| n),
        Ok(0),
        "the refused connection is closed"
    );
}

#[test]
fn a_flood_is_read_a_budget_at_a_time() {
    let mut s = Stepped::new("flood");
    let chatty = s.subscribe();
    (0..PACKETS_PER_PASS).for_each(|_| {
        send(
            &chatty,
            &ToDaemon::PointerOver {
                over: Over::NotOurs,
            },
        )
    });
    send(&chatty, &ToDaemon::PointerOver { over: Over::Ours });
    assert_eq!(s.server.pass(), None);
    assert!(
        s.claims.try_recv().is_err(),
        "the last packet waits for the next pass"
    );
    assert_eq!(s.server.pass(), None);
    assert_eq!(s.claims.try_recv(), Ok(FromSocket::Pointer(Over::Ours)));
}

#[test]
fn a_full_queue_drops_messages_instead_of_blocking_the_reader() {
    let mut s = Stepped::new("queue");
    let _peer = s.subscribe();
    for _ in 0..QUEUE_MAX + 10 {
        s.handle.send(ToSocket::Msg(TOUCHES));
    }
    assert_eq!(s.server.pass(), None);
}

#[test]
fn a_user_switch_closes_the_connection_of_the_one_who_left() {
    let mut s = Stepped::new("switch");
    let shell = s.subscribe();
    send(
        &shell,
        &ToDaemon::SessionLock {
            state: LockState::Unlocked,
        },
    );
    assert_eq!(s.server.pass(), None);
    assert_eq!(
        s.claims.try_recv(),
        Ok(FromSocket::Session(LockState::Unlocked))
    );

    seat_file(&s.dir, Some(me() + 1));
    assert_eq!(s.server.look_at_seat(), Some(me() + 1));
    assert_eq!(s.server.subscribers(), 0);
    let mut buf = [0u8; 8];
    assert_eq!(
        rustix::net::recv(&shell, &mut buf, RecvFlags::DONTWAIT).map(|(n, _)| n),
        Ok(0),
        "the connection is closed"
    );
    s.handle.send(ToSocket::Msg(TOUCHES));
    assert_eq!(s.server.pass(), None);
    assert_eq!(s.claims.try_recv(), Ok(FromSocket::UserConfig(None)));
    assert_eq!(
        s.claims.try_recv(),
        Ok(FromSocket::Session(LockState::Locked))
    );
}

#[test]
fn only_the_shell_is_asked_to_run_an_action() {
    let mut s = Stepped::new("run");
    let shell = s.subscribe();
    let other = s.connect(Role::Other);
    let action = protocol::HostAction::Command {
        argv: vec!["notify-send".into(), "hi".into()],
    };
    s.handle.send(ToSocket::Run(action.clone()));
    assert_eq!(s.server.pass(), None);
    let asked = ToHost::Run { action };
    assert_eq!(queued(&shell).last(), Some(&asked));
    assert!(!queued(&other).contains(&asked));
}
