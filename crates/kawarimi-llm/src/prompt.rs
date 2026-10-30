//! What the prompt writer asks and how its answers become Wan's prompt. The questions are Swapjutsu's
//! (`DescriptionPrompts`, MIT): one JSON object per answer, only what is visible, never who a person is, and examples
//! that are neutral, since a small model copies an example's words into its answer. Swapjutsu's moves question leans
//! on a pose tracker; here the frames are all the model gets, so it says only what they show.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const CHARACTER: &str = r#"You write the character part of a video-edit prompt from a reference image. Describe only what is visible in the image; never name or guess who a person is.
Reply with one JSON object and nothing else:
{"noun": "one or two uppercase words for what it is: WOMAN, MAN, GIRL, BOY, CAT, DOG, ROBOT, ...",
 "pronoun": "her, his, its or their",
 "appearance": "one phrase: age range, apparent height, build and proportions (shoulders, waist, legs), skin or fur, face shape, hair (colour, length, style), eyes, expression",
 "wardrobe": "every visible garment, accessory and shoe, top to bottom, with colours and patterns; empty for an animal without clothes",
 "traits": "the 5 to 7 most recognisable visual traits of the face, hair, fur and clothes, comma-separated, e.g. short curly red hair, freckles, yellow raincoat; never the pose or where it stands",
 "moving": "the parts that swing or flow when it moves, e.g. skirt, scarf and hair; empty when none"}"#;

/// The moves question for frames of a `seconds`-long clip at `times`, with the replacement's `wardrobe` when
/// known, so the model says what the body does rather than what the performer wears.
pub fn moves(times: &[f64], seconds: f64, wardrobe: &str) -> String {
    let count = times.len();
    let times = times.iter().map(|t| format!("{t:.2}")).collect::<Vec<_>>().join(", ");
    let wardrobe = if wardrobe.is_empty() { "(unknown)" } else { wardrobe };
    format!(
        r#"These {count} frames are from a {seconds:.1} s video, at {times} s. Write the movement part of a video-edit prompt. Say only what the frames show: the activity, the kind of step, the gesture, the direction of a turn, where they look, what they do with the things around them. Never describe or name who the people are; say "their" for a performer, never "his" or "her".
The performer is replaced by a character who wears: {wardrobe}. So say what the body does, never the performer's own clothes, pockets or accessories: write "hand at the hip", not "hand in the coat pocket".
Reply with one JSON object and nothing else:
{{"camera": "the camera angle and how it moves, judged by the background from frame to frame: static only when the background stays in place in every frame; else handheld, panning, tilting, tracking (moving with the performer), pushing in or pulling out",
 "shot": "how much of the performer the frame shows: close-up, chest-up, waist-up, knees-up, full-body or wide",
 "beats": "3 to 6 short beat phrases in time order, comma-separated, e.g. side step to the left, wave toward the camera, clockwise full-body turn, hands raised overhead, final front-facing pose",
 "gaze": "where the performer looks for most of the clip, in a few words, e.g. at the person beside them, down at the floor, into the camera",
 "scene": "everything in the frame besides the performer that stays as filmed, the other people first, by look and place, e.g. the woman on the left in a yellow raincoat, a wooden bench, a brick wall",
 "props": "what the performer holds or uses that stays, with how, e.g. the umbrella held over the head; never clothes or accessories; empty when none"}}"#,
    )
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    pub noun: String,
    pub pronoun: String,
    pub appearance: String,
    pub wardrobe: String,
    pub traits: String,
    pub moving: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Motion {
    pub camera: String,
    pub shot: String,
    pub beats: String,
    pub gaze: String,
    pub scene: String,
    pub props: String,
}

/// What the prompt writer made of a clip and its character, and the prompt written from it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    pub character: Character,
    pub motion: Option<Motion>,
    pub prompt: String,
    /// The model that answered.
    pub source: String,
}

/// The first JSON object in a model's answer (code fences and chatter around it are skipped), its values as text,
/// placeholder answers ("empty", "none") taken as nothing; nil when there is none or nothing in it. An object cut
/// off at the token limit gives up its complete `"key": "text"` pairs.
pub fn fields(answer: &str) -> Option<BTreeMap<String, String>> {
    let start = answer.find('{')?;
    let end = answer.rfind('}').filter(|&e| e > start);
    let object = end.and_then(|e| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&answer[start..=e]).ok());
    let Some(object) = object else { return salvaged(&answer[start..]) };
    let fields: BTreeMap<_, _> = object
        .into_iter()
        .map(|(key, value)| {
            let text = match value {
                serde_json::Value::String(s) => s,
                serde_json::Value::Array(items) => {
                    items.iter().map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).collect::<Vec<_>>().join(", ")
                }
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            (key.to_lowercase(), clean(&text))
        })
        .collect();
    fields.values().any(|v| !v.is_empty()).then_some(fields)
}

