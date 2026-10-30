//! The Python side of ComfyUI: uv installs CPython, makes a venv and installs exactly the locked packages. A marker
//! records what was installed, so a changed lock or ComfyUI version counts as not installed.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use kawarimi_assets::{Status, Store, manifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{PYTHON, TORCH_INDEX, requirements};

const MARKER: &str = "installed.json";

pub struct Install {
    /// The folder the app owns for ComfyUI.
    pub root: PathBuf,
    /// The uv binary (the `uv` asset).
    pub uv: PathBuf,
    /// ComfyUI's source (the `comfyui` asset's `ComfyUI` folder).
    pub source: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallState {
    pub python: String,
    /// SHA-256 of the locked requirements.
    pub lock: String,
    pub source: PathBuf,
}

impl Install {
    /// ComfyUI in `root`, from the `uv` and `comfyui` assets in `store`; fails naming the one not installed.
    pub fn from_store(root: &Path, store: &Store) -> Result<Self> {
        for asset in [&manifest::UV, &manifest::COMFYUI] {
            anyhow::ensure!(store.status(asset) == Status::Installed, "{} is not installed (kawarimi-fetch {})", asset.title, asset.id);
        }
        let uv_name = if cfg!(windows) { "uv.exe" } else { "uv" };
        Ok(Self {
            root: std::path::absolute(root)?,
            uv: store.dir(&manifest::UV).join(uv_name),
            source: std::fs::canonicalize(store.dir(&manifest::COMFYUI).join("ComfyUI"))?,
        })
    }

    pub fn venv(&self) -> PathBuf {
        self.root.join("venv")
    }

    pub fn python(&self) -> PathBuf {
        if cfg!(windows) { self.venv().join("Scripts").join("python.exe") } else { self.venv().join("bin").join("python") }
    }

    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn wanted(&self) -> InstallState {
        InstallState {
            python: PYTHON.into(),
            lock: hex::encode(Sha256::digest(requirements().as_bytes())),
            source: self.source.clone(),
        }
    }

    pub fn is_installed(&self) -> bool {
        let marker = std::fs::read(self.root.join(MARKER)).ok();
        let state: Option<InstallState> = marker.and_then(|m| serde_json::from_slice(&m).ok());
        state.as_ref() == Some(&self.wanted()) && self.python().is_file()
    }

    /// Installs CPython, the venv and the locked packages. `log` gets a line per step; uv's own output goes to
    /// `logs/install.log`. Safe to run again: uv only fetches what is missing.
    pub fn install(&self, mut log: impl FnMut(&str)) -> Result<()> {
        anyhow::ensure!(self.source.join("main.py").is_file(), "no ComfyUI in {}", self.source.display());
        std::fs::create_dir_all(self.root.join("logs"))?;
        let _ = std::fs::remove_file(self.root.join(MARKER));
        let log_file = self.root.join("logs").join("install.log");

        log(&format!("Python {PYTHON}"));
        self.uv(&["python", "install", PYTHON], &log_file)?;
        log("virtual environment");
        let venv = self.venv();
        self.uv(&["venv", "--allow-existing", "--python", PYTHON, &venv.to_string_lossy()], &log_file)?;
        log("ComfyUI's packages (PyTorch for CUDA 13 is the large part)");
        let lock = self.root.join("requirements.txt");
        std::fs::write(&lock, requirements())?;
        self.uv(
            &[
                "pip",
                "install",
                "--require-hashes",
                "--python",
                &self.python().to_string_lossy(),
                "--index",
                TORCH_INDEX,
                "--index-strategy",
                "unsafe-best-match",
                "-r",
                &lock.to_string_lossy(),
            ],
            &log_file,
        )?;
        for dir in ["models", "custom_nodes", "input", "output", "temp", "user"] {
            std::fs::create_dir_all(self.data().join(dir))?;
        }
        std::fs::write(self.root.join(MARKER), serde_json::to_vec_pretty(&self.wanted())?)?;
        log("installed");
        Ok(())
    }

    /// Runs uv with its own Python installs inside the root, appending its output to `log_file`.
    fn uv(&self, args: &[&str], log_file: &Path) -> Result<()> {
        let out = std::fs::OpenOptions::new().create(true).append(true).open(log_file)?;
        let err = out.try_clone()?;
        let mut cmd = Command::new(&self.uv);
        cmd.args(args)
            .env("UV_PYTHON_INSTALL_DIR", self.root.join("python"))
            .env("UV_PYTHON_PREFERENCE", "only-managed")
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        // A private cache unless the developer points uv at a shared one.
        if std::env::var_os("UV_CACHE_DIR").is_none() {
            cmd.env("UV_CACHE_DIR", self.root.join("cache"));
        }
        quiet(&mut cmd);
        let status = cmd.status().with_context(|| format!("running {}", self.uv.display()))?;
        if !status.success() {
            bail!("uv {} failed; see {}", args.join(" "), log_file.display());
        }
        Ok(())
    }
}

pub(crate) fn quiet(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lock_pins_cuda_torch_with_hashes() {
        let lock = requirements();
        assert!(lock.contains("torch==2.14.1+cu130"));
        assert!(lock.contains("--hash=sha256:"));
    }

    #[test]
    fn a_changed_lock_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let install = Install { root: dir.path().into(), uv: "uv".into(), source: "src".into() };
        let mut stale = install.wanted();
        stale.lock = "0".repeat(64);
        std::fs::write(dir.path().join(MARKER), serde_json::to_vec(&stale).unwrap()).unwrap();
        assert!(!install.is_installed());
    }
}
