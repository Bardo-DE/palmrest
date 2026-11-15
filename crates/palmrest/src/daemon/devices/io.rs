//! The shell around the lifecycle: it holds the emitter and the factory, feeds `step` what
//! happened and carries out what it asks.

use super::model::{Effect, Event, Life, Shape, Why};
use super::step::{first, step};
use crate::config::Config;
use crate::keys::KeyCombo;
use crate::outputs::pad::PadFrame;
use crate::outputs::{Emitter, OutputError};
use crate::time::Mono;

/// Makes the emitter for a configuration (the uinput one in the daemon).
pub type Factory = Box<dyn Fn(&[KeyCombo]) -> Result<Box<dyn Emitter>, OutputError>>;

/// The emitter, or the wait for one. A write that fails is returned once as an error; that error
/// is the news that the devices were lost, and frames meanwhile are dropped without a message.
pub struct Devices {
    factory: Factory,
    shape: Shape,
    life: Life,
    /// Held exactly while `life` is `Ready`.
    emitter: Option<Box<dyn Emitter>>,
}

impl Devices {
    pub fn new(factory: Factory, c: &Config, now: Mono) -> Self {
        let (life, effects) = first(now);
        let mut devices = Self {
            factory,
            shape: Shape::of(c),
            life,
            emitter: None,
        };
        devices.perform_all(effects);
        devices
    }

    /// Recreates the devices if the configuration changed what they must be.
    pub fn reconfigure(mut self, c: &Config, now: Mono) -> Self {
        let shape = Shape::of(c);
        if shape != self.shape {
            self.shape = shape;
            self.feed(Event::ShapeChanged(now));
        }
        self
    }

    /// Starts the wait after a failed write, and tries again if a retry is due.
    pub fn poll(mut self, now: Mono) -> Self {
        self.feed(Event::Due(now));
        self
    }

    /// Runs `write` on the ready emitter, if there is one.
    fn write(
        &mut self,
        write: impl FnOnce(&mut dyn Emitter) -> Result<(), OutputError>,
    ) -> Result<(), OutputError> {
        let Some(emitter) = self.emitter.as_mut() else {
            return Ok(());
        };
        let result = write(emitter.as_mut());
        let event = match &result {
            Ok(()) => Event::Written,
            Err(err) => Event::WriteFailed(Why::of(err)),
        };
        let lost = self.feed(event);
        debug_assert_eq!(lost, result.is_err(), "a failed write is a lost device");
        result
    }

    /// Moves the lifecycle by `event` and carries out its effects; whether the devices were lost.
    fn feed(&mut self, event: Event) -> bool {
        let (life, effects) = step(self.life, event);
        self.life = life;
        self.perform_all(effects)
    }

    fn perform_all(&mut self, effects: Vec<Effect>) -> bool {
        effects
            .into_iter()
            .fold(false, |lost, effect| self.perform(effect) || lost)
    }

    /// Whether the effect says the devices were lost.
    fn perform(&mut self, effect: Effect) -> bool {
        match effect {
            Effect::Create { at } => {
                self.create(at);
                false
            }
            Effect::DropEmitter => {
                self.emitter = None;
                false
            }
            Effect::Log(line) => {
                eprintln!("palmrest: {line}");
                false
            }
            Effect::Lost => true,
        }
    }

    fn create(&mut self, at: Mono) {
        let event = match (self.factory)(&self.shape.combos) {
            Ok(emitter) => {
                self.emitter = Some(emitter);
                Event::Created
            }
            Err(err) => Event::CreateFailed {
                why: Why::of(&err),
                at,
            },
        };
        self.feed(event);
    }
}

impl Emitter for Devices {
    fn pad(&mut self, f: &PadFrame) -> Result<(), OutputError> {
        self.write(|e| e.pad(f))
    }

    fn key(&mut self, k: &KeyCombo) -> Result<(), OutputError> {
        self.write(|e| e.key(k))
    }
}
