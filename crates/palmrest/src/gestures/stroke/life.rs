//! The phases of one stroke as the shell stream guarantees them: MayBegin when a set of one or
//! two fingers lands; Began when the intent is decided; Changed per motion; Ended when the set
//! ends, changes or the lock times out; Cancelled on a click or a palm. Every MayBegin is closed
//! by exactly one Ended or Cancelled, so a host never holds a gesture open.

use super::lock::Signal;
use super::{Event, Intent};
use palmrest_protocol::{GestureId, Phase};

/// How far the current gesture has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// MayBegin sent, no decision yet.
    Announced,
    Began(Intent),
    /// The last gesture was Cancelled and the fingers are still down; the next decision opens
    /// a new gesture.
    Quiet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Open {
    id: GestureId,
    fingers: u8,
    stage: Stage,
}

impl Open {
    fn event(self, phase: Phase) -> Event {
        let intent = match self.stage {
            Stage::Began(intent) => Some(intent),
            Stage::Announced | Stage::Quiet => None,
        };
        Event::bare(self.id, self.fingers, phase, intent)
    }
}

/// The gesture between reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Life {
    open: Option<Open>,
    next_id: u32,
}

impl Default for Life {
    fn default() -> Self {
        Self {
            open: None,
            next_id: 1,
        }
    }
}

type Step = (Life, Vec<Event>);

impl Life {
    /// Turns the lock's signals of one report into the phases they cause.
    pub fn apply(self, signals: Vec<Signal>) -> Step {
        signals
            .into_iter()
            .fold((self, vec![]), |(life, events), s| {
                let (life, more) = life.signal(s);
                (life, events.into_iter().chain(more).collect())
            })
    }

    /// Cancels the running gesture (P3 palm, P4 click, disconnect); the fingers stay down.
    pub fn cancel(self) -> Step {
        match self.open {
            Some(open) if open.stage != Stage::Quiet => (
                Self {
                    open: Some(Open {
                        id: GestureId(self.next_id),
                        fingers: open.fingers,
                        stage: Stage::Quiet,
                    }),
                    next_id: self.next_id + 1,
                },
                vec![open.event(Phase::Cancelled)],
            ),
            _ => (self, vec![]),
        }
    }

    /// Cancels and forgets everything (device gone).
    pub fn disconnect(self) -> Step {
        let (life, events) = self.cancel();
        (Self { open: None, ..life }, events)
    }

    fn signal(self, s: Signal) -> Step {
        match s {
            Signal::Reset => self.close(),
            Signal::Start { fingers } => and_then(self.close(), |l| l.announce(fingers)),
            Signal::Unlocked => {
                let fingers = self.open.map_or(1, |o| o.fingers);
                and_then(self.close(), |l| l.announce(fingers))
            }
            Signal::Locked { intent, lead } => self.locked(intent, lead),
            Signal::Moved { motion, dt_s } => match self.open {
                Some(Open {
                    stage: Stage::Began(intent),
                    id,
                    fingers,
                }) => (
                    self,
                    vec![Event {
                        motion,
                        dt_s,
                        ..Event::bare(id, fingers, Phase::Changed, Some(intent))
                    }],
                ),
                _ => (self, vec![]),
            },
        }
    }

    fn locked(self, intent: Intent, lead: super::Motion) -> Step {
        let Some(open) = self.open else {
            return (self, vec![]);
        };
        let may_begin = (open.stage == Stage::Quiet).then(|| open.event(Phase::MayBegin));
        let began = Open {
            stage: Stage::Began(intent),
            ..open
        };
        let event = Event {
            motion: lead,
            ..began.event(Phase::Began)
        };
        (
            Self {
                open: Some(began),
                ..self
            },
            may_begin.into_iter().chain([event]).collect(),
        )
    }

    fn close(self) -> Step {
        let ended = self
            .open
            .filter(|o| o.stage != Stage::Quiet)
            .map(|o| o.event(Phase::Ended));
        (Self { open: None, ..self }, ended.into_iter().collect())
    }

    fn announce(self, fingers: u8) -> Step {
        let open = Open {
            id: GestureId(self.next_id),
            fingers,
            stage: Stage::Announced,
        };
        (
            Self {
                open: Some(open),
                next_id: self.next_id + 1,
            },
            vec![open.event(Phase::MayBegin)],
        )
    }
}

/// `first`'s events, then those of `next` run on the state `first` came to.
fn and_then(first: Step, next: impl FnOnce(Life) -> Step) -> Step {
    let (life, events) = first;
    let (life, more) = next(life);
    (life, events.into_iter().chain(more).collect())
}
