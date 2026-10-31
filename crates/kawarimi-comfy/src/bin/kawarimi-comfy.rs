//! ComfyUI from the command line, for development and checks.
//!
//!     kawarimi-comfy install [--root .cache/comfy] [--models .models]
//!     kawarimi-comfy check   [--root …]     start, print /system_stats, stop
//!     kawarimi-comfy render  --prep <folder> --character <png> --out <mp4> [--jobs .cache/jobs]
//!                            [--preset draft|final] [--frames N] [--steps N] [--short-side N] [--window N]
//!                            [--seed 0] [--prompt <text>] [--model wan-animate-q3km|q4km|fp8, by default from the VRAM] [--comfy-args "<ComfyUI flags>"]
//!     kawarimi-comfy resume  [--jobs .cache/jobs]     runs every unfinished job, oldest first
//!     kawarimi-comfy finish  --prep <folder> --video <ComfyUI's mp4> --out <mp4> [--frames N]
//!
//! `render` is a job (`kawarimi_comfy::job`) in `--jobs`, as in the app: killed, it carries on with `resume`. The
//! preset (final by default) picks the size, steps, window and frame limit for the GPU ComfyUI reports; the flags
//! override it. It copies ComfyUI's video to `<out>.comfy.mp4` and the finished one to `<out>`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use kawarimi_assets::{Status, Store, manifest};
use kawarimi_comfy::job::{self, Api, End, Job, Jobs, Runner, Source, Spec};
use kawarimi_comfy::render::{Clip, Settings};
use kawarimi_comfy::{Client, Extras, Install, Server, VARIANTS, Variant, wan};
use kawarimi_media::Ffmpeg;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let number = |name: &str| -> Result<Option<u32>> {
        flag(name).map(|v| v.parse()).transpose().with_context(|| format!("{name} is not a number"))
    };
    let required = |name: &str, what: &str| flag(name).map(PathBuf::from).with_context(|| format!("{name} <{what}> is required"));
    let store = Store::new(flag("--models").unwrap_or_else(|| ".models".into()));
    let install = Install::from_store(Path::new(&flag("--root").unwrap_or_else(|| ".cache/comfy".into())), &store)?;
    let missing = Extras::missing(&store);
    let mut extras = if missing.is_empty() { Extras::render(&store) } else { Extras::default() };
    if let Some(more) = flag("--comfy-args") {
        extras.args.extend(more.split_whitespace().map(String::from));
    }
    let ffmpeg = || -> Result<Ffmpeg> {
        anyhow::ensure!(store.status(&manifest::FFMPEG) == Status::Installed, "FFmpeg is not installed (kawarimi-fetch ffmpeg)");
        Ffmpeg::locate(Some(&store.dir(&manifest::FFMPEG))).context("no FFmpeg")
    };
    let jobs = Jobs::new(flag("--jobs").unwrap_or_else(|| ".cache/jobs".into()));
    let renders = || -> Result<Renders<'_>> {
        let ids: Vec<&str> = missing.iter().map(|a| a.id).collect();
        anyhow::ensure!(ids.is_empty(), "render assets missing (kawarimi-fetch {})", ids.join(" "));
        Ok(Renders { install: &install, extras: &extras, ffmpeg: ffmpeg()?, server: RefCell::new(None), start: Instant::now() })
    };
    let start = Instant::now();
    match args.first().map(String::as_str) {
        Some("install") => install.install(|line| eprintln!("install: {line} ({:.0} s)", start.elapsed().as_secs_f64()))?,
        Some("check") => {
            let server = Server::start(&install, &extras, Duration::from_secs(600))?;
            eprintln!("ready on {} in {:.1} s", server.url(), start.elapsed().as_secs_f64());
            let stats = server.system_stats()?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
        Some("render") => {
            let model = flag("--model");
            if let Some(id) = &model {
                let variant = Variant::find(id).with_context(|| {
                    format!("unknown --model; one of {}", VARIANTS.iter().map(|v| v.asset.id).collect::<Vec<_>>().join(", "))
                })?;
                let asset = variant.asset;
                anyhow::ensure!(store.status(asset) == Status::Installed, "{} is not installed (kawarimi-fetch {})", asset.title, asset.id);
            }
            let quality = match flag("--preset").as_deref() {
                Some("draft") => wan::Quality::Draft,
                None | Some("final") => wan::Quality::Final,
                Some(other) => anyhow::bail!("--preset is draft or final, not {other}"),
            };
            let settings = Settings {
                quality,
                seed: flag("--seed").map(|v| v.parse()).transpose().context("--seed is not a number")?.unwrap_or(0),
                prompt: flag("--prompt").unwrap_or_default(),
                model,
                frames: number("--frames")?,
                steps: number("--steps")?,
                short_side: number("--short-side")?,
                window: number("--window")?,
            };
            let spec = Spec {
                source: Source::Prepared { folder: std::path::absolute(required("--prep", "folder")?)? },
                character: required("--character", "png")?,
                settings,
            };
            let out = required("--out", "mp4")?;
            let mut job = jobs.create(spec)?;
            eprintln!("job {} in {}", job.id, jobs.dir(&job.id).display());
            renders()?.run(&jobs, &mut job)?;
            report(&job);
            let result = done(&job)?;
            std::fs::copy(jobs.dir(&job.id).join("comfy.mp4"), out.with_extension("comfy.mp4"))?;
            std::fs::copy(result, &out)?;
            eprintln!("saved {}", out.display());
        }
        Some("resume") => {
            let active: Vec<Job> = jobs.all().into_iter().filter(Job::is_active).collect();
            eprintln!("{} unfinished jobs", active.len());
            if !active.is_empty() {
                let renders = renders()?;
                for mut job in active {
                    eprintln!("job {}: from {:?}", job.id, job.step);
                    renders.run(&jobs, &mut job)?;
                    match done(&job) {
                        Ok(result) => eprintln!("job {}: {}", job.id, result.display()),
                        Err(e) => eprintln!("job {}: {e:#}", job.id),
                    }
                }
            }
        }
        Some("finish") => {
            let prep = required("--prep", "folder")?;
            let frames = number("--frames")?.unwrap_or(Clip::read(&prep)?.frames);
            finish(&ffmpeg()?, &prep, &required("--video", "mp4")?, &required("--out", "mp4")?, frames)?;
        }
        _ => anyhow::bail!("usage: kawarimi-comfy install|check|render|resume|finish [--root <folder>] [--models <folder>]"),
    }
    Ok(())
}

