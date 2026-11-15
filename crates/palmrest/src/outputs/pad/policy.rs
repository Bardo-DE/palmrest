//! What the pad may have down, given who holds the pointer and the lock screen.

use crate::gestures::Intent;
use palmrest_protocol::{LockState, Over};

/// The conditions under which an intent may be on the pad. The session asks `allows` before it
/// lands anything and lifts whatever stops being allowed (`Pad::enforce`), so every rule about
/// the pad being silent is here and nowhere else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    /// Who holds the pointer: a scroll is silent on the pad while a host does. A Spaces swipe
    /// is the compositor's, whoever holds the pointer.
    pub claimed: Over,
    /// A Spaces swipe is a desktop action, so it waits for an unlocked session.
    pub session: LockState,
    /// Whether the two-finger swipes are the workspace defaults, which the compositor runs live
    /// from the pad; any other mapping is fired by `SwipeFire` instead.
    pub live_swipe: bool,
}

impl Policy {
    /// Whether a stroke of `intent` may have fingers down now.
    pub fn allows(self, intent: Intent) -> bool {
        match intent {
            Intent::Scroll(_) => self.claimed == Over::NotOurs,
            Intent::SpaceSwipe => self.live_swipe && self.session == LockState::Unlocked,
        }
    }
}
