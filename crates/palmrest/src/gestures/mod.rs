//! The gesture layer: contact tracking and rejection, then the stroke recogniser (G1, G2, G5) and G4, G6 and G7 over the contact set.

pub mod bands;
pub mod click;
pub mod contacts;
pub mod motion;
pub mod one_swipe;
pub mod stroke;
pub mod swipe;
pub mod tap;

pub use bands::Bands;
pub use click::{ClickGuard, Hold};
pub use contacts::{Contact, Contacts, Sample, Seen};
pub use motion::MotionWindow;
pub use one_swipe::OneSwipe;
pub use stroke::{Event as StrokeEvent, Intent, Motion, Params, Recogniser};
pub use swipe::{FingerDirection, SwipeFire};
pub use tap::TapG;
