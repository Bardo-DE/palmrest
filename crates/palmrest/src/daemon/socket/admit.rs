//! Letting a connection in after its greeting, or closing it at once.

use super::limits::MAX_CLIENTS;
use super::model::{ClientId, SocketEffect, Subscriber, Subscribers};
use super::seat::Peer;
use latchkey::guard::{Claim, Voice};
use palmrest_protocol::{DeviceInfo, PROTOCOL_VERSION, ToHost};
use std::iter::once;
use std::time::Instant;

impl Subscribers {
    /// Admits the connection when it is the person at the seat's and the daemon has room; closes
    /// it otherwise. An admitted one is greeted with the mouse, or told there is none.
    pub(super) fn admit(
        mut self,
        id: ClientId,
        uid: u32,
        claim: Claim,
        now: Instant,
    ) -> (Self, Vec<SocketEffect>) {
        let newcomer = Subscriber::new(uid, claim);
        if newcomer.peer(self.active) == Peer::Observer {
            return (self, vec![SocketEffect::Close { id }]);
        }
        if self.members.len() >= MAX_CLIENTS {
            let (complaints, voice) = self.complaints.note(now);
            let log = (voice == Voice::Speak).then(|| {
                SocketEffect::Log(format!("refused a subscriber: {MAX_CLIENTS} are connected"))
            });
            let effects = once(SocketEffect::Close { id }).chain(log).collect();
            return (Self { complaints, ..self }, effects);
        }
        let greeting: Vec<SocketEffect> = self
            .greeting()
            .map(|msg| SocketEffect::Send { id, msg })
            .collect();
        self.members.insert(id, newcomer);
        (self, greeting)
    }

    /// The `Hello` that opens every connection (an empty device when there is no mouse), and
    /// `DeviceGone` after it when there is none.
    fn greeting(&self) -> impl Iterator<Item = ToHost> {
        let device = self.device.clone().unwrap_or_else(|| DeviceInfo {
            name: String::new(),
            path: String::new(),
            vendor: 0,
            product: 0,
        });
        let hello = ToHost::Hello {
            version: PROTOCOL_VERSION,
            device,
        };
        once(hello).chain(self.device.is_none().then_some(ToHost::DeviceGone))
    }
}
