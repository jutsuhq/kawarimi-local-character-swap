//! Wan 2.2 Animate's graph in ComfyUI's API format, filled in for one render.
//!
//! `data/wan-animate.api.json` is Swapjutsu's `wan-animate-character-swap.api.json` (MIT): Comfy's "Wan2.2 Animate"
//! template in replacement mode, with prep's `mask.mp4` for the person. Kawarimi swaps its loaders for the GGUF ones.
//! It holds the first window (nodes 101–105) and one extend window (201–206); [`make`] drops the extend for a short
//! clip and repeats it, renumbered, for a long one.

use anyhow::{Context, Result};
use serde_json::{Value, json};

const TEMPLATE: &str = include_str!("../data/wan-animate.api.json");

/// The frame rate Wan 2.2 Animate generates at; prep's videos are already at it.
pub const FPS: u32 = 16;
/// Frames per window (`WanAnimateToVideo.length`, 4k + 1).
pub const WINDOW: u32 = 77;
/// The window for cards under [`LARGE_CARD`]: on 8 GB it puts 2.7 GB of Wan on the GPU, where 77 frames leave none
/// (docs/RENDER.md).
pub const SHORT_WINDOW: u32 = 33;
/// VRAM from which a card gets 77-frame windows and 720p finals; unmeasured, 8 GB is the only card measured.
pub const LARGE_CARD: u64 = 11 << 30;
/// Frames each extend window repeats from the one before (`continue_motion_max_frames`).
pub const OVERLAP: u32 = 5;
/// The node whose output is the video.
pub const SAVE_NODE: &str = "901";
pub const DEFAULT_PROMPT: &str = "The character moves naturally in the scene.";

#[derive(Debug, Clone, PartialEq)]
pub struct Inputs {
    /// Names the files were uploaded under (ComfyUI's input folder).
    pub source: String,
    pub mask: String,
    pub character: String,
    /// The uploaded anchor first frame ([`crate::anchor`]), if any: the first window starts from it.
    pub anchor: Option<String>,
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    /// Frames of the 16 fps source; the output is cut to exactly this many.
    pub frames: u32,
    pub seed: u64,
    /// Sampler steps; the LightX2V LoRA is made for 4–6 at CFG 1.
    pub steps: u32,
    /// Frames per window, 4k + 1 and more than [`OVERLAP`].
    pub window: u32,
    pub diffusion_model: String,
    pub text_encoder: String,
    /// Where in ComfyUI's output folder the video is saved (`SaveVideo.filename_prefix`), e.g. `kawarimi/<job>/render`.
    pub prefix: String,
}

/// The render size: the short side at `short_side`, the long side keeping the aspect ratio, both rounded to
/// multiples of 16 as `WanAnimateToVideo` requires.
pub fn size(width: u32, height: u32, short_side: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (16, 16);
    }
    let scale = short_side as f64 / width.min(height) as f64;
    let snap = |side: u32| (((side as f64 * scale) / 16.0).round() as u32 * 16).max(16);
    (snap(width), snap(height))
}

/// The character fitted whole into the render frame on grey, the cutout's background. `WanAnimateToVideo` scales the
/// reference to the frame with a centre crop, which would cut a portrait character down to its middle.
pub fn reference(character: &image::DynamicImage, width: u32, height: u32) -> image::RgbImage {
    const GREY: u8 = 128;
    let fitted = character.resize(width, height, image::imageops::FilterType::Lanczos3).to_rgb8();
    let mut canvas = image::RgbImage::from_pixel(width, height, image::Rgb([GREY; 3]));
    let (x, y) = ((width - fitted.width()) / 2, (height - fitted.height()) / 2);
    image::imageops::replace(&mut canvas, &fitted, x as i64, y as i64);
    canvas
}

