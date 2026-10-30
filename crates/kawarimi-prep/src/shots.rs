//! Shot cuts with TransNetV2 (`edgetools/transnetv2`): 48×27 frames in windows of 100, sliding by 50, keeping the
//! middle 50 predictions of each, as upstream's `predict_frames`.

use std::path::Path;

use anyhow::Result;
use edgetam_onnx::onnx::{self, Array, Device};
use serde::{Deserialize, Serialize};

pub const WIDTH: u32 = 48;
pub const HEIGHT: u32 = 27;
const WINDOW: usize = 100;
const STEP: usize = 50;
const CONTEXT: usize = 25;
/// Upstream's default threshold on the single-frame prediction.
pub const THRESHOLD: f32 = 0.5;

/// A shot: frames `start..end` of the normalised clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shot {
    pub start: usize,
    pub end: usize,
}

impl Shot {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

pub struct ShotDetector {
    session: ort::session::Session,
    /// The graph's first output, the single-frame prediction.
    single: String,
}

impl ShotDetector {
    pub fn load(model: &Path, device: Device) -> Result<Self> {
        let session = onnx::session(model, device)?;
        let single = session.outputs()[0].name().to_owned();
        Ok(Self { session, single })
    }

    /// The probability, per frame, that it is the last frame before a cut. `frames` are 48×27 RGB.
    pub fn predict(&mut self, frames: &[Vec<u8>]) -> Result<Vec<f32>> {
        let n = frames.len();
        if n == 0 {
            return Ok(Vec::new());
        }
        let tail = CONTEXT + STEP - if n % STEP == 0 { STEP } else { n % STEP };
        let padded: Vec<&[u8]> = std::iter::repeat_n(&frames[0][..], CONTEXT)
            .chain(frames.iter().map(|f| &f[..]))
            .chain(std::iter::repeat_n(&frames[n - 1][..], tail))
            .collect();
        let mut out = Vec::with_capacity(n + STEP);
        let mut at = 0;
        while at + WINDOW <= padded.len() {
            let data: Vec<f32> = padded[at..at + WINDOW].iter().flat_map(|f| f.iter().map(|&b| b as f32)).collect();
            let input = Array::new([1, WINDOW, HEIGHT as usize, WIDTH as usize, 3], data);
            let single = onnx::run(&mut self.session, vec![("input", input.value()?)], &[&self.single])?.remove(0);
            out.extend_from_slice(&single.data[CONTEXT..CONTEXT + STEP]);
            at += STEP;
        }
        out.truncate(n);
        Ok(out)
    }
}

/// Upstream's `predictions_to_scenes`: a frame above the threshold ends its shot.
pub fn shots(predictions: &[f32], threshold: f32) -> Vec<Shot> {
    let mut shots = Vec::new();
    let (mut start, mut previous) = (0, false);
    for (i, &p) in predictions.iter().enumerate() {
        let cut = p > threshold;
        if previous && !cut {
            start = i;
        }
        if !previous && cut && i != 0 {
            shots.push(Shot { start, end: i + 1 });
        }
        previous = cut;
    }
    if !previous && !predictions.is_empty() {
        shots.push(Shot { start, end: predictions.len() });
    }
    if shots.is_empty() && !predictions.is_empty() {
        shots.push(Shot { start: 0, end: predictions.len() });
    }
    shots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peak_ends_the_shot() {
        let mut p = vec![0.0; 10];
        p[3] = 0.9;
        assert_eq!(shots(&p, THRESHOLD), [Shot { start: 0, end: 4 }, Shot { start: 4, end: 10 }]);
    }

    #[test]
    fn no_cut_is_one_shot() {
        assert_eq!(shots(&[0.1; 5], THRESHOLD), [Shot { start: 0, end: 5 }]);
    }

    #[test]
    fn a_transition_over_several_frames_is_skipped() {
        let mut p = vec![0.0; 12];
        p[4..7].fill(0.8);
        assert_eq!(shots(&p, THRESHOLD), [Shot { start: 0, end: 5 }, Shot { start: 7, end: 12 }]);
    }
}
