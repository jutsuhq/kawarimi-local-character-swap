//! The whole prep for one clip, into one folder: `normalized.mp4`, `mask.mp4`, `people/<id>.png`, `manifest.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};
use kawarimi_assets::{Status, Store, manifest};
use kawarimi_media::{Ffmpeg, Normalize, Probe};
use edgetam_onnx::{Device, edgetam::EdgeTam};
use serde::Serialize;

use crate::cache::{Cache, SourceKey, StoredTracklet, TrackingKey};
use crate::faces::{Face, FaceDetector};
use crate::frames::{Bits, FrameCache};
use crate::identity::{self, Identity};
use crate::people::{Box2, PersonDetector};
use crate::shots::{self, Shot, ShotDetector};
use crate::track::{self, Tracklet, Tuning};

pub const MANIFEST_VERSION: u32 = 1;

/// Where the installed models are.
pub struct Models {
    pub transnetv2: PathBuf,
    pub rtdetr: PathBuf,
    pub edgetam: PathBuf,
    pub yunet: PathBuf,
    /// The models' SHA-256 pins: tracklets from an earlier run are reused only with the same models.
    pub pins: Vec<String>,
}

impl Models {
    /// The installed models in `store`, with the CUDA runtime made visible to ONNX Runtime.
    pub fn installed(store: &Store) -> Result<Self> {
        let need = |asset: &manifest::Asset| -> Result<PathBuf> {
            anyhow::ensure!(store.status(asset) == Status::Installed, "{} is not installed (kawarimi-fetch {})", asset.title, asset.id);
            Ok(store.dir(asset))
        };
        edgetam_onnx::cuda::use_runtime(&need(&manifest::CUDA_RUNTIME)?)?;
        Ok(Self {
            transnetv2: need(&manifest::TRANSNETV2)?.join("model.onnx"),
            rtdetr: need(&manifest::RTDETR)?.join("model.onnx"),
            edgetam: need(&manifest::EDGETAM)?,
            yunet: need(&manifest::YUNET)?.join("model.onnx"),
            pins: [manifest::TRANSNETV2, manifest::RTDETR, manifest::EDGETAM]
                .iter()
                .flat_map(|a| a.files.iter().map(|f| f.sha256.to_owned()))
                .collect(),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub enum Targets {
    /// The person with the most screen presence.
    #[default]
    Main,
    Everyone,
    Ids(Vec<usize>),
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub normalize: Normalize,
    pub targets: Targets,
    pub tuning: Tuning,
    /// Set to stop: prep fails with [`Cancelled`] at the next stage or tracked frame.
    pub cancel: Arc<AtomicBool>,
}

/// The error a cancelled prep fails with.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

impl Options {
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::SeqCst) { Err(Cancelled.into()) } else { Ok(()) }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: u32,
    pub source: PathBuf,
    pub video: Probe,
    pub frames: usize,
    pub shots: Vec<Shot>,
    pub tracklets: Vec<TrackletInfo>,
    pub identities: Vec<IdentityInfo>,
    /// Colour distance between every two tracklets (Bhattacharyya); identities merge below the threshold.
    pub appearance_distances: Vec<Vec<f32>>,
    pub merge_distance: f32,
    /// Tracklet pairs from different shots within `identity::CLOSE_CALL` of the threshold: worth a look.
    pub close_calls: Vec<(usize, usize, f32)>,
    pub targets: Vec<usize>,
    pub files: Files,
    pub tracking: track::TrackStats,
    /// Seconds per stage.
    pub timings: Vec<(String, f64)>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackletInfo {
    pub id: usize,
    pub shot: usize,
    pub first: usize,
    pub last: usize,
    pub frames_seen: usize,
    pub seed: Box2,
    pub seed_score: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityInfo {
    pub id: usize,
    pub tracklets: Vec<usize>,
    pub frames_seen: usize,
    /// The share of all frames this person is on screen.
    pub on_screen: f32,
    pub thumbnail: PathBuf,
    /// The person's face on the frame of the thumbnail, when one is found inside their mask.
    pub face: Option<FaceInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceInfo {
    pub frame: usize,
    /// In the frame's pixels.
    pub bounds: Box2,
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
    pub thumbnail: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Files {
    pub normalized: PathBuf,
    pub mask: PathBuf,
}

pub fn prepare(
    ffmpeg: &Ffmpeg,
    models: &Models,
    clip: &Path,
    out: &Path,
    options: &Options,
    mut log: impl FnMut(&str),
) -> Result<Manifest> {
    std::fs::create_dir_all(out.join("people"))?;
    let mut timings = Vec::new();
    let mut stage = |name: &str, started: Instant, log: &mut dyn FnMut(&str)| -> Result<()> {
        let s = started.elapsed().as_secs_f64();
        log(&format!("{name}: {s:.1} s"));
        timings.push((name.to_owned(), (s * 10.0).round() / 10.0));
        options.check()
    };
    options.check()?;

    let normalized = out.join("normalized.mp4");
    let source_key = SourceKey::of(clip, &options.normalize)?;
    let tracking_key = TrackingKey { tuning: options.tuning.clone(), models: models.pins.clone() };
    let cached = Cache::load(out).filter(|c| c.source == source_key && normalized.is_file());

    let (video, shots, stored) = match cached {
        Some(c) => {
            log("normalise and shots: reused");
            let stored = c.tracking.filter(|(k, _)| *k == tracking_key).map(|(_, t)| t);
            (c.video, c.shots, stored)
        }
        None => {
            let t = Instant::now();
            let video = ffmpeg.normalize(clip, &normalized, &options.normalize)?;
            stage("normalise", t, &mut log)?;
            let t = Instant::now();
            let small: Vec<Vec<u8>> =
                ffmpeg.frames(&normalized, shots::WIDTH, shots::HEIGHT, None)?.collect::<Result<_>>()?;
            let predictions = ShotDetector::load(&models.transnetv2, Device::Cuda)?.predict(&small)?;
            let shots = shots::shots(&predictions, shots::THRESHOLD);
            stage("shots", t, &mut log)?;
            (video, shots, None)
        }
    };

    let t = Instant::now();
    let mut frames = FrameCache::build(ffmpeg, &normalized, video.width, video.height, &out.join("frames.rgb"))?;
    stage("decode", t, &mut log)?;

    let (tracklets, stats) = match stored {
        Some(stored) => {
            log("people and tracking: reused");
            let pixels = frames.width * frames.height;
            let tracklets: Vec<Tracklet> =
                stored.iter().enumerate().map(|(i, s)| s.restore(i, frames.width, pixels)).collect();
            let stats = track::TrackStats { reused: true, tracklets: tracklets.len(), ..Default::default() };
            (tracklets, stats)
        }
        None => {
            let t = Instant::now();
            let mut detector = PersonDetector::load(&models.rtdetr, Device::Cuda)?;
            let mut tracker = EdgeTam::load(&models.edgetam, Device::Cuda, frames.count)?;
            stage("people and tracking models", t, &mut log)?;
            let t = Instant::now();
            let count = frames.count;
            let found = track::track_shots(&mut frames, &shots, &mut detector, &mut tracker, &options.tuning, |f| {
                if f % 16 == 0 {
                    log(&format!("people and tracking: frame {f} of {count}"));
                }
                options.check()
            })?;
            drop(tracker);
            stage("people and tracking", t, &mut log)?;
            found
        }
    };
    Cache {
        source: source_key,
        video: video.clone(),
        shots: shots.clone(),
        tracking: Some((tracking_key, tracklets.iter().map(StoredTracklet::store).collect())),
    }
    .save(out)?;

    let t = Instant::now();
    let appearances = tracklets.iter().map(|tl| identity::appearance(&mut frames, tl)).collect::<Result<Vec<_>>>()?;
    let distances: Vec<Vec<f32>> =
        appearances.iter().map(|a| appearances.iter().map(|b| a.distance(b)).collect()).collect();
    let overlap = |i: usize, j: usize| identity::overlapping(&tracklets[i], &tracklets[j]);
    let identities = identity::cluster(&distances, overlap, identity::MERGE_DISTANCE);
    let identities = identity::attach_fragments(identities, &distances, overlap, |t| {
        tracklets[t].masks.len() < identity::FRAGMENT_FRAMES
    });
    stage("identities", t, &mut log)?;

    let presence = |id: &Identity| id.tracklets.iter().map(|&t| tracklets[t].presence()).sum::<u64>();
    let targets: Vec<usize> = match &options.targets {
        Targets::Main => identities.iter().max_by_key(|i| presence(i)).map(|i| vec![i.id]).unwrap_or_default(),
        Targets::Everyone => identities.iter().map(|i| i.id).collect(),
        Targets::Ids(ids) => {
            for id in ids {
                anyhow::ensure!(identities.iter().any(|i| i.id == *id), "no person {id} in this clip");
            }
            ids.clone()
        }
    };

    let t = Instant::now();
    let mask = out.join("mask.mp4");
    let target_tracklets: Vec<&Tracklet> = identities
        .iter()
        .filter(|i| targets.contains(&i.id))
        .flat_map(|i| i.tracklets.iter().map(|&t| &tracklets[t]))
        .collect();
    write_mask(ffmpeg, &mask, &video, frames.count, &target_tracklets)?;
    stage("mask video", t, &mut log)?;

    let t = Instant::now();
    let mut faces = FaceDetector::load(&models.yunet, Device::Cuda)?;
    let mut identity_infos = Vec::new();
    for ident in &identities {
        let thumbnail = PathBuf::from("people").join(format!("{}.png", ident.id));
        let face_thumbnail = PathBuf::from("people").join(format!("{}-face.png", ident.id));
        let face = save_thumbnails(
            &mut frames,
            &mut faces,
            ident.tracklets.iter().map(|&t| &tracklets[t]),
            out,
            &thumbnail,
            &face_thumbnail,
        )?;
        let frames_seen: usize = ident.tracklets.iter().map(|&t| tracklets[t].masks.len()).sum();
        identity_infos.push(IdentityInfo {
            id: ident.id,
            tracklets: ident.tracklets.clone(),
            frames_seen,
            on_screen: frames_seen as f32 / frames.count.max(1) as f32,
            thumbnail,
            face,
        });
    }
    stage("people previews and faces", t, &mut log)?;
    let frame_count = frames.count;
    frames.remove()?;

    let manifest = Manifest {
        version: MANIFEST_VERSION,
        source: clip.to_owned(),
        video,
        frames: frame_count,
        shots,
        tracklets: tracklets
            .iter()
            .map(|t| TrackletInfo {
                id: t.id,
                shot: t.shot,
                first: t.first(),
                last: t.last(),
                frames_seen: t.masks.len(),
                seed: t.seed.bounds,
                seed_score: t.seed.score,
            })
            .collect(),
        identities: identity_infos,
        appearance_distances: distances.iter().map(|r| r.iter().map(|d| (d * 1000.0).round() / 1000.0).collect()).collect(),
        merge_distance: identity::MERGE_DISTANCE,
        close_calls: close_calls(&tracklets, &distances),
        targets,
        files: Files { normalized: "normalized.mp4".into(), mask: "mask.mp4".into() },
        tracking: stats,
        timings,
    };
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?).context("writing manifest.json")?;
    Ok(manifest)
}

/// The union of `tracklets`' masks on each of `count` frames, as the renderers' mask video.
fn write_mask(ffmpeg: &Ffmpeg, path: &Path, video: &Probe, count: usize, tracklets: &[&Tracklet]) -> Result<()> {
    let mut writer = ffmpeg.mask_writer(path, video.width, video.height, video.fps)?;
    let mut plane = vec![0u8; (video.width * video.height) as usize];
    for f in 0..count {
        plane.fill(0);
        for tl in tracklets {
            if let Some(m) = tl.mask_at(f) {
                m.paint(&mut plane);
            }
        }
        writer.write(&plane)?;
    }
    writer.finish()
}

/// The mask video of the tracklets with these indices (`manifest.json`'s identities list them) from a prepared
/// folder, on the CPU: the masks come from `cache.json`.
pub fn mask_of(ffmpeg: &Ffmpeg, prep: &Path, tracklets: &[usize], dest: &Path) -> Result<()> {
    let cache = Cache::load(prep).context("the prep folder has no cache.json")?;
    let (_, stored) = cache.tracking.context("the prep folder has no tracklets")?;
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(prep.join("manifest.json"))?)?;
    let count = manifest["frames"].as_u64().context("manifest has no frames")? as usize;
    let (w, h) = (cache.video.width as usize, cache.video.height as usize);
    let restored: Vec<Tracklet> = tracklets
        .iter()
        .map(|&i| stored.get(i).map(|s| s.restore(i, w, w * h)).with_context(|| format!("no tracklet {i}")))
        .collect::<Result<_>>()?;
    write_mask(ffmpeg, dest, &cache.video, count, &restored.iter().collect::<Vec<_>>())
}

fn close_calls(tracklets: &[Tracklet], distances: &[Vec<f32>]) -> Vec<(usize, usize, f32)> {
    let mut calls = Vec::new();
    for i in 0..tracklets.len() {
        for j in i + 1..tracklets.len() {
            let d = distances[i][j];
            if tracklets[i].shot != tracklets[j].shot && (d - identity::MERGE_DISTANCE).abs() < identity::CLOSE_CALL {
                calls.push((i, j, (d * 1000.0).round() / 1000.0));
            }
        }
    }
    calls
}

/// The person's preview, cropped with a margin, and their face on it. Among the frames where they are at least half
/// their largest size (the largest is often a close-up out of focus), frames with a face on the person's own mask win,
/// the sharpest face first; without any, the sharpest body. Text laid over a person hides the face, so it loses. The
/// face is searched in the person's crop, so a small face is scaled up for the detector.
fn save_thumbnails<'a>(
    frames: &mut FrameCache,
    faces: &mut FaceDetector,
    tracklets: impl Iterator<Item = &'a Tracklet>,
    out: &Path,
    thumbnail: &Path,
    face_thumbnail: &Path,
) -> Result<Option<FaceInfo>> {
    struct View {
        rank: (bool, f32),
        frame: usize,
        crop: [u32; 4],
        img: image::RgbImage,
        face: Option<(Face, Box2)>,
    }
    let views: Vec<&(usize, Bits)> = tracklets.flat_map(|t| t.masks.iter()).collect();
    let Some(largest) = views.iter().map(|(_, m)| m.area).max() else { return Ok(None) };
    let large: Vec<_> = views.into_iter().filter(|(_, m)| m.area * 2 >= largest).collect();
    let (w, h) = (frames.width as u32, frames.height as u32);
    let mut best: Option<View> = None;
    for &(frame, mask) in large.iter().step_by(large.len().div_ceil(THUMBNAIL_CANDIDATES).max(1)) {
        let mut rgb = Vec::new();
        frames.read(*frame, &mut rgb)?;
        let img = image::RgbImage::from_raw(w, h, rgb).expect("frame size");
        let [x0, y0, x1, y1] = expand(mask, w, h);
        let crop = image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image();
        let on_mask = |f: &Face| {
            let (cx, cy) = (x0 as f32 + (f.bounds.x0 + f.bounds.x1) / 2.0, y0 as f32 + (f.bounds.y0 + f.bounds.y1) / 2.0);
            mask.get(cy as usize * w as usize + cx as usize)
        };
        let face = faces.detect(crop.as_raw(), crop.width() as usize, crop.height() as usize)?.into_iter().find(on_mask);
        let face = face.map(|f| {
            let bounds = Box2 { x0: f.bounds.x0 + x0 as f32, y0: f.bounds.y0 + y0 as f32, x1: f.bounds.x1 + x0 as f32, y1: f.bounds.y1 + y0 as f32 };
            (f, bounds)
        });
        let detail = match &face {
            Some((_, b)) => {
                let [fx0, fy0, fx1, fy1] = [b.x0.max(0.0) as u32, b.y0.max(0.0) as u32, (b.x1 as u32).min(w), (b.y1 as u32).min(h)];
                sharpness(&image::imageops::crop_imm(&img, fx0, fy0, (fx1 - fx0).max(3), (fy1 - fy0).max(3)).to_image(), |_, _| true)
            }
            None => sharpness(&crop, |x, y| mask.get(((y + y0) * w + x + x0) as usize)),
        };
        let rank = (face.is_some(), detail);
        if best.as_ref().is_none_or(|b| rank.0 > b.rank.0 || (rank.0 == b.rank.0 && rank.1 > b.rank.1)) {
            best = Some(View { rank, frame: *frame, crop: [x0, y0, x1, y1], img, face });
        }
    }
    let Some(View { frame, crop: [x0, y0, x1, y1], img, face, .. }) = best else { return Ok(None) };
    image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image().save(out.join(thumbnail))?;
    let Some((face, bounds)) = face else { return Ok(None) };
    let landmarks = face.landmarks.map(|[x, y]| [x + x0 as f32, y + y0 as f32]);
    // The face with 40 % of its size around it.
    let (fw, fh) = (bounds.x1 - bounds.x0, bounds.y1 - bounds.y0);
    let (cx0, cy0) = ((bounds.x0 - 0.4 * fw).max(0.0) as u32, (bounds.y0 - 0.4 * fh).max(0.0) as u32);
    let (cx1, cy1) = (((bounds.x1 + 0.4 * fw) as u32).min(w), ((bounds.y1 + 0.4 * fh) as u32).min(h));
    image::imageops::crop_imm(&img, cx0, cy0, cx1 - cx0, cy1 - cy0).to_image().save(out.join(face_thumbnail))?;
    Ok(Some(FaceInfo { frame, bounds, landmarks, score: face.score, thumbnail: face_thumbnail.to_owned() }))
}

/// Frames looked at for a person's preview.
const THUMBNAIL_CANDIDATES: usize = 16;

/// The variance of the Laplacian of the crop's luma over the person's own pixels (`on(x, y)` in crop coordinates), at
/// a fixed height: how much fine detail the person holds, comparable between crops of different sizes and blind to
/// text or backgrounds around them.
fn sharpness(crop: &image::RgbImage, on: impl Fn(u32, u32) -> bool) -> f32 {
    const HEIGHT: u32 = 160;
    let (cw, ch) = crop.dimensions();
    let width = (cw * HEIGHT / ch.max(1)).max(3);
    let luma = image::imageops::resize(
        &image::DynamicImage::ImageRgb8(crop.clone()).to_luma8(),
        width,
        HEIGHT,
        image::imageops::FilterType::Triangle,
    );
    let px = |x: u32, y: u32| luma.get_pixel(x, y).0[0] as f32;
    let inside = |x: u32, y: u32| on(x * cw / width, y * ch / HEIGHT);
    let values: Vec<f32> = (1..HEIGHT - 1)
        .flat_map(|y| (1..width - 1).map(move |x| (x, y)))
        .filter(|&(x, y)| inside(x, y))
        .map(|(x, y)| px(x - 1, y) + px(x + 1, y) + px(x, y - 1) + px(x, y + 1) - 4.0 * px(x, y))
        .collect();
    let n = values.len().max(1) as f32;
    let mean = values.iter().sum::<f32>() / n;
    values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n
}

fn expand(mask: &Bits, width: u32, height: u32) -> [u32; 4] {
    let [x0, y0, x1, y1] = mask.bounds;
    let (mx, my) = ((x1 - x0) / 8, (y1 - y0) / 8);
    [x0.saturating_sub(mx), y0.saturating_sub(my), (x1 + mx).min(width), (y1 + my).min(height)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blurred_crop_has_less_detail() {
        let sharp = image::RgbImage::from_fn(120, 200, |x, y| {
            if (x / 4 + y / 4) % 2 == 0 { image::Rgb([230; 3]) } else { image::Rgb([20; 3]) }
        });
        let blurred = image::imageops::blur(&sharp, 3.0);
        let all = |_, _| true;
        assert!(sharpness(&sharp, all) > 4.0 * sharpness(&blurred, all));
        // Detail outside the person's pixels does not count.
        let mut texted = blurred.clone();
        for y in 0..40 {
            for x in 0..120 {
                texted.put_pixel(x, y, if x % 3 == 0 { image::Rgb([255; 3]) } else { image::Rgb([0; 3]) });
            }
        }
        let person = |_, y: u32| y >= 60;
        assert!(sharpness(&texted, person) < sharpness(&sharp, person) / 4.0);
    }
}