fn salvaged(text: &str) -> Option<BTreeMap<String, String>> {
    let mut fields = BTreeMap::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        let key = &after[..close];
        let tail = after[close + 1..].trim_start();
        let Some(tail) = tail.strip_prefix(':').map(str::trim_start) else {
            rest = &after[close + 1..];
            continue;
        };
        let Some(body) = tail.strip_prefix('"') else {
            rest = tail;
            continue;
        };
        let mut value = String::new();
        let mut chars = body.char_indices();
        let mut closed = None;
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => value.extend(chars.next().map(|(_, c)| c)),
                '"' => {
                    closed = Some(i);
                    break;
                }
                c => value.push(c),
            }
        }
        let Some(i) = closed else { break };
        if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            fields.insert(key.to_lowercase(), clean(&value));
        }
        rest = &body[i + 1..];
    }
    (fields.len() >= 2 && fields.values().any(|v| !v.is_empty())).then_some(fields)
}

fn clean(text: &str) -> String {
    let trimmed = text.trim();
    let placeholder = ["empty", "none", "n/a", "na", "null", "none visible", "-"].contains(&trimmed.to_lowercase().as_str());
    if placeholder { String::new() } else { trimmed.to_owned() }
}

fn get(f: &BTreeMap<String, String>, key: &str) -> String {
    f.get(key).cloned().unwrap_or_default()
}

pub fn character(f: &BTreeMap<String, String>) -> Character {
    Character {
        noun: get(f, "noun"),
        pronoun: get(f, "pronoun").to_lowercase(),
        appearance: get(f, "appearance"),
        wardrobe: get(f, "wardrobe"),
        traits: looks(&get(f, "traits")),
        moving: get(f, "moving"),
    }
}

pub fn motion(f: &BTreeMap<String, String>) -> Motion {
    Motion {
        camera: strip_prefix_words(&get(f, "camera"), &["the", "camera", "is", "a", "an"]),
        shot: get(f, "shot"),
        beats: get(f, "beats"),
        gaze: strip_prefix_words(&get(f, "gaze"), &["the", "performer", "they", "their", "eyes", "gaze", "is", "looking", "looks", "look", "gazing"])
            .trim_end_matches('.')
            .to_owned(),
        scene: get(f, "scene"),
        props: get(f, "props"),
    }
}

/// The traits without the pose a model sometimes lists instead ("standing, facing camera"): the prompt renders the
/// traits in every frame, and a pose there would fight the performer's moves.
fn looks(traits: &str) -> String {
    const POSE: &[&str] = &["stand", "sit", "facing", "faces", "hand", "arm", "posture", "pose", "looking", "upright", "leaning"];
    traits
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty() && !POSE.iter().any(|p| t.to_lowercase().split_whitespace().any(|w| w.starts_with(p))))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `text` without the leading words in `words`, case aside: "The camera is static" → "static".
fn strip_prefix_words(text: &str, words: &[&str]) -> String {
    let mut rest = text.trim();
    while let Some((first, tail)) = rest.split_once(char::is_whitespace) {
        if !words.contains(&first.to_lowercase().as_str()) {
            break;
        }
        rest = tail.trim_start();
    }
    rest.to_owned()
}

