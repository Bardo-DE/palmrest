//! What a subscriber's message does. Anyone served may subscribe and may say the session is
//! locked (the safe word); only the shell is believed about the pointer, an unlocked session and
//! the person's settings.

use super::model::{ClientId, SocketEffect, Subscriber, Subscribers};
use super::seat::Peer;
use palmrest_protocol::{Filter, LockState, Over, ToDaemon, USER_CONFIG_MAX_BYTES};

/// A packet's worth, once the standing of whoever sent it is known.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Hearing {
    Subscribe(Filter),
    Pointer(Over),
    Lock(LockState),
    Settings(String),
    SettingsTooLarge,
    Malformed(String),
    Ignored,
}

/// What `heard` amounts to from `peer`.
pub(super) fn interpret(heard: Result<ToDaemon, String>, peer: Peer) -> Hearing {
    match (heard, peer) {
        (Err(why), _) => Hearing::Malformed(why),
        (Ok(ToDaemon::Subscribe { filter }), Peer::Shell | Peer::Plain) => {
            Hearing::Subscribe(filter)
        }
        (Ok(ToDaemon::PointerOver { over }), Peer::Shell) => Hearing::Pointer(over),
        (Ok(ToDaemon::SessionLock { state }), Peer::Shell) => Hearing::Lock(state),
        (
            Ok(ToDaemon::SessionLock {
                state: LockState::Locked,
            }),
            Peer::Plain,
        ) => Hearing::Lock(LockState::Locked),
        (Ok(ToDaemon::UserConfig { toml }), Peer::Shell) if toml.len() <= USER_CONFIG_MAX_BYTES => {
            Hearing::Settings(toml)
        }
        (Ok(ToDaemon::UserConfig { .. }), Peer::Shell) => Hearing::SettingsTooLarge,
        (Ok(_), _) => Hearing::Ignored,
    }
}

/// A subscriber after a packet, with what the packet asks beyond it.
struct Heard {
    subscriber: Subscriber,
    /// The person's settings it sent (`None` inside: blank text takes the layer away).
    settings: Option<Option<String>>,
    complaint: Option<String>,
}

impl Hearing {
    fn apply(self, sub: Subscriber) -> Heard {
        let said = |subscriber| Heard {
            subscriber,
            settings: None,
            complaint: None,
        };
        match self {
            Self::Subscribe(filter) => said(Subscriber { filter, ..sub }),
            Self::Pointer(over) => said(Subscriber { over, ..sub }),
            Self::Lock(state) => said(Subscriber {
                lock: Some(state),
                ..sub
            }),
            Self::Settings(text) => Heard {
                settings: Some((!text.trim().is_empty()).then_some(text)),
                ..said(sub)
            },
            Self::SettingsTooLarge => Heard {
                complaint: Some(format!(
                    "ignoring settings larger than {USER_CONFIG_MAX_BYTES} bytes"
                )),
                ..said(sub)
            },
            Self::Malformed(why) => Heard {
                complaint: Some(format!("ignoring a malformed subscriber message: {why}")),
                ..said(sub)
            },
            Self::Ignored => said(sub),
        }
    }
}

impl Subscribers {
    /// Applies a packet from subscriber `id`. The subscriber may have left since the packet was
    /// read, so a missing one is skipped.
    pub(super) fn hear(
        mut self,
        id: ClientId,
        heard: Result<ToDaemon, String>,
    ) -> (Self, Vec<SocketEffect>) {
        let Some(sub) = self.members.remove(&id) else {
            return (self, vec![]);
        };
        let hearing = interpret(heard, sub.peer(self.active));
        let Heard {
            subscriber,
            settings,
            complaint,
        } = hearing.apply(sub);
        self.members.insert(id, subscriber);
        let config = settings.or(self.config.take());
        (
            Self { config, ..self },
            complaint.map(SocketEffect::Log).into_iter().collect(),
        )
    }
}