/// Each window's length: the first up to `window` frames, each extend repeating the 5 before it and adding up to
/// `window - 5`, the last only as long as what is left needs. Every length is 4k + 1.
pub fn windows(frames: u32, window: u32) -> Vec<u32> {
    let grid = |n: u32| (n.max(1) + 2) / 4 * 4 + 1;
    let mut lengths = vec![window.min(grid(frames))];
    let mut covered = lengths[0];
    while covered < frames {
        let length = window.min(grid(frames - covered + OVERLAP));
        lengths.push(length);
        covered += length - OVERLAP;
    }
    lengths
}

/// How a render is asked for: a draft to check the motion, then the final with the same seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    Draft,
    Final,
}

/// Render settings for a quality on a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub short_side: u32,
    pub steps: u32,
    pub window: u32,
    /// At most this many frames from the start; `None` is the whole clip.
    pub frames: Option<u32>,
}

impl Preset {
    /// The draft is the first window at 480p. Below [`LARGE_CARD`] the final stays at 480p with the same windows, so
    /// with the same seed its first window is the draft's frames; from it the final is 720p (unmeasured).
    pub fn new(quality: Quality, vram: u64) -> Self {
        let large = vram >= LARGE_CARD;
        let window = if large { WINDOW } else { SHORT_WINDOW };
        match quality {
            Quality::Draft => Self { short_side: 480, steps: 4, window, frames: Some(window) },
            Quality::Final => Self { short_side: if large { 720 } else { 480 }, steps: 4, window, frames: None },
        }
    }
}

pub fn make(inputs: &Inputs) -> Result<Value> {
    anyhow::ensure!(
        inputs.window % 4 == 1 && inputs.window > OVERLAP,
        "a window of {} frames is not 4k + 1 above {OVERLAP}",
        inputs.window
    );
    let mut graph: serde_json::Map<String, Value> = serde_json::from_str(TEMPLATE)?;
    let set = |graph: &mut serde_json::Map<String, Value>, node: &str, input: &str, value: Value| -> Result<()> {
        let inputs = graph.get_mut(node).and_then(|n| n.get_mut("inputs")).and_then(Value::as_object_mut);
        inputs.with_context(|| format!("the Wan Animate template has no node {node}"))?.insert(input.into(), value);
        Ok(())
    };

    set(&mut graph, "1", "unet_name", json!(inputs.diffusion_model))?;
    set(&mut graph, "4", "clip_name", json!(inputs.text_encoder))?;
    let prompt = inputs.prompt.trim();
    set(&mut graph, "5", "text", json!(if prompt.is_empty() { DEFAULT_PROMPT } else { prompt }))?;
    set(&mut graph, "9", "image", json!(inputs.character))?;
    set(&mut graph, "11", "file", json!(inputs.source))?;
    set(&mut graph, "16", "file", json!(inputs.mask))?;
    for node in ["13", "18", "20", "23"] {
        set(&mut graph, node, "width", json!(inputs.width))?;
        set(&mut graph, node, "height", json!(inputs.height))?;
    }
    // What the template's Pixel Perfect Resolution works out for frames already at the render size.
    for node in ["14", "15"] {
        set(&mut graph, node, "resolution", json!(inputs.width.min(inputs.height)))?;
    }

    let extend: Vec<(u32, Value)> =
        (201..=206).map(|id| graph.remove(&id.to_string()).map(|n| (id - 200, n))).collect::<Option<_>>()
            .context("the Wan Animate template has no extend window")?;
    // The frames so far, and the window whose `video_frame_offset` the next one continues from.
    let (mut images, mut previous) = (json!(["105", 0]), "101".to_string());
    if let Some(anchor) = &inputs.anchor {
        // As kept motion its latent stays fixed; the window drops its own frame 0 (`trim_image`), so the anchor is
        // put back in front.
        graph.insert("30".into(), json!({ "class_type": "LoadImage", "inputs": { "image": anchor } }));
        set(&mut graph, "101", "continue_motion", json!(["30", 0]))?;
        set(&mut graph, "101", "continue_motion_max_frames", json!(1))?;
        graph.insert("106".into(), json!({ "class_type": "ImageBatch", "inputs": { "image1": ["30", 0], "image2": ["105", 0] } }));
        images = json!(["106", 0]);
    }
    for (index, &length) in windows(inputs.frames, inputs.window).iter().enumerate() {
        let base = (index as u32 + 1) * 100;
        if index > 0 {
            for (offset, node) in &extend {
                graph.insert((base + offset).to_string(), renumber(node, base, &images, &previous));
            }
            images = json!([(base + 6).to_string(), 0]);
        }
        let wan = (base + 1).to_string();
        set(&mut graph, &wan, "width", json!(inputs.width))?;
        set(&mut graph, &wan, "height", json!(inputs.height))?;
        set(&mut graph, &wan, "length", json!(length))?;
        let sampler = (base + 2).to_string();
        set(&mut graph, &sampler, "seed", json!(inputs.seed))?;
        set(&mut graph, &sampler, "steps", json!(inputs.steps))?;
        previous = wan;
    }
    set(&mut graph, "899", "image", images)?;
    set(&mut graph, "899", "length", json!(inputs.frames.max(1)))?;
    set(&mut graph, SAVE_NODE, "filename_prefix", json!(inputs.prefix))?;
    Ok(Value::Object(graph))
}

