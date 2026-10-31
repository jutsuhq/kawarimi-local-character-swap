//! Agents' render jobs, run one at a time on a thread of their own, the unfinished ones resumed when the server
//! starts. ComfyUI is started when a job needs it and stopped when the queue is empty, so an idle server holds no
//! video memory; every render takes the machine's GPU lock, so the app and the server never render at once.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use kawarimi_assets::{Status, Store, manifest};
use kawarimi_comfy::job::{self, Api, End, Jobs, Runner, Source};
use kawarimi_comfy::locks::{GpuLock, RunLock};
use kawarimi_comfy::{Client, Extras, Install, Server, wan};
use kawarimi_media::Ffmpeg;

use crate::ledger::{Kind, Ledger};

pub struct Worker {
    pub jobs: Jobs,
    queue: Mutex<Queue>,
    wake: Condvar,
    /// The running job's latest progress line, which the job file only gets at each step.
    detail: Mutex<Option<(String, String)>>,
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<String>,
    current: Option<(String, Arc<AtomicBool>)>,
}

impl Worker {
    /// Queues the agents' unfinished jobs and starts the thread that runs them.
    pub fn start(jobs: Jobs, backend: Backend) -> Arc<Self> {
        let mine = backend.ledger.renders();
        let waiting = jobs.all().into_iter().filter(|j| j.is_active() && mine.contains(&j.id)).map(|j| j.id).collect();
        let worker = Arc::new(Self { jobs, queue: Mutex::new(Queue { waiting, current: None }), wake: Condvar::new(), detail: Mutex::new(None) });
        let running = worker.clone();
        std::thread::spawn(move || running.work(backend));
        worker
    }

    fn work(&self, backend: Backend) {
        loop {
            let (id, cancel) = {
                if self.queue.lock().unwrap().waiting.is_empty() {
                    backend.stop_comfy();
                }
                let mut queue = self.wake.wait_while(self.queue.lock().unwrap(), |q| q.waiting.is_empty()).unwrap();
                let id = queue.waiting.pop_front().unwrap_or_default();
                let cancel = Arc::new(AtomicBool::new(false));
                queue.current = Some((id.clone(), cancel.clone()));
                (id, cancel)
            };
            let changed = |job: &job::Job| *self.detail.lock().unwrap() = Some((job.id.clone(), job.detail.clone()));
            let runner = Runner { jobs: &self.jobs, backend: &backend, cancel: &cancel, changed: &changed, poll: Duration::from_secs(1) };
            let ran = match RunLock::claim(&self.jobs.dir(&id)) {
                Ok(Some(_claim)) => self.jobs.load(&id).and_then(|mut job| runner.run(&mut job)),
                Ok(None) => {
                    eprintln!("job {id} is run by another process");
                    Ok(())
                }
                Err(e) => Err(e),
            };
            if let Err(e) = ran {
                eprintln!("job {id}: {e:#}");
            }
            self.queue.lock().unwrap().current = None;
            *self.detail.lock().unwrap() = None;
        }
    }

    /// The running job's latest progress line, when `id` is running.
    pub fn live(&self, id: &str) -> Option<String> {
        self.detail.lock().unwrap().as_ref().filter(|(running, _)| running == id).map(|(_, detail)| detail.clone())
    }

    pub fn submit(&self, id: &str) {
        self.queue.lock().unwrap().waiting.push_back(id.into());
        self.wake.notify_one();
    }

    /// A waiting job ends cancelled at once, the running one at its next check.
    pub fn cancel(&self, id: &str) -> Result<()> {
        let mut queue = self.queue.lock().unwrap();
        if let Some((current, cancel)) = &queue.current
            && current == id
        {
            cancel.store(true, Ordering::SeqCst);
            return Ok(());
        }
        queue.waiting.retain(|w| w != id);
        let mut job = self.jobs.load(id)?;
        if job.is_active() {
            job.end = Some(End::Cancelled);
            self.jobs.save(&job)?;
        }
        Ok(())
    }
}

/// What agents' jobs run on: the app's models and ComfyUI install, a ComfyUI of this process, the GPU lock.
pub struct Backend {
    pub store: Store,
    pub comfy_root: PathBuf,
    pub gpu_lock: PathBuf,
    pub ledger: Arc<Ledger>,
    server: Mutex<Option<Server>>,
}

impl Backend {
    pub fn new(store: Store, comfy_root: PathBuf, gpu_lock: PathBuf, ledger: Arc<Ledger>) -> Self {
        Self { store, comfy_root, gpu_lock, ledger, server: Mutex::new(None) }
    }

    fn stop_comfy(&self) {
        if let Some(mut server) = self.server.lock().unwrap().take() {
            server.stop();
        }
    }
}

pub fn ffmpeg(store: &Store) -> Result<Ffmpeg> {
    let installed = store.status(&manifest::FFMPEG) == Status::Installed;
    let dir = installed.then(|| store.dir(&manifest::FFMPEG));
    Ffmpeg::locate(dir.as_deref()).context("FFmpeg is not installed; install it with the install tool")
}

/// The GPU while a render holds it; its time goes into the ledger when dropped.
pub struct GpuTime {
    _lock: GpuLock,
    started: Instant,
    name: String,
    ledger: Arc<Ledger>,
}

impl Drop for GpuTime {
    fn drop(&mut self) {
        let _ = self.ledger.record(Kind::Gpu, &self.name, self.started.elapsed().as_secs_f64());
    }
}

impl job::Backend for Backend {
    type Comfy = Api;
    type Lease = GpuTime;

    fn prepare(&self, _: &Source, _: &Path, _: &mut dyn FnMut(&str)) -> Result<()> {
        bail!("agents render prepared clips; prepare_clip first")
    }

    fn lease(&self, name: &str) -> Result<GpuTime> {
        let lock = GpuLock::acquire(&self.gpu_lock, &format!("agent {name}"))?;
        Ok(GpuTime { _lock: lock, started: Instant::now(), name: name.into(), ledger: self.ledger.clone() })
    }

    fn comfy(&self) -> Result<Api> {
        let install = Install::from_store(&self.comfy_root, &self.store)?;
        let missing: Vec<&str> = Extras::missing(&self.store).iter().map(|a| a.id).collect();
        anyhow::ensure!(missing.is_empty(), "render models are not installed: {}", missing.join(", "));
        let mut server = self.server.lock().unwrap();
        if !server.as_mut().is_some_and(Server::is_running) {
            *server = Some(Server::start(&install, &Extras::render(&self.store), Duration::from_secs(600))?);
        }
        let url = server.as_ref().map(Server::url).unwrap_or_default();
        let guard = server.as_ref().map(Server::guard);
        Ok(Api { client: Client::new(url)?, output: Some(install.data().join("output")), guard })
    }

    fn finish(&self, video: &Path, prep: &Path, out: &Path, frames: u32) -> Result<()> {
        ffmpeg(&self.store)?.finish(video, &prep.join("normalized.mp4"), out, frames, wan::FPS as f64)?;
        Ok(())
    }

    fn frames(&self, video: &Path) -> Result<u32> {
        let probe = ffmpeg(&self.store)?.probe(video)?;
        Ok((probe.duration * probe.fps).round() as u32)
    }
}
