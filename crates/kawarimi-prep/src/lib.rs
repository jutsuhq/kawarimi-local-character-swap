//! Clip preparation, Swapjutsu's pipeline on open models: normalise, shots, people, tracking, identities across
//! cuts, targets, and the target's mask as a video for the renderer.

pub mod cache;
pub mod cutout;
pub mod faces;
pub mod fix;
pub mod frames;
pub mod identity;
pub mod people;
pub mod pipeline;
pub mod shots;
pub mod track;

pub use pipeline::{Cancelled, Manifest, Models, Options, Targets, mask_of, prepare};
