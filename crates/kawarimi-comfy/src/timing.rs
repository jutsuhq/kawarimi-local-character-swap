//! Each node's time in a prompt, from ComfyUI's websocket (`/ws?clientId=…`): it says `executing` with a node id when
//! a node starts and with none when the prompt ends, only to the client that queued the prompt. History keeps no
//! times. A node's time includes what ComfyUI loads lazily for it: Wan's weights land in the sampler's.

use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tungstenite::client::IntoClientRequest;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeTime {
    pub node: String,
    pub class_type: String,
    pub seconds: f64,
}

/// Prompts' node times as the messages come.
#[derive(Debug, Default)]
pub struct Timings {
    prompts: HashMap<String, Prompt>,
}

#[derive(Debug, Default)]
struct Prompt {
    running: Option<(String, Instant)>,
    done: Vec<(String, f64)>,
}

impl Prompt {
    fn close(&mut self, now: Instant) {
        if let Some((node, start)) = self.running.take() {
            self.done.push((node, now.duration_since(start).as_secs_f64()));
        }
    }
}

impl Timings {
    pub fn on(&mut self, message: &Value, now: Instant) {
        let data = &message["data"];
        let Some(id) = data["prompt_id"].as_str() else { return };
        let prompt = self.prompts.entry(id.into()).or_default();
        match message["type"].as_str() {
            Some("executing") => {
                prompt.close(now);
                if let Some(node) = data["node"].as_str() {
                    prompt.running = Some((node.into(), now));
                }
            }
            Some("execution_success" | "execution_error" | "execution_interrupted") => prompt.close(now),
            _ => {}
        }
    }

    /// Node id and seconds, in the order they ran; a node still running counts until `now`.
    pub fn of(&self, prompt: &str, now: Instant) -> Vec<(String, f64)> {
        let Some(p) = self.prompts.get(prompt) else { return Vec::new() };
        let mut times = p.done.clone();
        if let Some((node, start)) = &p.running {
            times.push((node.clone(), now.duration_since(*start).as_secs_f64()));
        }
        times
    }
}

/// A websocket to ComfyUI read on its own thread until [`Listener`] drops.
pub struct Listener {
    pub timings: Arc<Mutex<Timings>>,
    stream: TcpStream,
}

impl Listener {
    /// `base` is `http://host:port`; `token`, if any, goes as a bearer header like the HTTP calls'.
    pub fn connect(base: &str, client_id: &str, token: Option<&str>) -> Result<Self> {
        let host = base.strip_prefix("http://").context("timings need a plain http:// ComfyUI")?;
        let stream = TcpStream::connect(host)?;
        let mut request = format!("ws://{host}/ws?clientId={client_id}").into_client_request()?;
        if let Some(token) = token {
            request.headers_mut().insert("authorization", format!("Bearer {token}").parse()?);
        }
        let (mut socket, _) = tungstenite::client(request, stream.try_clone()?).map_err(|e| anyhow::anyhow!("websocket: {e}"))?;
        let timings = Arc::new(Mutex::new(Timings::default()));
        let filled = timings.clone();
        std::thread::spawn(move || {
            // Ends when the socket closes or Drop shuts it down; binary messages are previews.
            while let Ok(message) = socket.read() {
                if let Ok(text) = message.to_text()
                    && let Ok(value) = serde_json::from_str::<Value>(text)
                {
                    filled.lock().unwrap().on(&value, Instant::now());
                }
            }
        });
        Ok(Self { timings, stream })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn nodes_are_timed_between_executing_messages() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut timings = Timings::default();
        let executing = |node: Value, prompt: &str| json!({ "type": "executing", "data": { "node": node, "prompt_id": prompt } });
        timings.on(&json!({ "type": "execution_start", "data": { "prompt_id": "p" } }), at(0));
        timings.on(&json!({ "type": "execution_cached", "data": { "nodes": ["1"], "prompt_id": "p" } }), at(0));
        timings.on(&executing(json!("14"), "p"), at(1));
        timings.on(&json!({ "type": "progress", "data": { "value": 3, "max": 77, "node": "14", "prompt_id": "p" } }), at(5));
        timings.on(&executing(json!("102"), "p"), at(31));
        timings.on(&executing(json!("9"), "other"), at(40));
        assert_eq!(timings.of("p", at(100)), [("14".into(), 30.0), ("102".into(), 69.0)]);
        timings.on(&executing(Value::Null, "p"), at(331));
        assert_eq!(timings.of("p", at(1000)), [("14".into(), 30.0), ("102".into(), 300.0)]);
        assert_eq!(timings.of("other", at(41)), [("9".into(), 1.0)]);
        assert!(timings.of("unknown", at(0)).is_empty());
    }
}
