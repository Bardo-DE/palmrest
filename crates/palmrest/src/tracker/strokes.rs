//! A stroke event's outputs: the socket message for the shell's own surfaces, and the virtual
//! touchpad for every other client. Each sink gets its own feel (`feel::shell`, `feel::pad`)
//! from the same physical motion. The pad's policy (`Policy`) is the one place that decides
//! what it may have down; the tracker only hands it the facts.

use super::out::{Out, Source, action_outs, while_unlocked};
use super::{Tracker, pad_outs};
use crate::action::Gesture;
use crate::config::Count;
use crate::feel::pad as pad_feel;
use crate::feel::shell;
use crate::gestures::{Intent, Moment, Motion, StrokeEvent};
use crate::outputs::pad::{Pad, Policy};
use crate::time::Mono;
use palmrest_protocol::{Axis, Phase, ToHost};

/// The scroll message of `e` for the shell, if it has one. A Spaces swipe is not a scroll: the
/// shell sees it as a gesture that opens and closes without ever beginning, as it always has,
/// so its Began and Changed have no message while its MayBegin and Ended do.
pub fn host_scroll(e: &StrokeEvent, speed: Count, now: Mono) -> Option<Out> {
    let (phase, axis, (dx_px, dy_px)) = match e.moment {
        Moment::Announced => (Phase::MayBegin, None, (0.0, 0.0)),
        Moment::Began {
            intent: Intent::Scroll(axis),
            ..
        } => (Phase::Began, Some(axis), (0.0, 0.0)),
        Moment::Changed {
            intent: Intent::Scroll(axis),
            motion,
            dt_s,
        } => (
            Phase::Changed,
            Some(axis),
            scrolled(axis, motion, speed, dt_s),
        ),
        Moment::Began {
            intent: Intent::SpaceSwipe,
            ..
        }
        | Moment::Changed {
            intent: Intent::SpaceSwipe,
            ..
        } => return None,
        Moment::Ended { intent } => (Phase::Ended, scroll_axis(intent), (0.0, 0.0)),
        Moment::Cancelled { intent } => (Phase::Cancelled, scroll_axis(intent), (0.0, 0.0)),
    };
    Some(Out::Host(ToHost::Scroll {
        gesture: e.gesture,
        phase,
        dx_px,
        dy_px,
        fingers: e.fingers,
        axis,
        t_ns: now.0,
    }))
}

/// The axis of a scroll intent.
fn scroll_axis(intent: Option<Intent>) -> Option<Axis> {
    match intent {
        Some(Intent::Scroll(axis)) => Some(axis),
        Some(Intent::SpaceSwipe) | None => None,
    }
}

/// The px a scroll's motion makes on its own axis.
fn scrolled(axis: Axis, motion: Motion, speed: Count, dt_s: f64) -> (f32, f32) {
    match axis {
        Axis::X => (shell::px(motion.x, Axis::X, speed, dt_s) as f32, 0.0),
        Axis::Y => (0.0, shell::px(motion.y, Axis::Y, speed, dt_s) as f32),
    }
}

/// The pad's frames for `e`.
pub fn pad_step(
    pad: Pad,
    e: &StrokeEvent,
    policy: Policy,
    speed: Count,
    now: Mono,
) -> (Pad, Vec<Out>) {
    let (pad, frames) = match e.moment {
        Moment::Began { intent, lead } if policy.allows(intent) => pad.land(intent, lead, now),
        Moment::Changed { intent, motion, .. } => {
            pad.moved(intent, travel(intent, motion, speed), now)
        }
        Moment::Ended { .. } => pad.end(now),
        Moment::Cancelled { .. } => pad.cancel(now),
        Moment::Announced | Moment::Began { .. } => (pad, vec![]),
    };
    (pad, frames.into_iter().map(Out::Pad).collect())
}

/// The pad's motion: a scroll's is the fingers' times the speed multiplier, a swipe's is the
/// fingers' exactly.
fn travel(intent: Intent, motion: Motion, speed: Count) -> Motion {
    match intent {
        Intent::Scroll(_) => motion.map(|mm| pad_feel::travel(mm, speed)),
        Intent::SpaceSwipe => motion,
    }
}

impl Tracker {
    /// What the stroke's phases cause: the shell's scroll messages, the touchpad's fingers and,
    /// for a remapped two-finger swipe, the action it fires.
    pub(super) fn apply(self, events: &[StrokeEvent], now: Mono) -> (Self, Vec<Out>) {
        let (s, mut outs) = self.route(events, now);
        let (s, fired) = s.fire(events, now);
        outs.extend(fired);
        (s, outs)
    }

    fn route(self, events: &[StrokeEvent], now: Mono) -> (Self, Vec<Out>) {
        let policy = self.policy();
        let speed = self.cfg.scroll.speed;
        let mut pad = self.pad;
        let mut outs = vec![];
        for e in events {
            outs.extend(host_scroll(e, speed, now));
            let (next, drawn) = pad_step(pad, e, policy, speed, now);
            pad = next;
            outs.extend(drawn);
        }
        (Self { pad, ..self }, outs)
    }

    /// The action of a remapped two-finger swipe, once per threshold of travel.
    fn fire(self, events: &[StrokeEvent], now: Mono) -> (Self, Vec<Out>) {
        let map = &self.cfg.gestures.gesture_action_map;
        if map.two_finger_swipe_is_live() {
            return (self, vec![]);
        }
        let mut swipe = self.swipe;
        let mut outs = vec![];
        for e in events {
            let (next, fired) = swipe.step(e, now);
            swipe = next;
            for dir in fired {
                let action = map.action_for(Gesture::two_finger_swipe(dir));
                outs.extend(action_outs(action, Source::Other, self.claimed));
            }
        }
        let outs = while_unlocked(self.session, outs);
        (Self { swipe, ..self }, outs)
    }

    /// What the pad may have down under the pointer's holder, the lock screen and the settings.
    fn policy(&self) -> Policy {
        Policy {
            claimed: self.claimed,
            session: self.session,
            live_swipe: self
                .cfg
                .gestures
                .gesture_action_map
                .two_finger_swipe_is_live(),
        }
    }

    /// Lifts at once whatever the pad's policy no longer allows (after a claim or a lock).
    pub(super) fn enforce(self) -> (Self, Vec<Out>) {
        let (pad, frames) = self.pad.enforce(self.policy());
        (Self { pad, ..self }, pad_outs(frames))
    }
}
