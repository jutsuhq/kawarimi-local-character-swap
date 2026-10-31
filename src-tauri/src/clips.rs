//! Create's clips: a trimmed source, its prep, a character per person and their prompts, in `<clips>/<id>/`:
//! `clip.json`, `prep-<time>/` (the current prep; a new one is made beside it and swapped in only when it succeeds, so
//! a cancelled or failed prep leaves the clip as it was), `characters/` and `renders/<n>/` (the prep as one render
//! sees it: the clip, that person's mask and the manifest; a job's `Source::Prepared` points there).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use kawarimi_assets::{Status, Store, manifest};
use kawarimi_comfy::job::{End, Job, Source, Spec};
use kawarimi_comfy::ram;
use kawarimi_comfy::render::{Clip as RenderClip, Estimate, Settings};
use kawarimi_comfy::wan::{self, Quality};
use kawarimi_prep::fix::Fix;
use kawarimi_prep::{Models, Options, Targets};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::async_runtime::block_on;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::gpu::{Gpu, Work};
use crate::jobs::Renders;

/// The trim Create allows, in seconds.
const MIN_SECONDS: f64 = 4.0;
const MAX_SECONDS: f64 = 30.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    /// Milliseconds since the Unix epoch.
    pub created: u64,
    pub source: PathBuf,
    pub start: f64,
    pub duration: f64,
    /// Who the clip's mask covers, identity ids; empty is prep's main person.
    #[serde(default)]
    pub targets: Vec<usize>,
    /// The current prep folder's name, once one succeeded.
    #[serde(default)]
    pub prep: Option<String>,
    /// Why the last prep did not finish.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub characters: BTreeMap<usize, Character>,
    #[serde(default)]
    pub renders: Vec<RenderRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    /// The image as picked, copied into `characters/`.
    pub image: PathBuf,
    /// On grey, once cut out.
    pub cutout: Option<PathBuf>,
    pub use_cutout: bool,
    /// As edited; `None` renders with the template.
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRef {
    pub job: String,
    pub person: usize,
}

/// A clip as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipView {
    #[serde(flatten)]
    pub clip: Clip,
    pub dir: PathBuf,
    pub prep_dir: Option<PathBuf>,
    pub manifest: Option<Value>,
    pub running: Option<Running>,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Running {
    /// "Waiting for the GPU", a stage's name, or "Fixing the person".
    pub stage: String,
    pub fraction: f32,
}

pub struct Clips {
    root: PathBuf,
    store: Store,
    gpu: Gpu,
    renders: Arc<Renders>,
    app: AppHandle,
    /// Preps on their way, with their cancel flags.
    running: Mutex<HashMap<String, (Running, Arc<AtomicBool>)>>,
    /// Clip files are read and written under this.
    files: Mutex<()>,
}

impl Clips {
    pub fn new(root: PathBuf, store: Store, gpu: Gpu, renders: Arc<Renders>, app: AppHandle) -> Arc<Self> {
        let clips = Arc::new(Self { root, store, gpu, renders, app, running: Default::default(), files: Default::default() });
        let _ = std::fs::create_dir_all(&clips.root);
        let _ = clips.app.asset_protocol_scope().allow_directory(&clips.root, true);
        for clip in clips.all() {
            let _ = clips.app.asset_protocol_scope().allow_file(&clip.source);
            clips.sweep(&clip);
        }
        clips
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn load(&self, id: &str) -> Result<Clip> {
        // Ids come from the WebView; a clip's is its creation time, so anything else is no clip.
        anyhow::ensure!(!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()), "no clip {id}");
        let path = self.dir(id).join("clip.json");
        serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
            .with_context(|| format!("reading {}", path.display()))
    }

    fn save(&self, clip: &Clip) -> Result<()> {
        let path = self.dir(&clip.id).join("clip.json");
        kawarimi_comfy::job::atomically(&path, |part| Ok(std::fs::write(part, serde_json::to_vec_pretty(clip)?)?))
    }

    /// Loads, changes and saves a clip, then tells the UI.
    fn update(&self, id: &str, change: impl FnOnce(&mut Clip) -> Result<()>) -> Result<ClipView> {
        let clip = {
            let _files = self.files.lock().unwrap();
            let mut clip = self.load(id)?;
            change(&mut clip)?;
            self.save(&clip)?;
            clip
        };
        let view = self.view(clip);
        let _ = self.app.emit("clip-state", &view);
        Ok(view)
    }

