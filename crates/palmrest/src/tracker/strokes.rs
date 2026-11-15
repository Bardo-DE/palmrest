//! A stroke event's outputs: the socket message for the shell's own surfaces, and the virtual
//! touchpad for every other client. Each sink gets its own feel (`feel::shell`, `feel::pad`)
//! from the same physical motion.

use super::out::Out;
use crate::config::Count;
use crate::feel::pad as pad_feel;
use crate::feel::shell::{self, Carry};
use crate::gestures::{Intent, StrokeEvent};
use crate::outputs::pad::Pad;
use crate::time::Mono;
use palmrest_protocol::{Axis, LockState, Over, Phase, ToHost};

/// What decides whether the pad takes a stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Route {
    /// Who holds the pointer: a scroll is silent on the pad while a host does.
    pub claimed: Over,
    /// A Spaces swipe is a desktop action, so it waits for an unlocked session.
    pub session: LockState,
    /// Whether the two-finger swipes are the workspace defaults, which the compositor runs live
    /// from the pad; any other mapping is fired by `SwipeFire` instead.
    pub live_swipe: bool,
    /// `scroll.speed`.
    pub speed: Count,
}

/// The scroll message of `e` for the shell, if it has one, and the carry after it. A Spaces
/// swipe is not a scroll: the shell sees it as a gesture that opens and closes without ever
/// beginning, as it always has.
pub fn host_scroll(carry: Carry, e: &StrokeEvent, speed: Count, now: Mono) -> (Carry, Option<Out>) {
    let axis = match e.intent {
        Some(Intent::Scroll(axis)) => Some(axis),
        Some(Intent::SpaceSwipe) | None => None,
    };
    let carry = if e.phase == Phase::Began {
        Carry::default()
    } else {
        carry
    };
    let (carry, (dx_px, dy_px)) = match (e.phase, axis) {
        (Phase::Changed, Some(Axis::X)) => {
            let (carry, px) = shell::px(carry, e.motion.x, Axis::X, speed, e.dt_s);
            (carry, (px as f32, 0.0))
        }
        (Phase::Changed, Some(Axis::Y)) => {
            let (carry, px) = shell::px(carry, e.motion.y, Axis::Y, speed, e.dt_s);
            (carry, (0.0, px as f32))
        }
        _ => (carry, (0.0, 0.0)),
    };
    let shown = match e.phase {
        Phase::MayBegin | Phase::Ended | Phase::Cancelled => true,
        Phase::Began | Phase::Changed => axis.is_some(),
    };
    let out = shown.then_some(Out::Host(ToHost::Scroll {
        gesture: e.gesture,
        phase: e.phase,
        dx_px,
        dy_px,
        fingers: e.fingers,
        axis,
        t_ns: now.0,
    }));
    (carry, out)
}

/// The pad's frames for `e`.
pub fn pad_step(pad: Pad, e: &StrokeEvent, route: &Route, now: Mono) -> (Pad, Vec<Out>) {
    let (pad, frames) = match (e.phase, e.intent) {
        (Phase::Began, Some(intent)) if lands(intent, route) => pad.land(intent, e.motion, now),
        (Phase::Changed, Some(intent)) => pad.moved(intent, travel(intent, e, route), now),
        (Phase::Ended | Phase::Cancelled, _) => pad.end(now),
        _ => (pad, vec![]),
    };
    (pad, frames.into_iter().map(Out::Pad).collect())
}

/// Whether a stroke of `intent` puts fingers on the pad now.
fn lands(intent: Intent, route: &Route) -> bool {
    match intent {
        Intent::Scroll(_) => route.claimed == Over::NotOurs,
        Intent::SpaceSwipe => route.live_swipe && route.session == LockState::Unlocked,
    }
}

/// The pad's motion: a scroll's is the fingers' times the speed multiplier, a swipe's is the
/// fingers' exactly.
fn travel(intent: Intent, e: &StrokeEvent, route: &Route) -> crate::gestures::Motion {
    match intent {
        Intent::Scroll(_) => e.motion.map(|mm| pad_feel::travel(mm, route.speed)),
        Intent::SpaceSwipe => e.motion,
    }
}
