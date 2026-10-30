//! Tracklets joined into people across cuts by the colour of what they wear (Swapjutsu's stage 05): an HSV histogram
//! inside the mask, average-linkage merging below a distance, and never two tracklets seen in the same frame.

use anyhow::Result;
use serde::Serialize;

use crate::frames::FrameCache;
use crate::track::Tracklet;

const H_BINS: usize = 12;
const S_BINS: usize = 4;
const V_BINS: usize = 4;
const SAMPLES: usize = 8;
/// Bhattacharyya distance under which two groups of tracklets are one person. On cuts.mp4 (EdgeTAM's bedroom around
/// a surfing shot) the same child across the cut measured 0.106 and 0.36, the two children 0.456–0.498.
pub const MERGE_DISTANCE: f32 = 0.40;
/// Pairs this close to the threshold either way are reported, as Swapjutsu warns within 0.05.
pub const CLOSE_CALL: f32 = 0.05;
/// Tracklets shorter than this (half a second at 16 fps) are fragments: a person glimpsed across a cut or a
/// transition, too short to be told apart by appearance.
pub const FRAGMENT_FRAMES: usize = 8;

/// A normalised colour histogram.
#[derive(Debug, Clone)]
pub struct Appearance(Vec<f32>);

impl Appearance {
    /// Bhattacharyya distance: 0 for the same distribution, 1 for disjoint ones.
    pub fn distance(&self, other: &Appearance) -> f32 {
        let bc: f32 = self.0.iter().zip(&other.0).map(|(a, b)| (a * b).sqrt()).sum();
        (1.0 - bc.min(1.0)).sqrt()
    }
}

/// The colour histogram inside a tracklet's masks, from up to eight frames spread over it.
pub fn appearance(frames: &mut FrameCache, tracklet: &Tracklet) -> Result<Appearance> {
    let mut hist = vec![0f32; H_BINS * S_BINS * V_BINS];
    let n = tracklet.masks.len();
    let picks: Vec<usize> = (0..SAMPLES.min(n)).map(|k| k * n / SAMPLES.min(n)).collect();
    let mut rgb = Vec::new();
    for i in picks {
        let (frame, mask) = &tracklet.masks[i];
        frames.read(*frame, &mut rgb)?;
        for (p, px) in rgb.chunks_exact(3).enumerate() {
            if mask.get(p) {
                let (h, s, v) = hsv(px[0], px[1], px[2]);
                let bin = ((h * H_BINS as f32) as usize).min(H_BINS - 1) * S_BINS * V_BINS
                    + ((s * S_BINS as f32) as usize).min(S_BINS - 1) * V_BINS
                    + ((v * V_BINS as f32) as usize).min(V_BINS - 1);
                hist[bin] += 1.0;
            }
        }
    }
    let total: f32 = hist.iter().sum();
    if total > 0.0 {
        hist.iter_mut().for_each(|v| *v /= total);
    }
    Ok(Appearance(hist))
}

fn hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

#[derive(Debug, Clone, Serialize)]
pub struct Identity {
    pub id: usize,
    pub tracklets: Vec<usize>,
}

/// Average-linkage clustering of tracklets. `distances[i][j]` between tracklets; `overlap(i, j)` true when both are
/// seen in one frame, which keeps them apart. Identities are numbered by first appearance.
pub fn cluster(distances: &[Vec<f32>], overlap: impl Fn(usize, usize) -> bool, threshold: f32) -> Vec<Identity> {
    let mut groups: Vec<Vec<usize>> = (0..distances.len()).map(|i| vec![i]).collect();
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for a in 0..groups.len() {
            for b in a + 1..groups.len() {
                if groups[a].iter().any(|&i| groups[b].iter().any(|&j| overlap(i, j))) {
                    continue;
                }
                let sum: f32 = groups[a].iter().flat_map(|&i| groups[b].iter().map(move |&j| distances[i][j])).sum();
                let mean = sum / (groups[a].len() * groups[b].len()) as f32;
                if mean < threshold && best.is_none_or(|(_, _, d)| mean < d) {
                    best = Some((a, b, mean));
                }
            }
        }
        let Some((a, b, _)) = best else { break };
        let merged = groups.remove(b);
        groups[a].extend(merged);
    }
    groups.iter_mut().for_each(|g| g.sort());
    groups.sort_by_key(|g| g[0]);
    groups.into_iter().enumerate().map(|(id, tracklets)| Identity { id: id + 1, tracklets }).collect()
}

