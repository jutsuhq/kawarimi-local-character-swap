//! One client for both modes: llama-server and any OpenAI-compatible service speak the same `/chat/completions`.
//! A remote endpoint is reachable only through [`Endpoint::remote`], which needs the person's consent to that exact
//! address; the key goes only into the request's header and is cut out of every error.

use std::time::Duration;

use anyhow::{Result, bail};
use base64::Engine;
use serde_json::{Value, json};

/// An API key. Never printed: its `Debug` is a mask, and errors have it replaced.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into().trim().to_owned())
    }

    fn redact(&self, text: &str) -> String {
        if self.0.is_empty() { text.to_owned() } else { text.replace(&self.0, "***") }
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

/// The person did not agree to send frames to this address.
#[derive(Debug)]
pub struct NoConsent(pub String);

impl std::fmt::Display for NoConsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sending frames to {} needs your consent in Settings", self.0)
    }
}

impl std::error::Error for NoConsent {}

#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Up to `/v1`, without the trailing slash.
    base: String,
    model: String,
    key: Option<ApiKey>,
    /// llama-server on this machine: no consent needed, and it takes the GPU.
    local: bool,
}

impl Endpoint {
    pub fn local(port: u16) -> Self {
        Self { base: format!("http://127.0.0.1:{port}/v1"), model: "local".into(), key: None, local: true }
    }

    /// A service the person named. `consented` is the address they agreed to send frames to; any other, or none,
    /// fails before anything is sent.
    pub fn remote(base: &str, model: &str, key: Option<ApiKey>, consented: Option<&str>) -> Result<Self> {
        let base = normalize(base);
        if !base.starts_with("https://") && !base.starts_with("http://") {
            bail!("the API address must start with https://");
        }
        if consented.map(normalize) != Some(base.clone()) {
            return Err(NoConsent(base).into());
        }
        Ok(Self { base, model: model.trim().to_owned(), key, local: false })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// The model's answer to `prompt` about `images` (JPEG bytes), at most `max_tokens` long.
    pub fn ask(&self, prompt: &str, images: &[Vec<u8>], max_tokens: u32, timeout: Duration) -> Result<String> {
        let mut content = vec![json!({ "type": "text", "text": prompt })];
        for jpeg in images {
            let url = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg));
            content.push(json!({ "type": "image_url", "image_url": { "url": url } }));
        }
        let body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": content }],
            "max_tokens": max_tokens,
            "temperature": 0.2,
        });
        let redact = |text: String| match &self.key {
            Some(key) => key.redact(&text),
            None => text,
        };
        let client = reqwest::blocking::Client::builder().timeout(timeout).build()?;
        let mut request = client.post(format!("{}/chat/completions", self.base)).json(&body);
        if let Some(key) = &self.key {
            request = request.bearer_auth(&key.0);
        }
        let response = request.send().map_err(|e| anyhow::anyhow!(redact(format!("{} did not answer: {e}", self.base))))?;
        let status = response.status();
        let text = response.text().unwrap_or_default();
        if !status.is_success() {
            let message = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_owned));
            bail!(redact(format!("{} answered {status}: {}", self.base, message.unwrap_or_else(|| text.chars().take(300).collect()))));
        }
        let reply: Value = serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("{} sent no JSON: {e}", self.base))?;
        let answer = reply["choices"][0]["message"]["content"].as_str().unwrap_or_default();
        if answer.trim().is_empty() {
            bail!("{} sent an empty answer", self.base);
        }
        Ok(answer.to_owned())
    }

    pub fn is_local(&self) -> bool {
        self.local
    }
}

fn normalize(base: &str) -> String {
    base.trim().trim_end_matches('/').to_owned()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A one-request-at-a-time HTTP server that records each request and answers `status` with `body`.
    pub(crate) fn server(status: u16, body: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap_or(0);
                    }
                    head += &line;
                }
                let mut payload = vec![0; length];
                let _ = reader.read_exact(&mut payload);
                log.lock().unwrap().push(head + &String::from_utf8_lossy(&payload));
                let mut stream = stream;
                let _ = write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (base, seen)
    }

    const ANSWER: &str = r#"{"choices":[{"message":{"content":"{\"noun\":\"WOMAN\"}"}}]}"#;

    #[test]
    fn nothing_is_sent_without_consent() {
        let (base, seen) = server(200, ANSWER);
        let key = Some(ApiKey::new("sk-secret"));
        let err = Endpoint::remote(&base, "gpt", key.clone(), None).unwrap_err();
        assert!(err.downcast_ref::<NoConsent>().is_some());
        let err = Endpoint::remote(&base, "gpt", key.clone(), Some("https://api.other.example/v1")).unwrap_err();
        assert!(err.downcast_ref::<NoConsent>().is_some());
        assert!(seen.lock().unwrap().is_empty());

        let endpoint = Endpoint::remote(&format!("{base}/"), "gpt", key, Some(&base)).unwrap();
        assert_eq!(endpoint.ask("describe", &[vec![0xff, 0xd8]], 10, Duration::from_secs(5)).unwrap(), r#"{"noun":"WOMAN"}"#);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].to_lowercase().contains("authorization: bearer sk-secret"));
        assert!(seen[0].contains("data:image/jpeg;base64,/9g="));
    }

    #[test]
    fn the_key_never_shows() {
        let key = ApiKey::new("sk-live-1234567890");
        let (base, _) = server(401, r#"{"error":{"message":"Incorrect API key provided: sk-live-1234567890"}}"#);
        let endpoint = Endpoint::remote(&base, "gpt", Some(key.clone()), Some(&base)).unwrap();
        let err = endpoint.ask("describe", &[], 10, Duration::from_secs(5)).unwrap_err();
        let shown = format!("{err:#} {err:?} {endpoint:?} {key:?}");
        assert!(!shown.contains("sk-live-1234567890"), "{shown}");
        assert!(shown.contains("Incorrect API key provided: ***"), "{shown}");
    }

    #[test]
    fn a_local_endpoint_needs_no_consent() {
        let endpoint = Endpoint::local(8080);
        assert!(endpoint.is_local());
        assert_eq!(endpoint.base(), "http://127.0.0.1:8080/v1");
    }
}
