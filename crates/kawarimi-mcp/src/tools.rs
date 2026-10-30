//! The tools agents call. Each returns text (JSON) and, where seeing helps, images; a failure is a tool error whose
//! text says what to do.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use base64::Engine;
use kawarimi_assets::{Asset, Status, Store, manifest};
use kawarimi_comfy::job::{Source, Spec};
use kawarimi_comfy::locks::GpuLock;
use kawarimi_comfy::render::{self, Settings};
use kawarimi_comfy::wan::Quality;
use kawarimi_comfy::{Extras, Install, Variant, nvidia};
use kawarimi_prep::{Models, Options, Targets};
use serde_json::{Value, json};

use crate::dirs::Dirs;
use crate::ledger::{Kind, Ledger, now};
use crate::worker::{Worker, ffmpeg};

/// What a tool hands back.
pub struct Reply {
    pub text: String,
    pub images: Vec<Vec<u8>>,
    pub data: Option<Value>,
}

impl Reply {
    fn json(value: Value) -> Self {
        Self { text: serde_json::to_string_pretty(&value).unwrap_or_default(), images: Vec::new(), data: Some(value) }
    }

    fn with_images(mut self, images: Vec<Vec<u8>>) -> Self {
        self.images = images;
        self
    }
}

pub struct Kawarimi {
    pub dirs: Dirs,
    pub store: Store,
    pub ledger: Arc<Ledger>,
    pub worker: Arc<Worker>,
    /// Asset or ComfyUI installs in the background: what is being installed and how far it got.
    pub installs: Arc<Mutex<BTreeMap<String, String>>>,
}

pub fn list() -> Value {
    let prep = json!({ "type": "string", "description": "A prep id from prepare_clip." });
    let person = json!({ "type": "integer", "description": "A person's id from prepare_clip." });
    let quality = json!({ "type": "string", "enum": ["draft", "final"], "description": "draft: the first window only, to check the motion (default). final: the whole clip, same seed." });
    let tools = [
        tool("status", "Kawarimi status", "Start here. The GPU and whether it can run prep and renders, which models are installed or missing, whether ComfyUI is installed, installs in progress, who holds the GPU now, and this agent's usage against its limits.", json!({}), &[], true),
        tool("install", "Install models", "Downloads what is missing, in the background (gigabytes; check progress with status). what: \"prep\" (FFmpeg, CUDA runtime, prep and cutout models, ~2 GB), \"render\" (ComfyUI, its nodes and the Wan weights for this GPU, ~15 GB), \"comfyui\" (ComfyUI's Python packages, ~5 GB, after \"render\"), or a list of asset ids from status.", json!({ "what": { "description": "\"prep\", \"render\", \"comfyui\", or an array of asset ids." } }), &["what"], false),
        tool("probe_clip", "Probe a clip", "Duration, size, frame rate and sound of a video file.", json!({ "path": { "type": "string", "description": "A video file on this computer." } }), &["path"], true),
        tool("prepare_clip", "Prepare a clip", "Finds the shots and the people in 4–30 s of a video and tracks each of them on the GPU (about 15–40 s). Returns a prep id and the people with their preview images; look at them with view_person or view_frame to pick whom to replace. Counts against the prep and GPU limits.", json!({
            "path": { "type": "string", "description": "A video file on this computer." },
            "start": { "type": "number", "description": "Seconds into the video; default 0." },
            "duration": { "type": "number", "description": "Seconds, 4 to 30; default as much as fits." }
        }), &["path"], false),
        tool("view_person", "View a person", "The preview and face images of one person found by prepare_clip.", json!({ "prep": prep, "person": person }), &["prep", "person"], true),
        tool("view_frame", "View a frame", "A frame of the prepared clip, with a person's mask tinted red when person is given, to check what would be replaced.", json!({
            "prep": prep, "person": person, "time": { "type": "number", "description": "Seconds into the prepared clip; default the middle." }
        }), &["prep"], true),
        tool("cut_character", "Cut out a character", "Lifts a character image off its background (BiRefNet on the GPU) and crops it to the character, as the render wants it. Returns the cutout's path for start_render.", json!({ "image": { "type": "string", "description": "A PNG or JPEG of the character, ideally 800 px or more and full body." } }), &["image"], false),
        tool("prompt_template", "Prompt template", "The prompt Kawarimi writes for replacing a person with a character, from the clip and the image's file name. Edit it and pass it to start_render if you like.", json!({ "prep": prep, "person": person, "character": { "type": "string", "description": "The character image's path." } }), &["prep", "person", "character"], true),
        tool("estimate_render", "Estimate a render", "Size, frames, windows, steps, model, seconds and video memory a render of the prepared clip would take on this GPU.", json!({ "prep": prep, "quality": quality }), &["prep"], true),
        tool("start_render", "Start a render", "Queues replacing a person with a character (Wan 2.2 Animate in ComfyUI). Returns a job id at once; the render takes minutes (about 7 for a draft on an 8 GB card). Follow it with job_status. Counts against the render and GPU limits.", json!({
            "prep": prep, "person": person,
            "character": { "type": "string", "description": "The character image, best the cutout from cut_character." },
            "quality": quality,
            "prompt": { "type": "string", "description": "Default: prompt_template's." },
            "seed": { "type": "integer", "description": "Default 0; keep it to get the draft's look in the final." }
        }), &["prep", "person", "character"], false),
        tool("job_status", "Job status", "A render job's step, progress line and end; when done, the finished video's path.", json!({ "id": { "type": "string" } }), &["id"], true),
        tool("list_jobs", "List jobs", "This agent's render jobs, newest first; all: true lists every job, the app's too.", json!({ "all": { "type": "boolean" } }), &[], true),
        tool("cancel_job", "Cancel a job", "Cancels one of this agent's render jobs.", json!({ "id": { "type": "string" } }), &["id"], false),
    ];
    json!({ "tools": tools })
}

