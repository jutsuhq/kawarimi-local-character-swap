//! The app's folders, so agents and the app share models, ComfyUI and the jobs folder, and Library shows what an
//! agent rendered. The same environment variables as the app move them.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub struct Dirs {
    pub data: PathBuf,
    pub assets: PathBuf,
    pub comfy: PathBuf,
    pub jobs: PathBuf,
    /// The server's own: limits, the ledger, preps and character cutouts.
    pub mcp: PathBuf,
}

impl Dirs {
    pub fn from_env() -> Result<Self> {
        let data = match std::env::var_os("KAWARIMI_DATA") {
            Some(dir) => PathBuf::from(dir),
            None => app_data()?,
        };
        let dir = |var: &str, name: &str| std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| data.join(name));
        Ok(Self {
            assets: dir("KAWARIMI_ASSETS", "assets"),
            comfy: dir("KAWARIMI_COMFY", "comfy"),
            jobs: dir("KAWARIMI_JOBS", "jobs"),
            mcp: data.join("mcp"),
            data,
        })
    }
}

/// Tauri's app data folder for the app's identifier.
fn app_data() -> Result<PathBuf> {
    const IDENTIFIER: &str = "dev.kawarimi.app";
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
    };
    Ok(base.context("no APPDATA or HOME to find the app's folder; set KAWARIMI_DATA")?.join(IDENTIFIER))
}
