//! A render job: one clip and one character, from prep to the finished video, kept in `job.json` in its own folder
//! so that killing the app or ComfyUI at any step neither loses the result nor renders it twice.
//!
//! Steps: prep → upload → queued → rendering → downloading → finishing, then an end: done, failed with a reason, or
//! cancelled. Every file a step makes is written under a `.part` name and renamed once whole. The graph saves its
//! video under `kawarimi/<job id>/render` in ComfyUI's output folder, so after a kill the job finds its prompt or its
//! video again instead of queueing a second one; it queues again, the same graph, only when ComfyUI has neither.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{Client, Output, PromptState};
use crate::ram::{self, Guard, Stats, Verdict};
use crate::render::{self, Clip, Settings, Uploads};
use crate::timing::NodeTime;
use crate::wan;

pub const VERSION: u32 = 1;
/// Queueing the graph again is for a ComfyUI that lost it; one that keeps losing it has a problem a retry won't fix.
const MAX_SUBMISSIONS: u32 = 3;
/// ComfyUI restarts within one run of a job.
const MAX_RESTARTS: u32 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Source {
    /// A clip to prepare into the job's `prep` folder; `targets` are identity ids, by default the main person.
    #[serde(rename_all = "camelCase")]
    Clip {
        path: PathBuf,
        #[serde(default)]
        start: f64,
        duration: Option<f64>,
        targets: Option<Vec<usize>>,
    },
    /// A folder prep already made.
    Prepared { folder: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    pub source: Source,
    /// The character image; the job keeps its own copy.
    pub character: PathBuf,
    pub settings: Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    Prep,
    Upload,
    Queued,
    Rendering,
    Downloading,
    Finishing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum End {
    Done { result: PathBuf },
    Failed { reason: String },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub version: u32,
    pub id: String,
    /// Milliseconds since the Unix epoch.
    pub created: u64,
    pub spec: Spec,
    /// The step running, or the one the job ended at.
    pub step: Step,
    pub end: Option<End>,
    /// ComfyUI's id for the graph as last queued.
    pub prompt: Option<String>,
    /// Times the graph was queued: more than one only when a ComfyUI lost it.
    pub submissions: u32,
    /// Frames the render makes; set with the graph.
    pub frames: Option<u32>,
    /// The video in ComfyUI's output folder, once the graph saved it.
    pub output: Option<Output>,
    /// The latest progress line.
    #[serde(default)]
    pub detail: String,
    /// System memory while ComfyUI rendered, when it runs on this machine.
    #[serde(default)]
    pub ram: Option<Stats>,
    /// Each node's time, longest first; empty when ComfyUI's websocket was not reached.
    #[serde(default)]
    pub nodes: Vec<NodeTime>,
}

impl Job {
    /// Where the graph saves its video, relative to ComfyUI's output folder.
    pub fn prefix(&self) -> String {
        format!("kawarimi/{}/render", self.id)
    }

    /// What the job renders, for the UI and the GPU's holder: the clip's or the prep folder's name.
    pub fn name(&self) -> String {
        let file = |path: &Path| path.file_name().map(|n| n.to_string_lossy().into_owned());
        let named = match &self.spec.source {
            Source::Clip { path, .. } => file(path),
            // A prep folder is often just "renders/3"; its manifest names the clip it came from.
            Source::Prepared { folder } => std::fs::read(folder.join("manifest.json"))
                .ok()
                .and_then(|m| serde_json::from_slice::<Value>(&m).ok())
                .and_then(|m| m["source"].as_str().and_then(|s| file(Path::new(s))))
                .or_else(|| file(folder)),
        };
        named.unwrap_or_else(|| self.id.clone())
    }

    pub fn is_active(&self) -> bool {
        self.end.is_none()
    }
}

/// Makes `path` under a sibling `<stem>.part.<ext>` name (the extension stays, as FFmpeg and image writers need it)
/// and renames it into place once `make` succeeds, so no kill leaves a half file under the final name.
pub fn atomically<T>(path: &Path, make: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let stem = path.file_stem().context("a file name")?.to_string_lossy();
    let part = path.with_file_name(match path.extension() {
        Some(ext) => format!("{stem}.part.{}", ext.to_string_lossy()),
        None => format!("{stem}.part"),
    });
    let made = make(&part);
    match made {
        Ok(value) => {
            std::fs::rename(&part, path).with_context(|| format!("renaming {}", part.display()))?;
            Ok(value)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// The folder of all jobs, one folder each: `job.json`, the character's copy, `prep/` for a clip, `reference.png`,
/// `graph.json`, `comfy.mp4` (ComfyUI's video) and `result.mp4`.
#[derive(Debug, Clone)]
pub struct Jobs {
    root: PathBuf,
}

impl Jobs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    /// A new job, saved, with its own copy of the character.
    pub fn create(&self, mut spec: Spec) -> Result<Job> {
        match &spec.source {
            Source::Clip { path, .. } => anyhow::ensure!(path.is_file(), "no clip at {}", path.display()),
            Source::Prepared { folder } => {
                anyhow::ensure!(folder.join("manifest.json").is_file(), "{} is not a prep folder", folder.display())
            }
        }
        anyhow::ensure!(spec.character.is_file(), "no character image at {}", spec.character.display());
        std::fs::create_dir_all(&self.root)?;
        let mut created = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let dir = loop {
            let dir = self.dir(&created.to_string());
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => created += 1,
                Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
            }
        };
        let ext = spec.character.extension().map_or("png".into(), |e| e.to_string_lossy().to_lowercase());
        let character = dir.join(format!("character.{ext}"));
        atomically(&character, |part| Ok(std::fs::copy(&spec.character, part)?))?;
        spec.character = character;
        let job = Job {
            version: VERSION,
            id: created.to_string(),
            created,
            step: if matches!(spec.source, Source::Clip { .. }) { Step::Prep } else { Step::Upload },
            spec,
            end: None,
            prompt: None,
            submissions: 0,
            frames: None,
            output: None,
            detail: String::new(),
            ram: None,
            nodes: Vec::new(),
        };
        self.save(&job)?;
        Ok(job)
    }

    pub fn save(&self, job: &Job) -> Result<()> {
        let path = self.dir(&job.id).join("job.json");
        atomically(&path, |part| Ok(std::fs::write(part, serde_json::to_vec_pretty(job)?)?))
    }

    pub fn load(&self, id: &str) -> Result<Job> {
        // Ids come from the WebView and from agents; a job's is its creation time, so anything else is no job.
        anyhow::ensure!(!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()), "no job {id}");
        let path = self.dir(id).join("job.json");
        let job: Job = serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
            .with_context(|| format!("reading {}", path.display()))?;
        anyhow::ensure!(job.version == VERSION, "{} is version {}", path.display(), job.version);
        Ok(job)
    }

    /// Every job that can be read, oldest first.
    pub fn all(&self) -> Vec<Job> {
        let entries = std::fs::read_dir(&self.root).into_iter().flatten().flatten();
        let mut jobs: Vec<Job> = entries.filter_map(|e| self.load(&e.file_name().to_string_lossy()).ok()).collect();
        jobs.sort_by_key(|j| (j.created, j.id.clone()));
        jobs
    }

    /// Where prep's files are for this job.
    pub fn prep_dir(&self, job: &Job) -> PathBuf {
        match &job.spec.source {
            Source::Clip { .. } => self.dir(&job.id).join("prep"),
            Source::Prepared { folder } => folder.clone(),
        }
    }
}

/// The ComfyUI a job talks to.
pub trait Comfy {
    /// The first GPU's memory in bytes; it also tells whether ComfyUI still answers.
    fn vram(&self) -> Result<u64>;
    /// Puts a file in ComfyUI's input folder; returns the name nodes load it by.
    fn upload(&self, path: &Path, name: &str) -> Result<String>;
    fn queue(&self, graph: &Value) -> Result<String>;
    /// The prompt, queued, running or finished, whose graph saves under `prefix`.
    fn find(&self, prefix: &str) -> Result<Option<String>>;
    fn state(&self, prompt: &str) -> Result<PromptState>;
    fn cancel(&self, prompt: &str) -> Result<()>;
    /// Videos in ComfyUI's output folder under `prefix`, whole or not, oldest first.
    fn saved(&self, prefix: &str) -> Vec<Output>;
    fn fetch(&self, output: &Output, dest: &Path) -> Result<()>;
    /// The RAM watchdog of a ComfyUI on this machine; none for one elsewhere.
    fn guard(&self) -> Option<&Guard> {
        None
    }
    /// Node id and seconds of a prompt queued through this ComfyUI.
    fn node_times(&self, _prompt: &str) -> Vec<(String, f64)> {
        Vec::new()
    }
}

/// What a job needs besides ComfyUI.
pub trait Backend {
    type Comfy: Comfy;
    type Lease;
    /// Prepares the clip into `out`, holding the GPU for prep while it runs.
    fn prepare(&self, source: &Source, out: &Path, log: &mut dyn FnMut(&str)) -> Result<()>;
    /// The GPU for rendering, until the lease drops; `name` says for what.
    fn lease(&self, name: &str) -> Result<Self::Lease>;
    /// A ComfyUI that answers: the running one, or started (again).
    fn comfy(&self) -> Result<Self::Comfy>;
    /// ComfyUI's video cut to `frames`, with the sound of prep's `normalized.mp4`, into `out`.
    fn finish(&self, video: &Path, prep: &Path, out: &Path, frames: u32) -> Result<()>;
    /// The frames in a video; fails on one that is not whole.
    fn frames(&self, video: &Path) -> Result<u32>;
}

/// A ComfyUI over its HTTP API. With `output`, the server's output folder on this machine, read directly; without it
/// (a server elsewhere) files come over `/view` and a video saved before a restart is found through the history.
pub struct Api {
    pub client: Client,
    pub output: Option<PathBuf>,
    /// The watchdog of a ComfyUI this process started ([`crate::Server::guard`]).
    pub guard: Option<Arc<Guard>>,
}

impl Comfy for Api {
    fn vram(&self) -> Result<u64> {
        self.client.vram()
    }

    fn upload(&self, path: &Path, name: &str) -> Result<String> {
        self.client.upload(path, name)
    }

    fn queue(&self, graph: &Value) -> Result<String> {
        self.client.queue(graph)
    }

    fn find(&self, prefix: &str) -> Result<Option<String>> {
        self.client.find(prefix)
    }

    fn state(&self, prompt: &str) -> Result<PromptState> {
        self.client.state(prompt)
    }

    fn cancel(&self, prompt: &str) -> Result<()> {
        self.client.cancel(prompt)
    }

    fn saved(&self, prefix: &str) -> Vec<Output> {
        self.output.as_deref().map(|dir| saved_in(dir, prefix)).unwrap_or_default()
    }

    fn fetch(&self, output: &Output, dest: &Path) -> Result<()> {
        let local = self.output.as_ref().map(|dir| dir.join(&output.subfolder).join(&output.filename));
        if let Some(local) = local.filter(|l| output.kind == "output" && l.is_file()) {
            std::fs::copy(&local, dest).with_context(|| format!("copying {}", local.display()))?;
        } else {
            self.client.download(output, dest)?;
        }
        Ok(())
    }

    fn guard(&self) -> Option<&Guard> {
        self.guard.as_deref()
    }

    fn node_times(&self, prompt: &str) -> Vec<(String, f64)> {
        self.client.node_times(prompt)
    }
}

/// SaveVideo's files for `prefix` in an output folder: `<prefix>_<counter>_.mp4`.
pub fn saved_in(output: &Path, prefix: &str) -> Vec<Output> {
    let (subfolder, base) = prefix.rsplit_once('/').unwrap_or(("", prefix));
    let mut names: Vec<String> = std::fs::read_dir(output.join(subfolder))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&format!("{base}_")) && n.ends_with(".mp4"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|filename| Output { node: wan::SAVE_NODE.into(), filename, subfolder: subfolder.into(), kind: "output".into() })
        .collect()
}

/// Takes jobs to their end, one at a time.
pub struct Runner<'a, B: Backend> {
    pub jobs: &'a Jobs,
    pub backend: &'a B,
    /// Set to cancel the job being run.
    pub cancel: &'a AtomicBool,
    /// Called on every change of the job, saved or not.
    pub changed: &'a dyn Fn(&Job),
    /// How often ComfyUI is asked about a prompt.
    pub poll: Duration,
}