/// Identities made only of fragments join the closest identity they are never seen with, whatever the distance, so
/// a glimpse at a cut does not become a person of its own. They stay apart only when every identity overlaps them.
pub fn attach_fragments(
    identities: Vec<Identity>,
    distances: &[Vec<f32>],
    overlap: impl Fn(usize, usize) -> bool,
    fragment: impl Fn(usize) -> bool,
) -> Vec<Identity> {
    let (loose, mut kept): (Vec<_>, Vec<_>) = identities.into_iter().partition(|i| i.tracklets.iter().all(|&t| fragment(t)));
    if kept.is_empty() {
        return renumber(loose);
    }
    for group in loose {
        let mean = |into: &Identity| {
            let sum: f32 = group.tracklets.iter().flat_map(|&i| into.tracklets.iter().map(move |&j| distances[i][j])).sum();
            sum / (group.tracklets.len() * into.tracklets.len()) as f32
        };
        let free = |into: &Identity| !group.tracklets.iter().any(|&i| into.tracklets.iter().any(|&j| overlap(i, j)));
        match kept.iter_mut().filter(|k| free(k)).min_by(|a, b| mean(a).total_cmp(&mean(b))) {
            Some(into) => into.tracklets.extend(group.tracklets),
            None => kept.push(group),
        }
    }
    renumber(kept)
}

fn renumber(mut identities: Vec<Identity>) -> Vec<Identity> {
    identities.iter_mut().for_each(|i| i.tracklets.sort());
    identities.sort_by_key(|i| i.tracklets[0]);
    identities.into_iter().enumerate().map(|(n, i)| Identity { id: n + 1, tracklets: i.tracklets }).collect()
}

/// Whether two tracklets are ever seen in the same frame.
pub fn overlapping(a: &Tracklet, b: &Tracklet) -> bool {
    a.shot == b.shot && a.first() <= b.last() && b.first() <= a.last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_close_and_keeps_concurrent_apart() {
        // 0 and 2 look alike; 1 is seen with 0, so even though it also looks alike it stays apart.
        let d = vec![vec![0.0, 0.1, 0.1], vec![0.1, 0.0, 0.9], vec![0.1, 0.9, 0.0]];
        let ids = cluster(&d, |i, j| (i, j) == (0, 1) || (i, j) == (1, 0), 0.35);
        assert_eq!(ids.iter().map(|i| i.tracklets.clone()).collect::<Vec<_>>(), [vec![0, 2], vec![1]]);
    }

    #[test]
    fn fragments_join_a_person_instead_of_making_one() {
        // 0 and 1 are two people in a long shot; 2 and 3 are a two-frame glimpse of both after a cut, too far from
        // anyone by appearance to merge. 2 is seen with 3, so they go to different people.
        let d = vec![
            vec![0.0, 0.9, 0.5, 0.6],
            vec![0.9, 0.0, 0.6, 0.45],
            vec![0.5, 0.6, 0.0, 0.9],
            vec![0.6, 0.45, 0.9, 0.0],
        ];
        let overlap = |i: usize, j: usize| matches!((i.min(j), i.max(j)), (0, 1) | (2, 3));
        let ids = cluster(&d, overlap, 0.40);
        assert_eq!(ids.len(), 4);
        let ids = attach_fragments(ids, &d, overlap, |t| t >= 2);
        assert_eq!(ids.iter().map(|i| i.tracklets.clone()).collect::<Vec<_>>(), [vec![0, 2], vec![1, 3]]);
        // Only fragments: nothing to join, they stay.
        let alone = attach_fragments(cluster(&d, overlap, 0.40), &d, overlap, |_| true);
        assert_eq!(alone.len(), 4);
    }

    #[test]
    fn hsv_of_primaries() {
        assert_eq!(hsv(255, 0, 0), (0.0, 1.0, 1.0));
        let (h, _, _) = hsv(0, 0, 255);
        assert!((h - 4.0 / 6.0).abs() < 1e-6);
    }
}
