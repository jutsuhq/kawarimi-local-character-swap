//! Which Wan 2.2 Animate weights a card renders with, and the loader node that reads them (docs/RENDER.md,
//! "Choice per card").

use anyhow::{Context, Result};
use kawarimi_assets::{Asset, manifest};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct Variant {
    pub asset: &'static Asset,
    /// The ComfyUI node class that loads the weights.
    pub loader: &'static str,
    /// Cards with at least this many GB, as `nvidia-smi` reports them rounded, take this variant.
    min_gb: u64,
}

/// Biggest first.
pub const VARIANTS: &[Variant] = &[
    Variant { asset: &manifest::WAN_ANIMATE_FP8, loader: "UNETLoader", min_gb: 16 },
    Variant { asset: &manifest::WAN_ANIMATE_Q4, loader: "UnetLoaderGGUF", min_gb: 12 },
    Variant { asset: &manifest::WAN_ANIMATE_Q3, loader: "UnetLoaderGGUF", min_gb: 0 },
];

/// The node in `wan-animate.api.json` that loads Wan.
const LOADER_NODE: &str = "1";

impl Variant {
    /// The variant for a card with `vram_mb` of memory; the smallest when it is unknown.
    pub fn for_vram(vram_mb: Option<u64>) -> &'static Self {
        let gb = vram_mb.map_or(0, |mb| (mb + 512) / 1024);
        VARIANTS.iter().find(|v| gb >= v.min_gb).unwrap_or(&VARIANTS[VARIANTS.len() - 1])
    }

    /// The variant for the first NVIDIA GPU.
    pub fn for_this_gpu() -> &'static Self {
        Self::for_vram(crate::nvidia::gpus().first().map(|g| g.memory_mb))
    }

    pub fn find(id: &str) -> Option<&'static Self> {
        VARIANTS.iter().find(|v| v.asset.id == id)
    }

    /// The weights' file name in the diffusion_models folder.
    pub fn file(&self) -> &'static str {
        self.asset.files[0].path
    }

    /// Points the graph's loader at these weights, with the node class that reads them.
    pub fn apply(&self, graph: &mut Value) -> Result<()> {
        let node = graph.get_mut(LOADER_NODE).context("the Wan Animate graph has no loader node")?;
        let inputs = match self.loader {
            "UNETLoader" => json!({ "unet_name": self.file(), "weight_dtype": "default" }),
            _ => json!({ "unet_name": self.file() }),
        };
        *node = json!({ "class_type": self.loader, "inputs": inputs });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wan;

    #[test]
    fn vram_picks_the_variant() {
        let id = |mb| Variant::for_vram(mb).asset.id;
        assert_eq!(id(None), manifest::WAN_ANIMATE_Q3.id);
        assert_eq!(id(Some(8192)), manifest::WAN_ANIMATE_Q3.id);
        assert_eq!(id(Some(11264)), manifest::WAN_ANIMATE_Q3.id);
        assert_eq!(id(Some(12282)), manifest::WAN_ANIMATE_Q4.id);
        assert_eq!(id(Some(16376)), manifest::WAN_ANIMATE_FP8.id);
        assert_eq!(id(Some(24564)), manifest::WAN_ANIMATE_FP8.id);
    }

    fn graph() -> Value {
        wan::make(&wan::Inputs {
            source: "s.mp4".into(),
            mask: "m.mp4".into(),
            character: "c.png".into(),
            prompt: String::new(),
            width: 832,
            height: 480,
            frames: 40,
            seed: 0,
            steps: 4,
            window: wan::WINDOW,
            diffusion_model: "ignored".into(),
            text_encoder: "umt5.gguf".into(),
            prefix: "kawarimi/wan".into(),
        })
        .unwrap()
    }

    #[test]
    fn gguf_weights_load_with_the_gguf_loader() {
        let mut graph = graph();
        Variant::find(manifest::WAN_ANIMATE_Q4.id).unwrap().apply(&mut graph).unwrap();
        assert_eq!(graph["1"], json!({ "class_type": "UnetLoaderGGUF", "inputs": { "unet_name": "Wan2.2-Animate-14B-Q4_K_M.gguf" } }));
        assert_eq!(graph["2"]["inputs"]["model"], json!(["1", 0]));
    }

    #[test]
    fn fp8_weights_load_with_comfys_loader() {
        let mut graph = graph();
        Variant::find(manifest::WAN_ANIMATE_FP8.id).unwrap().apply(&mut graph).unwrap();
        assert_eq!(graph["1"]["class_type"], "UNETLoader");
        assert_eq!(graph["1"]["inputs"]["unet_name"], "Wan2_2-Animate-14B_fp8_scaled_e4m3fn_KJ_v2.safetensors");
        assert_eq!(graph["1"]["inputs"]["weight_dtype"], "default");
    }
}
