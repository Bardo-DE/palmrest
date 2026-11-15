//! What the subscribers jointly say: one answer from many.

use palmrest_protocol::{LockState, Over};

/// The pointer is ours if any subscriber says so.
pub(super) fn aggregate(overs: impl IntoIterator<Item = Over>) -> Over {
    if overs.into_iter().any(|o| o == Over::Ours) {
        Over::Ours
    } else {
        Over::NotOurs
    }
}

/// Whether the session counts as locked, from what each subscriber last said (`None`: nothing
/// yet). Locked is the safe answer, so it wins whenever it is in doubt: any subscriber says
/// locked, or nobody has said unlocked (no subscriber is connected, or the ones connected have
/// said nothing). A lock screen the shell has not reported must never let a gesture run a key or
/// a desktop command. Only a subscriber saying unlocked, with none saying locked, opens it.
pub(super) fn aggregate_lock(locks: impl IntoIterator<Item = Option<LockState>>) -> LockState {
    let said: Vec<LockState> = locks.into_iter().flatten().collect();
    if said.contains(&LockState::Unlocked) && !said.contains(&LockState::Locked) {
        LockState::Unlocked
    } else {
        LockState::Locked
    }
}
