//! The app's ComfyUI: installed from the app, started when something needs it, reused while the app runs, stopped
//! when the app exits. Its state is a watch channel the app forwards to the UI as `comfy-state` events.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use kawarimi_assets::Store;
use kawarimi_comfy::ram::Guard;
use kawarimi_comfy::{Client, Extras, Install, Server};
use serde::Serialize;
use tokio::sync::watch;

/// Cold imports of the custom nodes once took more than 180 s.
const START_TIMEOUT: Duration = Duration::from_secs(600);
const LOG_LINES: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ComfyState {
    NotInstalled,
    Installing { step: String },
    Stopped,
    Starting,
    Ready { url: String },
    /// It failed to install or start, or died; `log` is the end of its log.
    Failed { error: String, log: String },
}

pub struct Comfy {
    store: Store,
    root: PathBuf,
    state: watch::Sender<ComfyState>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    server: Option<Server>,
    /// Bumped on every start and stop, so a watcher of an older server stops watching.
    generation: u64,
    installing: bool,
}

impl Comfy {
    pub fn new(store: Store, root: PathBuf) -> Arc<Self> {
        let comfy = Arc::new(Self { store, root, state: watch::Sender::new(ComfyState::Stopped), inner: Default::default() });
        comfy.state.send_replace(comfy.idle());
        comfy
    }

    pub fn subscribe(&self) -> watch::Receiver<ComfyState> {
        self.state.subscribe()
    }

    pub fn state(&self) -> ComfyState {
        self.state.borrow().clone()
    }

    fn install(&self) -> Result<Install> {
        Install::from_store(&self.root, &self.store)
    }

    fn idle(&self) -> ComfyState {
        if self.install().is_ok_and(|i| i.is_installed()) { ComfyState::Stopped } else { ComfyState::NotInstalled }
    }

