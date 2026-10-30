//! Locks between processes: the app and `kawarimi-mcp` share one GPU and one jobs folder. The OS releases a lock
//! when its file is closed, so a killed process never leaves one behind.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The GPU, held by one process at a time. Who has it goes into a file beside the lock: Windows locks keep even
/// readers out of a locked file.
pub struct GpuLock {
    file: File,
}

impl GpuLock {
    /// `<data>/gpu.lock`, next to the app's other folders.
    pub fn path(data: &Path) -> PathBuf {
        data.join("gpu.lock")
    }

    /// Waits for the GPU. `holder` is what the others see while this process has it.
    pub fn acquire(path: &Path, holder: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().write(true).create(true).truncate(false).open(path)?;
        file.lock().with_context(|| format!("locking {}", path.display()))?;
        std::fs::write(path.with_extension("holder"), format!("{} {holder}", std::process::id()))?;
        Ok(Self { file })
    }

    /// The GPU if it is free now; `None` while another holder has it.
    pub fn try_acquire(path: &Path, holder: &str) -> Result<Option<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().write(true).create(true).truncate(false).open(path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e).with_context(|| format!("locking {}", path.display())),
        }
        std::fs::write(path.with_extension("holder"), format!("{} {holder}", std::process::id()))?;
        Ok(Some(Self { file }))
    }

    /// Who holds the GPU now, when it is held.
    pub fn holder(path: &Path) -> Option<String> {
        let file = OpenOptions::new().write(true).open(path).ok()?;
        match file.try_lock() {
            Err(TryLockError::WouldBlock) => std::fs::read_to_string(path.with_extension("holder")).ok(),
            _ => None,
        }
    }
}

impl Drop for GpuLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// A job being run by this process; another process that finds it held leaves the job alone.
pub struct RunLock {
    _file: File,
}

impl RunLock {
    /// `None` when another process runs the job.
    pub fn claim(job_dir: &Path) -> Result<Option<Self>> {
        let file = OpenOptions::new().write(true).create(true).truncate(false).open(job_dir.join("run.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_gpu_names_its_holder_and_frees_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = GpuLock::path(dir.path());
        let lock = GpuLock::acquire(&path, "render bedroom.mp4").unwrap();
        // The same process opening the file again is another handle, as a second process would be.
        assert!(GpuLock::holder(&path).unwrap().ends_with("render bedroom.mp4"));
        assert!(GpuLock::try_acquire(&path, "agent prep").unwrap().is_none());
        drop(lock);
        assert_eq!(GpuLock::holder(&path), None);
        assert!(GpuLock::try_acquire(&path, "agent prep").unwrap().is_some());
    }

    #[test]
    fn a_claimed_job_is_not_claimed_twice() {
        let dir = tempfile::tempdir().unwrap();
        let first = RunLock::claim(dir.path()).unwrap();
        assert!(first.is_some());
        assert!(RunLock::claim(dir.path()).unwrap().is_none());
        drop(first);
        assert!(RunLock::claim(dir.path()).unwrap().is_some());
    }
}
