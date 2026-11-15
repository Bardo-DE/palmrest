//! The lifecycle of the virtual devices as data: where they are, what can happen to them and what
//! each happening asks the shell around them to do.

use crate::config::Config;
use crate::daemon::discover::Backoff;
use crate::keys::KeyCombo;
use crate::outputs::OutputError;
use crate::time::Mono;

/// What the devices are made from; a change recreates them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Shape {
    pub(super) combos: Vec<KeyCombo>,
}

impl Shape {
    pub(super) fn of(c: &Config) -> Self {
        Self {
            combos: c.gestures.gesture_action_map.combos(),
        }
    }
}

/// Times the service cut palmrest off since a frame last went through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Cuts(pub(super) u8);

impl Cuts {
    /// Rebuilds tried after a cut before the devices are given up: a service restarting or
    /// a daemon reloaded answers within a few seconds, and one that does not is not coming back.
    const RECONNECT_TRIES: u8 = 3;

    pub(super) fn one_more(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// The first cut is the loss itself; each rebuild that fails or is cut again is another.
    pub(super) fn spent(self) -> bool {
        self.0 > Self::RECONNECT_TRIES
    }
}

/// Where the devices are. Whether an emitter exists is the shell's to keep: it holds one exactly
/// while the state is `Ready`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Life {
    Ready {
        cuts: Cuts,
    },
    /// A write failed (the device was destroyed, the module reloaded): the next `Due` starts the
    /// wait for a rebuilt one.
    Lost {
        cuts: Cuts,
    },
    Waiting {
        retry_at: Mono,
        backoff: Backoff,
        cuts: Cuts,
    },
    /// The service kept cutting palmrest off: frames are dropped until the shape changes.
    Dead,
}

impl Life {
    /// The wait before the first build, due at `retry_at`.
    pub(super) fn start(retry_at: Mono) -> Self {
        Self::Waiting {
            retry_at,
            backoff: Backoff::UINPUT,
            cuts: Cuts::default(),
        }
    }
}

/// Why a write or a build failed, as far as the lifecycle cares: whether the service cut
/// palmrest off, and the words for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Why {
    Cut(String),
    Other(String),
}

impl Why {
    pub(super) fn of(err: &OutputError) -> Self {
        match err {
            OutputError::Cut(_) => Self::Cut(err.to_string()),
            _ => Self::Other(err.to_string()),
        }
    }

    pub(super) fn text(&self) -> &str {
        match self {
            Self::Cut(text) | Self::Other(text) => text,
        }
    }

    /// `cuts` after this failure: only a cut counts.
    pub(super) fn tally(&self, cuts: Cuts) -> Cuts {
        match self {
            Self::Cut(_) => cuts.one_more(),
            Self::Other(_) => cuts,
        }
    }
}

/// What happened to the devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Event {
    /// The ready emitter wrote a frame.
    Written,
    /// The ready emitter failed to write one.
    WriteFailed(Why),
    /// The factory made an emitter, and the shell holds it.
    Created,
    CreateFailed {
        why: Why,
        at: Mono,
    },
    /// The clock reached `Mono`: a retry may be due.
    Due(Mono),
    /// The settings changed what the devices must be.
    ShapeChanged(Mono),
}

/// What the shell must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Effect {
    /// Ask the factory for an emitter, and report `Created` or `CreateFailed` at `at`.
    Create { at: Mono },
    /// Drop the emitter, which destroys its devices.
    DropEmitter,
    /// One line for the log.
    Log(String),
    /// The devices went away under what the tracker believes is down on them.
    Lost,
}
