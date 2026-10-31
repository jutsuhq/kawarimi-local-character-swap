//! ComfyUI's HTTP API: upload inputs, queue a graph, find it and follow it, cancel it, and download what it saved.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use reqwest::blocking::{Client as Http, multipart};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::timing::Listener;
use crate::wan::SAVE_NODE;

pub struct Client {
    base: String,
    http: Http,
    /// Sent with every prompt, so ComfyUI's history tells our prompts apart.
    client_id: String,
    token: Option<String>,
    /// The websocket timing this client's prompts, opened with the first queue.
    listener: Mutex<Option<Listener>>,
}

/// A file a node saved: where `/view` finds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub node: String,
    pub filename: String,
    pub subfolder: String,
    pub kind: String,
}

/// Where a prompt is in ComfyUI.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptState {
    Pending,
    Running,
    /// Every file the graph saved.
    Done(Vec<Output>),
    Failed(String),
    /// Neither queued nor in the history: this ComfyUI never had it (it was restarted since).
    Unknown,
}

impl Client {
    pub fn new(base: impl Into<String>) -> Result<Self> {
        Self::with_token(base, None)
    }

    /// A client that sends `token` as `Authorization: Bearer`, for a ComfyUI behind a proxy; ComfyUI has none itself.
    pub fn with_token(base: impl Into<String>, token: Option<&str>) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(token) = token.filter(|t| !t.is_empty()) {
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).context("the token has characters a header cannot carry")?;
            value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
        Ok(Self {
            base: base.into().trim_end_matches('/').to_string(),
            http: Http::builder().default_headers(headers).connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(600)).build()?,
            client_id: format!("kawarimi-{}", std::process::id()),
            token: token.filter(|t| !t.is_empty()).map(String::from),
            listener: Mutex::new(None),
        })
    }

    /// Every node class the server has, with its inputs and their choices.
    pub fn object_info(&self) -> Result<Value> {
        self.get("object_info")
    }

    /// Puts a file in ComfyUI's input folder (overwriting one of the same name) and returns the name nodes load it by.
    /// `/upload/image` takes any file, video included.
    pub fn upload(&self, path: &Path, name: &str) -> Result<String> {
        let part = multipart::Part::file(path).with_context(|| format!("reading {}", path.display()))?.file_name(name.to_string());
        let form = multipart::Form::new().part("image", part).text("overwrite", "true").text("type", "input");
        let reply: Value = self.http.post(format!("{}/upload/image", self.base)).multipart(form).send()?.error_for_status()?.json()?;
        let name = reply["name"].as_str().context("ComfyUI's upload reply has no name")?;
        Ok(match reply["subfolder"].as_str() {
            Some(sub) if !sub.is_empty() => format!("{sub}/{name}"),
            _ => name.to_string(),
        })
    }

    /// Queues a graph and returns its prompt id. A graph ComfyUI rejects fails with its node errors.
    pub fn queue(&self, graph: &Value) -> Result<String> {
        self.listen();
        let response = self
            .http
            .post(format!("{}/prompt", self.base))
            .json(&json!({ "prompt": graph, "client_id": self.client_id }))
            .send()?;
        let status = response.status();
        let reply: Value = response.json()?;
        if !status.is_success() {
            bail!("ComfyUI rejected the graph ({status}): {}", rejection(&reply));
        }
        Ok(reply["prompt_id"].as_str().context("ComfyUI's reply has no prompt_id")?.to_string())
    }

    /// Opens the websocket before a prompt is queued, so its first node is seen; without it there are no timings.
    fn listen(&self) {
        let mut listener = self.listener.lock().unwrap();
        if listener.is_none() {
            match Listener::connect(&self.base, &self.client_id, self.token.as_deref()) {
                Ok(l) => *listener = Some(l),
                Err(e) => eprintln!("no node timings: {e:#}"),
            }
        }
    }

    /// Node id and seconds of a prompt this client queued, in the order the nodes ran.
    pub fn node_times(&self, prompt: &str) -> Vec<(String, f64)> {
        let listener = self.listener.lock().unwrap();
        listener.as_ref().map(|l| l.timings.lock().unwrap().of(prompt, Instant::now())).unwrap_or_default()
    }

    /// Where the prompt is. The queue is read before the history: ComfyUI files a prompt in its history before
    /// taking it off the queue, so a prompt that finishes in between is not taken for an unknown one.
    pub fn state(&self, prompt: &str) -> Result<PromptState> {
        let queue = self.get("queue")?;
        let queued = |key: &str| queue[key].as_array().into_iter().flatten().any(|item| item[1] == prompt);
        if queued("queue_running") {
            return Ok(PromptState::Running);
        }
        if queued("queue_pending") {
            return Ok(PromptState::Pending);
        }
        let history = self.get(&format!("history/{prompt}"))?;
        let Some(entry) = history.get(prompt) else { return Ok(PromptState::Unknown) };
        let status = &entry["status"];
        Ok(if status["status_str"] == "error" {
            PromptState::Failed(execution_error(status))
        } else if status["completed"] == true {
            PromptState::Done(outputs(&entry["outputs"]))
        } else {
            PromptState::Running
        })
    }

    /// The prompt, queued, running or finished, whose graph saves its video under `prefix`.
    pub fn find(&self, prefix: &str) -> Result<Option<String>> {
        let queue = self.get("queue")?;
        let history = self.get("history")?;
        let queued = ["queue_running", "queue_pending"].into_iter().flat_map(|k| queue[k].as_array().cloned().unwrap_or_default());
        let finished = history.as_object().into_iter().flatten().map(|(_, entry)| entry["prompt"].clone());
        Ok(queued.chain(finished).find(|item| saves_under(&item[2], prefix)).and_then(|item| item[1].as_str().map(String::from)))
    }

    /// Takes the prompt off the queue, or stops it if it is running; other prompts go on.
    pub fn cancel(&self, prompt: &str) -> Result<()> {
        self.http.post(format!("{}/queue", self.base)).json(&json!({ "delete": [prompt] })).send()?.error_for_status()?;
        self.http.post(format!("{}/interrupt", self.base)).json(&json!({ "prompt_id": prompt })).send()?.error_for_status()?;
        Ok(())
    }

    /// The first GPU's memory as ComfyUI reports it, in bytes.
    pub fn vram(&self) -> Result<u64> {
        Ok(self.get("system_stats")?["devices"][0]["vram_total"].as_u64().unwrap_or(0))
    }

    fn get(&self, path: &str) -> Result<Value> {
        let response = self.http.get(format!("{}/{path}", self.base)).send()?;
        if matches!(response.status().as_u16(), 401 | 403) {
            bail!("ComfyUI (or the proxy in front of it) refused the token (HTTP {})", response.status());
        }
        Ok(response.error_for_status()?.json()?)
    }

    /// Downloads a saved file to `dest`.
    pub fn download(&self, output: &Output, dest: &Path) -> Result<PathBuf> {
        let mut response = self
            .http
            .get(format!("{}/view", self.base))
            .query(&[("filename", &output.filename), ("subfolder", &output.subfolder), ("type", &output.kind)])
            .send()?
            .error_for_status()?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(dest)?;
        response.copy_to(&mut file)?;
        Ok(dest.to_path_buf())
    }

    /// Frees the models ComfyUI keeps loaded between prompts.
    pub fn free(&self) -> Result<()> {
        self.http.post(format!("{}/free", self.base)).json(&json!({ "unload_models": true, "free_memory": true })).send()?.error_for_status()?;
        Ok(())
    }
}