/// Wan's prompt: who the character is and wears, how the camera and the performer move, and what stays as filmed.
/// Parts no model described are left out; with nothing described it is Swapjutsu's default.
pub fn assemble(character: &Character, motion: Option<&Motion>) -> String {
    let noun = match character.noun.trim() {
        "" => "character".to_owned(),
        n => n.to_lowercase(),
    };
    let (subject, possessive) = match character.pronoun.as_str() {
        "her" => ("She", "Her"),
        "his" => ("He", "His"),
        "its" => ("It", "Its"),
        _ => ("They", "Their"),
    };
    let mut sentences = Vec::new();
    let looks = [character.appearance.as_str(), character.traits.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("; ");
    match (looks.is_empty(), character.wardrobe.is_empty()) {
        (true, true) => sentences.push(format!("A {noun}.")),
        (false, true) => sentences.push(format!("A {noun}: {looks}.")),
        (true, false) => sentences.push(format!("A {noun} wearing {}.", character.wardrobe)),
        (false, false) => sentences.push(format!("A {noun}: {looks}, wearing {}.", character.wardrobe)),
    }
    if let Some(m) = motion {
        let camera = [m.shot.as_str(), m.camera.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
        if !camera.is_empty() {
            sentences.push(format!("{}, the same camera as the original.", capitalized(&camera)));
        }
        if !m.beats.is_empty() {
            sentences.push(format!("The {noun} follows the original moves beat for beat: {}.", m.beats.trim_end_matches('.')));
        }
        if !m.gaze.is_empty() {
            sentences.push(format!("{subject} {} {}.", if subject == "They" { "look" } else { "looks" }, m.gaze));
        }
        if !m.props.is_empty() {
            sentences.push(format!("{subject} {} {}.", if subject == "They" { "keep" } else { "keeps" }, m.props.trim_end_matches('.')));
        }
    }
    if !character.moving.is_empty() {
        sentences.push(format!("{possessive} {} swing naturally with the motion.", character.moving.trim_end_matches('.')));
    }
    if let Some(scene) = motion.map(|m| m.scene.trim_end_matches('.')).filter(|s| !s.is_empty()) {
        sentences.push(format!("Everything else stays as filmed: {scene}."));
    }
    if sentences.len() == 1 && character.noun.is_empty() && looks.is_empty() && character.wardrobe.is_empty() {
        return DEFAULT.into();
    }
    sentences.join(" ")
}

/// Swapjutsu's prompt for Wan when nothing is described.
pub const DEFAULT: &str = "The character moves naturally in the scene.";

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_object_inside_chatter_and_fences() {
        let answer = "Sure!\n```json\n{\"noun\": \"WOMAN\", \"pronoun\": \"her\", \"wardrobe\": \"none\", \"traits\": [\"red hair\", \"freckles\"]}\n```";
        let f = fields(answer).unwrap();
        assert_eq!(f["noun"], "WOMAN");
        assert_eq!(f["wardrobe"], "");
        assert_eq!(f["traits"], "red hair, freckles");
        assert!(fields("I cannot describe this image.").is_none());
        assert!(fields("{\"noun\": \"none\"}").is_none());
    }

    #[test]
    fn salvages_an_answer_cut_off_at_the_limit() {
        let f = fields(r#"{"noun": "MAN", "pronoun": "his", "appearance": "tall, short \"buzz\" cut", "wardrobe": "a grey hoo"#).unwrap();
        assert_eq!(f["appearance"], "tall, short \"buzz\" cut");
        assert!(!f.contains_key("wardrobe"));
    }

    #[test]
    fn traits_drop_the_pose() {
        assert_eq!(looks("short curly red hair, standing upright, freckles, hands by sides, yellow raincoat"), "short curly red hair, freckles, yellow raincoat");
    }

    #[test]
    fn moves_question_counts_the_frames() {
        let q = moves(&[0.0, 1.25, 2.5, 3.75], 5.0, "a yellow raincoat");
        assert!(q.starts_with("These 4 frames are from a 5.0 s video, at 0.00, 1.25, 2.50, 3.75 s."));
        assert!(q.contains("wears: a yellow raincoat."));
    }

    #[test]
    fn assembles_the_wan_prompt() {
        let character = Character {
            noun: "WOMAN".into(),
            pronoun: "her".into(),
            appearance: "young adult, slim, long black hair".into(),
            wardrobe: "a yellow raincoat and white sneakers".into(),
            traits: "freckles".into(),
            moving: "raincoat hem and hair".into(),
        };
        let motion = Motion {
            camera: "static".into(),
            shot: "waist-up".into(),
            beats: "side step to the left, wave toward the camera".into(),
            gaze: "into the camera".into(),
            scene: "a wooden bench, a brick wall".into(),
            props: String::new(),
        };
        assert_eq!(
            assemble(&character, Some(&motion)),
            "A woman: young adult, slim, long black hair; freckles, wearing a yellow raincoat and white sneakers. \
             Waist-up, static, the same camera as the original. \
             The woman follows the original moves beat for beat: side step to the left, wave toward the camera. \
             She looks into the camera. Her raincoat hem and hair swing naturally with the motion. \
             Everything else stays as filmed: a wooden bench, a brick wall."
        );
        assert_eq!(assemble(&Character::default(), None), DEFAULT);
    }

    #[test]
    fn motion_strips_the_models_lead_in() {
        let f = fields(r#"{"camera": "The camera is static", "gaze": "They look down at the floor."}"#).unwrap();
        let m = motion(&f);
        assert_eq!(m.camera, "static");
        assert_eq!(m.gaze, "down at the floor");
    }
}
