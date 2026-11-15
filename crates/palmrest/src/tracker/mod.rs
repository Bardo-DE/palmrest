//! `Tracker::step`: one sensor frame in, the outputs it causes out (12.4).
//!
//! The whole gesture state of the daemon is this one value. It never reads a clock or touches a
//! device: `now` is an argument and every effect is an `Out`, so a recording replays to exactly
//! the outputs the live daemon produced.
//!
//! It composes three things: the stroke recogniser (what the fingers did), the feel (what each
//! output makes of it: `strokes`) and the pad session (the fingers the virtual touchpad puts
//! down).

mod out;
mod strokes;

use out::while_unlocked;
pub use out::{Out, Source, action_outs, tap_outs};

use crate::action::Gesture;
use crate::config::{Config, Count, Fraction, Ms};
use crate::feel::shell::Carry;
use crate::gestures::{
    Bands, ClickGuard, Contacts, Hold, Intent, MotionWindow, OneSwipe, Params, Recogniser, Sample,
    StrokeEvent, SwipeFire, TapG,
};
use crate::outputs::pad::{Pad, PadFrame};
use crate::sensor::{Frame, Surface, magic_mouse};
use crate::time::Mono;
use palmrest_protocol::{ActiveTouch, LockState, Over, ToHost};
use strokes::Route;

/// Touches this soon after a click are ignored.
const CLICK_GUARD: Ms = Ms(200);
// TODO(untested): P5 is in the sensor's own mouse counts per second, so it means whatever
// another mouse's CPI makes it; only the Magic Mouse's counts are known.
/// P5: the mouse is moving above this speed, in the sensor's mouse counts per second...
const MOUSE_MOVING_THRESHOLD: Count = Count(200);
/// ...measured over this window...
const MOUSE_MOVING_WINDOW: Ms = Ms(100);
/// ...and the axis lock is this much harder to pass meanwhile (2.0).
const MOUSE_MOVING_LOCK_MULTIPLIER: Fraction = Fraction(2000);

/// Every piece of gesture state between reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracker {
    cfg: Config,
    surface: Surface,
    contacts: Contacts,
    click: ClickGuard,
    motion: MotionWindow,
    stroke: Recogniser,
    one: OneSwipe,
    swipe: SwipeFire,
    tap: TapG,
    carry: Carry,
    pad: Pad,
    claimed: Over,
    session: LockState,
    touching: ActiveTouch,
}

/// A tracker and the outputs the stages that made it caused.
type Staged = (Tracker, Vec<Out>);

/// One stage after another: the next runs on the tracker the last came to, and the outputs stay
/// in order.
trait Stages {
    fn then(self, next: impl FnOnce(Tracker) -> Staged) -> Staged;
}

impl Stages for Staged {
    fn then(self, next: impl FnOnce(Tracker) -> Staged) -> Staged {
        let (tracker, outs) = self;
        let (tracker, more) = next(tracker);
        (tracker, outs.into_iter().chain(more).collect())
    }
}

impl Tracker {
    /// A tracker for the Magic Mouse 2's surface.
    pub fn new(cfg: Config) -> Self {
        Self::on(cfg, magic_mouse::surface())
    }

    /// A tracker for the frames of a sensor with this surface.
    pub fn on(cfg: Config, surface: Surface) -> Self {
        Self::idle(cfg, surface, Over::NotOurs, LockState::Unlocked)
    }

    /// Nothing running: no contact, no stroke, nothing down on the pad. What outlives the mouse
    /// is named: the settings, the surface, who holds the pointer and the lock screen's state.
    fn idle(cfg: Config, surface: Surface, claimed: Over, session: LockState) -> Self {
        Self {
            cfg,
            surface,
            contacts: Contacts::default(),
            click: ClickGuard::Idle,
            motion: MotionWindow::default(),
            stroke: Recogniser::default(),
            one: OneSwipe::Idle,
            swipe: SwipeFire::default(),
            tap: TapG::Idle,
            carry: Carry::default(),
            pad: Pad::Up,
            claimed,
            session,
            touching: ActiveTouch::Up,
        }
    }

    /// Frames now come from a sensor with this surface (a new device was opened).
    pub fn with_surface(self, surface: Surface) -> Self {
        Self { surface, ..self }
    }

