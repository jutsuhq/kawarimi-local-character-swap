//! llama-server as a child process, like ComfyUI's: on a free local port, ready once `/health` answers 200 (it
//! answers 503 while the model loads), its output in a log file, and stopped by killing it, which frees all its video
//! memory. On Windows a job object kills it with us if we are killed.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use kawarimi_assets::{Status, Store, manifest};

use crate::client::Endpoint;

/// Long enough for a cold read of the 6 GB model off a hard disk.
pub const START_TIMEOUT: Duration = Duration::from_secs(300);
/// Wide enough for a reference image, several keyframes and the answer.
const CONTEXT: &str = "8192";

/// The installed files llama-server runs from.
#[derive(Debug, Clone)]
pub struct Paths {
    pub bin: PathBuf,
    pub cuda: PathBuf,
    pub model: PathBuf,
    pub mmproj: PathBuf,
}

impl Paths {
    pub fn from_store(store: &Store) -> Result<Self> {
        let model = &manifest::QWEN35_9B;
        Ok(Self {
            bin: store.path(&manifest::LLAMA_SERVER, "")?,
            cuda: store.path(&manifest::CUDA_RUNTIME, "")?,
            model: store.path(model, model.files[0].path)?,
            mmproj: store.path(model, model.files[1].path)?,
        })
    }

    /// Whether every asset the local prompt writer needs is installed.
    pub fn installed(store: &Store) -> bool {
        [&manifest::LLAMA_SERVER, &manifest::QWEN35_9B, &manifest::CUDA_RUNTIME].iter().all(|a| store.status(a) == Status::Installed)
    }

    fn exe(&self) -> PathBuf {
        self.bin.join(format!("llama-server{}", std::env::consts::EXE_SUFFIX))
    }
}

pub struct Server {
    child: Option<Child>,
    #[cfg(windows)]
    _job: Option<std::os::windows::io::OwnedHandle>,
    pub port: u16,
    pub log: PathBuf,
}

impl Server {
    /// Starts llama-server and waits until the model is loaded, at most `timeout`.
    pub fn start(paths: &Paths, log: &Path, timeout: Duration) -> Result<Self> {
        let mut server = Self::spawn(paths, log)?;
        server.wait_ready(timeout)?;
        Ok(server)
    }

    /// Starts llama-server without waiting: it is ready once [`Server::health`] is true.
    pub fn spawn(paths: &Paths, log: &Path) -> Result<Self> {
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        if let Some(dir) = log.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let out = std::fs::File::create(log).with_context(|| format!("creating {}", log.display()))?;
        let err = out.try_clone()?;
        let mut cmd = Command::new(paths.exe());
        cmd.arg("--model")
            .arg(&paths.model)
            .arg("--mmproj")
            .arg(&paths.mmproj)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            // One request at a time; --fit (on by default) puts as many layers on the GPU as fit.
            .args(["--ctx-size", CONTEXT, "--parallel", "1", "--reasoning", "off", "--no-webui", "--offline"])
            .current_dir(&paths.bin)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        // The loader finds cuBLAS and the CUDA runtime in the cuda-runtime asset.
        #[cfg(windows)]
        cmd.env("PATH", search_path("PATH", &[&paths.bin, &paths.cuda])?);
        #[cfg(unix)]
        cmd.env("LD_LIBRARY_PATH", search_path("LD_LIBRARY_PATH", &[&paths.bin, &paths.cuda])?);
        kawarimi_media::quiet(&mut cmd);
        let child = cmd.spawn().with_context(|| format!("starting {}", paths.exe().display()))?;
        Ok(Self {
            #[cfg(windows)]
            _job: kill_on_close(&child),
            child: Some(child),
            port,
            log: log.to_owned(),
        })
    }

    pub fn endpoint(&self) -> Endpoint {
        Endpoint::local(self.port)
    }

    /// Whether the model is loaded and the server takes requests.
    pub fn health(&self) -> bool {
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(2)).build();
        client.ok().and_then(|c| c.get(format!("http://127.0.0.1:{}/health", self.port)).send().ok()).is_some_and(|r| r.status().is_success())
    }

    pub fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            if !self.is_running() {
                bail!("llama-server exited before it was ready:\n{}", self.log_tail(20));
            }
            if self.health() {
                return Ok(());
            }
            if start.elapsed() > timeout {
                let tail = self.log_tail(20);
                self.stop();
                bail!("llama-server did not load the model in {} s:\n{tail}", timeout.as_secs());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    pub fn log_tail(&self, lines: usize) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }

    /// Kills llama-server and waits until it is gone, so its video memory is free when this returns.
    pub fn stop(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        let _ = child.kill();
        let _ = child.wait();
        let address = SocketAddr::from(([127, 0, 0, 1], self.port));
        let start = Instant::now();
        while TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `dirs` (made absolute) in front of the variable's current value.
fn search_path(var: &str, dirs: &[&Path]) -> Result<std::ffi::OsString> {
    let mut all = dirs.iter().map(std::path::absolute).collect::<std::io::Result<Vec<_>>>()?;
    all.extend(std::env::var_os(var).iter().flat_map(std::env::split_paths));
    Ok(std::env::join_paths(all)?)
}

/// `None` if Windows refuses the job; stop() and Drop still end the server then, only a crash would not.
#[cfg(windows)]
fn kill_on_close(child: &Child) -> Option<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    // SAFETY: plain Win32 calls on a handle we own and the child's live process handle.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let job = OwnedHandle::from_raw_handle(job);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const info).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        (set != 0 && AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) != 0).then_some(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_folders_come_first_on_the_search_path() {
        let joined = search_path("KAWARIMI_TEST_UNSET_VAR", &[Path::new("bin"), Path::new("cuda")]).unwrap();
        let dirs: Vec<_> = std::env::split_paths(&joined).collect();
        assert_eq!(dirs.len(), 2);
        assert!(dirs[0].is_absolute() && dirs[0].ends_with("bin") && dirs[1].ends_with("cuda"));
    }
}
