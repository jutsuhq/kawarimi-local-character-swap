//! An anchor first frame: Qwen-Image-Edit puts the character, outfit included, into the clip's frame 0, and Wan's
//! first window starts from it ([`crate::wan::Inputs::anchor`]). On 8 GB it raised likeness from 0.441 to 0.530 and
//! kept the face and hair, at 26 minutes for the edit (docs/RENDER.md). Imagination Studio's Character Swap
//! (Apache-2.0, `spark/spark_swap.py`) does the same, with the prompt and the no-op check used here.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use image::{GrayImage, RgbImage};
use kawarimi_assets::{Status, Store, manifest};
use kawarimi_media::Ffmpeg;
use serde_json::{Value, json};

use crate::wan::Inputs;
use crate::{Client, Extras, PromptState};

const TEMPLATE: &str = include_str!("../data/anchor-edit.api.json");
/// The node whose output is the anchor.
pub const SAVE_NODE: &str = "18";
/// Mean grey-level change inside the person's mask below which the edit returned the frame unchanged.
pub const MIN_CHANGE: f32 = 6.0;
/// The pixel count Qwen-Image-Edit encodes its reference images at (`TextEncodeQwenImageEditPlus`).
const EDIT_PIXELS: f64 = 1024.0 * 1024.0;

/// Points ComfyUI at the edit model's folder, if it is installed.
pub fn extras(extras: &mut Extras, store: &Store) {
    if store.status(&manifest::QWEN_EDIT) == Status::Installed {
        let dir = store.dir(&manifest::QWEN_EDIT);
        extras.folders.extend(["diffusion_models", "text_encoders", "vae", "loras"].map(|kind| (kind, dir.clone())));
    }
}

/// The edit graph: `frame` (uploaded, `width`×`height`) edited at ~1 MP with `character` in it, saved under `prefix`
/// at the frame's size.
pub fn graph(frame: &str, character: &str, width: u32, height: u32, seed: u64, prefix: &str) -> Result<Value> {
    let mut graph: Value = serde_json::from_str(TEMPLATE)?;
    let scale = (EDIT_PIXELS / (width * height).max(1) as f64).sqrt();
    let snap = |side: u32| ((side as f64 * scale / 8.0).round() as u32 * 8).max(8);
    for (node, input, value) in [
        ("7", "image", json!(frame)),
        ("8", "image", json!(character)),
        ("9", "width", json!(snap(width))),
        ("9", "height", json!(snap(height))),
        ("15", "seed", json!(seed)),
        ("17", "width", json!(width)),
        ("17", "height", json!(height)),
        ("18", "filename_prefix", json!(prefix)),
    ] {
        *graph.pointer_mut(&format!("/{node}/inputs/{input}")).with_context(|| format!("the edit template has no {node}.{input}"))? = value;
    }
    Ok(graph)
}

/// Mean absolute grey-level change between `frame` and `anchor` where `mask` is set; `None` without a person.
pub fn change(frame: &RgbImage, anchor: &RgbImage, mask: &GrayImage) -> Option<f32> {
    let grey = |p: &image::Rgb<u8>| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
    let (sum, count) = frame
        .pixels()
        .zip(anchor.pixels())
        .zip(mask.pixels())
        .filter(|(_, m)| m[0] > 127)
        .fold((0.0, 0u32), |(sum, count), ((a, b), _)| (sum + (grey(a) - grey(b)).abs(), count + 1));
    (count > 0).then(|| sum / count as f32)
}

