//! The text encoder's output kept between renders. Loading umt5 and encoding the two prompts took ~85 s of a 6-minute
//! draft on 8 GB (docs/RENDER.md), for prompts that rarely change. A render saves each prompt's conditioning into
//! ComfyUI's output folder with the core `SaveConditioning` node; a later render with the same encoder and text loads
//! it with `ConditioningLoader`, which reads the `embeddings` folder pointed at it ([`FOLDER`]), and umt5 is never
//! loaded. Only for a ComfyUI whose output folder is on this machine: the file is looked for there.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Where in ComfyUI's output folder conditionings are saved, also given to ComfyUI as an `embeddings` folder.
pub const FOLDER: &str = "kawarimi/conditioning";

/// Where a prompt's conditioning comes from in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    /// Encoded and saved under this `SaveConditioning` prefix.
    Save(String),
    /// Loaded: the saved file's name in the embeddings folder.
    Load(String),
}

impl Text {
    /// Load the conditioning of `text` by `encoder` if ComfyUI's `output` folder has it, else encode and save it.
    pub fn plan(output: &Path, encoder: &str, text: &str) -> Self {
        let name = hex::encode(&Sha256::digest(format!("{encoder}\n{text}").as_bytes())[..12]);
        // `SaveConditioning` numbers its files; the first save of a name is the only one.
        let file = format!("{name}_00001_.safetensors");
        if output.join(FOLDER).join(&file).is_file() { Self::Load(file) } else { Self::Save(format!("{FOLDER}/{name}")) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_prompt_is_loaded_and_a_new_one_saved() {
        let output = tempfile::tempdir().unwrap();
        let Text::Save(prefix) = Text::plan(output.path(), "umt5.gguf", "walks") else { panic!("not saved") };
        let name = prefix.strip_prefix("kawarimi/conditioning/").unwrap();
        let dir = output.path().join(FOLDER);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}_00001_.safetensors")), b"").unwrap();
        assert_eq!(Text::plan(output.path(), "umt5.gguf", "walks"), Text::Load(format!("{name}_00001_.safetensors")));
        assert!(matches!(Text::plan(output.path(), "umt5.gguf", "runs"), Text::Save(_)));
        assert!(matches!(Text::plan(output.path(), "other.gguf", "walks"), Text::Save(_)));
    }
}
