//! What a render asks for, and the Wan graph it makes for a prepared clip on a card.

use std::path::Path;

use anyhow::{Context, Result};
use kawarimi_assets::manifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::variant::Variant;
use crate::wan::{self, Quality};

/// The render settings a person or the CLI picks; the preset for the card fills what is left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub quality: Quality,
    pub seed: u64,
    #[serde(default)]
    pub prompt: String,
    /// A Wan variant's asset id; by default the one for the card.
    pub model: Option<String>,
    pub frames: Option<u32>,
    pub steps: Option<u32>,
    pub short_side: Option<u32>,
    pub window: Option<u32>,
}

/// What a render needs from prep's `manifest.json`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
}

impl Clip {
    pub fn read(prep: &Path) -> Result<Self> {
        let path = prep.join("manifest.json");
        let manifest: Value = serde_json::from_slice(&std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?)?;
        let number = |value: &Value, what: &str| value.as_u64().map(|n| n as u32).with_context(|| format!("manifest has no {what}"));
        Ok(Self {
            width: number(&manifest["video"]["width"], "width")?,
            height: number(&manifest["video"]["height"], "height")?,
            frames: number(&manifest["frames"], "frames")?,
        })
    }
}

/// Names the inputs were uploaded under.
pub struct Uploads {
    pub source: String,
    pub mask: String,
    pub character: String,
}

impl Settings {
    pub fn variant(&self, vram: u64) -> Result<&'static Variant> {
        match &self.model {
            Some(id) => Variant::find(id).with_context(|| format!("unknown Wan model {id}")),
            None => Ok(Variant::for_vram(Some(vram >> 20))),
        }
    }

    /// The graph's inputs for `clip` on a card with `vram` bytes, the preset filling what the settings leave out.
    pub fn inputs(&self, clip: &Clip, vram: u64, model: &Variant, uploads: Uploads, prefix: &str) -> wan::Inputs {
        let preset = wan::Preset::new(self.quality, vram);
        let (width, height) = wan::size(clip.width, clip.height, self.short_side.unwrap_or(preset.short_side));
        wan::Inputs {
            source: uploads.source,
            mask: uploads.mask,
            character: uploads.character,
            anchor: None,
            prompt: self.prompt.clone(),
            width,
            height,
            frames: self.frames.or(preset.frames).unwrap_or(clip.frames).min(clip.frames),
            seed: self.seed,
            steps: self.steps.unwrap_or(preset.steps),
            window: self.window.unwrap_or(preset.window),
            diffusion_model: model.file().into(),
            text_encoder: manifest::UMT5_Q4.files[0].path.into(),
            prefix: prefix.into(),
        }
    }
}

/// What a render will take, before it starts: its size, windows, time and memory.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    /// Each window's length.
    pub windows: Vec<u32>,
    pub steps: u32,
    pub model: &'static str,
    pub seconds: u32,
    /// Peak video memory, GB.
    pub vram_gb: f32,
}

impl Settings {
    /// Measured on an 8 GB card with 33-frame windows (docs/RENDER.md): 21 s per step, ~4 min of fixed costs, ~2 s
    /// per frame of pose and decoding, peak 6.8 GB. Bigger cards hold Wan on the GPU; their rate is a guess.
    pub fn estimate(&self, clip: &Clip, vram: u64) -> Result<Estimate> {
        let model = self.variant(vram)?;
        let uploads = Uploads { source: String::new(), mask: String::new(), character: String::new() };
        let inputs = self.inputs(clip, vram, model, uploads, "");
        let windows = wan::windows(inputs.frames, inputs.window);
        let per_step_frame = if vram >= wan::LARGE_CARD { 0.4 } else { 0.64 };
        let sampling: f64 = windows.iter().map(|&w| w as f64 * per_step_frame * inputs.steps as f64).sum();
        let pixels = (inputs.width * inputs.height) as f64 / (848.0 * 480.0);
        let seconds = 240.0 + sampling * pixels + 2.0 * inputs.frames as f64 * pixels;
        let card = vram as f32 / (1u64 << 30) as f32;
        let activations = 2.0 * inputs.window as f32 / wan::SHORT_WINDOW as f32 * pixels as f32;
        let weights = model.asset.download_size() as f32 / (1u64 << 30) as f32;
        Ok(Estimate {
            width: inputs.width,
            height: inputs.height,
            frames: inputs.frames,
            windows,
            steps: inputs.steps,
            model: model.asset.title,
            seconds: seconds.round() as u32,
            vram_gb: (weights + activations).min(card - 1.2).max(1.0),
        })
    }
}

/// The graph for these inputs, loading `model`.
pub fn graph(inputs: &wan::Inputs, model: &Variant) -> Result<Value> {
    let mut graph = wan::make(inputs)?;
    model.apply(&mut graph)?;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preset_fills_what_is_left_out() {
        let clip = Clip { width: 1920, height: 1080, frames: 80 };
        let uploads = || Uploads { source: "s".into(), mask: "m".into(), character: "c".into() };
        let draft = Settings { quality: Quality::Draft, seed: 3, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
        let model = draft.variant(8 << 30).unwrap();
        let inputs = draft.inputs(&clip, 8 << 30, model, uploads(), "kawarimi/j/render");
        assert_eq!((inputs.width, inputs.height, inputs.frames, inputs.window, inputs.seed), (848, 480, wan::SHORT_WINDOW, wan::SHORT_WINDOW, 3));
        let longer = Settings { frames: Some(500), steps: Some(6), ..draft.clone() };
        let inputs = longer.inputs(&clip, 8 << 30, model, uploads(), "p");
        assert_eq!((inputs.frames, inputs.steps), (80, 6));
        assert!(Settings { model: Some("nope".into()), ..draft }.variant(0).is_err());
    }

    #[test]
    fn estimates_the_measured_draft() {
        let clip = Clip { width: 960, height: 540, frames: 160 };
        let draft = Settings { quality: Quality::Draft, seed: 0, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
        let e = draft.estimate(&clip, 8 << 30).unwrap();
        assert_eq!((e.width, e.height, e.frames, e.windows.clone()), (848, 480, 33, vec![33]));
        // The measured run: 6:30 for the graph, 6.8 GB peak.
        assert!((360..420).contains(&e.seconds), "{e:?}");
        assert!((6.5..=6.8).contains(&e.vram_gb), "{e:?}");
        let full = Settings { quality: Quality::Final, ..draft }.estimate(&clip, 8 << 30).unwrap();
        assert_eq!(full.windows, wan::windows(160, wan::SHORT_WINDOW));
        assert!(full.seconds > e.seconds);
    }
}
