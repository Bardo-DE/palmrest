//! What the socket tests on real sockets share: a seat file, a scripted vouch, and a way to
//! talk to the thread.

use crate::daemon::socket::SeatFile;
use latchkey::guard::{Held, Origin, Vouch};
use palmrest_protocol::{self as protocol, ActiveTouch, ToDaemon, ToHost};
use peephole::Pinned;
use peephole_core::{Pid, Pin, Uid};
use rustix::fd::OwnedFd;
use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketAddrUnix, SocketType};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The uid of this test process, who has the seat in these tests.
pub(super) fn me() -> u32 {
    rustix::process::getuid().as_raw()
}

/// A seat file that names `active` as the person at the seat (none: no session).
pub(super) fn seat_file(dir: &Path, active: Option<u32>) -> SeatFile {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("seat0");
    match active {
        Some(uid) => std::fs::write(&path, format!("IS_SEAT0=1\nACTIVE_UID={uid}\n")).unwrap(),
        None => std::fs::write(&path, "IS_SEAT0=1\n").unwrap(),
    }
    SeatFile(path)
}

/// A held shell of `user`, for the tests that need a connection believed about the lock.
pub(super) fn shell_of(user: u32) -> Origin {
    let process = Pinned::introduced(Pid::new(4242).unwrap(), Uid::new(user), Pin::Reopened);
    Origin::Shell(Held { process, user })
}

/// What a test connection is to the daemon.
#[derive(Debug, Clone, Copy)]
pub(super) enum Role {
    /// The shell of the person at the seat.
    Shell,
    /// Another program of theirs.
    Other,
}

/// Names each connection in the order the test makes them.
#[derive(Clone, Default)]
pub(super) struct Scripted(pub(super) Arc<Mutex<VecDeque<Role>>>);

impl Vouch for Scripted {
    fn origin(&self, _stream: &OwnedFd) -> Origin {
        match self.0.lock().unwrap().pop_front() {
            Some(Role::Shell) => shell_of(me()),
            Some(Role::Other) | None => Origin::Other,
        }
    }
}

pub(super) fn connect(path: &Path) -> OwnedFd {
    let fd = rustix::net::socket(AddressFamily::UNIX, SocketType::SEQPACKET, None).unwrap();
    rustix::net::connect(&fd, &SocketAddrUnix::new(path).unwrap()).unwrap();
    rustix::net::sockopt::set_socket_timeout(
        &fd,
        rustix::net::sockopt::Timeout::Recv,
        Some(Duration::from_secs(2)),
    )
    .unwrap();
    fd
}

pub(super) fn recv(fd: &OwnedFd) -> ToHost {
    let mut buf = [0u8; protocol::MAX_PACKET];
    let (n, _) = rustix::net::recv(fd, &mut buf, RecvFlags::empty()).unwrap();
    protocol::decode(&buf[..n]).unwrap()
}

pub(super) fn send(fd: &OwnedFd, m: &ToDaemon) {
    rustix::net::send(fd, &protocol::encode(m), SendFlags::NOSIGNAL).unwrap();
}

/// Everything queued for a subscriber, decoded.
pub(super) fn queued(fd: &OwnedFd) -> Vec<ToHost> {
    let mut buf = [0u8; protocol::MAX_PACKET];
    let mut out = Vec::new();
    while let Ok((n, _)) = rustix::net::recv(fd, &mut buf, RecvFlags::DONTWAIT) {
        if n == 0 {
            break;
        }
        out.push(protocol::decode(&buf[..n]).unwrap());
    }
    out
}

pub(super) const TOUCHES: ToHost = ToHost::Touches {
    active: ActiveTouch::Down,
};
