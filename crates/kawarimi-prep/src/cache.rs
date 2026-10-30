//! What a prep can reuse when it runs again on the same folder: the normalised clip and its shots while the source
//! and the normalising options are the same, the tracklets too while the tracking options and models are as well.
//! Changing only the targets then redoes just the identities and the mask video.

use std::path::Path;

use anyhow::Result;
use kawarimi_media::{Normalize, Probe};
use serde::{Deserialize, Serialize};

use crate::frames::Bits;
use crate::people::Detection;
use crate::shots::Shot;
use crate::track::{Tracklet, Tuning};

const FILE: &str = "cache.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceKey {
    pub size: u64,
    pub modified: u64,
    pub normalize: Normalize,
}

impl SourceKey {
    pub fn of(source: &Path, normalize: &Normalize) -> Result<Self> {
        let meta = std::fs::metadata(source)?;
        let modified = meta.modified()?.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        Ok(Self { size: meta.len(), modified, normalize: normalize.clone() })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackingKey {
    pub tuning: Tuning,
    /// The SHA-256 pins of the shot, person and tracking models.
    pub models: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Cache {
    pub source: SourceKey,
    pub video: Probe,
    pub shots: Vec<Shot>,
    pub tracking: Option<(TrackingKey, Vec<StoredTracklet>)>,
}

impl Cache {
    pub fn load(out: &Path) -> Option<Self> {
        let bytes = std::fs::read(out.join(FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn save(&self, out: &Path) -> Result<()> {
        std::fs::write(out.join(FILE), serde_json::to_vec(self)?)?;
        Ok(())
    }
}

/// A tracklet with its masks run-length encoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTracklet {
    pub shot: usize,
    pub seed: Detection,
    /// (frame, runs): alternating lengths of unset and set pixels, starting with unset.
    pub masks: Vec<(usize, Vec<u32>)>,
}

impl StoredTracklet {
    pub fn store(t: &Tracklet) -> Self {
        Self { shot: t.shot, seed: t.seed, masks: t.masks.iter().map(|(f, m)| (*f, runs(m))).collect() }
    }

    pub fn restore(&self, id: usize, width: usize, pixels: usize) -> Tracklet {
        let masks = self.masks.iter().map(|(f, r)| (*f, from_runs(r, width, pixels))).collect();
        Tracklet { id, shot: self.shot, seed: self.seed, masks }
    }
}

fn runs(mask: &Bits) -> Vec<u32> {
    let pixels = mask.data.len() * 8;
    let mut out = Vec::new();
    let (mut current, mut length) = (false, 0u32);
    for i in 0..pixels {
        let v = mask.get(i);
        if v == current {
            length += 1;
        } else {
            out.push(length);
            current = v;
            length = 1;
        }
    }
    out.push(length);
    out
}

fn from_runs(runs: &[u32], width: usize, pixels: usize) -> Bits {
    let mut plain = vec![0u8; runs.iter().map(|&r| r as usize).sum::<usize>().max(pixels)];
    let mut at = 0;
    for (k, &r) in runs.iter().enumerate() {
        if k % 2 == 1 {
            plain[at..at + r as usize].fill(1);
        }
        at += r as usize;
    }
    plain.truncate(pixels);
    Bits::pack(&plain, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_round_trip() {
        let (w, h) = (13, 7);
        let mut plain = vec![0u8; w * h];
        for i in [0, 1, 2, 20, 21, 40, 90] {
            plain[i] = 1;
        }
        let bits = Bits::pack(&plain, w);
        let back = from_runs(&runs(&bits), w, w * h);
        assert_eq!(back, bits);
    }
}
