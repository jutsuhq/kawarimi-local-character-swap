//! The prompt writer: the character image, then a few keyframes of the clip, each asked about once, and Wan's
//! prompt assembled from the answers.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, RgbImage};
use kawarimi_media::Ffmpeg;

use crate::client::Endpoint;
use crate::prompt::{self, Description};

const KEYFRAMES: usize = 4;
const FRAME_SIDE: u32 = 512;
const CHARACTER_SIDE: u32 = 768;
const MAX_TOKENS: u32 = 700;
const TIMEOUT: Duration = Duration::from_secs(180);

/// Describes `character` (an image) and the moves in `video`, and writes Wan's prompt. `log` gets a line per step.
pub fn describe(endpoint: &Endpoint, ffmpeg: &Ffmpeg, video: &Path, character: &Path, log: impl Fn(&str)) -> Result<Description> {
    log("Reading the character");
    let reference = jpeg(&fit(&image::open(character).with_context(|| format!("reading {}", character.display()))?, CHARACTER_SIDE))?;
    let answer = endpoint.ask(prompt::CHARACTER, &[reference], MAX_TOKENS, TIMEOUT)?;
    let fields = prompt::fields(&answer).with_context(|| format!("the model gave no JSON for the character: {}", short(&answer)))?;
    let character = prompt::character(&fields);

    log("Reading the moves");
    let (times, seconds, frames) = keyframes(ffmpeg, video)?;
    let answer = endpoint.ask(&prompt::moves(&times, seconds, &character.wardrobe), &frames, MAX_TOKENS, TIMEOUT)?;
    // Without the moves the prompt still has the character.
    let motion = prompt::fields(&answer).map(|f| prompt::motion(&f));
    if motion.is_none() {
        log(&format!("The model gave no JSON for the moves: {}", short(&answer)));
    }
    let prompt = prompt::assemble(&character, motion.as_ref());
    Ok(Description { character, motion, prompt, source: endpoint.base().to_owned() })
}

/// `KEYFRAMES` frames spread over the clip, as JPEG, with their times and the clip's length.
fn keyframes(ffmpeg: &Ffmpeg, video: &Path) -> Result<(Vec<f64>, f64, Vec<Vec<u8>>)> {
    let probe = ffmpeg.probe(video)?;
    let scale = FRAME_SIDE as f64 / probe.width.max(probe.height) as f64;
    let even = |side: u32| ((side as f64 * scale.min(1.0)) as u32 / 2 * 2).max(2);
    let (width, height) = (even(probe.width), even(probe.height));
    let fps = KEYFRAMES as f64 / probe.duration.max(0.1);
    let frames = ffmpeg.frames(video, width, height, Some(fps))?.take(KEYFRAMES).collect::<Result<Vec<_>>>()?;
    let times = (0..frames.len()).map(|i| i as f64 / fps).collect();
    let jpegs = frames
        .into_iter()
        .map(|rgb| jpeg(&DynamicImage::ImageRgb8(RgbImage::from_raw(width, height, rgb).context("short frame")?)))
        .collect::<Result<_>>()?;
    Ok((times, probe.duration, jpegs))
}

/// The image at most `side` on its long side, transparency over white (a cut-out character).
fn fit(image: &DynamicImage, side: u32) -> DynamicImage {
    let (w, h) = image.dimensions();
    let image = if w.max(h) > side { image.resize(side, side, image::imageops::FilterType::Triangle) } else { image.clone() };
    let mut rgb = RgbImage::new(image.width(), image.height());
    for (x, y, pixel) in image.to_rgba8().enumerate_pixels() {
        let [r, g, b, a] = pixel.0;
        let over = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8;
        rgb.put_pixel(x, y, image::Rgb([over(r), over(g), over(b)]));
    }
    DynamicImage::ImageRgb8(rgb)
}

fn jpeg(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&image.to_rgb8())?;
    Ok(out)
}

fn short(answer: &str) -> String {
    answer.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_out_character_is_put_over_white() {
        let mut rgba = image::RgbaImage::new(1000, 500);
        rgba.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        let fitted = fit(&DynamicImage::ImageRgba8(rgba), 768);
        assert_eq!(fitted.dimensions(), (768, 384));
        assert_eq!(fitted.to_rgb8().get_pixel(767, 383).0, [255, 255, 255]);
        assert!(jpeg(&fitted).unwrap().starts_with(&[0xff, 0xd8]));
    }
}
