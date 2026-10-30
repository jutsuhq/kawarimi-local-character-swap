//! People as boxes, with RT-DETRv2 R18 (`onnx-community/rtdetr_v2_r18vd-ONNX`): 640² input scaled to 0–1 (no mean
//! or std, as its preprocessor config says), 300 queries, COCO class 0 is a person.

use std::path::Path;

use anyhow::Result;
use edgetam_onnx::onnx::{self, Array, Device};
use serde::{Deserialize, Serialize};

const SIDE: u32 = 640;
const PERSON: usize = 0;
const CLASSES: usize = 80;
pub const MIN_SCORE: f32 = 0.5;
/// Boxes overlapping more than this are the same person found twice.
const DUPLICATE_IOU: f32 = 0.7;

/// A box in the video's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Box2 {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Box2 {
    pub fn area(&self) -> f32 {
        (self.x1 - self.x0).max(0.0) * (self.y1 - self.y0).max(0.0)
    }

    pub fn iou(&self, other: &Box2) -> f32 {
        let w = (self.x1.min(other.x1) - self.x0.max(other.x0)).max(0.0);
        let h = (self.y1.min(other.y1) - self.y0.max(other.y0)).max(0.0);
        let inter = w * h;
        let union = self.area() + other.area() - inter;
        if union <= 0.0 { 0.0 } else { inter / union }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub frame: usize,
    pub bounds: Box2,
    pub score: f32,
}

pub struct PersonDetector {
    session: ort::session::Session,
}

impl PersonDetector {
    pub fn load(model: &Path, device: Device) -> Result<Self> {
        Ok(Self { session: onnx::session(model, device)? })
    }

    /// Every person in a frame (RGB, `width`×`height`), best first.
    pub fn detect(&mut self, rgb: &[u8], width: usize, height: usize, frame: usize) -> Result<Vec<Detection>> {
        let img = image::RgbImage::from_raw(width as u32, height as u32, rgb.to_vec()).expect("frame size");
        let resized = image::imageops::resize(&img, SIDE, SIDE, image::imageops::FilterType::Triangle);
        let plane = (SIDE * SIDE) as usize;
        let mut data = vec![0f32; 3 * plane];
        for (i, px) in resized.pixels().enumerate() {
            for c in 0..3 {
                data[c * plane + i] = px[c] as f32 / 255.0;
            }
        }
        let input = Array::new([1, 3, SIDE as usize, SIDE as usize], data);
        let [logits, boxes]: [Array; 2] =
            onnx::run(&mut self.session, vec![("pixel_values", input.value()?)], &["logits", "pred_boxes"])?.try_into().unwrap();
        let queries = logits.data.len() / CLASSES;
        let mut found: Vec<Detection> = (0..queries)
            .filter_map(|q| {
                let score = sigmoid(logits.data[q * CLASSES + PERSON]);
                (score >= MIN_SCORE).then(|| {
                    let b = &boxes.data[q * 4..q * 4 + 4];
                    let (cx, cy, w, h) = (b[0] * width as f32, b[1] * height as f32, b[2] * width as f32, b[3] * height as f32);
                    let bounds = Box2 {
                        x0: (cx - w / 2.0).max(0.0),
                        y0: (cy - h / 2.0).max(0.0),
                        x1: (cx + w / 2.0).min(width as f32),
                        y1: (cy + h / 2.0).min(height as f32),
                    };
                    Detection { frame, bounds, score }
                })
            })
            .collect();
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut kept: Vec<Detection> = Vec::new();
        for d in found {
            if kept.iter().all(|k| k.bounds.iou(&d.bounds) <= DUPLICATE_IOU) {
                kept.push(d);
            }
        }
        Ok(kept)
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_iou() {
        let a = Box2 { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 };
        let b = Box2 { x0: 5.0, y0: 0.0, x1: 15.0, y1: 10.0 };
        assert!((a.iou(&b) - 50.0 / 150.0).abs() < 1e-6);
        assert_eq!(a.iou(&Box2 { x0: 20.0, y0: 20.0, x1: 30.0, y1: 30.0 }), 0.0);
    }
}
