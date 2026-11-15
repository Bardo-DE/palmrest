//! The phases of one stroke as the shell stream guarantees them: MayBegin when a set of one or
//! two fingers lands; Began when the intent is decided; Changed per motion; Ended when the set
//! ends, changes or a scroll's lock times out; Cancelled on a click or a palm. Every MayBegin is
//! closed by exactly one Ended or Cancelled, so a host never holds a gesture open.

use super::lock::Signal;
use super::{Event, Intent, Moment, Motion};
use palmrest_protocol::GestureId;

/// How far the current gesture has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// MayBegin sent, no decision yet.
    Announced,
    Began(Intent),
    /// The last gesture was Cancelled and the fingers are still down; the next decision opens
    /// a new gesture, announcing it first.
    Quiet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Open {
    id: GestureId,
    fingers: u8,
    stage: Stage,
}

impl Open {
    fn event(self, moment: Moment) -> Event {
        Event {
            gesture: self.id,
            fingers: self.fingers,
            moment,
        }
    }

    /// The intent, once decided.
    fn intent(self) -> Option<Intent> {
        match self.stage {
            Stage::Began(intent) => Some(intent),
            Stage::Announced | Stage::Quiet => None,
        }
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
        let mut life = self;
        let mut events = vec![];
        for s in signals {
            let (next, more) = life.signal(s);
            life = next;
            events.extend(more);
        }
        (life, events)
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
                vec![open.event(Moment::Cancelled {
                    intent: open.intent(),
                })],
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
            Signal::Start { fingers } => self.reopen(fingers),
            Signal::Unlocked => {
                let fingers = self.open.map_or(1, |o| o.fingers);
                self.reopen(fingers)
            }
            Signal::Locked { intent, lead } => self.locked(intent, lead),
            Signal::Moved { motion, dt_s } => match self.open {
                Some(Open {
                    stage: Stage::Began(intent),
                    id,
                    fingers,
                }) => {
                    let moment = Moment::Changed {
                        intent,
                        motion,
                        dt_s,
                    };
                    let event = Event {
                        gesture: id,
                        fingers,
                        moment,
                    };
                    (self, vec![event])
                }
                _ => (self, vec![]),
            },
        }
    }

    /// Closes the running gesture and announces a new one of `fingers`.
    fn reopen(self, fingers: u8) -> Step {
        let (life, mut events) = self.close();
        let (life, announced) = life.announce(fingers);
        events.extend(announced);
        (life, events)
    }

    fn locked(self, intent: Intent, lead: Motion) -> Step {
        let Some(open) = self.open else {
            return (self, vec![]);
        };
        let may_begin = (open.stage == Stage::Quiet).then(|| open.event(Moment::Announced));
        let began = Open {
            stage: Stage::Began(intent),
            ..open
        };
        let event = began.event(Moment::Began { intent, lead });
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
            .map(|o| o.event(Moment::Ended { intent: o.intent() }));
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
            vec![open.event(Moment::Announced)],
        )
    }
}