    /// The surface the frames come from.
    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    /// The configuration in force.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Applies a reloaded configuration; running gestures continue under it.
    pub fn with_config(self, cfg: Config) -> Self {
        Self { cfg, ..self }
    }

    /// What the stroke recogniser has decided after the last report (for parity comparison).
    pub fn intent(&self) -> Option<Intent> {
        self.stroke.intent()
    }

    /// The lock screen's state, as the shell last said it. While the session is locked no
    /// gesture runs a key or a desktop action and no Spaces swipe starts; scrolling and the
    /// socket stream go on.
    pub fn with_session_lock(self, session: LockState) -> Self {
        Self { session, ..self }
    }

    /// Who holds the pointer.
    pub fn claimed(&self) -> Over {
        self.claimed
    }

    /// When `tick` next has work to do, if ever: a delayed touchpad lift.
    pub fn deadline(&self) -> Option<Mono> {
        self.pad.deadline()
    }

    /// A host claimed or released the pointer (the aggregate over subscribers). A scroll on the
    /// virtual touchpad lifts at once when a host takes over, so nothing is left pressed under
    /// our own surfaces. A Spaces swipe is the compositor's and goes on.
    pub fn claim(self, over: Over) -> (Self, Vec<Out>) {
        let (pad, frames) = match over {
            Over::Ours => self.pad.lift_scroll(),
            Over::NotOurs => (self.pad, vec![]),
        };
        (
            Self {
                claimed: over,
                pad,
                ..self
            },
            pad_outs(frames),
        )
    }

    /// The virtual devices were lost and rebuilt: nothing is down on them any more, so the pad
    /// is forgotten without writing anything.
    pub fn devices_lost(self) -> Self {
        Self {
            pad: Pad::Up,
            ..self
        }
    }

    /// Time passed with no report: a delayed touchpad lift that has come due.
    pub fn tick(self, now: Mono) -> (Self, Vec<Out>) {
        let (pad, frames) = self.pad.tick(now);
        (Self { pad, ..self }, pad_outs(frames))
    }

    /// The mouse went away: cancel what is running, lift everything, say so, and start over
    /// with only what outlives the mouse.
    pub fn disconnect(self, now: Mono) -> (Self, Vec<Out>) {
        let (stroke, events) = self.stroke.disconnect();
        let (s, cancelled) = Self { stroke, ..self }.apply(&events, now);
        let (_, frames) = s.pad.lift_now();
        let up = (s.touching == ActiveTouch::Down).then_some(Out::Host(ToHost::Touches {
            active: ActiveTouch::Up,
        }));
        let outs = cancelled
            .into_iter()
            .chain(pad_outs(frames))
            .chain(up)
            .chain([Out::Host(ToHost::DeviceGone)])
            .collect();
        (Self::idle(s.cfg, s.surface, s.claimed, s.session), outs)
    }

    /// Consumes one frame.
    pub fn step(self, r: &Frame, now: Mono) -> (Self, Vec<Out>) {
        let (click, press) = self.click.step(r.buttons, now, CLICK_GUARD.duration());
        let window = MOUSE_MOVING_WINDOW.duration();
        let motion = self.motion.step(r.motion, now, window);
        let moving = motion.moving(f64::from(MOUSE_MOVING_THRESHOLD.0), window);
        let params = Params::standard(if moving {
            MOUSE_MOVING_LOCK_MULTIPLIER.ratio()
        } else {
            1.0
        });
        let (contacts, seen) = self
            .contacts
            .step(&r.contacts, now, &Bands::resolve(&self.surface));
        let hold = click.hold(now);
        let cancel = press || seen.palm_turned;
        let samples = seen.samples;
        let s = Self {
            click,
            motion,
            contacts,
            ..self
        };

        s.tick(now)
            .then(Self::release_swipe_while_locked)
            .then(|s| s.touches(active(&samples), ActiveTouch::Down))
            .then(|s| s.cancel_if(cancel, now))
            .then(|s| s.recognise(&samples, now, &params, hold))
            .then(|s| s.gestures(&samples, now, hold))
            .then(|s| s.touches(active(&samples), ActiveTouch::Up))
    }

    /// `Touches{Down}` before this report's stroke phases, `Touches{Up}` after them: `next` is
    /// this report's state, `which` the edge this call may emit.
    fn touches(self, next: ActiveTouch, which: ActiveTouch) -> Staged {
        if next == which && next != self.touching {
            (
                Self {
                    touching: next,
                    ..self
                },
                vec![Out::Host(ToHost::Touches { active: next })],
            )
        } else {
            (self, vec![])
        }
    }

