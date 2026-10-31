//! People followed through each shot: a person detector every few frames finds them, EdgeTAM follows each from the
//! box it was first seen in to the end of the shot or until it is lost.
//!
//! A shot is gone through once, frame by frame, with everyone in it: each frame is encoded once and then stepped for
//! every person followed on it.

use anyhow::Result;
use edgetam_onnx::Step;
use edgetam_onnx::edgetam::{EdgeTam, ObjectId};
use serde::{Deserialize, Serialize};

use crate::frames::{Bits, FrameCache};
use crate::people::{Box2, Detection, PersonDetector};
use crate::shots::Shot;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    /// Frames between person detections (0.5 s at 16 fps).
    pub detect_every: usize,
    /// A tracklet ends after this many frames in a row without the object (Swapjutsu's `lostFrames`).
    pub lost_frames: usize,
    /// A detection overlapping a tracklet's mask box at least this much is that tracklet, not someone new.
    pub same_person_iou: f32,
    /// A tracklet seen in fewer than this share of its shot's frames (and fewer than `min_frames`) is dropped: a
    /// poster, a mannequin, a false detection. Swapjutsu's `minConfirmed`.
    pub min_share: f32,
    pub min_frames: usize,
}

impl Default for Tuning {
    fn default() -> Self {
        Self { detect_every: 8, lost_frames: 12, same_person_iou: 0.3, min_share: 0.2, min_frames: 8 }
    }
}

/// One person inside one shot.
#[derive(Debug, Clone)]
pub struct Tracklet {
    pub id: usize,
    pub shot: usize,
    /// The frame it was seeded on.
    pub seed: Detection,
    /// (frame, mask) for every frame the tracker saw the object.
    pub masks: Vec<(usize, Bits)>,
}

impl Tracklet {
    pub fn first(&self) -> usize {
        self.masks.first().map_or(self.seed.frame, |m| m.0)
    }

    pub fn last(&self) -> usize {
        self.masks.last().map_or(self.seed.frame, |m| m.0)
    }

    pub fn mask_at(&self, frame: usize) -> Option<&Bits> {
        self.masks.binary_search_by_key(&frame, |m| m.0).ok().map(|i| &self.masks[i].1)
    }

    /// The mask on `frame`, or on a frame inside the tracklet where the tracker lost the object for a while, the
    /// nearest frame's (the earlier on a tie), with whether it was borrowed. None outside the tracklet.
    pub fn mask_held(&self, frame: usize) -> Option<(&Bits, bool)> {
        match self.masks.binary_search_by_key(&frame, |m| m.0) {
            Ok(i) => Some((&self.masks[i].1, false)),
            Err(0) => None,
            Err(i) if i == self.masks.len() => None,
            Err(i) => {
                let (before, after) = (&self.masks[i - 1], &self.masks[i]);
                Some((if frame - before.0 <= after.0 - frame { &before.1 } else { &after.1 }, true))
            }
        }
    }

