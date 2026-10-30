//! Render jobs in the app: created from the UI, run one at a time on a worker thread, and the unfinished ones
//! resumed from where they stood when the app starts. Every change goes to the UI as a `job-state` event.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use kawarimi_assets::Store;
use kawarimi_comfy::job::{self, Api, End, Job, Jobs, Runner, Source, Spec};
use kawarimi_comfy::locks::RunLock;
use kawarimi_comfy::{Client, wan};
use kawarimi_prep::{Models, Options, Targets};
use tauri::async_runtime::block_on;

use crate::comfy::Comfy;
use crate::gpu::{Gpu, Lease, Work};
use crate::own_comfy::OwnComfy;

pub struct Renders {
    jobs: Jobs,
    queue: Mutex<Queue>,
    wake: Condvar,
    emit: Box<dyn Fn(&Job) + Send + Sync>,
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<String>,
    /// The job being run and its cancel flag.
    current: Option<(String, Arc<AtomicBool>)>,
}

impl Renders {
    /// Queues the unfinished jobs in `jobs`, oldest first, and starts the worker that runs them.
    pub fn start(jobs: Jobs, backend: Backend, emit: impl Fn(&Job) + Send + Sync + 'static) -> Arc<Self> {
        let waiting = jobs.all().into_iter().filter(Job::is_active).map(|j| j.id).collect();
        let renders = Arc::new(Self { jobs, queue: Mutex::new(Queue { waiting, current: None }), wake: Condvar::new(), emit: Box::new(emit) });
        let worker = renders.clone();
        std::thread::spawn(move || worker.work(backend));
        renders
    }

    fn work(&self, backend: Backend) {
        loop {
            let (id, cancel) = {
                let mut queue = self.wake.wait_while(self.queue.lock().unwrap(), |q| q.waiting.is_empty()).unwrap();
                let id = queue.waiting.pop_front().unwrap_or_default();
                let cancel = Arc::new(AtomicBool::new(false));
                queue.current = Some((id.clone(), cancel.clone()));
                (id, cancel)
            };
            let runner = Runner { jobs: &self.jobs, backend: &backend, cancel: &cancel, changed: &*self.emit, poll: Duration::from_secs(1) };
            let ran = match RunLock::claim(&self.jobs.dir(&id)) {
                Ok(Some(_claim)) => self.jobs.load(&id).and_then(|mut job| runner.run(&mut job)),
                Ok(None) => {
                    eprintln!("job {id} is run by another process (kawarimi-mcp)");
                    Ok(())
                }
                Err(e) => Err(e),
            };
            if let Err(e) = ran {
                eprintln!("job {id}: {e:#}");
            }
            let idle = {
                let mut queue = self.queue.lock().unwrap();
                queue.current = None;
                queue.waiting.is_empty()
            };
            // Another process may want the GPU next; an idle ComfyUI keeps its models in video memory.
            if idle && backend.own.server().is_none() {
                let _ = backend.comfy.free();
            }
        }
    }

    pub fn create(&self, spec: Spec) -> Result<Job> {
        let job = self.jobs.create(spec)?;
        self.queue.lock().unwrap().waiting.push_back(job.id.clone());
        self.wake.notify_one();
        (self.emit)(&job);
        Ok(job)
    }

    /// Every job, oldest first.
    pub fn list(&self) -> Vec<Job> {
        self.jobs.all()
    }

    pub fn jobs(&self) -> &Jobs {
        &self.jobs
    }

    /// Runs a failed or cancelled job again from the step it stopped at, with a fresh count of submissions.
    pub fn retry(&self, id: &str) -> Result<Job> {
        let mut queue = self.queue.lock().unwrap();
        let mut job = self.jobs.load(id)?;
        anyhow::ensure!(matches!(job.end, Some(End::Failed { .. } | End::Cancelled)), "only a failed or cancelled job runs again");
        job.end = None;
        job.submissions = 0;
        job.detail.clear();
        self.jobs.save(&job)?;
        queue.waiting.push_back(job.id.clone());
        self.wake.notify_one();
        (self.emit)(&job);
        Ok(job)
    }

    /// Moves an ended job's folder, its finished video included, to the recycle bin.
    pub fn remove(&self, id: &str) -> Result<()> {
        let _queue = self.queue.lock().unwrap();
        to_trash(&self.jobs, id)
    }

    /// Cancels a waiting job at once, the running one at its next check (within a second while it renders, after
    /// prep when it is preparing); an ended job stays as it is.
    pub fn cancel(&self, id: &str) -> Result<()> {
        let mut queue = self.queue.lock().unwrap();
        if let Some((current, cancel)) = &queue.current
            && current == id
        {
            cancel.store(true, Ordering::SeqCst);
            return Ok(());
        }
        let mut job = self.jobs.load(id)?;
        queue.waiting.retain(|w| w != id);
        if job.is_active() {
            job.end = Some(End::Cancelled);
            self.jobs.save(&job)?;
            (self.emit)(&job);
        }
        Ok(())
    }
}

fn to_trash(jobs: &Jobs, id: &str) -> Result<()> {
    anyhow::ensure!(!jobs.load(id)?.is_active(), "cancel the job before removing it");
    trash::delete(jobs.dir(id)).context("moving the job to the recycle bin")
}

/// What jobs run on in the app: the shared GPU, the app's ComfyUI or the person's own, and the installed models.
pub struct Backend {
    pub store: Store,
    pub gpu: Gpu,
    pub comfy: Arc<Comfy>,
    pub own: Arc<OwnComfy>,
}