    fn cancel_if(self, cancel: bool, now: Mono) -> Staged {
        if !cancel {
            return (self, vec![]);
        }
        let (stroke, events) = self.stroke.cancel();
        Self { stroke, ..self }.apply(&events, now)
    }

    fn recognise(self, samples: &[Sample], now: Mono, params: &Params, hold: Hold) -> Staged {
        let (stroke, events) = self.stroke.step(samples, now, params, hold);
        Self { stroke, ..self }.apply(&events, now)
    }

    /// What the stroke's phases cause: the shell's scroll messages, the touchpad's fingers and,
    /// for a remapped two-finger swipe, the action it fires.
    fn apply(self, events: &[StrokeEvent], now: Mono) -> Staged {
        (self, vec![])
            .then(|s| s.route(events, now))
            .then(|s| s.fire(events, now))
    }

    fn route(self, events: &[StrokeEvent], now: Mono) -> Staged {
        let routing = Route {
            claimed: self.claimed,
            session: self.session,
            live_swipe: self
                .cfg
                .gestures
                .gesture_action_map
                .two_finger_swipe_is_live(),
            speed: self.cfg.scroll.speed,
        };
        let (pad, carry, outs) =
            events
                .iter()
                .fold((self.pad, self.carry, vec![]), |(pad, carry, outs), e| {
                    let (pad, drawn) = strokes::pad_step(pad, e, &routing, now);
                    let (carry, scroll) = strokes::host_scroll(carry, e, routing.speed, now);
                    let outs = outs.into_iter().chain(scroll).chain(drawn).collect();
                    (pad, carry, outs)
                });
        (Self { pad, carry, ..self }, outs)
    }

    /// The action of a remapped two-finger swipe, once per threshold of travel.
    fn fire(self, events: &[StrokeEvent], now: Mono) -> Staged {
        let map = &self.cfg.gestures.gesture_action_map;
        if map.two_finger_swipe_is_live() {
            return (self, vec![]);
        }
        let (swipe, fired) = events
            .iter()
            .fold((self.swipe, vec![]), |(swipe, fired), e| {
                let (swipe, dir) = swipe.step(e, now);
                (swipe, fired.into_iter().chain(dir).collect::<Vec<_>>())
            });
        let outs = fired
            .into_iter()
            .flat_map(|dir| {
                let action = map.action_for(Gesture::two_finger_swipe(dir));
                action_outs(action, Source::Other, self.claimed)
            })
            .collect();
        let session = self.session;
        (Self { swipe, ..self }, while_unlocked(session, outs))
    }

    /// G4 and the double-taps, after the recogniser has seen the report.
    fn gestures(self, samples: &[Sample], now: Mono, hold: Hold) -> Staged {
        let map = &self.cfg.gestures.gesture_action_map;
        let (one, g4) = self.one.step(samples, self.stroke.intent(), now, hold);
        let (tap, kind) = self.tap.step(samples, now, hold);
        let outs = g4
            .into_iter()
            .flat_map(|dir| {
                let action = map.action_for(Gesture::one_finger_swipe(dir));
                action_outs(action, Source::OneFingerSwipe, self.claimed)
            })
            .chain(kind.into_iter().flat_map(|kind| {
                let action = map.action_for(Gesture::tap(kind));
                tap_outs(kind, action, self.claimed, now.0)
            }))
            .collect();
        let session = self.session;
        (Self { one, tap, ..self }, while_unlocked(session, outs))
    }

    /// A locked session has no Spaces swipe: one that was running when the screen locked lifts.
    fn release_swipe_while_locked(self) -> Staged {
        match self.session {
            LockState::Unlocked => (self, vec![]),
            LockState::Locked => {
                let (pad, frames) = self.pad.lift_swipe();
                (Self { pad, ..self }, pad_outs(frames))
            }
        }
    }
}

fn pad_outs(frames: Vec<PadFrame>) -> Vec<Out> {
    frames.into_iter().map(Out::Pad).collect()
}

/// Whether any contact is down in this report.
fn active(samples: &[Sample]) -> ActiveTouch {
    if samples.is_empty() {
        ActiveTouch::Up
    } else {
        ActiveTouch::Down
    }
}