/// The job's node times, longest first, and its RAM.
fn report(job: &Job) {
    if !job.nodes.is_empty() {
        eprintln!("{:>8}  {:>5}  class", "seconds", "node");
        for n in &job.nodes {
            eprintln!("{:>8.1}  {:>5}  {}", n.seconds, n.node, n.class_type);
        }
        eprintln!("{:>8.1}  total", job.nodes.iter().map(|n| n.seconds).sum::<f64>());
    }
    if let Some(ram) = job.ram {
        let gb = |b: u64| b as f64 / kawarimi_comfy::ram::GB as f64;
        eprintln!(
            "RAM: ComfyUI peak {:.1} GB, free RAM min {:.2} GB, commit headroom min {:.1} GB",
            gb(ram.peak_rss),
            gb(ram.min_free),
            gb(ram.min_headroom)
        );
    }
}

fn done(job: &Job) -> Result<&Path> {
    match &job.end {
        Some(End::Done { result }) => Ok(result),
        Some(End::Failed { reason }) => anyhow::bail!("the render failed: {reason}"),
        other => anyhow::bail!("the render did not finish ({other:?})"),
    }
}

/// Jobs on a ComfyUI this process starts when the first job needs it, and again if it dies.
struct Renders<'a> {
    install: &'a Install,
    extras: &'a Extras,
    ffmpeg: Ffmpeg,
    server: RefCell<Option<Server>>,
    start: Instant,
}

impl Renders<'_> {
    fn run(&self, jobs: &Jobs, job: &mut Job) -> Result<()> {
        let last = RefCell::new(String::new());
        let changed = |job: &Job| {
            let line = format!("{:?}: {}", job.step, job.detail);
            let mut last = last.borrow_mut();
            if *last != line && (!job.detail.starts_with("rendering") || job.detail.ends_with("0 s")) {
                eprintln!("{line} ({:.0} s)", self.start.elapsed().as_secs_f64());
                *last = line;
            }
        };
        let cancel = AtomicBool::new(false);
        Runner { jobs, backend: self, cancel: &cancel, changed: &changed, poll: Duration::from_secs(1) }.run(job)
    }
}

impl job::Backend for Renders<'_> {
    type Comfy = Api;
    type Lease = ();

    fn prepare(&self, _: &Source, _: &Path, _: &mut dyn FnMut(&str)) -> Result<()> {
        anyhow::bail!("the CLI renders a prep folder; prepare the clip with kawarimi-prep")
    }

    fn lease(&self, _: &str) -> Result<()> {
        Ok(())
    }

    fn comfy(&self) -> Result<Api> {
        let mut server = self.server.borrow_mut();
        if !server.as_mut().is_some_and(Server::is_running) {
            let started = Server::start(self.install, self.extras, Duration::from_secs(600))?;
            eprintln!("ComfyUI ready on {} ({:.0} s)", started.url(), self.start.elapsed().as_secs_f64());
            *server = Some(started);
        }
        let url = server.as_ref().map(Server::url).unwrap_or_default();
        let guard = server.as_ref().map(Server::guard);
        Ok(Api { client: Client::new(url)?, output: Some(self.install.data().join("output")), guard })
    }

    fn finish(&self, video: &Path, prep: &Path, out: &Path, frames: u32) -> Result<()> {
        finish(&self.ffmpeg, prep, video, out, frames)
    }

    fn frames(&self, video: &Path) -> Result<u32> {
        let probe = self.ffmpeg.probe(video)?;
        Ok((probe.duration * probe.fps).round() as u32)
    }
}

fn finish(ffmpeg: &Ffmpeg, prep: &Path, video: &Path, out: &Path, frames: u32) -> Result<()> {
    let probe = ffmpeg.finish(video, &prep.join("normalized.mp4"), out, frames, wan::FPS as f64)?;
    let sound = if probe.has_audio { "with the clip's sound" } else { "the clip has no sound" };
    eprintln!("finished {}: {frames} frames, {:.3} s, {sound}", out.display(), probe.duration);
    Ok(())
}