    fn all(&self) -> Vec<Clip> {
        let entries = std::fs::read_dir(&self.root).into_iter().flatten().flatten();
        let mut clips: Vec<Clip> = entries.filter_map(|e| self.load(&e.file_name().to_string_lossy()).ok()).collect();
        clips.sort_by_key(|c| std::cmp::Reverse(c.created));
        clips
    }

    /// Prep folders a kill or a locked file left behind.
    fn sweep(&self, clip: &Clip) {
        let entries = std::fs::read_dir(self.dir(&clip.id)).into_iter().flatten().flatten();
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("prep-") && Some(&name) != clip.prep.as_ref() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    pub fn view(&self, clip: Clip) -> ClipView {
        let dir = self.dir(&clip.id);
        let prep_dir = clip.prep.as_ref().map(|p| dir.join(p));
        let manifest = prep_dir
            .as_ref()
            .and_then(|p| std::fs::read(p.join("manifest.json")).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let running = self.running.lock().unwrap().get(&clip.id).map(|(r, _)| r.clone());
        let ids: Vec<&str> = clip.renders.iter().map(|r| r.job.as_str()).collect();
        let jobs: Vec<Job> = self.renders.list().into_iter().filter(|j| ids.contains(&j.id.as_str())).collect();
        // Results live in the jobs folder, outside the clips the asset scope allows.
        for job in &jobs {
            if let Some(End::Done { result }) = &job.end {
                let _ = self.app.asset_protocol_scope().allow_file(result);
            }
        }
        ClipView { clip, dir, prep_dir, manifest, running, jobs }
    }

    fn get(&self, id: &str) -> Result<ClipView> {
        Ok(self.view(self.load(id)?))
    }

    fn ffmpeg(&self) -> Result<kawarimi_media::Ffmpeg> {
        crate::ffmpeg(&self.store).context("FFmpeg is not installed; add it in Settings")
    }

    fn create(&self, source: PathBuf, start: f64, duration: f64) -> Result<ClipView> {
        anyhow::ensure!(source.is_file(), "no video at {}", source.display());
        anyhow::ensure!(
            (MIN_SECONDS - 0.01..=MAX_SECONDS + 0.01).contains(&duration),
            "a clip is {MIN_SECONDS}–{MAX_SECONDS} s; this range is {duration:.1} s"
        );
        let mut created = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        let dir = loop {
            let dir = self.dir(&created.to_string());
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => created += 1,
                Err(e) => return Err(e).with_context(|| format!("creating {}", dir.display())),
            }
        };
        let clip = Clip {
            id: created.to_string(),
            created,
            source,
            start,
            duration,
            targets: Vec::new(),
            prep: None,
            error: None,
            characters: BTreeMap::new(),
            renders: Vec::new(),
        };
        self.save(&clip)?;
        let _ = std::fs::create_dir_all(dir.join("characters"));
        Ok(self.view(clip))
    }

    fn remove(&self, id: &str) -> Result<()> {
        let view = self.get(id)?;
        anyhow::ensure!(view.running.is_none(), "stop its prep first");
        anyhow::ensure!(!view.jobs.iter().any(Job::is_active), "cancel its render first");
        std::fs::remove_dir_all(self.dir(id)).with_context(|| format!("removing {}", self.dir(id).display()))
    }

    /// Starts a prep of the clip in the background: the whole pipeline the first time, then only what changed (the
    /// targets, or a fix).
    fn prepare(self: &Arc<Self>, id: &str, targets: Option<Vec<usize>>, fix: Option<Fix>) -> Result<ClipView> {
        let clip = self.load(id)?;
        if fix.is_some() {
            anyhow::ensure!(clip.prep.is_some(), "prep the clip before fixing a person");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = self.running.lock().unwrap();
            anyhow::ensure!(!running.contains_key(id), "this clip is already being prepared");
            let stage = Running { stage: "Waiting for the GPU".into(), fraction: 0.0 };
            running.insert(id.to_owned(), (stage, cancel.clone()));
        }
        let clips = self.clone();
        let owned = id.to_owned();
        std::thread::spawn(move || {
            let id = owned;
            let result = clips.run_prep(&id, targets, fix, &cancel);
            clips.running.lock().unwrap().remove(&id);
            let outcome = match result {
                Ok(()) => None,
                Err(e) if e.is::<kawarimi_prep::Cancelled>() => None,
                Err(e) => Some(human(&e)),
            };
            if let Ok(clip) = clips.update(&id, |clip| {
                clip.error = outcome;
                Ok(())
            }) {
                clips.sweep(&clip.clip);
            }
        });
        self.get(id)
    }

    fn progress(&self, id: &str, stage: impl Into<String>, fraction: f32) {
        if let Some((running, _)) = self.running.lock().unwrap().get_mut(id) {
            running.stage = stage.into();
            running.fraction = fraction;
        }
        if let Ok(view) = self.get(id) {
            let _ = self.app.emit("clip-state", &view);
        }
    }

    fn run_prep(&self, id: &str, targets: Option<Vec<usize>>, fix: Option<Fix>, cancel: &Arc<AtomicBool>) -> Result<()> {
        let clip = self.load(id)?;
        let ffmpeg = self.ffmpeg()?;
        let models = Models::installed(&self.store).map_err(|e| anyhow::anyhow!(human(&e)))?;
        let dir = self.dir(id);
        // A name never used before, so the WebView does not show a cached video of an earlier prep.
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let next = format!("prep-{stamp}");
        let out = dir.join(&next);
        std::fs::create_dir_all(&out)?;
        // What the current prep can lend the new one: its clip and shots, and its tracklets.
        if let Some(current) = clip.prep.as_ref().map(|p| dir.join(p)) {
            for file in ["cache.json", "manifest.json"] {
                if current.join(file).is_file() {
                    std::fs::copy(current.join(file), out.join(file))?;
                }
            }
            link_or_copy(&current.join("normalized.mp4"), &out.join("normalized.mp4"))?;
        }
        let name = clip.source.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let lease = block_on(self.gpu.lease(Work::Prep, format!("prep {name}")));
        let result = lease.and_then(|_lease| {
            if let Some(fix) = &fix {
                self.progress(id, "Following the person through the shot", 0.1);
                kawarimi_prep::fix::apply(&ffmpeg, &models, &out, fix, |_| {})?;
            }
            let targets = targets.unwrap_or_else(|| clip.targets.clone());
            let options = Options {
                normalize: kawarimi_media::Normalize { start: clip.start, duration: Some(clip.duration), ..Default::default() },
                targets: if targets.is_empty() { Targets::Main } else { Targets::Ids(targets) },
                cancel: cancel.clone(),
                ..Default::default()
            };
            let mut stages = Stages::default();
            self.progress(id, STAGES[0].1, 0.0);
            let manifest = kawarimi_prep::prepare(&ffmpeg, &models, &clip.source, &out, &options, |line| {
                stages.read(line);
                self.progress(id, stages.current(), stages.fraction());
            })?;
            self.progress(id, "Mask preview", 0.97);
            ffmpeg.preview(&out.join("mask.mp4"), &out.join("mask-view.mp4"), 960)?;
            Ok(manifest)
        });
        let manifest = match result {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&out);
                return Err(e);
            }
        };
        self.update(id, |clip| {
            clip.prep = Some(next.clone());
            clip.targets = manifest.targets.clone();
            Ok(())
        })?;
        Ok(())
    }

    fn cancel(&self, id: &str) {
        if let Some((_, cancel)) = self.running.lock().unwrap().get(id) {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Copies the image into the clip and cuts it out on the GPU.
    fn set_character(&self, id: &str, person: usize, image: &Path) -> Result<ClipView> {
        self.load(id)?;
        let img = image::open(image).with_context(|| format!("{} is not an image Kawarimi can read", image.display()))?;
        let rgb = img.to_rgb8();
        let dir = self.dir(id).join("characters");
        std::fs::create_dir_all(&dir)?;
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let stem = image.file_stem().unwrap_or_default().to_string_lossy();
        let copy = dir.join(format!("{stem}.{person}-{stamp}.png"));
        rgb.save(&copy)?;
        self.update(id, |clip| {
            clip.characters.insert(person, Character { image: copy.clone(), cutout: None, use_cutout: false, prompt: None });
            Ok(())
        })?;
        let cutout = self.cut(&copy, &rgb)?;
        self.update(id, |clip| {
            if let Some(c) = clip.characters.get_mut(&person).filter(|c| c.image == copy) {
                c.cutout = Some(cutout.clone());
                c.use_cutout = true;
            }
            Ok(())
        })
    }

    fn cut(&self, image: &Path, rgb: &image::RgbImage) -> Result<PathBuf> {
        for asset in [&manifest::CUDA_RUNTIME, &manifest::BIREFNET_LITE] {
            anyhow::ensure!(self.store.status(asset) == Status::Installed, "{} is not installed; add it in Settings", asset.title);
        }
        edgetam_onnx::cuda::use_runtime(&self.store.dir(&manifest::CUDA_RUNTIME))?;
        let _lease = block_on(self.gpu.lease(Work::Prep, "character cutout"))?;
        let mut cutter = kawarimi_prep::cutout::Cutter::load(
            &self.store.dir(&manifest::BIREFNET_LITE).join("model.onnx"),
            edgetam_onnx::Device::Cuda,
        )?;
        let (w, h) = rgb.dimensions();
        let cut = cutter.cut(rgb.as_raw(), w as usize, h as usize)?;
        let out = image.with_extension("cutout.png");
        cut.crop(w, h).save(&out)?;
        Ok(out)
    }

    fn template(&self, id: &str, person: usize) -> Result<String> {
        let view = self.get(id)?;
        let manifest = view.manifest.context("prep the clip first")?;
        let image = view.clip.characters.get(&person).map_or_else(PathBuf::new, |c| c.image.clone());
        // The copy's name is "<original stem>.<person>-<stamp>.png".
        let original = image.file_name().map(|n| n.to_string_lossy().split('.').next().unwrap_or_default().to_owned());
        let named = PathBuf::from(format!("{}.png", original.unwrap_or_default()));
        Ok(kawarimi_comfy::prompt::Scene::read(&manifest, person as u64, &named)?.prompt())
    }

    fn settings(&self, id: &str, person: usize, quality: Quality) -> Result<Settings> {
        let clip = self.load(id)?;
        let prompt = match clip.characters.get(&person).and_then(|c| c.prompt.clone()) {
            Some(p) => p,
            None => self.template(id, person)?,
        };
        let seed = clip.created % 1_000_000;
        Ok(Settings { quality, seed, prompt, model: None, frames: None, steps: None, short_side: None, window: None })
    }

    /// `held`: what the app's ComfyUI holds now; none when renders go to the person's own ComfyUI elsewhere.
    fn estimate(&self, id: &str, quality: Quality, held: Option<u64>) -> Result<RenderEstimate> {
        let view = self.get(id)?;
        let prep = view.prep_dir.context("prep the clip first")?;
        let settings = Settings { quality, seed: 0, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
        let estimate = settings.estimate(&RenderClip::read(&prep)?, vram())?;
        let ram = match (held, ram::Memory::read()) {
            (Some(held), Some(memory)) => Some(ram::fit(ram::need(settings.variant(vram())?), memory, held)),
            _ => None,
        };
        Ok(RenderEstimate { estimate, ram })
    }

    /// A render of `person` as their character: the prep as this render sees it, then a job.
    fn render(&self, id: &str, person: usize, quality: Quality) -> Result<ClipView> {
        let view = self.get(id)?;
        anyhow::ensure!(view.running.is_none(), "wait for the prep to finish");
        let prep = view.prep_dir.clone().context("prep the clip first")?;
        let manifest = view.manifest.as_ref().context("prep the clip first")?;
        let character = view.clip.characters.get(&person).with_context(|| format!("add a character for person {person}"))?;
        let image = match (&character.cutout, character.use_cutout) {
            (Some(cutout), true) => cutout.clone(),
            _ => character.image.clone(),
        };
        let identity = manifest["identities"].as_array().into_iter().flatten().find(|i| i["id"] == person as u64);
        let tracklets: Vec<usize> = identity
            .with_context(|| format!("no person {person} in this prep"))?["tracklets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_u64().map(|t| t as usize))
            .collect();
        let renders = self.dir(id).join("renders");
        std::fs::create_dir_all(&renders)?;
        let folder = (1..)
            .map(|n| renders.join(n.to_string()))
            .find_map(|p| match std::fs::create_dir(&p) {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                made => Some(made.map(|()| p)),
            })
            .expect("a free name")?;
        let made = (|| -> Result<()> {
            link_or_copy(&prep.join("normalized.mp4"), &folder.join("normalized.mp4"))?;
            std::fs::copy(prep.join("manifest.json"), folder.join("manifest.json"))?;
            kawarimi_prep::mask_of(&self.ffmpeg()?, &prep, &tracklets, &folder.join("mask.mp4"))
        })();
        if let Err(e) = made {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(e);
        }
        let spec = Spec { source: Source::Prepared { folder }, character: image, settings: self.settings(id, person, quality)? };
        let job = self.renders.create(spec)?;
        self.update(id, |clip| {
            clip.renders.push(RenderRef { job: job.id.clone(), person });
            Ok(())
        })
    }
}

/// A render's estimate, with this machine's RAM against what the render needs; the job refuses the same way.
#[derive(Debug, Serialize)]
pub struct RenderEstimate {
    #[serde(flatten)]
    estimate: Estimate,
    ram: Option<ram::Fit>,
}

/// The first GPU's memory in bytes; 0 without one.
fn vram() -> u64 {
    static VRAM: OnceLock<u64> = OnceLock::new();
    *VRAM.get_or_init(|| kawarimi_comfy::nvidia::gpus().first().map_or(0, |g| g.memory_mb << 20))
}

fn link_or_copy(from: &Path, to: &Path) -> Result<()> {
    if !from.is_file() {
        return Ok(());
    }
    if std::fs::hard_link(from, to).is_err() {
        std::fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
    }
    Ok(())
}

/// An error as the person reads it: the CLI's hints become where to fix it in the app.
fn human(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    match text.find(" is not installed (kawarimi-fetch") {
        Some(at) => format!("{} is not installed; add it in Settings › Models.", &text[..at]),
        None => text,
    }
}

/// Prep's stages in order, with their share of a typical run's time and the name the UI shows.
const STAGES: [(&str, &str, f32); 8] = [
    ("normalise", "Converting the clip", 0.08),
    ("shots", "Finding the cuts", 0.08),
    ("decode", "Reading the frames", 0.05),
    ("people and tracking models", "Loading the tracker", 0.15),
    ("people and tracking", "Finding and following people", 0.5),
    ("identities", "Telling people apart", 0.03),
    ("mask video", "Writing the mask", 0.04),
    ("people previews and faces", "Previews and faces", 0.04),
];

/// Where a prep stands, from its log lines.
#[derive(Default)]
struct Stages {
    done: usize,
    /// Of the stage running, when it says.
    part: f32,
}

impl Stages {
    fn read(&mut self, line: &str) {
        let Some((name, rest)) = line.split_once(": ") else { return };
        if name == "normalise and shots" && rest == "reused" {
            self.done = self.done.max(2);
        } else if name == "people and tracking" && rest.starts_with("frame ") {
            let numbers: Vec<f32> = rest.split(' ').filter_map(|w| w.parse().ok()).collect();
            if let [f, n] = numbers[..] {
                self.part = f / n.max(1.0);
            }
        } else if let Some(i) = STAGES.iter().position(|s| s.0 == name) {
            // "people and tracking: reused" passes its models' stage too.
            self.done = self.done.max(i + 1);
            self.part = 0.0;
        }
    }

    fn current(&self) -> &'static str {
        STAGES.get(self.done).map_or("Finishing", |s| s.1)
    }

    fn fraction(&self) -> f32 {
        let done: f32 = STAGES[..self.done].iter().map(|s| s.2).sum();
        let part = STAGES.get(self.done).map_or(0.0, |s| s.2 * self.part);
        (done + part) * 0.97
    }
}

/// Render progress by windows, from ComfyUI's log since the job reached ComfyUI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderProgress {
    pub windows: u32,
    pub done: u32,
    pub step: u32,
    pub steps: u32,
}

pub struct Progress {
    log: PathBuf,
    /// Where each job's part of the log starts.
    offsets: Mutex<HashMap<String, u64>>,
}

impl Progress {
    pub fn new(log: PathBuf) -> Self {
        Self { log, offsets: Default::default() }
    }

    /// Called with every job change; marks where a job's sampling starts in the log.
    pub fn observe(&self, job: &Job) {
        use kawarimi_comfy::job::Step;
        let mut offsets = self.offsets.lock().unwrap();
        if job.step >= Step::Queued && job.is_active() && !offsets.contains_key(&job.id) {
            let len = std::fs::metadata(&self.log).map_or(0, |m| m.len());
            offsets.insert(job.id.clone(), len);
        }
    }

    fn of(&self, job: &Job) -> Option<RenderProgress> {
        let offset = *self.offsets.lock().unwrap().get(&job.id)?;
        let bytes = std::fs::read(&self.log).ok()?;
        let from = if (offset as usize) <= bytes.len() { offset as usize } else { 0 };
        let settings = &job.spec.settings;
        let preset = wan::Preset::new(settings.quality, vram());
        let steps = settings.steps.unwrap_or(preset.steps);
        let windows = wan::windows(job.frames?, settings.window.unwrap_or(preset.window)).len() as u32;
        let sampling = kawarimi_comfy::progress::sampling(&String::from_utf8_lossy(&bytes[from..]), steps);
        Some(RenderProgress { windows, done: sampling.done.min(windows), step: sampling.step, steps })
    }
}

type Shared<'a> = State<'a, Arc<Clips>>;

fn text<T>(r: Result<T>) -> Result<T, String> {
    r.map_err(|e| human(&e))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T, String> {
    text(tauri::async_runtime::spawn_blocking(f).await.map_err(|e| anyhow::anyhow!(e)).and_then(|r| r))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub path: PathBuf,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
}

/// Reads a video for import; it becomes playable in the WebView through the asset protocol.
#[tauri::command]
pub async fn clip_probe(app: AppHandle, clips: Shared<'_>, path: PathBuf) -> Result<SourceInfo, String> {
    let ffmpeg = text(clips.ffmpeg())?;
    let probe = blocking({
        let path = path.clone();
        move || ffmpeg.probe(&path).with_context(|| format!("FFmpeg cannot read {}", path.display()))
    })
    .await?;
    if probe.duration < MIN_SECONDS {
        return Err(format!("This video is {:.1} s; a clip needs at least {MIN_SECONDS} s.", probe.duration));
    }
    let _ = app.asset_protocol_scope().allow_file(&path);
    Ok(SourceInfo { path, duration: probe.duration, width: probe.width, height: probe.height, fps: probe.fps, has_audio: probe.has_audio })
}

/// A copy of the source the WebView can play, for codecs it cannot (made once per source).
#[tauri::command]
pub async fn clip_playable(clips: Shared<'_>, path: PathBuf) -> Result<PathBuf, String> {
    let ffmpeg = text(clips.ffmpeg())?;
    let out = clips.root.join("previews").join(format!("{}.mp4", key(&path)));
    blocking(move || {
        if !out.is_file() {
            std::fs::create_dir_all(out.parent().unwrap())?;
            kawarimi_comfy::job::atomically(&out, |part| ffmpeg.preview(&path, part, 960))?;
        }
        Ok(out)
    })
    .await
}

/// Twelve frames of the source side by side, for the trim strip.
#[tauri::command]
pub async fn clip_strip(clips: Shared<'_>, path: PathBuf, duration: f64) -> Result<PathBuf, String> {
    let ffmpeg = text(clips.ffmpeg())?;
    let out = clips.root.join("previews").join(format!("{}.strip.png", key(&path)));
    blocking(move || {
        if !out.is_file() {
            std::fs::create_dir_all(out.parent().unwrap())?;
            kawarimi_comfy::job::atomically(&out, |part| ffmpeg.strip(&path, part, duration, 12, 56))?;
        }
        Ok(out)
    })
    .await
}

/// The source's path, size and time, so a changed file gets new previews.
fn key(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    if let Ok(meta) = std::fs::metadata(path) {
        meta.len().hash(&mut hasher);
        meta.modified().ok().hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

#[tauri::command]
pub fn clips_list(clips: Shared<'_>) -> Vec<ClipView> {
    clips.all().into_iter().map(|c| clips.view(c)).collect()
}

/// A new clip from the trimmed range, prepared at once.
#[tauri::command]
pub fn clip_create(clips: Shared<'_>, source: PathBuf, start: f64, duration: f64) -> Result<ClipView, String> {
    let view = text(clips.create(source, start, duration))?;
    text(clips.prepare(&view.clip.id, None, None))
}

#[tauri::command]
pub fn clip_remove(clips: Shared<'_>, id: String) -> Result<(), String> {
    text(clips.remove(&id))
}

/// Prep again, with other targets when given.
#[tauri::command]
pub fn clip_prepare(clips: Shared<'_>, id: String, targets: Option<Vec<usize>>) -> Result<ClipView, String> {
    text(clips.prepare(&id, targets, None))
}

#[tauri::command]
pub fn clip_cancel(clips: Shared<'_>, id: String) {
    clips.cancel(&id)
}

/// Corrects a person (or adds one prep missed) from points on a frame, then preps again from the tracklets.
#[tauri::command]
pub fn clip_fix(clips: Shared<'_>, id: String, fix: Fix, person: Option<usize>) -> Result<ClipView, String> {
    let view = text(clips.get(&id))?;
    let mut targets = view.clip.targets.clone();
    if let Some(p) = person
        && !targets.contains(&p)
    {
        targets.push(p);
    }
    text(clips.prepare(&id, Some(targets), Some(fix)))
}

#[tauri::command]
pub async fn clip_set_character(clips: Shared<'_>, id: String, person: usize, image: PathBuf) -> Result<ClipView, String> {
    let clips = clips.inner().clone();
    blocking(move || clips.set_character(&id, person, &image)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterChange {
    pub use_cutout: Option<bool>,
    pub prompt: Option<String>,
    /// Back to the template.
    #[serde(default)]
    pub reset_prompt: bool,
    #[serde(default)]
    pub remove: bool,
}

#[tauri::command]
pub fn clip_change_character(clips: Shared<'_>, id: String, person: usize, change: CharacterChange) -> Result<ClipView, String> {
    text(clips.update(&id, |clip| {
        if change.remove {
            clip.characters.remove(&person);
            return Ok(());
        }
        let Some(character) = clip.characters.get_mut(&person) else { bail!("person {person} has no character") };
        if let Some(use_cutout) = change.use_cutout {
            character.use_cutout = use_cutout && character.cutout.is_some();
        }
        if let Some(prompt) = change.prompt {
            character.prompt = Some(prompt).filter(|p| !p.trim().is_empty());
        }
        if change.reset_prompt {
            character.prompt = None;
        }
        Ok(())
    }))
}

#[tauri::command]
pub fn clip_template(clips: Shared<'_>, id: String, person: usize) -> Result<String, String> {
    text(clips.template(&id, person))
}

#[tauri::command]
pub fn clip_estimate(clips: Shared<'_>, state: State<'_, crate::AppState>, id: String, quality: Quality) -> Result<RenderEstimate, String> {
    let held = state.own_comfy.server().is_none().then(|| state.comfy.guard().map_or(0, |g| g.held()));
    text(clips.estimate(&id, quality, held))
}

#[tauri::command]
pub async fn clip_render(clips: Shared<'_>, id: String, person: usize, quality: Quality) -> Result<ClipView, String> {
    let clips = clips.inner().clone();
    blocking(move || clips.render(&id, person, quality)).await
}

#[tauri::command]
pub fn render_progress(progress: State<'_, Arc<Progress>>, clips: Shared<'_>, job: String) -> Option<RenderProgress> {
    let job = clips.renders.list().into_iter().find(|j| j.id == job)?;
    progress.of(&job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_follow_the_log() {
        let mut s = Stages::default();
        assert_eq!((s.current(), s.fraction()), ("Converting the clip", 0.0));
        s.read("normalise: 1.0 s");
        s.read("shots: 1.2 s");
        assert_eq!(s.current(), "Reading the frames");
        s.read("decode: 0.8 s");
        s.read("people and tracking models: 4.0 s");
        s.read("people and tracking: frame 64 of 128");
        assert_eq!(s.current(), "Finding and following people");
        assert!((s.fraction() - (0.36 + 0.25) * 0.97).abs() < 1e-4, "{}", s.fraction());
        s.read("people and tracking: 10.0 s");
        assert_eq!(s.current(), "Telling people apart");

        let mut again = Stages::default();
        again.read("normalise and shots: reused");
        again.read("decode: 0.8 s");
        again.read("people and tracking: reused");
        assert_eq!(again.current(), "Telling people apart");
    }

    #[test]
    fn errors_point_to_settings() {
        let e = anyhow::anyhow!("EdgeTAM video is not installed (kawarimi-fetch edgetam-video)");
        assert_eq!(human(&e), "EdgeTAM video is not installed; add it in Settings › Models.");
    }
}