    /// Installs ComfyUI's Python side; blocking, minutes on a first install.
    pub fn install_blocking(&self) -> Result<()> {
        let install = self.install()?;
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.installing {
                bail!("ComfyUI is already installing");
            }
            if inner.server.is_some() {
                bail!("stop ComfyUI before installing it again");
            }
            inner.installing = true;
        }
        let result = install.install(|step| {
            self.state.send_replace(ComfyState::Installing { step: step.into() });
        });
        self.inner.lock().unwrap().installing = false;
        match &result {
            Ok(()) => self.state.send_replace(self.idle()),
            Err(e) => self.state.send_replace(ComfyState::Failed {
                error: format!("{e:#}"),
                log: kawarimi_comfy::log_tail(&install.root.join("logs").join("install.log"), LOG_LINES),
            }),
        };
        result
    }

    /// Stops ComfyUI, deletes its Python packages and installs them again; the models and its data stay. Blocking.
    pub fn reinstall_blocking(&self) -> Result<()> {
        if self.inner.lock().unwrap().installing {
            bail!("ComfyUI is already installing");
        }
        self.stop();
        let venv = self.install()?.venv();
        if venv.exists() {
            self.state.send_replace(ComfyState::Installing { step: "removing the old packages".into() });
            if let Err(e) = std::fs::remove_dir_all(&venv).with_context(|| format!("removing {}", venv.display())) {
                self.state.send_replace(ComfyState::Failed { error: format!("{e:#}"), log: String::new() });
                return Err(e);
            }
        }
        self.install_blocking()
    }

    /// The folder the app owns for ComfyUI.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Starts ComfyUI unless it is running or starting; returns at once, the state says when it is ready.
    pub fn start(self: &Arc<Self>) -> Result<()> {
        let install = self.install()?;
        let mut inner = self.inner.lock().unwrap();
        if inner.installing {
            bail!("ComfyUI is installing");
        }
        if !install.is_installed() {
            self.state.send_replace(ComfyState::NotInstalled);
            bail!("ComfyUI is not installed");
        }
        if inner.server.is_some() {
            return Ok(());
        }
        // Without the render assets ComfyUI still starts, for checks; a render asks for them itself.
        let extras = if Extras::missing(&self.store).is_empty() { Extras::render(&self.store) } else { Extras::default() };
        match Server::spawn(&install, &extras) {
            Ok(server) => {
                inner.server = Some(server);
                inner.generation += 1;
                self.state.send_replace(ComfyState::Starting);
                let (comfy, generation) = (self.clone(), inner.generation);
                std::thread::spawn(move || comfy.watch(generation));
                Ok(())
            }
            Err(e) => {
                self.state.send_replace(ComfyState::Failed { error: format!("{e:#}"), log: String::new() });
                Err(e)
            }
        }
    }

    /// Waits for readiness, then for an exit we did not ask for.
    fn watch(&self, generation: u64) {
        let started = Instant::now();
        let mut ready = false;
        loop {
            std::thread::sleep(Duration::from_millis(if ready { 2000 } else { 500 }));
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                return;
            }
            let Some(server) = inner.server.as_mut() else { return };
            let failed = if !server.is_running() {
                Some(match server.guard().tripped() {
                    Some(reason) => reason,
                    None if ready => "ComfyUI stopped on its own".into(),
                    None => "ComfyUI exited before it was ready".into(),
                })
            } else if !ready && server.system_stats().is_ok() {
                ready = true;
                self.state.send_replace(ComfyState::Ready { url: server.url() });
                None
            } else if !ready && started.elapsed() > START_TIMEOUT {
                Some("ComfyUI did not answer in time".into())
            } else {
                None
            };
            if let Some(error) = failed {
                let log = server.log_tail(LOG_LINES);
                inner.server = None;
                self.state.send_replace(ComfyState::Failed { error, log });
                return;
            }
        }
    }

    /// Stops ComfyUI and every process it started; does nothing if it is not running.
    pub fn stop(&self) {
        let server = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            inner.server.take()
        };
        if let Some(mut server) = server {
            server.stop();
            self.state.send_replace(self.idle());
        }
    }

    /// A running ComfyUI for a job: started (again, if it died) when needed. Its URL.
    pub async fn ensure(self: &Arc<Self>) -> Result<String> {
        let mut state = self.subscribe();
        self.start()?;
        let state = state
            .wait_for(|s| !matches!(s, ComfyState::Starting))
            .await
            .map(|s| s.clone())
            .unwrap_or(ComfyState::Stopped);
        match state {
            ComfyState::Ready { url } => Ok(url),
            ComfyState::Failed { error, log } => bail!("{error}:\n{log}"),
            other => bail!("ComfyUI did not start ({other:?})"),
        }
    }

    /// The RAM watchdog of the running ComfyUI.
    pub fn guard(&self) -> Option<Arc<Guard>> {
        self.inner.lock().unwrap().server.as_ref().map(Server::guard)
    }

    /// ComfyUI's output folder, where its graphs save.
    pub fn output(&self) -> Result<PathBuf> {
        Ok(self.install()?.data().join("output"))
    }

    /// Unloads ComfyUI's models from the GPU; stops it if it does not answer. Blocking.
    pub fn free(&self) -> Result<()> {
        let ComfyState::Ready { url } = self.state() else { return Ok(()) };
        if Client::new(url).and_then(|c| c.free()).is_err() {
            self.stop();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `KAWARIMI_ASSETS=<store> KAWARIMI_COMFY=<root> cargo test -p kawarimi comfy -- --ignored`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "starts the ComfyUI installed where KAWARIMI_ASSETS and KAWARIMI_COMFY point"]
    async fn starts_once_and_stops() {
        let var = |name| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")));
        let comfy = Comfy::new(Store::new(var("KAWARIMI_ASSETS")), var("KAWARIMI_COMFY"));
        assert_eq!(comfy.state(), ComfyState::Stopped);
        let url = comfy.ensure().await.unwrap();
        assert_eq!(comfy.ensure().await.unwrap(), url, "a running ComfyUI is reused");
        let freeing = comfy.clone();
        tokio::task::spawn_blocking(move || freeing.free()).await.unwrap().unwrap();
        comfy.stop();
        assert_eq!(comfy.state(), ComfyState::Stopped);
        let port = url.rsplit(':').next().unwrap();
        assert!(std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_err(), "nothing answers on {url}");
    }
}