impl<B: Backend> Runner<'_, B> {
    /// Runs the job from where it stands (a fresh one, or one a kill interrupted) to an end. Fails only when
    /// `job.json` cannot be written; everything else ends the job as failed.
    pub fn run(&self, job: &mut Job) -> Result<()> {
        if !job.is_active() {
            return Ok(());
        }
        if job.step == Step::Prep
            && !self.cancelled()
            && let Err(e) = self.prep(job)
        {
            return self.end(job, failed(e));
        }
        if self.cancelled() {
            return self.end(job, End::Cancelled);
        }
        job.detail = "waiting for the GPU".into();
        (self.changed)(job);
        let _lease = match self.backend.lease(&format!("render {}", job.name())) {
            Ok(lease) => lease,
            Err(e) => return self.end(job, failed(e)),
        };
        // Cancelled while waiting for the GPU: end before starting ComfyUI for nothing.
        if self.cancelled() && job.prompt.is_none() {
            return self.end(job, End::Cancelled);
        }
        self.render(job)
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn save(&self, job: &Job) -> Result<()> {
        self.jobs.save(job)?;
        (self.changed)(job);
        Ok(())
    }

    fn end(&self, job: &mut Job, end: End) -> Result<()> {
        job.end = Some(end);
        job.detail.clear();
        self.save(job)
    }

    fn to(&self, job: &mut Job, step: Step) -> Result<()> {
        job.step = step;
        self.save(job)
    }

    fn prep(&self, job: &mut Job) -> Result<()> {
        let source = job.spec.source.clone();
        let out = self.jobs.prep_dir(job);
        self.backend.prepare(&source, &out, &mut |line| {
            job.detail = line.to_string();
            (self.changed)(job);
        })?;
        self.to(job, Step::Upload)
    }

    fn render(&self, job: &mut Job) -> Result<()> {
        let mut comfy = match self.backend.comfy() {
            Ok(comfy) => comfy,
            Err(e) => return self.end(job, failed(e)),
        };
        let mut restarts = 0;
        while job.is_active() {
            if self.cancelled() {
                return self.stop(job, &comfy);
            }
            let Err(e) = self.step(job, &comfy) else { continue };
            // Stopped by the RAM watchdog: starting it again would run out the same way.
            if let Some(reason) = comfy.guard().and_then(Guard::tripped) {
                return self.end(job, End::Failed { reason });
            }
            // A ComfyUI that still answers failed for real; one that is gone is started again and asked anew.
            let lost = job.step < Step::Finishing && comfy.vram().is_err();
            if !lost || restarts == MAX_RESTARTS {
                return self.end(job, failed(e));
            }
            restarts += 1;
            job.detail = "ComfyUI stopped; starting it again".into();
            (self.changed)(job);
            comfy = match self.backend.comfy() {
                Ok(comfy) => comfy,
                Err(e) => return self.end(job, failed(e)),
            };
        }
        Ok(())
    }

    fn step(&self, job: &mut Job, comfy: &B::Comfy) -> Result<()> {
        match job.step {
            Step::Prep => unreachable!("prep runs before the render"),
            Step::Upload => self.submit(job, comfy),
            Step::Queued | Step::Rendering => self.follow(job, comfy),
            Step::Downloading => {
                let raw = self.jobs.dir(&job.id).join("comfy.mp4");
                if !raw.is_file() {
                    let output = job.output.clone().context("the job has no ComfyUI video to download")?;
                    job.detail = "downloading".into();
                    (self.changed)(job);
                    atomically(&raw, |part| comfy.fetch(&output, part))?;
                }
                self.to(job, Step::Finishing)
            }
            Step::Finishing => {
                let dir = self.jobs.dir(&job.id);
                let result = dir.join("result.mp4");
                if !result.is_file() {
                    let frames = job.frames.context("the job has no frame count")?;
                    job.detail = "finishing".into();
                    (self.changed)(job);
                    let prep = self.jobs.prep_dir(job);
                    atomically(&result, |part| self.backend.finish(&dir.join("comfy.mp4"), &prep, part, frames))?;
                }
                self.end(job, End::Done { result })
            }
        }
    }

    /// Queues the graph, unless ComfyUI already has it or its video.
    fn submit(&self, job: &mut Job, comfy: &B::Comfy) -> Result<()> {
        if self.collect(job, comfy)? {
            return Ok(());
        }
        if let Some(prompt) = comfy.find(&job.prefix())? {
            job.prompt = Some(prompt);
            return self.to(job, Step::Queued);
        }
        job.detail = "uploading".into();
        (self.changed)(job);
        let dir = self.jobs.dir(&job.id);
        let prep = self.jobs.prep_dir(job);
        let name = |what: &str| format!("kawarimi-{}-{what}", job.id);
        let (source, mask, character) = (name("source.mp4"), name("mask.mp4"), name("character.png"));
        let reference = dir.join("reference.png");
        let graph_file = dir.join("graph.json");
        let graph: Value = match std::fs::read(&graph_file).ok().filter(|_| job.frames.is_some()) {
            Some(bytes) => serde_json::from_slice(&bytes)?,
            None => {
                let clip = Clip::read(&prep)?;
                let vram = comfy.vram()?;
                let model = job.spec.settings.variant(vram)?;
                let uploads = Uploads { source: source.clone(), mask: mask.clone(), character: character.clone() };
                let inputs = job.spec.settings.inputs(&clip, vram, model, uploads, &job.prefix());
                let image = image::open(&job.spec.character).with_context(|| format!("reading {}", job.spec.character.display()))?;
                atomically(&reference, |part| Ok(wan::reference(&image, inputs.width, inputs.height).save(part)?))?;
                let graph = render::graph(&inputs, model)?;
                job.frames = Some(inputs.frames);
                self.save(job)?;
                atomically(&graph_file, |part| Ok(std::fs::write(part, serde_json::to_vec(&graph)?)?))?;
                graph
            }
        };
        self.check_ram(job, comfy)?;
        for (file, name) in [(prep.join("normalized.mp4"), &source), (prep.join("mask.mp4"), &mask), (reference, &character)] {
            let stored = comfy.upload(&file, name)?;
            anyhow::ensure!(&stored == name, "ComfyUI stored {} as {stored}", file.display());
        }
        if job.submissions >= MAX_SUBMISSIONS {
            bail!("ComfyUI lost the render {} times", job.submissions);
        }
        job.submissions += 1;
        job.prompt = None;
        self.save(job)?;
        if let Some(guard) = comfy.guard() {
            guard.reset();
        }
        job.prompt = Some(comfy.queue(&graph)?);
        self.to(job, Step::Queued)
    }

    /// Refuses a render that RAM and page file together cannot hold; says so when it will page.
    fn check_ram(&self, job: &mut Job, comfy: &B::Comfy) -> Result<()> {
        let Some(guard) = comfy.guard() else { return Ok(()) };
        let Some(memory) = ram::Memory::read() else { return Ok(()) };
        let fit = ram::fit(ram::need(job.spec.settings.variant(comfy.vram()?)?), memory, guard.held());
        match (fit.verdict, fit.message) {
            (Verdict::Short, Some(message)) => bail!(message),
            (Verdict::Pages, Some(message)) => {
                job.detail = message;
                (self.changed)(job);
            }
            _ => {}
        }
        Ok(())
    }

    /// Follows the queued prompt until it saved its video, failed, or turned out lost.
    fn follow(&self, job: &mut Job, comfy: &B::Comfy) -> Result<()> {
        let Some(prompt) = job.prompt.clone() else { return self.to(job, Step::Upload) };
        let started = Instant::now();
        while !self.cancelled() {
            if let Some(guard) = comfy.guard() {
                job.ram = Stats::merged(job.ram, guard.stats());
            }
            match comfy.state(&prompt)? {
                PromptState::Pending => job.detail = "waiting in ComfyUI's queue".into(),
                PromptState::Running => {
                    if job.step == Step::Queued {
                        self.to(job, Step::Rendering)?;
                    }
                    job.detail = format!("rendering, {} s", started.elapsed().as_secs());
                }
                PromptState::Done(outputs) => {
                    let video = outputs.into_iter().find(|o| o.node == wan::SAVE_NODE).context("the graph saved no video")?;
                    job.output = Some(video);
                    job.nodes = self.node_times(job, comfy, &prompt);
                    return self.to(job, Step::Downloading);
                }
                PromptState::Failed(error) => bail!("ComfyUI failed: {error}"),
                PromptState::Unknown => {
                    if self.collect(job, comfy)? {
                        return Ok(());
                    }
                    job.prompt = None;
                    job.detail = "ComfyUI lost the render; queueing it again".into();
                    return self.to(job, Step::Upload);
                }
            }
            (self.changed)(job);
            std::thread::sleep(self.poll);
        }
        Ok(())
    }

    /// The prompt's node times with their classes from `graph.json`, longest first.
    fn node_times(&self, job: &Job, comfy: &B::Comfy, prompt: &str) -> Vec<NodeTime> {
        let graph: Value = std::fs::read(self.jobs.dir(&job.id).join("graph.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut nodes: Vec<NodeTime> = comfy
            .node_times(prompt)
            .into_iter()
            .map(|(node, seconds)| NodeTime { class_type: graph[&node]["class_type"].as_str().unwrap_or("?").into(), node, seconds })
            .collect();
        nodes.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
        nodes
    }

    /// Takes a whole video the graph saved before ComfyUI forgot the prompt (it was restarted), so it is not
    /// rendered again. One cut short by a kill while saving is passed over.
    fn collect(&self, job: &mut Job, comfy: &B::Comfy) -> Result<bool> {
        let Some(frames) = job.frames else { return Ok(false) };
        let raw = self.jobs.dir(&job.id).join("comfy.mp4");
        for output in comfy.saved(&job.prefix()) {
            let whole = atomically(&raw, |part| {
                comfy.fetch(&output, part)?;
                let have = self.backend.frames(part)?;
                anyhow::ensure!(have >= frames, "{have} of {frames} frames");
                Ok(())
            });
            if whole.is_ok() {
                job.output = Some(output);
                self.to(job, Step::Finishing)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Ends the job as cancelled first, so a kill right after cannot resume it, then takes its prompt off ComfyUI.
    fn stop(&self, job: &mut Job, comfy: &B::Comfy) -> Result<()> {
        let step = job.step;
        self.end(job, End::Cancelled)?;
        if step <= Step::Rendering {
            let prompt = job.prompt.clone().or_else(|| comfy.find(&job.prefix()).ok().flatten());
            if let Some(prompt) = prompt {
                let _ = comfy.cancel(&prompt);
            }
        }
        Ok(())
    }
}

fn failed(e: anyhow::Error) -> End {
    End::Failed { reason: format!("{e:#}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::rc::Rc;

    use crate::wan::Quality;

    /// Frames the fake prep's clip has; a draft on 8 GB renders 33 of them.
    const CLIP_FRAMES: u32 = 40;

    /// A ComfyUI in memory with a real output folder. A prompt runs for `RUN_POLLS` polls, then saves its video.
    #[derive(Default)]
    struct World {
        alive: bool,
        next: u32,
        pending: Vec<(String, Value)>,
        running: Option<(String, Value, u32)>,
        history: HashMap<String, (Value, PromptState)>,
        queued: u32,
        /// A queue while ComfyUI had the prompt or its whole video: a render made twice.
        duplicates: u32,
        cancelled: Vec<String>,
        restarts: u32,
        /// The graph fails at this node.
        fail: Option<String>,
        /// Renders during which ComfyUI dies on its own, at the second poll.
        deaths: u32,
        /// Set the cancel flag at this many polls into a render.
        cancel_at: Option<u32>,
        /// Calls left before the app is killed.
        budget: Option<u32>,
        output: PathBuf,
    }

    const RUN_POLLS: u32 = 3;

    fn frames_of(graph: &Value) -> u32 {
        graph["899"]["inputs"]["length"].as_u64().unwrap() as u32
    }

    fn prefix_of(graph: &Value) -> String {
        graph[wan::SAVE_NODE]["inputs"]["filename_prefix"].as_str().unwrap().into()
    }

    impl World {
        /// One call; the app dies after it took effect when the budget runs out.
        fn tick(&mut self) {
            if let Some(budget) = &mut self.budget {
                if *budget == 0 {
                    panic!("killed");
                }
                *budget -= 1;
            }
        }

        /// A kill right after a call took effect, before the app saw the reply.
        fn kill_point(&mut self) {
            self.tick();
        }

        fn whole_videos(&self, prefix: &str) -> usize {
            saved_in(&self.output, prefix).iter().filter(|o| read_frames(&self.output.join(&o.subfolder).join(&o.filename)).is_ok()).count()
        }

        /// ComfyUI killed: the queue and history are gone, a video being saved is left cut short.
        fn kill_comfy(&mut self) {
            if let Some((_, graph, _)) = self.running.take() {
                let file = self.next_file(&prefix_of(&graph));
                std::fs::write(file, "partial").unwrap();
            }
            self.pending.clear();
            self.history.clear();
            self.alive = false;
        }

        fn next_file(&self, prefix: &str) -> PathBuf {
            let (sub, base) = prefix.rsplit_once('/').unwrap();
            let dir = self.output.join(sub);
            std::fs::create_dir_all(&dir).unwrap();
            let n = saved_in(&self.output, prefix).len() + 1;
            dir.join(format!("{base}_{n:05}_.mp4"))
        }

        fn advance(&mut self) {
            if self.running.is_none() && !self.pending.is_empty() {
                let (id, graph) = self.pending.remove(0);
                self.running = Some((id, graph, 0));
            }
            let Some((id, graph, polls)) = &mut self.running else { return };
            *polls += 1;
            if self.cancel_at == Some(*polls) {
                CANCEL.with(|c| c.set(true));
            }
            if *polls == 2 && self.deaths > 0 {
                self.deaths -= 1;
                self.kill_comfy();
                return;
            }
            if *polls < RUN_POLLS {
                return;
            }
            let (id, graph) = (id.clone(), graph.clone());
            self.running = None;
            if let Some(node) = &self.fail {
                let status = PromptState::Failed(format!("node {node} (DWPreprocessor): out of memory"));
                self.history.insert(id, (graph, status));
                return;
            }
            let file = self.next_file(&prefix_of(&graph));
            std::fs::write(&file, format!("frames={}", frames_of(&graph))).unwrap();
            let output = Output {
                node: wan::SAVE_NODE.into(),
                filename: file.file_name().unwrap().to_string_lossy().into(),
                subfolder: prefix_of(&graph).rsplit_once('/').unwrap().0.into(),
                kind: "output".into(),
            };
            self.history.insert(id, (graph, PromptState::Done(vec![output])));
        }

        fn has(&self, prefix: &str) -> Option<String> {
            let queued = self.running.iter().map(|(id, g, _)| (id, g)).chain(self.pending.iter().map(|(id, g)| (id, g)));
            let finished = self.history.iter().map(|(id, (g, _))| (id, g));
            queued.chain(finished).find(|(_, g)| prefix_of(g) == prefix).map(|(id, _)| id.clone())
        }
    }

    thread_local! {
        static CANCEL: Cell<bool> = const { Cell::new(false) };
    }

    fn read_frames(path: &Path) -> Result<u32> {
        let text = std::fs::read_to_string(path)?;
        Ok(text.strip_prefix("frames=").context("not a whole video")?.parse()?)
    }

    #[derive(Clone)]
    struct Fake(Rc<RefCell<World>>);

    impl Fake {
        fn alive(&self) -> Result<std::cell::RefMut<'_, World>> {
            let mut world = self.0.borrow_mut();
            world.tick();
            anyhow::ensure!(world.alive, "connection refused");
            Ok(world)
        }
    }

    impl Comfy for Fake {
        fn vram(&self) -> Result<u64> {
            self.alive().map(|_| 8 << 30)
        }

        fn upload(&self, path: &Path, name: &str) -> Result<String> {
            let _world = self.alive()?;
            anyhow::ensure!(path.is_file(), "no {}", path.display());
            Ok(name.into())
        }

        fn queue(&self, graph: &Value) -> Result<String> {
            let mut world = self.alive()?;
            let prefix = prefix_of(graph);
            if world.has(&prefix).is_some() || world.whole_videos(&prefix) > 0 {
                world.duplicates += 1;
            }
            world.next += 1;
            world.queued += 1;
            let id = format!("p{}", world.next);
            world.pending.push((id.clone(), graph.clone()));
            world.kill_point();
            Ok(id)
        }

        fn find(&self, prefix: &str) -> Result<Option<String>> {
            Ok(self.alive()?.has(prefix))
        }

        fn state(&self, prompt: &str) -> Result<PromptState> {
            let mut world = self.alive()?;
            world.advance();
            anyhow::ensure!(world.alive, "connection reset");
            Ok(if world.running.as_ref().is_some_and(|(id, ..)| id == prompt) {
                PromptState::Running
            } else if world.pending.iter().any(|(id, _)| id == prompt) {
                PromptState::Pending
            } else {
                world.history.get(prompt).map_or(PromptState::Unknown, |(_, s)| s.clone())
            })
        }

        fn cancel(&self, prompt: &str) -> Result<()> {
            let mut world = self.alive()?;
            world.pending.retain(|(id, _)| id != prompt);
            if world.running.as_ref().is_some_and(|(id, ..)| id == prompt) {
                world.running = None;
            }
            world.cancelled.push(prompt.into());
            Ok(())
        }

        fn saved(&self, prefix: &str) -> Vec<Output> {
            let mut world = self.0.borrow_mut();
            world.tick();
            saved_in(&world.output, prefix)
        }

        fn fetch(&self, output: &Output, dest: &Path) -> Result<()> {
            let mut world = self.0.borrow_mut();
            world.tick();
            std::fs::copy(world.output.join(&output.subfolder).join(&output.filename), dest)?;
            Ok(())
        }
    }

    impl Backend for Fake {
        type Comfy = Fake;
        type Lease = ();

        fn prepare(&self, _: &Source, out: &Path, log: &mut dyn FnMut(&str)) -> Result<()> {
            self.0.borrow_mut().tick();
            log("tracking");
            write_prep(out);
            self.0.borrow_mut().kill_point();
            Ok(())
        }

        fn lease(&self, _: &str) -> Result<()> {
            Ok(())
        }

        fn comfy(&self) -> Result<Fake> {
            let mut world = self.0.borrow_mut();
            world.tick();
            if !world.alive {
                world.alive = true;
                world.restarts += 1;
            }
            Ok(self.clone())
        }

        fn finish(&self, video: &Path, _: &Path, out: &Path, frames: u32) -> Result<()> {
            self.0.borrow_mut().tick();
            let have = read_frames(video)?;
            anyhow::ensure!(have >= frames, "{have} frames, the clip needs {frames}");
            std::fs::write(out, format!("finished {frames}"))?;
            self.0.borrow_mut().kill_point();
            Ok(())
        }

        fn frames(&self, video: &Path) -> Result<u32> {
            self.0.borrow_mut().tick();
            read_frames(video)
        }
    }

    fn write_prep(out: &Path) {
        std::fs::create_dir_all(out).unwrap();
        let manifest = serde_json::json!({ "video": { "width": 1920, "height": 1080 }, "frames": CLIP_FRAMES });
        std::fs::write(out.join("manifest.json"), manifest.to_string()).unwrap();
        std::fs::write(out.join("normalized.mp4"), "clip").unwrap();
        std::fs::write(out.join("mask.mp4"), "mask").unwrap();
    }

    struct Setup {
        _tmp: tempfile::TempDir,
        jobs: Jobs,
        fake: Fake,
        job: Job,
    }

    fn setup(from_clip: bool) -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("comfy-output");
        let fake = Fake(Rc::new(RefCell::new(World { alive: true, output, ..Default::default() })));
        let jobs = Jobs::new(tmp.path().join("jobs"));
        let character = tmp.path().join("character.png");
        image::RgbImage::from_pixel(40, 80, image::Rgb([200, 50, 50])).save(&character).unwrap();
        let source = if from_clip {
            let clip = tmp.path().join("clip.mp4");
            std::fs::write(&clip, "clip").unwrap();
            Source::Clip { path: clip, start: 0.0, duration: None, targets: None }
        } else {
            let folder = tmp.path().join("prep");
            write_prep(&folder);
            Source::Prepared { folder }
        };
        let settings = Settings { quality: Quality::Draft, seed: 7, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
        let job = jobs.create(Spec { source, character, settings }).unwrap();
        Setup { _tmp: tmp, jobs, fake, job }
    }

    /// Runs the job as the app would after a start: loaded from disk. True when it got to an end without a kill.
    fn run(s: &Setup) -> bool {
        CANCEL.with(|c| c.set(false));
        let cancel = AtomicBool::new(false);
        let mut job = s.jobs.load(&s.job.id).unwrap();
        let changed = |_: &Job| {
            if CANCEL.with(|c| c.get()) {
                cancel.store(true, Ordering::SeqCst);
            }
        };
        let runner = Runner { jobs: &s.jobs, backend: &s.fake, cancel: &cancel, changed: &changed, poll: Duration::ZERO };
        catch_unwind(AssertUnwindSafe(|| runner.run(&mut job).unwrap())).is_ok()
    }

    fn ended(s: &Setup) -> Job {
        let job = s.jobs.load(&s.job.id).unwrap();
        assert!(job.end.is_some(), "{job:?}");
        job
    }

    fn assert_one_result(s: &Setup) {
        let job = ended(s);
        let End::Done { result } = job.end.as_ref().unwrap() else { panic!("{job:?}") };
        assert_eq!(std::fs::read_to_string(result).unwrap(), "finished 33");
        let world = s.fake.0.borrow();
        assert_eq!(world.duplicates, 0, "queued while ComfyUI had the render");
        assert_eq!(world.whole_videos(&job.prefix()), 1, "{job:?}");
        let parts = std::fs::read_dir(s.jobs.dir(&job.id)).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().contains(".part"));
        assert_eq!(parts.count(), 0);
    }

    #[test]
    fn an_id_that_is_not_a_job_reads_nothing_outside_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Jobs::new(dir.path().join("jobs"));
        std::fs::write(dir.path().join("job.json"), "{}").unwrap();
        for id in ["..", "../jobs", "", "12/..", "/etc"] {
            assert!(jobs.load(id).unwrap_err().to_string().starts_with("no job"), "{id}");
        }
    }

    #[test]
    fn a_job_runs_from_the_clip_to_the_result() {
        let s = setup(true);
        assert_eq!(s.job.step, Step::Prep);
        assert!(run(&s));
        assert_one_result(&s);
        let job = ended(&s);
        assert_eq!((job.submissions, job.frames), (1, Some(33)));
        let graph: Value = serde_json::from_slice(&std::fs::read(s.jobs.dir(&job.id).join("graph.json")).unwrap()).unwrap();
        assert_eq!(prefix_of(&graph), format!("kawarimi/{}/render", job.id));
        assert_eq!(graph["102"]["inputs"]["seed"], 7);
        assert!(s.jobs.dir(&job.id).join("character.png").is_file());
    }

    /// Kills the app after every call it makes in turn, then starts it again (`app`: with a new ComfyUI, as on
    /// Windows, where ComfyUI dies with the app; else with the same one still running).
    fn kill_everywhere(app: bool) {
        for calls in 0.. {
            let s = setup(true);
            s.fake.0.borrow_mut().budget = Some(calls);
            if run(&s) {
                assert_one_result(&s);
                assert!(calls > 10, "the kill never landed");
                return;
            }
            {
                let mut world = s.fake.0.borrow_mut();
                world.budget = None;
                if app {
                    world.kill_comfy();
                }
            }
            assert!(s.jobs.load(&s.job.id).unwrap().is_active());
            assert!(run(&s), "killed with no budget");
            assert_one_result(&s);
            let world = s.fake.0.borrow();
            assert!(world.queued <= if app { 2 } else { 1 }, "after {calls} calls: queued {}", world.queued);
        }
    }

    #[test]
    fn a_kill_of_the_app_and_comfy_at_any_point_neither_loses_nor_repeats_the_result() {
        kill_everywhere(true);
    }

    #[test]
    fn a_kill_of_the_app_alone_at_any_point_never_queues_again() {
        kill_everywhere(false);
    }

    #[test]
    fn a_comfy_that_dies_mid_render_is_started_and_queued_again() {
        let s = setup(false);
        s.fake.0.borrow_mut().deaths = 1;
        assert!(run(&s));
        assert_one_result(&s);
        assert_eq!(ended(&s).submissions, 2);
        assert_eq!(s.fake.0.borrow().restarts, 1);
    }

    #[test]
    fn a_video_found_after_a_restart_is_taken_not_rendered_again() {
        let s = setup(false);
        s.fake.0.borrow_mut().budget = Some(10);
        assert!(!run(&s), "the budget kills the run");
        let mut world = s.fake.0.borrow_mut();
        world.budget = None;
        // Let the render finish, then lose ComfyUI's history.
        while world.running.is_some() || !world.pending.is_empty() {
            world.advance();
        }
        world.kill_comfy();
        drop(world);
        assert!(run(&s));
        assert_one_result(&s);
        assert_eq!(s.fake.0.borrow().queued, 1);
    }

    #[test]
    fn a_failed_graph_ends_the_job_with_the_node() {
        let s = setup(false);
        s.fake.0.borrow_mut().fail = Some("14".into());
        assert!(run(&s));
        let End::Failed { reason } = ended(&s).end.unwrap() else { panic!() };
        assert!(reason.contains("node 14 (DWPreprocessor): out of memory"), "{reason}");
        assert_eq!(s.fake.0.borrow().queued, 1);
    }

    #[test]
    fn a_comfy_that_keeps_losing_the_render_fails_the_job() {
        let s = setup(false);
        s.fake.0.borrow_mut().deaths = 10;
        assert!(run(&s));
        let End::Failed { reason } = ended(&s).end.unwrap() else { panic!() };
        assert!(reason.contains("lost the render 3 times"), "{reason}");
        assert_eq!(s.fake.0.borrow().queued, 3);
    }

    #[test]
    fn cancel_takes_only_this_prompt_off_comfy() {
        let s = setup(false);
        s.fake.0.borrow_mut().cancel_at = Some(2);
        assert!(run(&s));
        let job = ended(&s);
        assert_eq!(job.end, Some(End::Cancelled));
        let world = s.fake.0.borrow();
        assert_eq!(world.cancelled, vec![job.prompt.clone().unwrap()]);
        assert!(world.running.is_none() && world.history.is_empty());
        assert!(!s.jobs.dir(&job.id).join("result.mp4").exists());
        drop(world);
        assert!(run(&s), "a cancelled job stays cancelled");
        assert_eq!(s.fake.0.borrow().queued, 1);
    }

    #[test]
    fn a_job_cancelled_before_it_starts_never_reaches_comfy() {
        let s = setup(true);
        let cancel = AtomicBool::new(true);
        let mut job = s.job.clone();
        let runner = Runner { jobs: &s.jobs, backend: &s.fake, cancel: &cancel, changed: &|_| {}, poll: Duration::ZERO };
        runner.run(&mut job).unwrap();
        assert_eq!(ended(&s).end, Some(End::Cancelled));
        assert_eq!(s.fake.0.borrow().queued, 0);
    }

    #[test]
    fn a_job_cancelled_while_waiting_for_the_gpu_never_reaches_comfy() {
        let s = setup(false);
        let cancel = AtomicBool::new(false);
        let mut job = s.jobs.load(&s.job.id).unwrap();
        let changed = |job: &Job| {
            if job.detail == "waiting for the GPU" {
                cancel.store(true, Ordering::SeqCst);
            }
        };
        let runner = Runner { jobs: &s.jobs, backend: &s.fake, cancel: &cancel, changed: &changed, poll: Duration::ZERO };
        runner.run(&mut job).unwrap();
        assert_eq!(ended(&s).end, Some(End::Cancelled));
        assert_eq!(s.fake.0.borrow().queued, 0);
    }

    #[test]
    fn a_kill_leaves_no_half_file_under_the_final_name() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("result.mp4");
        assert!(atomically::<()>(&path, |part| {
            assert_eq!(part.file_name().unwrap(), "result.part.mp4");
            std::fs::write(part, "half")?;
            bail!("killed")
        })
        .is_err());
        assert!(!path.exists() && !tmp.path().join("result.part.mp4").exists());
        atomically(&path, |part| Ok(std::fs::write(part, "whole")?)).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "whole");
    }

    #[test]
    fn jobs_load_oldest_first() {
        let s = setup(false);
        let second = s.jobs.create(s.job.spec.clone()).unwrap();
        assert_ne!(second.id, s.job.id);
        let ids: Vec<String> = s.jobs.all().into_iter().map(|j| j.id).collect();
        assert_eq!(ids, vec![s.job.id.clone(), second.id]);
    }
}
