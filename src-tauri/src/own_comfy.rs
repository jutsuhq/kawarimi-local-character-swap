//! Own ComfyUI: a server the person runs (this PC, another one on the network, a rented GPU) that renders instead of
//! the app's own. The address and switch are in `own-comfy.json` in the app's data folder; the token, for a ComfyUI
//! behind a proxy, is in the OS keyring and never in a file or a log.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use kawarimi_comfy::Client;
use kawarimi_comfy::check::{self, Check};
use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "dev.kawarimi.app";
const KEYRING_USER: &str = "own-comfy-token";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub url: String,
    /// The address whose check passed; cleared when the address or the token changes.
    pub checked: Option<String>,
}

/// What the UI shows: the settings, and whether a token is saved (never the token).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    #[serde(flatten)]
    pub settings: Settings,
    pub has_token: bool,
}

pub struct OwnComfy {
    file: PathBuf,
    settings: Mutex<Settings>,
}

/// "192.168.1.20:8188" or "https://gpu.example/comfy/" as a base URL without the trailing slash; None for anything
/// that is not http(s) with a host.
pub fn parse(text: &str) -> Option<String> {
    let text = text.trim();
    let text = if text.contains("://") { text.to_string() } else { format!("http://{text}") };
    let (scheme, rest) = text.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let host = rest.split(['/', '?', '#']).next()?;
    if !matches!(scheme.as_str(), "http" | "https") || host.is_empty() || host.starts_with(':') || host.contains(char::is_whitespace) {
        return None;
    }
    let path = rest.split(['?', '#']).next()?;
    Some(format!("{scheme}://{}", path.trim_end_matches('/')))
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).context("opening the OS keyring")
}

pub fn token() -> Option<String> {
    entry().ok()?.get_password().ok().filter(|t| !t.is_empty())
}

impl OwnComfy {
    pub fn load(file: PathBuf) -> Self {
        let settings = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { file, settings: Mutex::new(settings) }
    }

    pub fn view(&self) -> View {
        View { settings: self.settings.lock().unwrap().clone(), has_token: token().is_some() }
    }

    /// Saves the switch and the address; a changed address needs a new check.
    pub fn set(&self, enabled: bool, url: String) -> Result<View> {
        {
            let mut settings = self.settings.lock().unwrap();
            if settings.url != url {
                settings.checked = None;
            }
            settings.enabled = enabled;
            settings.url = url;
            self.save(&settings)?;
        }
        Ok(self.view())
    }

    /// Saves the token in the keyring, or removes it when empty; either way the server is checked again.
    pub fn set_token(&self, token: &str) -> Result<View> {
        let entry = entry()?;
        match token.trim() {
            "" => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(e).context("removing the token from the OS keyring"),
            },
            token => entry.set_password(token).context("saving the token in the OS keyring")?,
        }
        {
            let mut settings = self.settings.lock().unwrap();
            settings.checked = None;
            self.save(&settings)?;
        }
        Ok(self.view())
    }

    fn save(&self, settings: &Settings) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        kawarimi_comfy::job::atomically(&self.file, |part| Ok(std::fs::write(part, serde_json::to_vec_pretty(settings)?)?))
    }

    /// A client for the server, with the saved token.
    fn client(url: &str) -> Result<Client> {
        Client::with_token(url, token().as_deref())
    }

    /// Asks the server at the saved address for its GPU and nodes. Blocking.
    pub fn check(&self) -> Result<Check> {
        let url = self.settings.lock().unwrap().url.clone();
        let base = parse(&url).context("the address is not an http(s) address")?;
        let client = Self::client(&base)?;
        let result = client.vram().and_then(|vram| check::evaluate(&client.object_info()?, vram));
        let result = result.with_context(|| format!("could not reach ComfyUI at {base}"));
        let mut settings = self.settings.lock().unwrap();
        settings.checked = result.as_ref().ok().filter(|c| c.is_ready()).map(|_| base);
        self.save(&settings)?;
        result
    }

    /// The server renders go to: Some while the mode is on and its check passed for the saved address.
    pub fn server(&self) -> Option<String> {
        let settings = self.settings.lock().unwrap();
        let base = parse(&settings.url)?;
        (settings.enabled && settings.checked.as_deref() == Some(base.as_str())).then_some(base)
    }

    /// The client for renders, when the mode is on.
    pub fn render_client(&self) -> Option<Result<Client>> {
        self.server().map(|url| Self::client(&url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_parsed_like_swapjutsu() {
        assert_eq!(parse("192.168.1.20:8188").as_deref(), Some("http://192.168.1.20:8188"));
        assert_eq!(parse(" https://gpu.example/comfy/ ").as_deref(), Some("https://gpu.example/comfy"));
        assert_eq!(parse("HTTP://box:8188/?x=1").as_deref(), Some("http://box:8188"));
        assert_eq!(parse(""), None);
        assert_eq!(parse("ftp://box"), None);
        assert_eq!(parse("http://"), None);
    }

    #[test]
    fn a_changed_address_needs_a_new_check() {
        let dir = tempfile::tempdir().unwrap();
        let own = OwnComfy::load(dir.path().join("own-comfy.json"));
        *own.settings.lock().unwrap() = Settings { enabled: true, url: "box:8188".into(), checked: Some("http://box:8188".into()) };
        assert_eq!(own.server().as_deref(), Some("http://box:8188"));
        own.set(true, "other:8188".into()).unwrap();
        assert_eq!(own.server(), None);
        let saved = OwnComfy::load(dir.path().join("own-comfy.json"));
        assert_eq!(saved.settings.lock().unwrap().url, "other:8188");
        assert!(!std::fs::read_to_string(dir.path().join("own-comfy.json")).unwrap().contains("token"));
    }
}