fn tool(name: &str, title: &str, description: &str, properties: Value, required: &[&str], read_only: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required },
        "annotations": { "readOnlyHint": read_only, "openWorldHint": false },
    })
}

impl Kawarimi {
    pub fn call(&self, name: &str, args: &Value) -> Result<Reply> {
        match name {
            "status" => self.status(),
            "install" => self.install(&args["what"]),
            "probe_clip" => self.probe(&path_arg(args, "path")?),
            "prepare_clip" => self.prepare(args),
            "view_person" => self.view_person(&str_arg(args, "prep")?, int_arg(args, "person")?),
            "view_frame" => self.view_frame(&str_arg(args, "prep")?, args["person"].as_u64(), args["time"].as_f64()),
            "cut_character" => self.cut(&path_arg(args, "image")?),
            "prompt_template" => {
                let prep = self.prep_dir(&str_arg(args, "prep")?)?;
                Ok(Reply::json(json!({ "prompt": template(&prep, int_arg(args, "person")?, &path_arg(args, "character")?)? })))
            }
            "estimate_render" => {
                let prep = self.prep_dir(&str_arg(args, "prep")?)?;
                Ok(Reply::json(serde_json::to_value(settings(args)?.estimate(&render::Clip::read(&prep)?, vram())?)?))
            }
            "start_render" => self.start_render(args),
            "job_status" => self.job_status(&str_arg(args, "id")?),
            "list_jobs" => self.list_jobs(args["all"].as_bool().unwrap_or(false)),
            "cancel_job" => {
                let id = str_arg(args, "id")?;
                anyhow::ensure!(self.ledger.renders().contains(&id), "job {id} is not one of this agent's renders");
                self.worker.cancel(&id)?;
                self.job_status(&id)
            }
            other => bail!("no tool {other}"),
        }
    }

    fn status(&self) -> Result<Reply> {
        let gpus = nvidia::gpus();
        let prep_missing = missing(&self.store, PREP);
        let mut render_missing: Vec<&Asset> = Extras::missing(&self.store);
        render_missing.extend(missing(&self.store, &[&manifest::UV, &manifest::COMFYUI]));
        let variant = Variant::for_vram(gpus.first().map(|g| g.memory_mb));
        if self.store.status(variant.asset) != Status::Installed {
            render_missing.push(variant.asset);
        }
        let comfy = Install::from_store(&self.dirs.comfy, &self.store).map(|i| i.is_installed()).unwrap_or(false);
        let ids = |assets: &[&Asset]| assets.iter().map(|a| json!({ "id": a.id, "title": a.title, "bytes": a.download_size() })).collect::<Vec<_>>();
        Ok(Reply::json(json!({
            "gpus": gpus,
            "gpuProblem": nvidia::Problem::of(&gpus),
            "renderModel": variant.asset.title,
            "prep": { "ready": prep_missing.is_empty(), "missing": ids(&prep_missing) },
            "render": { "ready": render_missing.is_empty() && comfy, "comfyuiInstalled": comfy, "missing": ids(&render_missing) },
            "installs": *self.installs.lock().unwrap(),
            "gpuHeldBy": GpuLock::holder(&GpuLock::path(&self.dirs.data)),
            "usage": self.ledger.usage(),
            "folders": { "assets": self.dirs.assets, "jobs": self.dirs.jobs, "limits": self.dirs.mcp.join("limits.json") },
        })))
    }

