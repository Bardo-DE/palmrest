//! Admission, fan-out, the seat and the accept pause, as pure steps.

use super::{OTHER, SHELL, USER, accepted, feed, id, logged, notes, seated, sends, tick};
use crate::daemon::socket::FromSocket;
use crate::daemon::socket::limits::MAX_CLIENTS;
use crate::daemon::socket::model::{SocketEffect, SocketEvent, Subscribers};
use latchkey::guard::{ACCEPT_PAUSE, COMPLAINT_INTERVAL};
use palmrest_protocol::{
    ActiveTouch, DeviceInfo, Filter, HostAction, LockState, Over, PROTOCOL_VERSION, TapKind,
    ToDaemon, ToHost, Topic,
};
use rustix::io::Errno;
use std::time::{Duration, Instant};

fn nobody() -> DeviceInfo {
    DeviceInfo {
        name: String::new(),
        path: String::new(),
        vendor: 0,
        product: 0,
    }
}

fn mouse() -> DeviceInfo {
    DeviceInfo {
        name: "mouse".into(),
        path: "/dev/hidraw9".into(),
        vendor: 0x4c,
        product: 0x269,
    }
}

fn hello(device: DeviceInfo) -> ToHost {
    ToHost::Hello {
        version: PROTOCOL_VERSION,
        device,
    }
}

const TOUCHES: ToHost = ToHost::Touches {
    active: ActiveTouch::Down,
};
const TAP: ToHost = ToHost::Tap {
    kind: TapKind::Two,
    t_ns: 1,
};

fn said(n: u64, message: ToDaemon) -> SocketEvent {
    SocketEvent::Packet {
        id: id(n),
        heard: Ok(message),
    }
}

fn reader(message: super::super::ToSocket) -> SocketEvent {
    SocketEvent::FromReader(message)
}

#[test]
fn a_newcomer_hears_the_mouse_or_that_there_is_none() {
    use super::super::ToSocket::{Device, Gone};
    let now = Instant::now();
    let (subs, asked) = feed(
        Subscribers::new(Some(USER)),
        [accepted(0, USER, SHELL)],
        now,
    );
    assert_eq!(
        sends(&asked),
        [(0, hello(nobody())), (0, ToHost::DeviceGone)],
        "no mouse yet"
    );

    let (subs, asked) = feed(subs, [reader(Device(mouse()))], now);
    assert_eq!(
        sends(&asked),
        [(0, hello(mouse()))],
        "the open ones are told"
    );

    let (subs, asked) = feed(subs, [accepted(1, USER, OTHER)], now);
    assert_eq!(
        sends(&asked),
        [(1, hello(mouse()))],
        "late subscribers learn the device"
    );

    let (_, asked) = feed(subs, [reader(Gone), accepted(2, USER, OTHER)], now);
    assert_eq!(
        sends(&asked),
        [(2, hello(nobody())), (2, ToHost::DeviceGone)]
    );
}

#[test]
fn only_the_person_at_the_seat_is_served() {
    let cases = [
        ("another user has the seat", Some(USER + 1), USER, SHELL),
        ("no session", None, USER, SHELL),
        (
            "a stranger while the user has the seat",
            Some(USER),
            USER + 1,
            OTHER,
        ),
        ("root is not special", Some(USER), 0, OTHER),
    ];
    for (name, active, uid, claim) in cases {
        let (subs, asked) = feed(
            Subscribers::new(active),
            [accepted(0, uid, claim)],
            Instant::now(),
        );
        assert_eq!(asked, [SocketEffect::Close { id: id(0) }], "{name}");
        assert!(subs.members.is_empty(), "{name}");
    }
}

#[test]
fn at_most_eight_are_served_and_a_refusal_is_logged_once_an_interval() {
    let t0 = Instant::now();
    let full: Vec<_> = (0..).take(MAX_CLIENTS).map(|n| (n, OTHER)).collect();
    let subs = seated(&full);
    assert_eq!(subs.members.len(), MAX_CLIENTS);

    let refuse = |subs, n, at| feed(subs, [accepted(n, USER, OTHER)], at);
    let (subs, first) = refuse(subs, 8, t0);
    assert_eq!(first[0], SocketEffect::Close { id: id(8) });
    assert_eq!((logged(&first), subs.members.len()), (1, MAX_CLIENTS));
    assert!(sends(&first).is_empty(), "no greeting for a refused one");

    let (subs, soon) = refuse(subs, 9, t0 + Duration::from_secs(1));
    assert_eq!(
        logged(&soon),
        0,
        "a subscriber that keeps knocking cannot fill the log"
    );
    let (_, later) = refuse(subs, 10, t0 + COMPLAINT_INTERVAL);
    assert_eq!(logged(&later), 1);
}

#[test]
fn a_message_goes_to_the_subscribers_whose_filter_admits_it() {
    use super::super::ToSocket::Msg;
    let now = Instant::now();
    let narrow = Filter::Only(vec![Topic::Tap]);
    let (subs, _) = feed(
        seated(&[(0, SHELL), (1, OTHER)]),
        [said(1, ToDaemon::Subscribe { filter: narrow })],
        now,
    );
    let (subs, touches) = feed(subs, [reader(Msg(TOUCHES))], now);
    assert_eq!(sends(&touches), [(0, TOUCHES)], "1 filtered Touches out");
    let (_, tap) = feed(subs, [reader(Msg(TAP))], now);
    assert_eq!(sends(&tap), [(0, TAP), (1, TAP)]);
}

