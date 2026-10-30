//! A person corrected or added by points on one frame: EdgeTAM is seeded there and follows them through that shot,
//! forwards and then backwards, and the result replaces the shot's tracklets for that person in `cache.json`. Prep
//! run again on the folder then reuses every other tracklet and redoes only identities, masks and previews.

use std::path::Path;

use anyhow::{Context, Result};
use kawarimi_media::Ffmpeg;
use edgetam_onnx::{Device, edgetam::EdgeTam};
use serde::{Deserialize, Serialize};

use crate::cache::{Cache, StoredTracklet};
use crate::frames::{Bits, FrameCache};
use crate::people::{Box2, Detection};
use crate::pipeline::Models;
use crate::track::{Tracklet, Tuning, box_of};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub frame: usize,
    /// In the normalised clip's pixels.
    pub points: Vec<[f32; 2]>,
    /// 1 for a point on the person, 0 for one off them.
    pub labels: Vec<i32>,
    /// The tracklets (manifest indices) of the person being corrected; empty for someone prep missed.
    pub replaces: Vec<usize>,
}

/// Applies `fix` to the prepared folder `prep`; returns the new tracklet's index.
pub fn apply(ffmpeg: &Ffmpeg, models: &Models, prep: &Path, fix: &Fix, mut log: impl FnMut(&str)) -> Result<usize> {
    anyhow::ensure!(!fix.points.is_empty() && fix.points.len() == fix.labels.len(), "a fix needs points with labels");
    anyhow::ensure!(fix.labels.contains(&1), "a fix needs a point on the person");
    let mut cache = Cache::load(prep).context("prep this clip first")?;
    let (key, stored) = cache.tracking.take().context("prep this clip first")?;
    let shot = cache.shots.iter().position(|s| (s.start..s.end).contains(&fix.frame)).context("no shot has that frame")?;
    let (start, end) = (cache.shots[shot].start, cache.shots[shot].end);

    let video = cache.video.clone();
    let (w, h) = (video.width as usize, video.height as usize);
    let restored: Vec<Tracklet> = stored.iter().enumerate().map(|(i, s)| s.restore(i, w, w * h)).collect();
    let current = restored.iter().filter(|t| fix.replaces.contains(&t.id)).find_map(|t| t.mask_at(fix.frame)).map(box_of);
    let seed = seed(fix, current);

    let mut frames = FrameCache::build(ffmpeg, &prep.join("normalized.mp4"), video.width, video.height, &prep.join("fix.rgb"))?;
    let tracked = follow(&mut frames, models, &seed, start, end, &key.tuning, &mut log);
    frames.remove()?;
    let masks = tracked?;
    anyhow::ensure!(!masks.is_empty(), "the points found no one on frame {}", fix.frame);

    let seed_mask = masks.iter().find(|(f, _)| *f == fix.frame).map(|(_, m)| m.clone()).unwrap_or_else(|| masks[0].1.clone());
    let fixed = Tracklet {
        id: 0,
        shot,
        seed: Detection { frame: fix.frame, bounds: box_of(&seed_mask), score: 1.0 },
        masks,
    };
    // The person's tracklets in this shot go, and so does anyone the new one mostly covers on its seed frame.
    let gone: Vec<bool> = restored
        .iter()
        .map(|t| {
            t.shot == shot
                && (fix.replaces.contains(&t.id) || t.mask_at(fix.frame).is_some_and(|m| overlap(m, &seed_mask) > 0.5))
        })
        .collect();
    let at = gone.iter().position(|&g| g).unwrap_or(stored.len());
    let mut kept: Vec<StoredTracklet> = stored.into_iter().zip(&gone).filter(|(_, g)| !**g).map(|(s, _)| s).collect();
    kept.insert(at, StoredTracklet::store(&fixed));
    cache.tracking = Some((key, kept));
    cache.save(prep)?;
    Ok(at)
}