/// Makes the anchor for the render `inputs` of a prepared clip and uploads it; its name goes into `inputs.anchor`.
/// `reference` is the character as Wan gets it (`wan::reference`). Every file goes into `dir`; ComfyUI's models are
/// freed after, for Wan. Fails when the edit fails or changed nothing; the render then goes on without an anchor.
pub fn make(client: &Client, ffmpeg: &Ffmpeg, prep: &Path, reference: &Path, dir: &Path, inputs: &Inputs) -> Result<String> {
    let (width, height, prefix) = (inputs.width, inputs.height, format!("{}-anchor", inputs.prefix));
    let first = |video: &Path| -> Result<Vec<u8>> {
        ffmpeg.frames(video, width, height, None)?.next().context("the video has no frames")?
    };
    let frame = RgbImage::from_raw(width, height, first(&prep.join("normalized.mp4"))?).context("short frame")?;
    let mask = RgbImage::from_raw(width, height, first(&prep.join("mask.mp4"))?).context("short mask frame")?;
    let frame_file = dir.join("anchor-frame.png");
    frame.save(&frame_file)?;
    let upload = |file: &Path, what: &str| client.upload(file, &format!("{}-{what}", prefix.replace('/', "-")));
    let graph = graph(&upload(&frame_file, "frame.png")?, &upload(reference, "character.png")?, width, height, inputs.seed, &prefix)?;
    let edited = run(client, &graph, &dir.join("anchor.png"));
    client.free()?;
    let edited = edited?;
    let anchor = image::open(&edited)?.to_rgb8();
    anyhow::ensure!(anchor.dimensions() == (width, height), "the anchor is {:?}, not {width}×{height}", anchor.dimensions());
    let change = change(&frame, &anchor, &image::DynamicImage::ImageRgb8(mask).to_luma8());
    match change {
        Some(change) if change >= MIN_CHANGE => upload(&edited, "anchor.png"),
        Some(change) => bail!("the edit left the person as it was (grey-level change {change:.1})"),
        None => bail!("frame 0 has no person"),
    }
}

fn run(client: &Client, graph: &Value, dest: &Path) -> Result<std::path::PathBuf> {
    let prompt = client.queue(graph)?;
    loop {
        match client.state(&prompt)? {
            PromptState::Pending | PromptState::Running => std::thread::sleep(Duration::from_secs(1)),
            PromptState::Done(outputs) => {
                let image = outputs.into_iter().find(|o| o.node == SAVE_NODE).context("the edit saved no image")?;
                return client.download(&image, dest);
            }
            PromptState::Failed(error) => bail!("the edit failed: {error}"),
            PromptState::Unknown => bail!("ComfyUI lost the edit"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_edit_graph_is_filled_in() {
        let graph = graph("f.png", "c.png", 848, 480, 7, "kawarimi/job/anchor").unwrap();
        assert_eq!(graph["7"]["inputs"]["image"], "f.png");
        assert_eq!(graph["8"]["inputs"]["image"], "c.png");
        assert_eq!((graph["9"]["inputs"]["width"].clone(), graph["9"]["inputs"]["height"].clone()), (json!(1360), json!(768)));
        assert_eq!((graph["17"]["inputs"]["width"].clone(), graph["17"]["inputs"]["height"].clone()), (json!(848), json!(480)));
        assert_eq!(graph["15"]["inputs"]["seed"], 7);
        assert_eq!(graph[SAVE_NODE]["inputs"]["filename_prefix"], "kawarimi/job/anchor");
        assert_eq!(graph["10"]["inputs"]["image2"], json!(["8", 0]));
    }

    #[test]
    fn change_counts_only_the_person() {
        let frame = RgbImage::from_pixel(4, 2, image::Rgb([100; 3]));
        let mut anchor = frame.clone();
        anchor.put_pixel(0, 0, image::Rgb([120; 3]));
        anchor.put_pixel(3, 1, image::Rgb([0; 3]));
        let mut mask = GrayImage::new(4, 2);
        mask.put_pixel(0, 0, image::Luma([255]));
        mask.put_pixel(1, 0, image::Luma([255]));
        assert!((change(&frame, &anchor, &mask).unwrap() - 10.0).abs() < 1e-3);
        assert_eq!(change(&frame, &anchor, &GrayImage::new(4, 2)), None);
    }
}