    fn install(&self, what: &Value) -> Result<Reply> {
        let assets: Vec<&'static Asset> = match what.as_str() {
            Some("prep") => PREP.to_vec(),
            Some("render") => {
                let mut assets = vec![&manifest::UV, &manifest::COMFYUI];
                assets.extend(Extras::assets().iter().copied());
                assets.push(Variant::for_vram(nvidia::gpus().first().map(|g| g.memory_mb)).asset);
                assets
            }
            Some("comfyui") => return self.install_comfy(),
            Some(other) => bail!("what is \"prep\", \"render\", \"comfyui\" or a list of asset ids, not {other}"),
            None => what
                .as_array()
                .context("what is \"prep\", \"render\", \"comfyui\" or a list of asset ids")?
                .iter()
                .map(|id| id.as_str().and_then(kawarimi_assets::find).with_context(|| format!("no asset {id}")))
                .collect::<Result<_>>()?,
        };
        let todo: Vec<&'static Asset> = assets.into_iter().filter(|a| self.store.status(a) != Status::Installed).collect();
        let started: Vec<&str> = todo.iter().map(|a| a.id).collect();
        let (store, installs) = (Store::new(&self.dirs.assets), self.installs.clone());
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
            for asset in todo {
                let progress = |done: u64, total: u64| {
                    installs.lock().unwrap().insert(asset.id.into(), format!("{}%", done * 100 / total.max(1)));
                };
                let result = runtime.block_on(store.install(asset, progress));
                let state = result.map_or_else(|e| format!("failed: {e:#}"), |()| "installed".into());
                installs.lock().unwrap().insert(asset.id.into(), state);
            }
        });
        Ok(Reply::json(json!({ "installing": started, "check": "status shows each one's progress" })))
    }

    fn install_comfy(&self) -> Result<Reply> {
        let install = Install::from_store(&self.dirs.comfy, &self.store)?;
        let installs = self.installs.clone();
        installs.lock().unwrap().insert("comfyui".into(), "starting".into());
        std::thread::spawn(move || {
            let result = install.install(|line| {
                installs.lock().unwrap().insert("comfyui".into(), line.into());
            });
            if let Err(e) = result {
                installs.lock().unwrap().insert("comfyui".into(), format!("failed: {e:#}"));
            }
        });
        Ok(Reply::json(json!({ "installing": ["comfyui"], "check": "status shows its progress (several minutes)" })))
    }

    fn probe(&self, path: &Path) -> Result<Reply> {
        anyhow::ensure!(path.is_file(), "no file at {}", path.display());
        Ok(Reply::json(serde_json::to_value(ffmpeg(&self.store)?.probe(path)?)?))
    }

    fn prepare(&self, args: &Value) -> Result<Reply> {
        let path = path_arg(args, "path")?;
        anyhow::ensure!(path.is_file(), "no file at {}", path.display());
        self.ledger.check_prep()?;
        let ffmpeg = ffmpeg(&self.store)?;
        let probe = ffmpeg.probe(&path)?;
        let start = args["start"].as_f64().unwrap_or(0.0).max(0.0);
        let duration = args["duration"].as_f64().unwrap_or((probe.duration - start).min(30.0));
        anyhow::ensure!((4.0..=30.0).contains(&duration), "a clip is 4 to 30 seconds; this is {duration:.1}");
        anyhow::ensure!(start + duration <= probe.duration + 0.05, "the video is {:.1} s long", probe.duration);
        let models = Models::installed(&self.store).map_err(|e| anyhow::anyhow!("{e:#}; install \"prep\" first"))?;
        let id = now().to_string();
        let out = self.dirs.mcp.join("preps").join(&id);
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let _gpu = self.gpu(&format!("agent prep {name}"))?;
        let started = Instant::now();
        let options = Options {
            normalize: kawarimi_media::Normalize { start, duration: Some(duration), ..Default::default() },
            targets: Targets::Everyone,
            ..Default::default()
        };
        let prepared = kawarimi_prep::prepare(&ffmpeg, &models, &path, &out, &options, |line| eprintln!("prep: {line}"));
        self.ledger.record(Kind::Prep, &id, started.elapsed().as_secs_f64())?;
        prepared?;
        let summary = summary(&id, &out)?;
        let previews = summary["people"].as_array().into_iter().flatten().filter_map(|p| p["preview"].as_str()).map(PathBuf::from);
        let images = previews.filter_map(|p| png(&p, 384).ok()).collect();
        Ok(Reply::json(summary).with_images(images))
    }

    /// The GPU for a call that runs now: a tool call that waited for a render in the app could outlast the client's
    /// timeout, so a busy GPU is an answer to try again later.
    fn gpu(&self, holder: &str) -> Result<GpuLock> {
        let path = GpuLock::path(&self.dirs.data);
        GpuLock::try_acquire(&path, holder)?.with_context(|| {
            let by = GpuLock::holder(&path).unwrap_or_else(|| "another process".into());
            format!("the GPU is busy ({by}); try again when status shows it free")
        })
    }

    fn prep_dir(&self, id: &str) -> Result<PathBuf> {
        anyhow::ensure!(id.chars().all(|c| c.is_ascii_digit()), "a prep id is digits, from prepare_clip");
        let dir = self.dirs.mcp.join("preps").join(id);
        anyhow::ensure!(dir.join("manifest.json").is_file(), "no prep {id}; prepare_clip first");
        Ok(dir)
    }

    fn view_person(&self, prep: &str, person: u64) -> Result<Reply> {
        let dir = self.prep_dir(prep)?;
        let people = dir.join("people");
        let preview = people.join(format!("{person}.png"));
        anyhow::ensure!(preview.is_file(), "no person {person} in prep {prep}");
        let mut images = vec![png(&preview, 512)?];
        let face = people.join(format!("{person}-face.png"));
        if face.is_file() {
            images.push(png(&face, 256)?);
        }
        Ok(Reply::json(json!({ "prep": prep, "person": person, "preview": preview, "face": face.is_file().then_some(face) })).with_images(images))
    }

    fn view_frame(&self, prep: &str, person: Option<u64>, time: Option<f64>) -> Result<Reply> {
        let dir = self.prep_dir(prep)?;
        let manifest = read_manifest(&dir)?;
        let fps = manifest["video"]["fps"].as_f64().unwrap_or(16.0);
        let frames = manifest["frames"].as_u64().unwrap_or(1);
        let frame = time.map_or(frames / 2, |t| ((t * fps) as u64).min(frames - 1));
        let ffmpeg = ffmpeg(&self.store)?;
        let (w, h) = (manifest["video"]["width"].as_u64().unwrap_or(0) as u32, manifest["video"]["height"].as_u64().unwrap_or(0) as u32);
        let grab = |video: &Path| -> Result<Vec<u8>> {
            ffmpeg.frames(video, w, h, None)?.nth(frame as usize).with_context(|| format!("no frame {frame}"))?
        };
        let mut img = image::RgbImage::from_raw(w, h, grab(&dir.join("normalized.mp4"))?).context("frame size")?;
        if let Some(person) = person {
            let scratch = self.dirs.mcp.join("scratch");
            std::fs::create_dir_all(&scratch)?;
            let mask_video = scratch.join(format!("{prep}-{person}-mask.mp4"));
            kawarimi_prep::mask_of(&ffmpeg, &dir, &tracklets(&manifest, person)?, &mask_video)?;
            let mask = grab(&mask_video)?;
            for (px, m) in img.pixels_mut().zip(mask.chunks_exact(3)) {
                if m[0] > 127 {
                    px.0 = [((px.0[0] as u16 + 255) / 2) as u8, px.0[1] / 2, px.0[2] / 2];
                }
            }
        }
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(fit(img, 768)).write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)?;
        Ok(Reply::json(json!({ "prep": prep, "frame": frame, "of": frames, "person": person })).with_images(vec![bytes]))
    }

    fn cut(&self, image_path: &Path) -> Result<Reply> {
        anyhow::ensure!(image_path.is_file(), "no file at {}", image_path.display());
        self.ledger.check_prep()?;
        for asset in [&manifest::CUDA_RUNTIME, &manifest::BIREFNET_LITE] {
            anyhow::ensure!(self.store.status(asset) == Status::Installed, "{} is not installed; install \"prep\" first", asset.title);
        }
        edgetam_onnx::cuda::use_runtime(&self.store.dir(&manifest::CUDA_RUNTIME))?;
        let rgb = image::open(image_path).with_context(|| format!("reading {}", image_path.display()))?.to_rgb8();
        let id = now().to_string();
        let _gpu = self.gpu("agent character cutout")?;
        let started = Instant::now();
        let cut = (|| {
            let mut cutter = kawarimi_prep::cutout::Cutter::load(&self.store.dir(&manifest::BIREFNET_LITE).join("model.onnx"), edgetam_onnx::Device::Cuda)?;
            let (w, h) = rgb.dimensions();
            Ok::<_, anyhow::Error>(cutter.cut(rgb.as_raw(), w as usize, h as usize)?.crop(w, h))
        })();
        self.ledger.record(Kind::Cutout, &id, started.elapsed().as_secs_f64())?;
        let cropped = cut?;
        let stem = image_path.file_stem().unwrap_or_default().to_string_lossy();
        // The file keeps the image's name: the prompt template reads the character's name from it.
        let dir = self.dirs.mcp.join("characters").join(&id);
        std::fs::create_dir_all(&dir)?;
        let out = dir.join(format!("{stem}.png"));
        cropped.save(&out)?;
        Ok(Reply::json(json!({ "cutout": out, "width": cropped.width(), "height": cropped.height() })).with_images(vec![png(&out, 512)?]))
    }

    fn start_render(&self, args: &Value) -> Result<Reply> {
        let prep_id = str_arg(args, "prep")?;
        let prep = self.prep_dir(&prep_id)?;
        let person = int_arg(args, "person")?;
        let character = path_arg(args, "character")?;
        anyhow::ensure!(character.is_file(), "no character image at {}", character.display());
        let manifest = read_manifest(&prep)?;
        let tracks = tracklets(&manifest, person)?;
        let mut settings = settings(args)?;
        let estimate = settings.estimate(&render::Clip::read(&prep)?, vram())?;
        let mine = self.ledger.renders();
        let queued = self.worker.jobs.all().iter().filter(|j| j.is_active() && mine.contains(&j.id)).count();
        self.ledger.check_render(estimate.seconds as f64, queued)?;
        if settings.prompt.trim().is_empty() {
            settings.prompt = template(&prep, person, &character)?;
        }
        let renders = prep.join("renders");
        std::fs::create_dir_all(&renders)?;
        let folder = (1..)
            .map(|n| renders.join(n.to_string()))
            .find_map(|p| match std::fs::create_dir(&p) {
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                made => Some(made.map(|()| p)),
            })
            .expect("a free name")?;
        let made = (|| -> Result<()> {
            if std::fs::hard_link(prep.join("normalized.mp4"), folder.join("normalized.mp4")).is_err() {
                std::fs::copy(prep.join("normalized.mp4"), folder.join("normalized.mp4"))?;
            }
            std::fs::copy(prep.join("manifest.json"), folder.join("manifest.json"))?;
            kawarimi_prep::mask_of(&ffmpeg(&self.store)?, &prep, &tracks, &folder.join("mask.mp4"))
        })();
        if let Err(e) = made {
            let _ = std::fs::remove_dir_all(&folder);
            return Err(e);
        }
        let job = self.worker.jobs.create(Spec { source: Source::Prepared { folder }, character, settings })?;
        self.ledger.record(Kind::Render, &job.id, 0.0)?;
        self.worker.submit(&job.id);
        Ok(Reply::json(json!({ "job": job.id, "estimate": estimate, "follow": "job_status" })))
    }

    fn job_status(&self, id: &str) -> Result<Reply> {
        let job = self.worker.jobs.load(id)?;
        let detail = self.worker.live(id).unwrap_or(job.detail.clone());
        Ok(Reply::json(json!({
            "id": job.id,
            "name": job.name(),
            "step": job.step,
            "detail": detail,
            "end": job.end,
            "submissions": job.submissions,
            "agent": self.ledger.renders().contains(&job.id),
        })))
    }

    fn list_jobs(&self, all: bool) -> Result<Reply> {
        let mine = self.ledger.renders();
        let mut jobs: Vec<_> = self.worker.jobs.all().into_iter().filter(|j| all || mine.contains(&j.id)).collect();
        jobs.reverse();
        let rows: Vec<Value> = jobs
            .iter()
            .map(|j| json!({ "id": j.id, "name": j.name(), "step": j.step, "end": j.end, "agent": mine.contains(&j.id) }))
            .collect();
        Ok(Reply::json(json!({ "jobs": rows })))
    }
}