/// EdgeTAM's prompt for a fix. Clicks alone pick a part of a person (docs/PREP.md, box seeds), so the + points widen
/// a box: the person's current one, or around the points; − points stay as negatives. A lone click on someone new
/// stays a click.
fn seed(fix: &Fix, current: Option<Box2>) -> Fix {
    let on: Vec<[f32; 2]> = fix.points.iter().zip(&fix.labels).filter(|(_, l)| **l == 1).map(|(p, _)| *p).collect();
    let off: Vec<[f32; 2]> = fix.points.iter().zip(&fix.labels).filter(|(_, l)| **l == 0).map(|(p, _)| *p).collect();
    let grow = |b: Box2, p: &[f32; 2]| Box2 { x0: b.x0.min(p[0]), y0: b.y0.min(p[1]), x1: b.x1.max(p[0]), y1: b.y1.max(p[1]) };
    let around = |first: [f32; 2]| Box2 { x0: first[0], y0: first[1], x1: first[0], y1: first[1] };
    let bounds = match current {
        Some(b) => on.iter().fold(b, grow),
        None if on.len() >= 2 => on.iter().fold(around(on[0]), grow),
        None => return fix.clone(),
    };
    let mut points = vec![[bounds.x0, bounds.y0], [bounds.x1, bounds.y1]];
    let mut labels = vec![2, 3];
    points.extend(&off);
    labels.extend(off.iter().map(|_| 0));
    Fix { points, labels, ..fix.clone() }
}

/// The person from the seed frame to the shot's end, then back to its start.
fn follow(
    frames: &mut FrameCache,
    models: &Models,
    fix: &Fix,
    start: usize,
    end: usize,
    tuning: &Tuning,
    log: &mut impl FnMut(&str),
) -> Result<Vec<(usize, Bits)>> {
    let (w, h) = (frames.width, frames.height);
    let mut tracker = EdgeTam::load(&models.edgetam, Device::Cuda, frames.count)?;
    let mut rgb = Vec::new();
    let mut masks = Vec::new();
    let forward: Vec<usize> = (fix.frame + 1..end).collect();
    let backward: Vec<usize> = (start..fix.frame).rev().collect();
    for (pass, order) in [forward, backward].into_iter().enumerate() {
        frames.read(fix.frame, &mut rgb)?;
        tracker.encode(&rgb, w, h)?;
        let (object, step) = tracker.add(&fix.points, &fix.labels)?;
        if pass == 0 {
            let bits = Bits::pack(&step.mask, w);
            if bits.area > 0 {
                masks.push((fix.frame, bits));
            }
        }
        let mut lost = 0;
        for f in order {
            frames.read(f, &mut rgb)?;
            tracker.encode(&rgb, w, h)?;
            let step = tracker.step(object)?;
            let bits = Bits::pack(&step.mask, w);
            if step.object_score > 0.0 && bits.area > 0 {
                masks.push((f, bits));
                lost = 0;
            } else {
                lost += 1;
                if lost >= tuning.lost_frames {
                    break;
                }
            }
        }
        tracker.remove(object);
        log(&format!("fix: {} pass done", if pass == 0 { "forward" } else { "backward" }));
    }
    masks.sort_by_key(|(f, _)| *f);
    Ok(masks)
}

/// The share of `a` that `b` covers.
fn overlap(a: &Bits, b: &Bits) -> f32 {
    let both: u32 = a.data.iter().zip(&b.data).map(|(x, y)| (x & y).count_ones()).sum();
    both as f32 / a.area.max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_points_widen_the_box() {
        let fix = Fix { frame: 3, points: vec![[50.0, 10.0], [5.0, 5.0], [20.0, 20.0]], labels: vec![1, 0, 1], replaces: vec![0] };
        let current = Box2 { x0: 10.0, y0: 15.0, x1: 30.0, y1: 40.0 };
        let s = seed(&fix, Some(current));
        assert_eq!(s.points, vec![[10.0, 10.0], [50.0, 40.0], [5.0, 5.0]]);
        assert_eq!(s.labels, vec![2, 3, 0]);
        let new = seed(&Fix { replaces: vec![], ..fix.clone() }, None);
        assert_eq!((new.points[0], new.points[1]), ([20.0, 10.0], [50.0, 20.0]));
        let click = Fix { points: vec![[1.0, 2.0]], labels: vec![1], ..fix };
        assert_eq!(seed(&click, None), click);
    }

    #[test]
    fn overlap_is_the_covered_share() {
        let w = 8;
        let a = Bits::pack(&[1, 1, 1, 1, 0, 0, 0, 0], w);
        let b = Bits::pack(&[0, 0, 1, 1, 1, 1, 0, 0], w);
        assert_eq!(overlap(&a, &b), 0.5);
        assert_eq!(overlap(&b, &a), 0.5);
        assert_eq!(overlap(&a, &a), 1.0);
    }
}
