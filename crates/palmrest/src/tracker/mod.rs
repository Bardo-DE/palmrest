//! `Tracker::step`: one sensor frame in, the outputs it causes out (12.4).
//!
//! The whole gesture state of the daemon is this one value. It never reads a clock or touches a
//! device: `now` is an argument and every effect is an `Out`, so a recording replays to exactly
//! the outputs the live daemon produced.
//!
//! It composes three things: the stroke recogniser (what the fingers did), the feel (what each
//! output makes of it: `strokes`) and the pad session (the fingers the virtual touchpad puts
//! down, and the policy that says when it may).

mod out;
mod strokes;

use out::while_unlocked;
pub use out::{Out, Source, action_outs, tap_outs};

use crate::action::Gesture;
use crate::config::{Config, Count, Fraction, Ms};
use crate::gestures::{
    Bands, ClickGuard, Contacts, Hold, Intent, MotionWindow, OneSwipe, Params, Recogniser, Sample,
    SwipeFire, TapG,
};
use crate::outputs::pad::{Pad, PadFrame};
use crate::sensor::{Frame, Surface, magic_mouse};
use crate::time::Mono;
use palmrest_protocol::{ActiveTouch, LockState, Over, ToHost};

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
    pad: Pad,
    claimed: Over,
    session: LockState,
    touching: ActiveTouch,
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

    /// Applies a reloaded configuration; running gestures continue under it.
    pub fn with_config(self, cfg: Config) -> Self {
        Self { cfg, ..self }
    }

    /// What the stroke recogniser has decided after the last report (for parity comparison).
    pub fn intent(&self) -> Option<Intent> {
        self.stroke.intent()
    }

    /// When `tick` next has work to do, if ever: a delayed touchpad lift.
    pub fn deadline(&self) -> Option<Mono> {
        self.pad.deadline()
    }

    /// The lock screen's state, as the shell last said it. While the session is locked no
    /// gesture runs a key or a desktop action and no Spaces swipe starts; a swipe that was
    /// running lifts at once. Scrolling and the socket stream go on.
    pub fn with_session_lock(self, session: LockState, now: Mono) -> (Self, Vec<Out>) {
        Self { session, ..self }.enforce(now)
    }

    /// A host claimed or released the pointer (the aggregate over subscribers). A scroll on the
    /// virtual touchpad lifts when a host takes over, so nothing is left pressed under our own
    /// surfaces: at once, or still at `MIN_DOWN` if it is younger, so it is no tap. A Spaces
    /// swipe is the compositor's and goes on.
    pub fn claim(self, over: Over, now: Mono) -> (Self, Vec<Out>) {
        Self {
            claimed: over,
            ..self
        }
        .enforce(now)
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
        let (s, mut outs) = Self { stroke, ..self }.apply(&events, now);
        let (_, frames) = s.pad.lift_now();
        outs.extend(pad_outs(frames));
        if s.touching == ActiveTouch::Down {
            outs.push(touches(ActiveTouch::Up));
        }
        outs.push(Out::Host(ToHost::DeviceGone));
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
        let samples = seen.samples;
        let edge = touch_edge(self.touching, active(&samples));
        let s = Self {
            click,
            motion,
            contacts,
            touching: active(&samples),
            ..self
        };

        // `tick` runs first, so a lift that has come due is written before this report's motion.
        let (s, mut outs) = s.tick(now);
        outs.extend(edge_out(edge, ActiveTouch::Down));
        let (s, cancelled) = if press || seen.palm_turned {
            s.cancel(now)
        } else {
            (s, vec![])
        };
        outs.extend(cancelled);
        let (s, recognised) = s.recognise(&samples, now, &params, hold);
        outs.extend(recognised);
        let (s, gestured) = s.gestures(&samples, now, hold);
        outs.extend(gestured);
        outs.extend(edge_out(edge, ActiveTouch::Up));
        (s, outs)
    }

    /// Cancels the running stroke (a click or a palm); the fingers stay followed.
    fn cancel(self, now: Mono) -> (Self, Vec<Out>) {
        let (stroke, events) = self.stroke.cancel();
        Self { stroke, ..self }.apply(&events, now)
    }

    fn recognise(
        self,
        samples: &[Sample],
        now: Mono,
        params: &Params,
        hold: Hold,
    ) -> (Self, Vec<Out>) {
        let (stroke, events) = self.stroke.step(samples, now, params, hold);
        Self { stroke, ..self }.apply(&events, now)
    }

    /// G4 and the double-taps, after the recogniser has seen the report.
    fn gestures(self, samples: &[Sample], now: Mono, hold: Hold) -> (Self, Vec<Out>) {
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

/// The edge a report makes: Down when the first contact lands, Up when the last one lifts.
fn touch_edge(was: ActiveTouch, now: ActiveTouch) -> Option<ActiveTouch> {
    (was != now).then_some(now)
}

/// The `Touches` message for `edge` if it is the `which` edge: Down goes out before the report's
/// stroke phases, Up after them.
fn edge_out(edge: Option<ActiveTouch>, which: ActiveTouch) -> Option<Out> {
    edge.filter(|e| *e == which).map(touches)
}

fn touches(active: ActiveTouch) -> Out {
    Out::Host(ToHost::Touches { active })
}