impl Backend {
    fn ffmpeg(&self) -> Result<kawarimi_media::Ffmpeg> {
        crate::ffmpeg(&self.store).context("FFmpeg is not installed")
    }
}

impl job::Backend for Backend {
    type Comfy = Api;
    /// None while renders go to the person's own ComfyUI, which does not use this GPU.
    type Lease = Option<Lease>;

    fn prepare(&self, source: &Source, out: &Path, log: &mut dyn FnMut(&str)) -> Result<()> {
        let Source::Clip { path, start, duration, targets } = source else { bail!("nothing to prepare") };
        let ffmpeg = self.ffmpeg()?;
        let models = Models::installed(&self.store)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let _lease = block_on(self.gpu.lease(Work::Prep, format!("prep {name}")))?;
        let options = Options {
            normalize: kawarimi_media::Normalize { start: *start, duration: *duration, ..Default::default() },
            targets: targets.clone().map_or(Targets::Main, Targets::Ids),
            ..Default::default()
        };
        kawarimi_prep::prepare(&ffmpeg, &models, path, out, &options, log)?;
        Ok(())
    }

    fn lease(&self, name: &str) -> Result<Option<Lease>> {
        if self.own.server().is_some() {
            return Ok(None);
        }
        block_on(self.gpu.lease(Work::Render, name)).map(Some)
    }

    fn comfy(&self) -> Result<Api> {
        if let Some(client) = self.own.render_client() {
            let client = client?;
            client.vram().context("your own ComfyUI does not answer")?;
            return Ok(Api { client, output: None });
        }
        let url = block_on(self.comfy.ensure())?;
        Ok(Api { client: Client::new(url)?, output: Some(self.comfy.output()?) })
    }

    fn finish(&self, video: &Path, prep: &Path, out: &Path, frames: u32) -> Result<()> {
        self.ffmpeg()?.finish(video, &prep.join("normalized.mp4"), out, frames, wan::FPS as f64)?;
        Ok(())
    }

    fn frames(&self, video: &Path) -> Result<u32> {
        let probe = self.ffmpeg()?.probe(video)?;
        Ok((probe.duration * probe.fps).round() as u32)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use kawarimi_comfy::render::Settings;

    use super::*;

    #[test]
    fn removing_moves_an_ended_job_to_the_recycle_bin() {
        let dir = tempfile::tempdir().unwrap();
        let (clip, character) = (dir.path().join("clip.mp4"), dir.path().join("me.png"));
        std::fs::write(&clip, b"clip").unwrap();
        std::fs::write(&character, b"png").unwrap();
        let jobs = Jobs::new(dir.path().join("jobs"));
        let settings = Settings { quality: wan::Quality::Draft, seed: 0, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
        let spec = Spec { source: Source::Clip { path: clip, start: 0.0, duration: None, targets: None }, character, settings };
        let mut job = jobs.create(spec).unwrap();
        assert!(to_trash(&jobs, &job.id).is_err(), "a running job stays");
        assert!(jobs.dir(&job.id).is_dir());
        job.end = Some(End::Done { result: jobs.dir(&job.id).join("result.mp4") });
        jobs.save(&job).unwrap();
        to_trash(&jobs, &job.id).unwrap();
        assert!(!jobs.dir(&job.id).exists());
        assert!(jobs.all().is_empty());
    }

    /// The manual check of kill and resume, on the app's own GPU lease and ComfyUI:
    ///
    ///     KAWARIMI_ASSETS=<store> KAWARIMI_COMFY=<root> KAWARIMI_JOBS=<folder> KAWARIMI_PREP=<prep folder>
    ///     KAWARIMI_CHARACTER=<png> cargo test -p kawarimi renders_a_draft -- --ignored --nocapture
    ///
    /// Starts a draft when KAWARIMI_JOBS has no unfinished job, else resumes them; kill it and run it again.
    #[test]
    #[ignore = "renders on the GPU with the ComfyUI and models where the variables point"]
    fn renders_a_draft() {
        let var = |name| PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")));
        let store = Store::new(var("KAWARIMI_ASSETS"));
        let comfy = Comfy::new(Store::new(var("KAWARIMI_ASSETS")), var("KAWARIMI_COMFY"));
        let freed = comfy.clone();
        let gpu = Gpu::new(move |last| if last == Work::Render { freed.free() } else { Ok(()) });
        let jobs = Jobs::new(var("KAWARIMI_JOBS"));
        let last = Mutex::new(String::new());
        let own = Arc::new(OwnComfy::load(std::env::temp_dir().join("kawarimi-no-own-comfy.json")));
        let renders = Renders::start(jobs, Backend { store, gpu, comfy: comfy.clone(), own }, move |job| {
            let line = format!("job {} {:?} {:?}: {}", job.id, job.step, job.end, job.detail);
            let mut last = last.lock().unwrap();
            if *last != line && (!job.detail.starts_with("rendering") || job.detail.ends_with("0 s")) {
                eprintln!("{line}");
                *last = line;
            }
        });
        if !renders.list().iter().any(Job::is_active) {
            let settings = Settings { quality: wan::Quality::Draft, seed: 0, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
            let spec = Spec { source: Source::Prepared { folder: var("KAWARIMI_PREP") }, character: var("KAWARIMI_CHARACTER"), settings };
            renders.create(spec).unwrap();
        }
        while renders.list().iter().any(Job::is_active) {
            std::thread::sleep(Duration::from_secs(2));
        }
        comfy.stop();
        for job in renders.list() {
            eprintln!("job {}: {:?}, queued {} times", job.id, job.end, job.submissions);
        }
        assert!(renders.list().iter().all(|j| matches!(j.end, Some(End::Done { .. }))));
    }
}
