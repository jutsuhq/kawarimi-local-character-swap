//! ComfyUI, where the render runs: installed from pinned parts into a folder the app owns, and started as a child
//! process the app controls.
//!
//! The folder holds `python/` (uv's CPython 3.12), `venv/` (ComfyUI's packages, installed from the locked
//! `data/requirements-<platform>.txt` with hashes required), `data/` (ComfyUI's models, custom nodes, input, output
//! and user folders, via `--base-directory`) and `logs/`. The ComfyUI source and uv are assets (`comfyui`, `uv`);
//! the custom nodes and weights are render assets that [`Extras::render`] points ComfyUI at where they are.

pub mod check;
mod client;
mod install;
pub mod job;
pub mod locks;
pub mod nvidia;
pub mod progress;
pub mod prompt;
pub mod render;
mod server;
mod variant;
pub mod wan;

pub use client::{Client, Output, PromptState};
pub use install::{Install, InstallState};
pub use server::{Extras, Server};
pub use variant::{VARIANTS, Variant};

/// The Python packages for this platform, with hashes.
pub fn requirements() -> &'static str {
    #[cfg(windows)]
    {
        include_str!("../data/requirements-x86_64-pc-windows-msvc.txt")
    }
    #[cfg(not(windows))]
    {
        include_str!("../data/requirements-x86_64-unknown-linux-gnu.txt")
    }
}

/// Where PyTorch's CUDA 13.0 wheels come from; everything else is on PyPI.
pub const TORCH_INDEX: &str = "https://download.pytorch.org/whl/cu130";
pub const PYTHON: &str = "3.12";

/// The last `lines` lines of a log file; empty if it cannot be read.
pub fn log_tail(path: &std::path::Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// The version of `package` in this platform's lock, for instance `2.14.1+cu130` for `torch`.
pub fn locked(package: &str) -> Option<&'static str> {
    requirements().lines().find_map(|line| line.strip_prefix(package)?.strip_prefix("=="))?.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    #[test]
    fn locked_versions_are_read_from_the_lock() {
        assert!(super::locked("torch").is_some_and(|v| v.contains("cu130")));
        assert_eq!(super::locked("no-such-package"), None);
    }
}
