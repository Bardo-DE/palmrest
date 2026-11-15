//! What a subscriber's packets do, by who sent them, as pure steps.

use super::{OTHER, SHELL, USER, feed, id, logged, seated, tick};
use crate::daemon::socket::FromSocket;
use crate::daemon::socket::model::SocketEvent;
use crate::daemon::socket::packet::{Hearing, interpret};
use crate::daemon::socket::seat::Peer;
use palmrest_protocol::{Filter, LockState, Over, ToDaemon, Topic, USER_CONFIG_MAX_BYTES};
use std::time::Instant;

fn said(n: u64, message: ToDaemon) -> SocketEvent {
    SocketEvent::Packet {
        id: id(n),
        heard: Ok(message),
    }
}

fn lock(n: u64, state: LockState) -> SocketEvent {
    said(n, ToDaemon::SessionLock { state })
}

fn settings(n: u64, toml: &str) -> SocketEvent {
    said(n, ToDaemon::UserConfig { toml: toml.into() })
}

#[test]
fn a_packet_means_what_the_standing_of_its_sender_allows() {
    let narrow = Filter::Only(vec![Topic::Tap]);
    let over = ToDaemon::PointerOver { over: Over::Ours };
    let unlocked = ToDaemon::SessionLock {
        state: LockState::Unlocked,
    };
    let locked = ToDaemon::SessionLock {
        state: LockState::Locked,
    };
    let subscribe = ToDaemon::Subscribe {
        filter: narrow.clone(),
    };
    let config = ToDaemon::UserConfig {
        toml: "[scroll]\nspeed = 40\n".into(),
    };
    let huge = ToDaemon::UserConfig {
        toml: "x".repeat(USER_CONFIG_MAX_BYTES + 1),
    };
    let cases: Vec<(&str, ToDaemon, Peer, Hearing)> = vec![
        (
            "the shell subscribes",
            subscribe.clone(),
            Peer::Shell,
            Hearing::Subscribe(narrow.clone()),
        ),
        (
            "a plain program subscribes",
            subscribe.clone(),
            Peer::Plain,
            Hearing::Subscribe(narrow),
        ),
        (
            "a stranger does not",
            subscribe,
            Peer::Observer,
            Hearing::Ignored,
        ),
        (
            "the shell says where the pointer is",
            over.clone(),
            Peer::Shell,
            Hearing::Pointer(Over::Ours),
        ),
        (
            "a plain program's pointer is ignored",
            over,
            Peer::Plain,
            Hearing::Ignored,
        ),
        (
            "the shell may unlock",
            unlocked.clone(),
            Peer::Shell,
            Hearing::Lock(LockState::Unlocked),
        ),
        (
            "a plain program may not unlock",
            unlocked,
            Peer::Plain,
            Hearing::Ignored,
        ),
        (
            "a plain program may lock",
            locked.clone(),
            Peer::Plain,
            Hearing::Lock(LockState::Locked),
        ),
        (
            "a stranger may not lock",
            locked,
            Peer::Observer,
            Hearing::Ignored,
        ),
        (
            "the shell's settings",
            config,
            Peer::Shell,
            Hearing::Settings("[scroll]\nspeed = 40\n".into()),
        ),
        (
            "settings over the bound",
            huge,
            Peer::Shell,
            Hearing::SettingsTooLarge,
        ),
        (
            "another program's settings",
            ToDaemon::UserConfig { toml: "a".into() },
            Peer::Plain,
            Hearing::Ignored,
        ),
    ];
    for (name, packet, peer, want) in cases {
        assert_eq!(interpret(Ok(packet), peer), want, "{name}");
    }
    assert_eq!(
        interpret(Err("junk".into()), Peer::Shell),
        Hearing::Malformed("junk".into())
    );
}

#[test]
fn a_malformed_packet_is_logged_and_changes_nothing() {
    let (subs, asked) = feed(
        seated(&[(0, SHELL)]),
        [SocketEvent::Packet {
            id: id(0),
            heard: Err("bad json".into()),
        }],
        Instant::now(),
    );
    assert_eq!(logged(&asked), 1);
    assert!(tick(subs).1.is_empty());
}

#[test]
fn a_packet_from_one_that_has_left_is_skipped() {
    let (_, asked) = feed(seated(&[]), [lock(7, LockState::Unlocked)], Instant::now());
    assert!(asked.is_empty());
}

