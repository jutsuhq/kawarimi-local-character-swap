//! Wan's prompt without an LLM (#50): who in the clip is replaced, by which character, and that the rest of the
//! scene stays as it is, from prep's `manifest.json` and the character image's name.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Alone,
    Left,
    Centre,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub position: Position,
    /// Other people in the clip, who stay.
    pub others: usize,
    pub cuts: usize,
    /// The character's name when the image's file name gives one.
    pub character: Option<String>,
}

impl Scene {
    /// `person` is an identity id of the manifest.
    pub fn read(manifest: &Value, person: u64, character: &Path) -> Result<Self> {
        let identities = manifest["identities"].as_array().context("manifest has no identities")?;
        let identity = identities.iter().find(|i| i["id"] == person).with_context(|| format!("no person {person} in the clip"))?;
        let width = manifest["video"]["width"].as_f64().context("manifest has no width")?;
        // Where the person stands: the middle of the boxes they were first seen in.
        let centres: Vec<f64> = identity["tracklets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                let seed = &manifest["tracklets"][t.as_u64()? as usize]["seed"];
                Some((seed["x0"].as_f64()? + seed["x1"].as_f64()?) / 2.0)
            })
            .collect();
        let x = centres.iter().sum::<f64>() / centres.len().max(1) as f64 / width;
        let others = identities.len() - 1;
        let position = match x {
            _ if others == 0 => Position::Alone,
            x if x < 0.4 => Position::Left,
            x if x > 0.6 => Position::Right,
            _ => Position::Centre,
        };
        let cuts = manifest["shots"].as_array().map_or(0, |s| s.len().saturating_sub(1));
        Ok(Self { position, others, cuts, character: name(character) })
    }

    pub fn prompt(&self) -> String {
        let who = match self.position {
            Position::Alone => "The person",
            Position::Left => "The person on the left",
            Position::Centre => "The person in the middle",
            Position::Right => "The person on the right",
        };
        let character = match &self.character {
            Some(name) => format!("{name}, the character in the reference image"),
            None => "the character in the reference image".into(),
        };
        let mut prompt = format!("{who} is replaced by {character}, moving exactly as they do");
        if self.cuts > 0 {
            prompt.push_str(" in every shot");
        }
        prompt.push_str(". Keep the scene as it is: the background, the light, the camera");
        prompt.push_str(match self.others {
            0 => " and the framing stay unchanged.",
            _ => " and the other people stay unchanged.",
        });
        prompt
    }
}

/// "anya_forger.png" → "Anya forger"; names without letters ("05.png") give none.
fn name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy().replace(['_', '-'], " ");
    let stem = stem.trim();
    if !stem.chars().any(char::is_alphabetic) {
        return None;
    }
    let mut chars = stem.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn manifest(people: usize) -> Value {
        let tracklet = |x0: f64| json!({ "seed": { "x0": x0, "y0": 0.0, "x1": x0 + 100.0, "y1": 300.0 } });
        let identities: Vec<Value> = (0..people).map(|i| json!({ "id": i + 1, "tracklets": [i] })).collect();
        json!({
            "video": { "width": 960, "height": 540 },
            "shots": [{ "start": 0, "end": 40 }, { "start": 40, "end": 80 }],
            "tracklets": [tracklet(50.0), tracklet(800.0), tracklet(430.0)],
            "identities": identities,
        })
    }

    #[test]
    fn says_who_by_where_they_stand() {
        let two = manifest(3);
        assert_eq!(Scene::read(&two, 1, Path::new("05.png")).unwrap().position, Position::Left);
        assert_eq!(Scene::read(&two, 2, Path::new("05.png")).unwrap().position, Position::Right);
        assert_eq!(Scene::read(&two, 3, Path::new("05.png")).unwrap().position, Position::Centre);
        assert_eq!(Scene::read(&manifest(1), 1, Path::new("x.png")).unwrap().position, Position::Alone);
        assert!(Scene::read(&two, 9, Path::new("x.png")).is_err());
    }

    #[test]
    fn writes_the_template() {
        let scene = Scene::read(&manifest(3), 2, Path::new("C:/chars/anya_forger.png")).unwrap();
        assert_eq!(
            scene.prompt(),
            "The person on the right is replaced by Anya forger, the character in the reference image, moving exactly \
             as they do in every shot. Keep the scene as it is: the background, the light, the camera and the other \
             people stay unchanged."
        );
        let alone = Scene { position: Position::Alone, others: 0, cuts: 0, character: None };
        assert_eq!(
            alone.prompt(),
            "The person is replaced by the character in the reference image, moving exactly as they do. Keep the \
             scene as it is: the background, the light, the camera and the framing stay unchanged."
        );
    }
}
