//! Kawarimi for AI agents: a Model Context Protocol server over stdio. It prepares clips, cuts out characters and
//! queues renders with the app's models, ComfyUI and jobs folder, within limits a person sets in
//! `<app data>/mcp/limits.json` (docs/MCP.md).
//!
//!     claude mcp add kawarimi -- <path>/kawarimi-mcp
//!
//! Messages are JSON-RPC 2.0, one per line on stdin and stdout; logs go to stderr.

mod dirs;
mod ledger;
mod tools;
mod worker;

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use kawarimi_assets::Store;
use kawarimi_comfy::job::Jobs;
use kawarimi_comfy::locks::GpuLock;
use serde_json::{Value, json};

use crate::dirs::Dirs;
use crate::ledger::Ledger;
use crate::tools::Kawarimi;
use crate::worker::{Backend, Worker};

/// Newest first; an older client gets its own version back.
const PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Kawarimi replaces a person in a short video with a character from an image, on this \
computer's NVIDIA GPU. Call status first. Then prepare_clip (4–30 s of a video), look at the people with view_person \
or view_frame, cut_character on the character image, and start_render with the person's id and the cutout; follow \
the job with job_status. A draft renders the first window to check the motion; a final renders the whole clip with \
the same seed. Renders take minutes and count against limits the person set.";

fn main() -> Result<()> {
    let dirs = Dirs::from_env()?;
    let store = Store::new(&dirs.assets);
    let ledger = Arc::new(Ledger::open(&dirs.mcp)?);
    let jobs = Jobs::new(&dirs.jobs);
    let backend = Backend::new(Store::new(&dirs.assets), dirs.comfy.clone(), GpuLock::path(&dirs.data), ledger.clone());
    let worker = Worker::start(jobs, backend);
    let kawarimi = Kawarimi { dirs, store, ledger, worker, installs: Arc::new(Mutex::new(BTreeMap::new())) };
    eprintln!("kawarimi-mcp {}: data in {}", env!("CARGO_PKG_VERSION"), kawarimi.dirs.data.display());

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&kawarimi, &message),
            Err(e) => Some(error(Value::Null, -32700, &format!("not JSON: {e}"))),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// The reply to one message; none for a notification.
fn handle(kawarimi: &Kawarimi, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = PROTOCOLS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOLS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "kawarimi", "title": "Kawarimi", "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })
        }
        "ping" => json!({}),
        "tools/list" => tools::list(),
        "tools/call" => call(kawarimi, &params),
        other => return Some(error(id, -32601, &format!("no method {other}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// A tool's result; its failures are results with `isError`, so the agent reads what to do.
fn call(kawarimi: &Kawarimi, params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    match kawarimi.call(name, &args) {
        Ok(reply) => {
            let mut content = vec![json!({ "type": "text", "text": reply.text })];
            content.extend(reply.images.iter().map(|png| json!({ "type": "image", "mimeType": "image/png", "data": tools::base64(png) })));
            let mut result = json!({ "content": content, "isError": false });
            if let Some(Value::Object(data)) = reply.data {
                result["structuredContent"] = Value::Object(data);
            }
            result
        }
        Err(e) => json!({ "content": [{ "type": "text", "text": format!("{e:#}") }], "isError": true }),
    }
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kawarimi(dir: &std::path::Path) -> Kawarimi {
        let data = dir.to_path_buf();
        let dirs = Dirs { assets: data.join("assets"), comfy: data.join("comfy"), jobs: data.join("jobs"), mcp: data.join("mcp"), data };
        let ledger = Arc::new(Ledger::open(&dirs.mcp).unwrap());
        let backend = Backend::new(Store::new(&dirs.assets), dirs.comfy.clone(), GpuLock::path(&dirs.data), ledger.clone());
        let worker = Worker::start(Jobs::new(&dirs.jobs), backend);
        Kawarimi { store: Store::new(&dirs.assets), dirs, ledger, worker, installs: Default::default() }
    }

    #[test]
    fn initialize_lists_tools_and_answers_errors_as_results() {
        let dir = tempfile::tempdir().unwrap();
        let k = kawarimi(dir.path());
        let init = handle(&k, &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } })).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(handle(&k, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let tools = handle(&k, &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"prepare_clip") && names.contains(&"start_render"));
        let call = handle(&k, &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "view_person", "arguments": { "prep": "123", "person": 1 } } })).unwrap();
        assert_eq!(call["result"]["isError"], true);
        assert!(call["result"]["content"][0]["text"].as_str().unwrap().contains("prepare_clip first"));
        let unknown = handle(&k, &json!({ "jsonrpc": "2.0", "id": 4, "method": "resources/list" })).unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }

    #[test]
    fn a_prep_id_cannot_leave_the_preps_folder() {
        let dir = tempfile::tempdir().unwrap();
        let k = kawarimi(dir.path());
        let reply = k.call("view_person", &json!({ "prep": "../../jobs", "person": 1 }));
        assert!(reply.err().unwrap().to_string().contains("digits"));
    }
}