/// Every file in a history entry's outputs, whatever the key (`images`, `gifs`, `videos`, …) a node uses.
fn outputs(outputs: &Value) -> Vec<Output> {
    let mut found = Vec::new();
    for (node, ui) in outputs.as_object().into_iter().flatten() {
        for files in ui.as_object().into_iter().flatten().filter_map(|(_, v)| v.as_array()) {
            for file in files {
                if let Some(filename) = file["filename"].as_str() {
                    found.push(Output {
                        node: node.clone(),
                        filename: filename.into(),
                        subfolder: file["subfolder"].as_str().unwrap_or_default().into(),
                        kind: file["type"].as_str().unwrap_or("output").into(),
                    });
                }
            }
        }
    }
    found
}

fn saves_under(graph: &Value, prefix: &str) -> bool {
    graph[SAVE_NODE]["inputs"]["filename_prefix"] == prefix
}

fn rejection(reply: &Value) -> String {
    let mut parts = vec![reply["error"]["message"].as_str().unwrap_or("unknown error").to_string()];
    for (node, error) in reply["node_errors"].as_object().into_iter().flatten() {
        for e in error["errors"].as_array().into_iter().flatten() {
            parts.push(format!("node {node} ({}): {} {}", error["class_type"].as_str().unwrap_or("?"), e["message"].as_str().unwrap_or(""), e["details"].as_str().unwrap_or("")));
        }
    }
    parts.join("; ")
}