/// What prep and character cutouts need.
const PREP: &[&Asset] = &[
    &manifest::FFMPEG,
    &manifest::CUDA_RUNTIME,
    &manifest::TRANSNETV2,
    &manifest::RTDETR,
    &manifest::EDGETAM,
    &manifest::YUNET,
    &manifest::BIREFNET_LITE,
];

fn missing(store: &Store, assets: &[&'static Asset]) -> Vec<&'static Asset> {
    assets.iter().copied().filter(|a| store.status(a) != Status::Installed).collect()
}

fn summary(id: &str, dir: &Path) -> Result<Value> {
    let m = read_manifest(dir)?;
    let people: Vec<Value> = m["identities"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            let person = i["id"].as_u64().unwrap_or(0);
            json!({
                "id": person,
                "onScreen": i["onScreen"],
                "preview": dir.join("people").join(format!("{person}.png")),
                "face": i["face"].is_object(),
            })
        })
        .collect();
    Ok(json!({
        "prep": id,
        "video": m["video"],
        "frames": m["frames"],
        "shots": m["shots"].as_array().map_or(0, Vec::len),
        "people": people,
        "closeCalls": m["closeCalls"],
        "next": "view_person or view_frame to choose, cut_character for the image, then start_render",
    }))
}

fn read_manifest(dir: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(dir.join("manifest.json"))?)?)
}

