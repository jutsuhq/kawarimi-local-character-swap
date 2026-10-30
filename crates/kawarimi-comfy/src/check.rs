//! Whether a ComfyUI someone runs can render Kawarimi's Wan graph: every node class it uses, and every model file the
//! graph loads, read from the server's `/object_info`.

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::render::{self, Clip, Settings, Uploads};
use crate::wan::Quality;

const MODEL_EXTENSIONS: &[&str] = &[".gguf", ".safetensors", ".pth", ".pt", ".ckpt", ".bin", ".onnx"];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// The first GPU's memory as the server reports it, in bytes.
    pub vram: u64,
    /// The Wan weights a render on it loads.
    pub model: &'static str,
    pub missing_nodes: Vec<String>,
    /// `folder/file`, for instance `diffusion_models/Wan2.2-Animate-14B-Q3_K_M.gguf`.
    pub missing_models: Vec<String>,
}

impl Check {
    pub fn is_ready(&self) -> bool {
        self.missing_nodes.is_empty() && self.missing_models.is_empty()
    }
}

/// Checks the graph a draft on a card with `vram` bytes makes against a server's `/object_info`.
pub fn evaluate(object_info: &Value, vram: u64) -> Result<Check> {
    let settings = Settings { quality: Quality::Draft, seed: 0, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
    let variant = settings.variant(vram)?;
    let clip = Clip { width: 832, height: 480, frames: 33 };
    let uploads = Uploads { source: "source.mp4".into(), mask: "mask.mp4".into(), character: "character.png".into() };
    let graph = render::graph(&settings.inputs(&clip, vram, variant, uploads, "kawarimi/check/render"), variant)?;

    let mut check = Check { vram, model: variant.asset.title, missing_nodes: Vec::new(), missing_models: Vec::new() };
    for node in graph.as_object().into_iter().flatten().map(|(_, n)| n) {
        let class = node["class_type"].as_str().unwrap_or_default();
        let Some(info) = object_info.get(class) else {
            if !check.missing_nodes.iter().any(|n| n == class) {
                check.missing_nodes.push(class.into());
            }
            continue;
        };
        for (name, value) in node["inputs"].as_object().into_iter().flatten() {
            let Some(file) = value.as_str().filter(|v| MODEL_EXTENSIONS.iter().any(|e| v.ends_with(e))) else { continue };
            let spec = ["required", "optional"].iter().find_map(|k| info["input"][k].get(name));
            let choices = spec.and_then(|s| s[0].as_array());
            if !choices.is_some_and(|c| c.iter().any(|c| c == file)) {
                let missing = format!("{}/{file}", folder(class));
                if !check.missing_models.contains(&missing) {
                    check.missing_models.push(missing);
                }
            }
        }
    }
    Ok(check)
}

/// The models folder a loader reads from.
fn folder(class: &str) -> &'static str {
    match class {
        c if c.starts_with("Unet") || c == "UNETLoader" => "diffusion_models",
        c if c.starts_with("CLIPVision") => "clip_vision",
        c if c.starts_with("CLIP") => "text_encoders",
        c if c.starts_with("Lora") => "loras",
        c if c.starts_with("VAE") => "vae",
        _ => "models",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// An `/object_info` with every class and model the graph uses, as ComfyUI lists combo choices.
    fn complete(vram: u64) -> Value {
        let check = evaluate(&json!({}), vram).unwrap();
        let mut info = serde_json::Map::new();
        for class in &check.missing_nodes {
            info.insert(class.clone(), json!({ "input": { "required": {} } }));
        }
        let settings = Settings { quality: Quality::Draft, seed: 0, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
        let variant = settings.variant(vram).unwrap();
        let uploads = Uploads { source: "s".into(), mask: "m".into(), character: "c".into() };
        let graph = render::graph(&settings.inputs(&Clip { width: 832, height: 480, frames: 33 }, vram, variant, uploads, "p"), variant).unwrap();
        for node in graph.as_object().unwrap().values() {
            for (name, value) in node["inputs"].as_object().unwrap() {
                if value.as_str().is_some_and(|v| MODEL_EXTENSIONS.iter().any(|e| v.ends_with(e))) {
                    let class = node["class_type"].as_str().unwrap();
                    info[class]["input"]["required"][name] = json!([[value, "other.safetensors"]]);
                }
            }
        }
        Value::Object(info)
    }

    #[test]
    fn a_complete_server_is_ready() {
        let check = evaluate(&complete(8 << 30), 8 << 30).unwrap();
        assert!(check.is_ready(), "{check:?}");
    }

    #[test]
    fn missing_nodes_and_models_are_named() {
        let mut info = complete(8 << 30);
        info.as_object_mut().unwrap().remove("DWPreprocessor");
        info["UnetLoaderGGUF"]["input"]["required"]["unet_name"] = json!([["something-else.gguf"]]);
        let check = evaluate(&info, 8 << 30).unwrap();
        assert_eq!(check.missing_nodes, ["DWPreprocessor"]);
        assert_eq!(check.missing_models, ["diffusion_models/Wan2.2-Animate-14B-Q3_K_M.gguf"]);
        assert!(!check.is_ready());
    }

    /// `KAWARIMI_OWN_COMFY=http://host:8188 cargo test -p kawarimi-comfy live_server -- --ignored --nocapture`
    #[test]
    #[ignore = "asks the ComfyUI at KAWARIMI_OWN_COMFY"]
    fn live_server() {
        let client = crate::Client::new(std::env::var("KAWARIMI_OWN_COMFY").unwrap()).unwrap();
        let check = evaluate(&client.object_info().unwrap(), client.vram().unwrap()).unwrap();
        eprintln!("{check:?}");
        assert!(check.is_ready());
    }

    #[test]
    fn a_bigger_card_is_checked_for_its_own_weights() {
        let check = evaluate(&complete(8 << 30), 24 << 30).unwrap();
        assert!(check.missing_nodes.contains(&"UNETLoader".to_string()), "{check:?}");
    }
}