fn execution_error(status: &Value) -> String {
    let messages = status["messages"].as_array().into_iter().flatten();
    messages
        .filter(|m| m[0] == "execution_error")
        .map(|m| {
            let d = &m[1];
            format!("node {} ({}): {}", d["node_id"].as_str().unwrap_or("?"), d["node_type"].as_str().unwrap_or("?"), d["exception_message"].as_str().unwrap_or("").trim())
        })
        .next()
        .unwrap_or_else(|| "unknown error".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server that answers one request with `status` and returns what it was sent.
    fn answer_once(status: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let status = status.to_string();
        let served = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = vec![0; 4096];
            let n = stream.read(&mut request).unwrap();
            let body = r#"{"devices":[{"vram_total":8589934592}]}"#;
            write!(stream, "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        (url, served)
    }

    #[test]
    fn the_token_goes_as_a_bearer_header() {
        let (url, served) = answer_once("200 OK");
        assert_eq!(Client::with_token(url, Some("s3cret")).unwrap().vram().unwrap(), 8 << 30);
        let request = served.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /system_stats "), "{request}");
        assert!(request.contains("authorization: bearer s3cret"), "{request}");

        let (url, served) = answer_once("401 Unauthorized");
        let error = Client::with_token(url, None).unwrap().vram().unwrap_err().to_string();
        assert!(error.contains("refused the token"), "{error}");
        assert!(!served.join().unwrap().to_ascii_lowercase().contains("authorization"));
    }

    #[test]
    fn outputs_are_found_under_any_key() {
        let history = json!({
            "901": { "images": [{ "filename": "wan_00001_.mp4", "subfolder": "kawarimi", "type": "output" }], "animated": [true] },
            "5": { "text": ["not a file"] }
        });
        let found = outputs(&history);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].node, "901");
        assert_eq!(found[0].subfolder, "kawarimi");
    }

    #[test]
    fn a_prompt_is_found_by_its_prefix() {
        let graph = json!({ "901": { "inputs": { "filename_prefix": "kawarimi/1/render" } } });
        assert!(saves_under(&graph, "kawarimi/1/render"));
        assert!(!saves_under(&graph, "kawarimi/2/render"));
        assert!(!saves_under(&json!({}), "kawarimi/1/render"));
    }

    #[test]
    fn errors_name_the_node() {
        let status = json!({ "messages": [["execution_start", {}], ["execution_error", { "node_id": "14", "node_type": "DWPreprocessor", "exception_message": "boom\n" }]] });
        assert_eq!(execution_error(&status), "node 14 (DWPreprocessor): boom");
        let reply = json!({ "error": { "message": "Prompt outputs failed validation" }, "node_errors": { "1": { "class_type": "UnetLoaderGGUF", "errors": [{ "message": "Value not in list", "details": "unet_name" }] } } });
        assert!(rejection(&reply).contains("node 1 (UnetLoaderGGUF): Value not in list unet_name"));
    }
}