/// An extend node at window `base`: links inside the window renumbered, links to the first window's frames (`105`)
/// and offset (`101`) pointed at the window before.
fn renumber(node: &Value, base: u32, images: &Value, previous: &str) -> Value {
    let mut node = node.clone();
    let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) else { return node };
    for value in inputs.values_mut() {
        let Some([target, slot]) = value.as_array().and_then(|a| <&[Value; 2]>::try_from(a.as_slice()).ok()) else {
            continue;
        };
        let (Some(target), slot) = (target.as_str(), slot.clone()) else { continue };
        *value = match target {
            "105" => images.clone(),
            "101" => json!([previous, slot]),
            id if id.len() == 3 && id.starts_with("20") => json!([(base + id[2..].parse::<u32>().unwrap_or(0)).to_string(), slot]),
            _ => continue,
        };
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(frames: u32) -> Inputs {
        Inputs {
            source: "s.mp4".into(),
            mask: "m.mp4".into(),
            character: "c.png".into(),
            anchor: None,
            prompt: String::new(),
            width: 832,
            height: 480,
            frames,
            seed: 7,
            steps: 4,
            window: WINDOW,
            diffusion_model: "wan.gguf".into(),
            text_encoder: "umt5.gguf".into(),
            prefix: "kawarimi/job/render".into(),
        }
    }

    #[test]
    fn windows_cover_the_clip_on_the_4k_plus_1_grid() {
        assert_eq!(windows(33, WINDOW), vec![33]);
        assert_eq!(windows(77, WINDOW), vec![77]);
        assert_eq!(windows(80, WINDOW), vec![77, 9]);
        assert_eq!(windows(150, WINDOW), vec![77, 77, 9]);
        assert_eq!(windows(77, SHORT_WINDOW), vec![33, 33, 21]);
        for window in [SHORT_WINDOW, WINDOW] {
            for frames in 1..400 {
                let w = windows(frames, window);
                assert!(w.iter().all(|l| l % 4 == 1 && *l <= window));
                let covered = w[0] + w[1..].iter().map(|l| l - OVERLAP).sum::<u32>();
                assert!(covered >= frames, "{frames}: {w:?}");
            }
        }
    }

    #[test]
    fn presets_follow_the_card() {
        let small = 8 << 30;
        let draft = Preset::new(Quality::Draft, small);
        assert_eq!(draft, Preset { short_side: 480, steps: 4, window: SHORT_WINDOW, frames: Some(SHORT_WINDOW) });
        assert_eq!(Preset::new(Quality::Final, small), Preset { frames: None, ..draft });
        let large = Preset::new(Quality::Final, 16 << 30);
        assert_eq!((large.short_side, large.window, large.frames), (720, WINDOW, None));
        assert_eq!(Preset::new(Quality::Draft, 16 << 30).short_side, 480);
    }

    #[test]
    fn a_short_window_chains_more_windows() {
        let graph = make(&Inputs { window: SHORT_WINDOW, ..inputs(77) }).unwrap();
        assert_eq!(graph["101"]["inputs"]["length"], 33);
        assert_eq!(graph["301"]["inputs"]["length"], 21);
        assert!(graph.get("401").is_none());
        assert_eq!(graph["899"]["inputs"]["image"], json!(["306", 0]));
        assert!(make(&Inputs { window: 32, ..inputs(77) }).is_err());
    }

    #[test]
    fn the_reference_keeps_the_whole_character() {
        let tall = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(160, 380, image::Rgb([255, 0, 0])));
        let frame = reference(&tall, 848, 480);
        assert_eq!(frame.dimensions(), (848, 480));
        assert_eq!(frame.get_pixel(424, 2).0, [255, 0, 0]);
        assert_eq!(frame.get_pixel(424, 477).0, [255, 0, 0]);
        assert_eq!(frame.get_pixel(5, 240).0, [128; 3]);
    }

    #[test]
    fn size_keeps_the_aspect_on_multiples_of_16() {
        assert_eq!(size(1920, 1080, 480), (848, 480));
        assert_eq!(size(540, 960, 480), (480, 848));
    }

    #[test]
    fn a_short_clip_has_one_window() {
        let graph = make(&inputs(40)).unwrap();
        assert!(graph.get("201").is_none());
        assert_eq!(graph["101"]["inputs"]["length"], 41);
        assert_eq!(graph["899"]["inputs"]["image"], json!(["105", 0]));
        assert_eq!(graph["1"]["inputs"]["unet_name"], "wan.gguf");
        assert_eq!(graph["5"]["inputs"]["text"], DEFAULT_PROMPT);
        assert_eq!(graph[SAVE_NODE]["inputs"]["filename_prefix"], "kawarimi/job/render");
    }

    #[test]
    fn an_anchor_starts_the_first_window() {
        let graph = make(&Inputs { anchor: Some("a.png".into()), window: SHORT_WINDOW, ..inputs(60) }).unwrap();
        assert_eq!(graph["30"]["inputs"]["image"], "a.png");
        assert_eq!(graph["101"]["inputs"]["continue_motion"], json!(["30", 0]));
        assert_eq!(graph["101"]["inputs"]["continue_motion_max_frames"], 1);
        assert_eq!(graph["106"]["inputs"], json!({ "image1": ["30", 0], "image2": ["105", 0] }));
        assert_eq!(graph["201"]["inputs"]["continue_motion"], json!(["106", 0]));
        assert_eq!(graph["206"]["inputs"]["image1"], json!(["106", 0]));
        assert!(make(&inputs(60)).unwrap().get("30").is_none());
    }

    #[test]
    fn a_long_clip_chains_extend_windows() {
        let graph = make(&inputs(150)).unwrap();
        assert_eq!(graph["201"]["inputs"]["continue_motion"], json!(["105", 0]));
        assert_eq!(graph["301"]["inputs"]["continue_motion"], json!(["206", 0]));
        assert_eq!(graph["301"]["inputs"]["video_frame_offset"], json!(["201", 5]));
        assert_eq!(graph["306"]["inputs"]["image1"], json!(["206", 0]));
        assert_eq!(graph["302"]["inputs"]["positive"], json!(["301", 0]));
        assert_eq!(graph["899"]["inputs"]["image"], json!(["306", 0]));
        assert_eq!(graph["302"]["inputs"]["steps"], 4);
    }
}