#[test]
fn the_session_is_unlocked_only_by_the_shell_and_any_subscriber_can_lock_it() {
    let now = Instant::now();
    let subs = seated(&[(0, SHELL), (1, OTHER)]);
    assert!(
        tick(subs.clone()).1.is_empty(),
        "silent subscribers leave it locked, and that is no news"
    );

    let (subs, _) = feed(subs, [lock(1, LockState::Unlocked)], now);
    let (subs, told) = tick(subs);
    assert!(
        told.is_empty(),
        "a program that is not the shell cannot unlock the session"
    );

    let (subs, _) = feed(subs, [lock(0, LockState::Unlocked)], now);
    let (subs, told) = tick(subs);
    assert_eq!(told, [FromSocket::Session(LockState::Unlocked)]);

    let (subs, _) = feed(subs, [lock(1, LockState::Locked)], now);
    let (subs, told) = tick(subs);
    assert_eq!(
        told,
        [FromSocket::Session(LockState::Locked)],
        "any program of the person can lock it"
    );

    let (subs, _) = feed(subs, [SocketEvent::Hangup { id: id(1) }], now);
    let (subs, told) = tick(subs);
    assert_eq!(
        told,
        [FromSocket::Session(LockState::Unlocked)],
        "the one that locked it left"
    );

    let (subs, _) = feed(subs, [SocketEvent::Hangup { id: id(0) }], now);
    assert_eq!(
        tick(subs).1,
        [FromSocket::Session(LockState::Locked)],
        "with no subscriber connected the session counts as locked"
    );
}

#[test]
fn only_the_shell_is_believed_about_the_pointer() {
    let over = |n| said(n, ToDaemon::PointerOver { over: Over::Ours });
    let (subs, _) = feed(seated(&[(0, SHELL), (1, OTHER)]), [over(1)], Instant::now());
    assert!(tick(subs.clone()).1.is_empty());
    let (subs, _) = feed(subs, [over(0)], Instant::now());
    assert_eq!(tick(subs).1, [FromSocket::Pointer(Over::Ours)]);
}

#[test]
fn the_persons_settings_are_believed_from_the_shell_and_go_with_the_person() {
    let now = Instant::now();
    let subs = seated(&[(0, SHELL), (1, OTHER)]);
    let (subs, _) = feed(subs, [settings(1, "[scroll]\nspeed = 1\n")], now);
    assert!(
        tick(subs.clone()).1.is_empty(),
        "another program's settings are not read"
    );

    let (subs, _) = feed(subs, [settings(0, "[scroll]\nspeed = 40\n")], now);
    let (subs, told) = tick(subs);
    assert_eq!(
        told,
        [FromSocket::UserConfig(Some(
            "[scroll]\nspeed = 40\n".into()
        ))]
    );
    assert!(tick(subs.clone()).1.is_empty(), "told once");

    let (subs, _) = feed(subs, [settings(0, "  \n")], now);
    let (subs, told) = tick(subs);
    assert_eq!(
        told,
        [FromSocket::UserConfig(None)],
        "blank takes the layer away"
    );

    let huge = "x".repeat(USER_CONFIG_MAX_BYTES + 1);
    let (subs, asked) = feed(subs, [settings(0, &huge)], now);
    assert_eq!(logged(&asked), 1);
    assert!(
        tick(subs.clone()).1.is_empty(),
        "a text over the bound is not read"
    );

    let (subs, _) = feed(subs, [settings(0, "[scroll]\nspeed = 41\n")], now);
    let (subs, told) = tick(subs);
    assert!(matches!(told[..], [FromSocket::UserConfig(Some(_))]));
    let (subs, _) = feed(
        subs,
        [SocketEvent::SeatSeen {
            uid: Some(USER + 1),
        }],
        now,
    );
    assert_eq!(
        tick(subs).1[0],
        FromSocket::UserConfig(None),
        "the next person does not inherit the settings"
    );
}

#[test]
fn the_latest_settings_of_a_pass_are_the_ones_told() {
    let now = Instant::now();
    let (subs, _) = feed(
        seated(&[(0, SHELL)]),
        [settings(0, "a = 1\n"), settings(0, "a = 2\n")],
        now,
    );
    assert_eq!(
        tick(subs).1,
        [FromSocket::UserConfig(Some("a = 2\n".into()))]
    );
}
