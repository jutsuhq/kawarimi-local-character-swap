//! Faces with YuNet (OpenCV Zoo, `face_detection_yunet_2023mar`, MIT): a box and five landmarks per face, decoded from
//! its three strides as OpenCV's `FaceDetectorYN` does.

use std::path::Path;

use anyhow::Result;
use edgetam_onnx::onnx::{self, Array, Device};
use serde::Serialize;

use crate::people::Box2;

const SIDE: usize = 640;
const STRIDES: [usize; 3] = [8, 16, 32];
/// OpenCV's default is 0.9; lower keeps small and turned faces, which the person's own crop then confirms.
pub const MIN_SCORE: f32 = 0.75;
const NMS_IOU: f32 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Face {
    pub bounds: Box2,
    /// Right eye, left eye, nose tip, right and left mouth corners, in the image's pixels.
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
}

pub struct FaceDetector {
    session: ort::session::Session,
    outputs: Vec<String>,
}

impl FaceDetector {
    pub fn load(model: &Path, device: Device) -> Result<Self> {
        let session = onnx::session(model, device)?;
        let outputs = ["cls", "obj", "bbox", "kps"]
            .iter()
            .flat_map(|k| STRIDES.iter().map(move |s| format!("{k}_{s}")))
            .collect();
        Ok(Self { session, outputs })
    }

    /// Faces in an RGB image, best first. The image is scaled to fit 640² (aspect kept, padded at the bottom right).
    pub fn detect(&mut self, rgb: &[u8], width: usize, height: usize) -> Result<Vec<Face>> {
        let scale = SIDE as f32 / width.max(height) as f32;
        let (sw, sh) = (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
        let img = image::RgbImage::from_raw(width as u32, height as u32, rgb.to_vec()).expect("image size");
        let resized = image::imageops::resize(&img, sw, sh, image::imageops::FilterType::Triangle);
        // YuNet takes BGR in 0–255.
        let plane = SIDE * SIDE;
        let mut data = vec![0f32; 3 * plane];
        for (x, y, px) in resized.enumerate_pixels() {
            let i = y as usize * SIDE + x as usize;
            data[i] = px[2] as f32;
            data[plane + i] = px[1] as f32;
            data[2 * plane + i] = px[0] as f32;
        }
        let input = Array::new([1, 3, SIDE, SIDE], data);
        let names: Vec<&str> = self.outputs.iter().map(String::as_str).collect();
        let out = onnx::run(&mut self.session, vec![("input", input.value()?)], &names)?;
        let (cls, obj, bbox, kps) = (&out[0..3], &out[3..6], &out[6..9], &out[9..12]);

        let mut faces = Vec::new();
        for (k, &stride) in STRIDES.iter().enumerate() {
            let cols = SIDE / stride;
            for idx in 0..cols * cols {
                let score = (cls[k].data[idx].clamp(0.0, 1.0) * obj[k].data[idx].clamp(0.0, 1.0)).sqrt();
                if score < MIN_SCORE {
                    continue;
                }
                let (r, c) = ((idx / cols) as f32, (idx % cols) as f32);
                let s = stride as f32;
                let b = &bbox[k].data[idx * 4..idx * 4 + 4];
                let (cx, cy, w, h) = ((c + b[0]) * s, (r + b[1]) * s, b[2].exp() * s, b[3].exp() * s);
                let p = &kps[k].data[idx * 10..idx * 10 + 10];
                let mut landmarks = [[0f32; 2]; 5];
                for (n, l) in landmarks.iter_mut().enumerate() {
                    *l = [(c + p[2 * n]) * s / scale, (r + p[2 * n + 1]) * s / scale];
                }
                let bounds = Box2 {
                    x0: ((cx - w / 2.0) / scale).max(0.0),
                    y0: ((cy - h / 2.0) / scale).max(0.0),
                    x1: ((cx + w / 2.0) / scale).min(width as f32),
                    y1: ((cy + h / 2.0) / scale).min(height as f32),
                };
                faces.push(Face { bounds, landmarks, score });
            }
        }
        faces.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut kept: Vec<Face> = Vec::new();
        for f in faces {
            if kept.iter().all(|k| k.bounds.iou(&f.bounds) <= NMS_IOU) {
                kept.push(f);
            }
        }
        Ok(kept)
    }
}
