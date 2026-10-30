//! ComfyUI's server as a child process: on a free local port, ready once `/system_stats` answers, its output in
//! `logs/server.log`, and stopped with every process it started.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use kawarimi_assets::{Asset, Status, Store, manifest};

use crate::install::{Install, quiet};
use crate::variant::VARIANTS;

const RENDER: &[&Asset] = &[
    &manifest::NODES_CONTROLNET_AUX,
    &manifest::NODES_GGUF,
    &manifest::DWPOSE,
    &manifest::UMT5_Q4,
    &manifest::WAN_VAE,
    &manifest::CLIP_VISION_H,
    &manifest::LIGHTX2V,
];

/// What the app adds to a bare ComfyUI: folders of custom nodes and models outside its data folder, and
/// environment for the nodes. Written to `extra_model_paths.yaml` in the root at start.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    /// ComfyUI folder kind (`custom_nodes`, `diffusion_models`, `text_encoders`, `vae`, `clip_vision`, `loras`)
    /// and a folder to search for it.
    pub folders: Vec<(&'static str, PathBuf)>,
    pub env: Vec<(&'static str, PathBuf)>,
    /// More arguments for ComfyUI's `main.py`.
    pub args: Vec<String>,
}

impl Extras {
    /// The render assets in `store`: the custom nodes, every installed Wan variant's weights and the other models
    /// where ComfyUI's loaders look, and DWPose's checkpoints where `comfyui_controlnet_aux` looks
    /// (`AUX_ANNOTATOR_CKPTS_PATH`/<repo>/<file>).
    pub fn render(store: &Store) -> Self {
        let dir = |asset| store.dir(asset);
        let mut folders = vec![
            ("custom_nodes", dir(&manifest::NODES_CONTROLNET_AUX)),
            ("custom_nodes", dir(&manifest::NODES_GGUF)),
            ("text_encoders", dir(&manifest::UMT5_Q4)),
            ("vae", dir(&manifest::WAN_VAE)),
            ("clip_vision", dir(&manifest::CLIP_VISION_H)),
            ("loras", dir(&manifest::LIGHTX2V)),
        ];
        let wan = VARIANTS.iter().filter(|v| store.status(v.asset) == Status::Installed);
        folders.extend(wan.map(|v| ("diffusion_models", dir(v.asset))));
        Self { folders, env: vec![("AUX_ANNOTATOR_CKPTS_PATH", dir(&manifest::DWPOSE))], args: vec![] }
    }

    /// The render assets besides the Wan weights, which each render picks.
    pub fn assets() -> &'static [&'static Asset] {
        RENDER
    }

    /// The render assets that are not installed, besides the Wan weights, which each render picks.
    pub fn missing(store: &Store) -> Vec<&'static Asset> {
        RENDER.iter().copied().filter(|a| store.status(a) != Status::Installed).collect()
    }

    fn yaml(&self) -> Result<String> {
        let mut out = String::from("kawarimi:\n");
        let mut kinds: Vec<&str> = Vec::new();
        for (kind, _) in &self.folders {
            if !kinds.contains(kind) {
                kinds.push(kind);
            }
        }
        for kind in kinds {
            out += &format!("  {kind}: |\n");
            for (_, dir) in self.folders.iter().filter(|(k, _)| *k == kind) {
                let dir = std::path::absolute(dir)?;
                out += &format!("    {}\n", dir.to_string_lossy().replace('\\', "/"));
            }
        }
        Ok(out)
    }
}

pub struct Server {
    child: Option<Child>,
    /// A job that kills ComfyUI's processes when the last handle closes, so they die with us if we are killed.
    #[cfg(windows)]
    _job: Option<std::os::windows::io::OwnedHandle>,
    pub port: u16,
    pub log: PathBuf,
}