    /// Mask pixels summed over every frame: the "screen presence" targets are chosen by.
    pub fn presence(&self) -> u64 {
        self.masks.iter().map(|(_, m)| m.area as u64).sum()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackStats {
    /// The tracklets came from an earlier run on the same clip and options.
    pub reused: bool,
    pub tracklets: usize,
    pub detections: usize,
    /// Tracklets dropped as too short to be a person.
    pub dropped: usize,
    pub tracked_frames: usize,
    /// Frames the tracker encoded: once each, whoever is on them.
    pub encoded_frames: usize,
    /// Target frames whose mask was borrowed from a nearby frame because the tracker had lost the person there.
    pub filled_frames: usize,
}

/// Every person in every shot, each as one tracklet per shot.
///
/// A detection seeds a tracklet unless a kept tracklet of its shot covers it on its frame. Whether a tracklet is kept
/// is only known once it is long enough or has ended, so a detection covered only by tracklets still undecided seeds
/// one that exists only if none of them is kept, and is dropped as soon as one is. The tracklets are those of
/// following every person to the end, one after another, in the order they were detected.
pub fn track_shots(
    frames: &mut FrameCache,
    shots: &[Shot],
    detector: &mut PersonDetector,
    tracker: &mut EdgeTam,
    tuning: &Tuning,
    mut progress: impl FnMut(usize) -> Result<()>,
) -> Result<(Vec<Tracklet>, TrackStats)> {
    let (w, h) = (frames.width, frames.height);
    let mut rgb = Vec::new();
    let mut tracklets: Vec<Tracklet> = Vec::new();
    let mut stats = TrackStats::default();
    for (shot_index, shot) in shots.iter().enumerate() {
        let needed = ((shot.len() as f32 * tuning.min_share) as usize).max(tuning.min_frames).min(shot.len());
        let mut lives: Vec<Live> = Vec::new();
        for f in shot.start..shot.end {
            let detect = (f - shot.start) % tuning.detect_every == 0;
            let following = lives.iter().any(|l| l.object.is_some());
            if !detect && !following {
                continue;
            }
            frames.read(f, &mut rgb)?;
            let mut encoded = following;
            if following {
                tracker.encode(&rgb, w, h)?;
                stats.encoded_frames += 1;
                for live in lives.iter_mut() {
                    if let Some(object) = live.object {
                        let step = tracker.step(object)?;
                        live.record(f, step, w, tuning, tracker);
                    }
                }
            }
            if detect {
                let mut detections = detector.detect(&rgb, w, h, f)?;
                stats.detections += detections.len();
                // Best first, as the per-person passes took them.
                detections.sort_by(|a, b| b.score.total_cmp(&a.score));
                for d in detections {
                    let status = status(&lives, needed);
                    let covering: Vec<usize> = (0..lives.len())
                        .filter(|&j| {
                            lives[j].tracklet.mask_at(f).is_some_and(|m| box_of(m).iou(&d.bounds) >= tuning.same_person_iou)
                        })
                        .collect();
                    if covering.iter().any(|&j| status[j].kept == Some(true)) {
                        continue;
                    }
                    if !encoded {
                        tracker.encode(&rgb, w, h)?;
                        stats.encoded_frames += 1;
                        encoded = true;
                    }
                    let b = d.bounds;
                    let (object, step) = tracker.add(&[[b.x0, b.y0], [b.x1, b.y1]], &[2, 3])?;
                    let mut live = Live {
                        tracklet: Tracklet { id: 0, shot: shot_index, seed: d, masks: Vec::new() },
                        object: Some(object),
                        lost: 0,
                        unless: covering.into_iter().filter(|&j| status[j].kept.is_none()).collect(),
                    };
                    live.record(f, step, w, tuning, tracker);
                    lives.push(live);
                }
            }
            // One that can no longer exist is not followed further.
            let status = status(&lives, needed);
            for (live, s) in lives.iter_mut().zip(status) {
                if s.exists == Some(false) {
                    live.end(tracker);
                }
            }
            progress(f)?;
        }
        for live in &mut lives {
            live.end(tracker);
        }
        let status = status(&lives, needed);
        for (live, s) in lives.into_iter().zip(status) {
            if s.exists != Some(true) {
                continue;
            }
            let t = live.tracklet;
            stats.tracked_frames += t.last() + 1 - t.first();
            if s.kept == Some(true) {
                tracklets.push(t);
            } else {
                stats.dropped += 1;
            }
        }
    }
    // Ids in order of appearance after empty ones were dropped.
    for (i, t) in tracklets.iter_mut().enumerate() {
        t.id = i;
    }
    stats.tracklets = tracklets.len();
    Ok((tracklets, stats))
}

/// A tracklet while its shot is gone through.
struct Live {
    tracklet: Tracklet,
    /// While it is followed.
    object: Option<ObjectId>,
    /// Frames in a row without the object.
    lost: usize,
    /// It exists only if none of these (earlier tracklets of the shot, undecided when they covered its detection)
    /// is kept.
    unless: Vec<usize>,
}

impl Live {
    fn record(&mut self, frame: usize, step: Step, width: usize, tuning: &Tuning, tracker: &mut EdgeTam) {
        let bits = Bits::pack(&step.mask, width);
        if step.object_score > 0.0 && bits.area > 0 {
            self.tracklet.masks.push((frame, bits));
            self.lost = 0;
        } else {
            self.lost += 1;
            if self.lost >= tuning.lost_frames {
                self.end(tracker);
            }
        }
    }

    fn end(&mut self, tracker: &mut EdgeTam) {
        if let Some(object) = self.object.take() {
            tracker.remove(object);
        }
    }
}

/// What is known so far of a tracklet; `None` is not yet known.
struct Status {
    exists: Option<bool>,
    /// It exists and is seen on enough frames.
    kept: Option<bool>,
}

fn status(lives: &[Live], needed: usize) -> Vec<Status> {
    decide(lives.iter().map(|l| (l.unless.as_slice(), l.tracklet.masks.len(), l.object.is_some())), needed)
}

/// From each tracklet's `unless`, frames seen so far and whether it is still followed.
fn decide<'a>(lives: impl Iterator<Item = (&'a [usize], usize, bool)>, needed: usize) -> Vec<Status> {
    let mut out: Vec<Status> = Vec::new();
    for (unless, seen, following) in lives {
        let exists = all(unless.iter().map(|&j| out[j].kept.map(|kept| !kept)));
        let long = if seen >= needed {
            Some(true)
        } else if following {
            None
        } else {
            Some(false)
        };
        out.push(Status { exists, kept: all([exists, long]) });
    }
    out
}