#[test]
fn a_subscriber_that_hung_up_is_not_written_to() {
    use super::super::ToSocket::Msg;
    let now = Instant::now();
    let (subs, _) = feed(
        seated(&[(0, SHELL), (1, SHELL)]),
        [SocketEvent::Hangup { id: id(1) }],
        now,
    );
    let (_, asked) = feed(subs, [reader(Msg(TOUCHES))], now);
    assert_eq!(sends(&asked), [(0, TOUCHES)]);
}

#[test]
fn a_hang_up_releases_the_claim_and_the_lock_word() {
    let now = Instant::now();
    let (subs, _) = feed(
        seated(&[(0, SHELL)]),
        [
            said(0, ToDaemon::PointerOver { over: Over::Ours }),
            said(
                0,
                ToDaemon::SessionLock {
                    state: LockState::Unlocked,
                },
            ),
        ],
        now,
    );
    let (subs, told) = tick(subs);
    assert_eq!(
        told,
        [
            FromSocket::Pointer(Over::Ours),
            FromSocket::Session(LockState::Unlocked)
        ]
    );
    let (subs, _) = feed(subs, [SocketEvent::Hangup { id: id(0) }], now);
    assert_eq!(
        tick(subs).1,
        [
            FromSocket::Pointer(Over::NotOurs),
            FromSocket::Session(LockState::Locked)
        ]
    );
}

#[test]
fn a_shell_whose_process_exited_is_believed_no_more() {
    let now = Instant::now();
    let (subs, _) = feed(
        seated(&[(0, SHELL)]),
        [said(0, ToDaemon::PointerOver { over: Over::Ours })],
        now,
    );
    let (subs, told) = tick(subs);
    assert_eq!(told, [FromSocket::Pointer(Over::Ours)]);
    let (subs, _) = feed(subs, [SocketEvent::ClaimsSeen(vec![(id(0), OTHER)])], now);
    assert_eq!(tick(subs).1, [FromSocket::Pointer(Over::NotOurs)]);
}

#[test]
fn a_user_switch_closes_the_connections_of_the_one_who_left() {
    let now = Instant::now();
    let (subs, _) = feed(
        seated(&[(0, SHELL)]),
        [said(
            0,
            ToDaemon::SessionLock {
                state: LockState::Unlocked,
            },
        )],
        now,
    );
    let (subs, told) = tick(subs);
    assert_eq!(told, [FromSocket::Session(LockState::Unlocked)]);

    let (subs, same) = feed(subs, [SocketEvent::SeatSeen { uid: Some(USER) }], now);
    assert!(same.is_empty(), "no news when nobody switched");

    let (subs, asked) = feed(
        subs,
        [SocketEvent::SeatSeen {
            uid: Some(USER + 1),
        }],
        now,
    );
    assert_eq!(asked, [SocketEffect::Close { id: id(0) }]);
    assert!(subs.members.is_empty());
    assert_eq!(
        tick(subs).1,
        [
            FromSocket::UserConfig(None),
            FromSocket::Session(LockState::Locked)
        ],
        "the one who left takes their settings with them, and the session is locked for the next"
    );
}

#[test]
fn only_the_shell_is_asked_to_run_an_action() {
    use super::super::ToSocket::Run;
    let now = Instant::now();
    let action = HostAction::Command {
        argv: vec!["notify-send".into(), "hi".into()],
    };
    let asked_run = ToHost::Run {
        action: action.clone(),
    };
    // Even a shell that narrowed its filter is asked.
    let narrow = said(
        0,
        ToDaemon::Subscribe {
            filter: Filter::Only(vec![Topic::Tap]),
        },
    );
    let (subs, _) = feed(seated(&[(0, SHELL), (1, OTHER)]), [narrow], now);
    let (_, asked) = feed(subs, [reader(Run(action.clone()))], now);
    assert_eq!(sends(&asked), [(0, asked_run)]);

    let (_, dropped) = feed(seated(&[(1, OTHER)]), [reader(Run(action))], now);
    assert!(sends(&dropped).is_empty());
    assert_eq!(logged(&dropped), 1, "a dropped action is logged");
}

#[test]
fn a_failed_accept_pauses_the_listener_only_when_retrying_cannot_help() {
    let now = Instant::now();
    let fail = |errno| {
        feed(
            Subscribers::new(Some(USER)),
            [SocketEvent::AcceptFailed { errno }],
            now,
        )
        .1
    };
    assert!(fail(Errno::AGAIN).is_empty(), "nothing pending");
    assert!(fail(Errno::INTR).is_empty(), "interrupted: ask again");
    let paused = fail(Errno::MFILE);
    assert_eq!(logged(&paused), 1);
    assert!(paused.contains(&SocketEffect::PauseAccept {
        until: now + ACCEPT_PAUSE
    }));
    assert!(notes(&paused).is_empty());
}