impl Server {
    /// Starts ComfyUI and waits until it answers, at most `timeout`. Fails with the end of its log if it exits.
    pub fn start(install: &Install, extras: &Extras, timeout: Duration) -> Result<Self> {
        let mut server = Self::spawn(install, extras)?;
        server.wait_ready(timeout)?;
        Ok(server)
    }

    /// Starts ComfyUI without waiting: it is ready once [`Server::system_stats`] answers.
    pub fn spawn(install: &Install, extras: &Extras) -> Result<Self> {
        anyhow::ensure!(install.is_installed(), "ComfyUI is not installed in {}", install.root.display());
        let port = free_port()?;
        let paths = install.root.join("extra_model_paths.yaml");
        std::fs::write(&paths, extras.yaml()?)?;
        let log = install.root.join("logs").join("server.log");
        let out = std::fs::File::create(&log)?;
        let err = out.try_clone()?;
        let mut cmd = Command::new(install.python());
        cmd.arg(install.source.join("main.py"))
            .args(["--listen", "127.0.0.1", "--port", &port.to_string(), "--disable-auto-launch"])
            .arg("--base-directory")
            // ComfyUI runs in its source folder, so a relative path would land there.
            .arg(std::path::absolute(install.data())?)
            .arg("--extra-model-paths-config")
            .arg(&paths)
            .args(&extras.args)
            .current_dir(&install.source)
            .env("PYTHONUNBUFFERED", "1")
            // Every model is a pinned asset; nothing may fetch one behind our back.
            .env("HF_HUB_OFFLINE", "1")
            .envs(extras.env.iter().map(|(k, v)| (k, std::path::absolute(v).unwrap_or_else(|_| v.clone()))))
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        quiet(&mut cmd);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Its own process group, so stop() reaches what ComfyUI starts.
            cmd.process_group(0);
        }
        let child = cmd.spawn().context("starting ComfyUI")?;
        Ok(Self {
            #[cfg(windows)]
            _job: kill_on_close(&child),
            child: Some(child),
            port,
            log,
        })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// ComfyUI's `/system_stats`: versions, devices and their memory.
    pub fn system_stats(&self) -> Result<serde_json::Value> {
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(5)).build()?;
        Ok(client.get(format!("{}/system_stats", self.url())).send()?.error_for_status()?.json()?)
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
                bail!("ComfyUI exited ({status}) before it was ready:\n{}", self.log_tail(20));
            }
            if self.system_stats().is_ok() {
                return Ok(());
            }
            if start.elapsed() > timeout {
                self.stop();
                bail!("ComfyUI did not answer in {} s:\n{}", timeout.as_secs(), self.log_tail(20));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// The last `lines` lines of the server's log.
    pub fn log_tail(&self, lines: usize) -> String {
        crate::log_tail(&self.log, lines)
    }

    /// Stops ComfyUI and every process it started.
    pub fn stop(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        #[cfg(windows)]
        {
            let mut kill = Command::new("taskkill");
            kill.args(["/T", "/F", "/PID", &child.id().to_string()]).stdout(Stdio::null()).stderr(Stdio::null());
            quiet(&mut kill);
            let _ = kill.status();
        }
        #[cfg(unix)]
        {
            // The negative pid is the whole process group.
            let _ = Command::new("kill").args(["-TERM", &format!("-{}", child.id())]).status();
        }
        let _ = child.kill();
        let _ = child.wait();
        // The venv's python.exe is a launcher; the interpreter holding the GPU exits a moment after it.
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

/// `None` if Windows refuses the job; stop() and Drop still end the tree then, only a crash would not.
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

fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extras_group_folders_by_kind() {
        let extras = Extras {
            folders: vec![("loras", "/a".into()), ("loras", "/b".into()), ("vae", "/c".into())],
            env: vec![],
            args: vec![],
        };
        let yaml = extras.yaml().unwrap();
        assert_eq!(yaml.matches("loras:").count(), 1);
        assert!(yaml.contains("  vae: |"));
    }
}