/// Three-valued AND: false if any is, true if all are, otherwise not known.
fn all(values: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut known = true;
    for v in values {
        match v {
            Some(false) => return Some(false),
            None => known = false,
            Some(true) => {}
        }
    }
    known.then_some(true)
}

pub fn box_of(mask: &Bits) -> Box2 {
    let [x0, y0, x1, y1] = mask.bounds;
    Box2 { x0: x0 as f32, y0: y0 as f32, x1: x1 as f32, y1: y1 as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_under_an_undecided_tracklet_waits_for_it() {
        let exists = |zero: (usize, bool)| {
            // 1 was seeded where 0 covered it, 2 where 1 did; both still followed with a frame seen.
            let lives = [(&[][..], zero.0, zero.1), (&[0][..], 1, true), (&[1][..], 1, true)];
            decide(lives.into_iter(), 8).iter().map(|s| s.exists).collect::<Vec<_>>()
        };
        assert_eq!(exists((3, true)), [Some(true), None, None]);
        // 0 ends too short: 1 exists, and 2 waits on 1.
        assert_eq!(exists((3, false)), [Some(true), Some(true), None]);
        // 0 is kept: 1 never existed, so nothing covered 2.
        assert_eq!(exists((8, true)), [Some(true), Some(false), Some(true)]);
    }

    #[test]
    fn a_lost_frame_inside_a_tracklet_takes_the_nearest_mask() {
        // One pixel each, at the frame number modulo 8, marking which frame the mask came from.
        let mask = |p: usize| Bits::pack(&(0..8).map(|i| u8::from(i == p % 8)).collect::<Vec<_>>(), 8);
        let bounds = crate::people::Box2 { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
        let tracklet = Tracklet {
            id: 0,
            shot: 0,
            seed: Detection { frame: 2, bounds, score: 1.0 },
            masks: vec![(2, mask(2)), (6, mask(6)), (9, mask(9))],
        };
        let held = |f| tracklet.mask_held(f).map(|(m, borrowed)| (m.data[0].trailing_zeros(), borrowed));
        assert_eq!(held(2), Some((2, false)));
        assert_eq!(held(3), Some((2, true)));
        assert_eq!(held(4), Some((2, true)), "a tie goes to the earlier frame");
        assert_eq!(held(5), Some((6, true)));
        assert_eq!(held(8), Some((1, true)));
        assert_eq!(held(1), None);
        assert_eq!(held(10), None, "the person left the shot");
    }
}
