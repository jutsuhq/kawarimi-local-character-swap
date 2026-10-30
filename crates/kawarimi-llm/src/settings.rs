//! The prompt writer's settings, a JSON file in the app's folder, and the API key, which lives only in the OS
//! keyring (Windows Credential Manager, the Secret Service on Linux).

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::client::{ApiKey, Endpoint};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Off by default: without the prompt writer the prompt is the template.
    pub enabled: bool,
    pub mode: Mode,
    pub api: Api,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// llama-server on this machine.
    #[default]
    Local,
    /// An OpenAI-compatible service, with the person's key.
    Api,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Api {
    /// Up to `/v1`, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    pub model: String,
    /// The address the person agreed to send frames and character images to; none until they do.
    pub consented: Option<String>,
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?).with_context(|| format!("writing {}", path.display()))
    }

    /// Takes `next` from the UI, which cannot grant consent this way: an unchanged address keeps the consent given,
    /// any other drops it.
    pub fn update(&mut self, next: Settings) {
        let consented = self.api.consented.take().filter(|c| *c == next.api.base_url.trim());
        *self = next;
        self.api.consented = consented;
    }

    /// The person agreed to send frames to the current address.
    pub fn consent(&mut self) {
        self.api.consented = Some(self.api.base_url.trim().to_owned());
    }

    /// The API endpoint, which fails without consent to its address.
    pub fn remote(&self, key: Option<ApiKey>) -> Result<Endpoint> {
        Endpoint::remote(&self.api.base_url, &self.api.model, key, self.api.consented.as_deref())
    }
}

const SERVICE: &str = "kawarimi";
const USER: &str = "llm-api-key";

fn entry() -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, USER)?)
}

pub fn key() -> Result<Option<ApiKey>> {
    match entry()?.get_password() {
        Ok(key) => Ok(Some(ApiKey::new(key))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn set_key(key: &str) -> Result<()> {
    Ok(entry()?.set_password(key.trim())?)
}

pub fn clear_key() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::NoConsent;

    fn api(base_url: &str) -> Settings {
        Settings { enabled: true, mode: Mode::Api, api: Api { base_url: base_url.into(), model: "m".into(), consented: None } }
    }

    #[test]
    fn consent_is_for_one_address() {
        let mut settings = api("https://api.example.com/v1");
        assert!(settings.remote(None).unwrap_err().downcast_ref::<NoConsent>().is_some());
        settings.consent();
        assert!(settings.remote(None).is_ok());

        let mut same = api("https://api.example.com/v1");
        same.api.model = "other".into();
        settings.update(same);
        assert!(settings.remote(None).is_ok(), "a new model keeps the consent");

        let mut forged = api("https://evil.example.com/v1");
        forged.api.consented = Some("https://evil.example.com/v1".into());
        settings.update(forged);
        assert_eq!(settings.api.consented, None, "the UI cannot grant consent by saving settings");
        assert!(settings.remote(None).is_err());
    }

    #[test]
    fn settings_survive_a_restart_without_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("llm.json");
        assert_eq!(Settings::load(&path), Settings::default());
        let mut settings = api("https://api.example.com/v1");
        settings.consent();
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        assert!(!std::fs::read_to_string(&path).unwrap().to_lowercase().contains("key"));
    }

    #[test]
    fn the_key_round_trips_through_the_keyring() {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        // The mock keeps nothing between entries, so this checks the calls, not storage.
        assert!(key().unwrap().is_none());
        set_key(" sk-1 ").unwrap();
        clear_key().unwrap();
    }
}
