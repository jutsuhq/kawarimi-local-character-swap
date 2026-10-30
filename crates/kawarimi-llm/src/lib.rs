//! The optional prompt writer. Locally it is llama-server (llama.cpp's own CUDA build, a pinned asset) with a Qwen
//! vision model, started when a description is asked for and stopped before prep or render need the GPU. Or it is an
//! OpenAI-compatible service with the person's key, which gets frames only after the person agreed to that address.

mod client;
pub mod prompt;
mod server;
pub mod settings;
mod writer;

pub use client::{ApiKey, Endpoint, NoConsent};
pub use prompt::Description;
pub use server::{Paths, START_TIMEOUT, Server};
pub use settings::{Mode, Settings};
pub use writer::describe;
