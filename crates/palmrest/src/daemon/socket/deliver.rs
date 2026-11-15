//! What the reader's messages do: whom each goes to.

use super::ToSocket;
use super::model::{SocketEffect, Subscribers};
use super::seat::Peer;
use palmrest_protocol::{HostAction, PROTOCOL_VERSION, ToHost};

impl Subscribers {
    pub(super) fn deliver(self, message: ToSocket) -> (Self, Vec<SocketEffect>) {
        match message {
            ToSocket::Device(device) => {
                let hello = ToHost::Hello {
                    version: PROTOCOL_VERSION,
                    device: device.clone(),
                };
                let sends = self.broadcast(&hello);
                (
                    Self {
                        device: Some(device),
                        ..self
                    },
                    sends,
                )
            }
            ToSocket::Gone => (
                Self {
                    device: None,
                    ..self
                },
                vec![],
            ),
            ToSocket::Msg(message) => {
                let sends = self.broadcast(&message);
                (self, sends)
            }
            ToSocket::Run(action) => {
                let sends = self.ask_the_shell(action);
                (self, sends)
            }
        }
    }

    /// `m` to every subscriber whose filter admits it.
    fn broadcast(&self, m: &ToHost) -> Vec<SocketEffect> {
        self.members
            .iter()
            .filter(|(_, member)| member.filter.admits(m))
            .map(|(&id, _)| SocketEffect::Send { id, msg: m.clone() })
            .collect()
    }

    /// Asks the shell of the person at the seat to carry out `action`, whatever its filter says. No
    /// other subscriber is asked: it would run a program for someone who is not the shell.
    fn ask_the_shell(&self, action: HostAction) -> Vec<SocketEffect> {
        let msg = ToHost::Run { action };
        let asked: Vec<SocketEffect> = self
            .members
            .iter()
            .filter(|(_, member)| member.peer(self.active) == Peer::Shell)
            .map(|(&id, _)| SocketEffect::Send {
                id,
                msg: msg.clone(),
            })
            .collect();
        if asked.is_empty() {
            vec![SocketEffect::Log(
                "a gesture's action was dropped: no shell is connected".into(),
            )]
        } else {
            asked
        }
    }
}
