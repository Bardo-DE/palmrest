//! Telling the reader what the subscribers jointly say, when it changed.

use super::FromSocket;
use super::aggregate::{aggregate, aggregate_lock};
use super::model::{SocketEffect, Subscribers, Told};

impl Subscribers {
    /// The notes for the end of a pass: the person's settings if they were sent, then the pointer
    /// and the lock if their joint answer differs from what the reader last heard.
    pub(super) fn publish(self) -> (Self, Vec<SocketEffect>) {
        let pointer = aggregate(self.members.values().map(|m| m.pointer(self.active)));
        let lock = aggregate_lock(self.members.values().map(|m| m.session(self.active)));
        let notes = self
            .config
            .map(FromSocket::UserConfig)
            .into_iter()
            .chain((pointer != self.told.pointer).then_some(FromSocket::Pointer(pointer)))
            .chain((lock != self.told.lock).then_some(FromSocket::Session(lock)))
            .map(SocketEffect::Note)
            .collect();
        let told = Told { pointer, lock };
        (
            Self {
                told,
                config: None,
                ..self
            },
            notes,
        )
    }
}