fn tracklets(manifest: &Value, person: u64) -> Result<Vec<usize>> {
    let identity = manifest["identities"].as_array().into_iter().flatten().find(|i| i["id"] == person);
    Ok(identity
        .with_context(|| format!("no person {person} in this prep"))?["tracklets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_u64().map(|t| t as usize))
        .collect())
}

fn template(prep: &Path, person: u64, character: &Path) -> Result<String> {
    Ok(kawarimi_comfy::prompt::Scene::read(&read_manifest(prep)?, person, character)?.prompt())
}

fn settings(args: &Value) -> Result<Settings> {
    let quality: Quality = serde_json::from_value(args.get("quality").cloned().unwrap_or(json!("draft")))
        .context("quality is \"draft\" or \"final\"")?;
    Ok(Settings {
        quality,
        seed: args["seed"].as_u64().unwrap_or(0),
        prompt: args["prompt"].as_str().unwrap_or_default().into(),
        model: None,
        frames: None,
        steps: None,
        short_side: None,
        window: None,
    })
}

/// The first GPU's memory in bytes; 0 without one.
fn vram() -> u64 {
    nvidia::gpus().first().map_or(0, |g| g.memory_mb << 20)
}

fn fit(img: image::RgbImage, side: u32) -> image::RgbImage {
    if img.width().max(img.height()) <= side {
        return img;
    }
    image::DynamicImage::ImageRgb8(img).resize(side, side, image::imageops::FilterType::Triangle).to_rgb8()
}

fn png(path: &Path, side: u32) -> Result<Vec<u8>> {
    let img = fit(image::open(path)?.to_rgb8(), side);
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)?;
    Ok(bytes)
}

pub fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn str_arg(args: &Value, name: &str) -> Result<String> {
    match &args[name] {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => bail!("{name} is required"),
    }
}

fn int_arg(args: &Value, name: &str) -> Result<u64> {
    args[name].as_u64().with_context(|| format!("{name} is required, a whole number"))
}

fn path_arg(args: &Value, name: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(str_arg(args, name)?))
}
