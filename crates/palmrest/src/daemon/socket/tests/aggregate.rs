//! How many subscribers' words become one.

use super::super::aggregate::{aggregate, aggregate_lock};
use palmrest_protocol::{LockState, Over};

#[test]
fn the_session_is_locked_unless_a_subscriber_has_said_unlocked_and_none_says_locked() {
    use LockState::{Locked, Unlocked};
    let table: &[(&str, Vec<Option<LockState>>, LockState)] = &[
        ("no subscriber is connected", vec![], Locked),
        ("one that has said nothing", vec![None], Locked),
        ("several that have said nothing", vec![None, None], Locked),
        ("one that says unlocked", vec![Some(Unlocked)], Unlocked),
        (
            "unlocked beside one that is silent",
            vec![None, Some(Unlocked)],
            Unlocked,
        ),
        ("one that says locked", vec![Some(Locked)], Locked),
        (
            "locked beats unlocked",
            vec![Some(Unlocked), Some(Locked)],
            Locked,
        ),
        (
            "locked beats unlocked, either order",
            vec![Some(Locked), Some(Unlocked), None],
            Locked,
        ),
    ];
    for (name, said, want) in table {
        assert_eq!(aggregate_lock(said.iter().copied()), *want, "{name}");
    }
}

#[test]
fn any_claim_wins() {
    assert_eq!(aggregate([]), Over::NotOurs);
    assert_eq!(aggregate([Over::NotOurs, Over::Ours]), Over::Ours);
    assert_eq!(aggregate([Over::NotOurs, Over::NotOurs]), Over::NotOurs);
}
