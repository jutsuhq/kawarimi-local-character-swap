//! A character image lifted off its background with BiRefNet lite (`onnx-community/BiRefNet_lite-ONNX`, MIT):
//! 1024² input normalised with ImageNet's mean and std, a logit per pixel. The character is put on plain grey, as
//! Swapjutsu's stage 10 does, so the renderer reads the character and not the background it was drawn on.

use std::path::Path;

use anyhow::Result;
use edgetam_onnx::frame::resize_bilinear;
use edgetam_onnx::onnx::{self, Array, Device};

const SIDE: usize = 1024;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];
pub const GREY: u8 = 128;

pub struct Cutter {
    session: ort::session::Session,
}

/// The cutout: the alpha at the image's size, and the image composited on grey.
pub struct Cutout {
    pub alpha: Vec<u8>,
    pub on_grey: Vec<u8>,
}

impl Cutout {
    /// The character on grey cropped to where the alpha is solid, with a margin of 4 % of its size: Wan's reference
    /// fills the render frame with whatever it gets, so empty borders would only shrink the character. The whole
    /// image when nothing is solid.
    pub fn crop(&self, width: u32, height: u32) -> image::RgbImage {
        let img = image::RgbImage::from_raw(width, height, self.on_grey.clone()).expect("cutout size");
        let solid = |i: usize| self.alpha[i] > 127;
        let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
        for y in 0..height {
            for x in 0..width {
                if solid((y * width + x) as usize) {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                }
            }
        }
        if x1 <= x0 || y1 <= y0 {
            return img;
        }
        let (mx, my) = ((x1 - x0) / 25 + 1, (y1 - y0) / 25 + 1);
        let (x0, y0, x1, y1) = (x0.saturating_sub(mx), y0.saturating_sub(my), (x1 + mx).min(width), (y1 + my).min(height));
        image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image()
    }
}

impl Cutter {
    pub fn load(model: &Path, device: Device) -> Result<Self> {
        Ok(Self { session: onnx::lean_session(model, device)? })
    }

    pub fn cut(&mut self, rgb: &[u8], width: usize, height: usize) -> Result<Cutout> {
        let img = image::RgbImage::from_raw(width as u32, height as u32, rgb.to_vec()).expect("image size");
        let resized = image::imageops::resize(&img, SIDE as u32, SIDE as u32, image::imageops::FilterType::Triangle);
        let plane = SIDE * SIDE;
        let mut data = vec![0f32; 3 * plane];
        for (i, px) in resized.pixels().enumerate() {
            for c in 0..3 {
                data[c * plane + i] = (px[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
        let input = Array::new([1, 3, SIDE, SIDE], data);
        let logits = onnx::run(&mut self.session, vec![("input_image", input.value()?)], &["output_image"])?.remove(0);
        let full = resize_bilinear(&logits.data, SIDE, SIDE, width, height);
        let alpha: Vec<u8> = full.iter().map(|&l| (255.0 / (1.0 + (-l).exp())).round() as u8).collect();
        let mut on_grey = Vec::with_capacity(rgb.len());
        for (px, &a) in rgb.chunks_exact(3).zip(&alpha) {
            let a = a as f32 / 255.0;
            on_grey.extend(px.iter().map(|&v| (v as f32 * a + GREY as f32 * (1.0 - a)).round() as u8));
        }
        Ok(Cutout { alpha, on_grey })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crop_keeps_the_solid_part_with_a_margin() {
        let (w, h) = (100u32, 80u32);
        let alpha: Vec<u8> = (0..w * h).map(|i| if (40..60).contains(&(i % w)) && (20..70).contains(&(i / w)) { 255 } else { 0 }).collect();
        let cut = Cutout { alpha, on_grey: vec![GREY; (w * h * 3) as usize] };
        assert_eq!(cut.crop(w, h).dimensions(), (20 + 2 * 1, 50 + 2 * 3));
        let empty = Cutout { alpha: vec![0; (w * h) as usize], on_grey: vec![GREY; (w * h * 3) as usize] };
        assert_eq!(empty.crop(w, h).dimensions(), (w, h));
    }
}
