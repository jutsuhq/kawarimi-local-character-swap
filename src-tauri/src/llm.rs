//! The app's prompt writer: its settings, llama-server started on demand under the GPU lease (so prep and render
//! evict it), and the describe command behind Create's «описание от LLM» button. State changes go to the UI as
//! `llm-state` events.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use kawarimi_assets::{Store, manifest};
use kawarimi_llm::{Description, Endpoint, Mode, Paths, START_TIMEOUT, Server, Settings, settings};
use serde::Serialize;
use tauri::State;
use tokio::sync::watch;

use crate::AppState;
use crate::gpu::Work;

const LOG_LINES: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LlmState {
    NotInstalled,
    Stopped,
    Starting,
    Ready,
    Failed { error: String, log: String },
}

pub struct Llm {
    store: Store,
    dir: PathBuf,
    settings: Mutex<Settings>,
    server: Mutex<Option<Server>>,
    state: watch::Sender<LlmState>,
}

impl Llm {
    pub fn new(store: Store, dir: PathBuf) -> Self {
        let settings = Settings::load(&dir.join("llm.json"));
        let llm = Self { store, dir, settings: Mutex::new(settings), server: Default::default(), state: watch::Sender::new(LlmState::Stopped) };
        llm.state.send_replace(llm.idle());
        llm
    }

    pub fn subscribe(&self) -> watch::Receiver<LlmState> {
        self.state.subscribe()
    }

    fn idle(&self) -> LlmState {
        if Paths::installed(&self.store) { LlmState::Stopped } else { LlmState::NotInstalled }
    }

    fn log(&self) -> PathBuf {
        self.dir.join("llama-server.log")
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    fn change(&self, edit: impl FnOnce(&mut Settings)) -> Result<()> {
        let mut settings = self.settings.lock().unwrap();
        edit(&mut settings);
        settings.save(&self.dir.join("llm.json"))
    }

    /// A loaded llama-server's endpoint, started (again, if it died) when needed. Blocking; call with the GPU leased
    /// for `Work::Llm`.
    pub fn ensure(&self) -> Result<Endpoint> {
        {
            let mut server = self.server.lock().unwrap();
            if let Some(running) = server.as_mut() {
                if running.is_running() && running.health() {
                    return Ok(running.endpoint());
                }
                *server = None;
            }
            let paths = Paths::from_store(&self.store).inspect_err(|_| {
                self.state.send_replace(LlmState::NotInstalled);
            })?;
            *server = Some(Server::spawn(&paths, &self.log()).inspect_err(|e| {
                self.state.send_replace(LlmState::Failed { error: format!("{e:#}"), log: String::new() });
            })?);
            self.state.send_replace(LlmState::Starting);
        }
        // The lock is let go between polls, so a stop is not stuck behind a model loading.
        let started = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let mut guard = self.server.lock().unwrap();
            let Some(server) = guard.as_mut() else { bail!("llama-server was stopped while it started") };
            let failed = if !server.is_running() {
                "llama-server exited before it was ready"
            } else if server.health() {
                self.state.send_replace(LlmState::Ready);
                return Ok(server.endpoint());
            } else if started.elapsed() > START_TIMEOUT {
                "llama-server did not load the model in time"
            } else {
                continue;
            };
            let log = server.log_tail(LOG_LINES);
            *guard = None;
            self.state.send_replace(LlmState::Failed { error: failed.into(), log: log.clone() });
            bail!("{failed}:\n{log}");
        }
    }

    /// Stops llama-server; its video memory is free when this returns. Blocking.
    pub fn stop(&self) {
        let server = self.server.lock().unwrap().take();
        if let Some(mut server) = server {
            server.stop();
        }
        self.state.send_replace(self.idle());
    }

    fn state(&self) -> LlmState {
        let died = self.server.lock().unwrap().as_mut().is_some_and(|s| !s.is_running());
        if died {
            let log = self.server.lock().unwrap().take().map(|s| s.log_tail(LOG_LINES)).unwrap_or_default();
            self.state.send_replace(LlmState::Failed { error: "llama-server stopped on its own".into(), log });
        }
        self.state.borrow().clone()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmInfo {
    settings: Settings,
    state: LlmState,
    has_key: bool,
    /// Whether the person agreed to send frames to the API address in `settings`.
    consented: bool,
    server: &'static str,
    model: &'static str,
    /// The assets the local mode needs, for download and reinstall.
    assets: [&'static str; 2],
}

fn info(llm: &Llm) -> LlmInfo {
    let settings = llm.settings();
    let consented = settings.api.consented.as_deref().is_some_and(|c| c == settings.api.base_url.trim());
    LlmInfo {
        state: llm.state(),
        has_key: settings::key().ok().flatten().is_some(),
        consented,
        settings,
        server: manifest::LLAMA_SERVER.title,
        model: manifest::QWEN35_9B.title,
        assets: [manifest::LLAMA_SERVER.id, manifest::QWEN35_9B.id],
    }
}

fn shown(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
pub fn llm_info(state: State<'_, AppState>) -> LlmInfo {
    info(&state.llm)
}

/// Saves the settings from the UI. Consent cannot be given this way; see [`llm_consent`].
#[tauri::command]
pub fn llm_settings_set(state: State<'_, AppState>, settings: Settings) -> Result<LlmInfo, String> {
    state.llm.change(|s| s.update(settings)).map_err(shown)?;
    Ok(info(&state.llm))
}

/// The person agreed, in the consent dialog, to send frames to the API address now in the settings.
#[tauri::command]
pub fn llm_consent(state: State<'_, AppState>, granted: bool) -> Result<LlmInfo, String> {
    state.llm.change(|s| if granted { s.consent() } else { s.api.consented = None }).map_err(shown)?;
    Ok(info(&state.llm))
}

#[tauri::command]
pub fn llm_key_set(state: State<'_, AppState>, key: String) -> Result<LlmInfo, String> {
    if key.trim().is_empty() { settings::clear_key() } else { settings::set_key(&key) }.map_err(shown)?;
    Ok(info(&state.llm))
}

/// Starts llama-server by hand, under the GPU lease, so the next prep or render stops it.
#[tauri::command]
pub async fn llm_start(state: State<'_, AppState>) -> Result<(), String> {
    let _lease = state.gpu.lease(Work::Llm, "prompt writer").await.map_err(shown)?;
    let llm = state.llm.clone();
    tauri::async_runtime::spawn_blocking(move || llm.ensure().map(drop)).await.map_err(|e| e.to_string())?.map_err(shown)
}

#[tauri::command]
pub async fn llm_stop(state: State<'_, AppState>) -> Result<(), String> {
    let llm = state.llm.clone();
    tauri::async_runtime::spawn_blocking(move || llm.stop()).await.map_err(|e| e.to_string())
}

/// Describes `character` and the moves in `video`, and writes Wan's prompt from it.
#[tauri::command]
pub async fn llm_describe(state: State<'_, AppState>, video: PathBuf, character: PathBuf) -> Result<Description, String> {
    let settings = state.llm.settings();
    if !settings.enabled {
        return Err("The prompt writer is off; turn it on in Settings.".into());
    }
    let ffmpeg = crate::ffmpeg(&state.store).ok_or("FFmpeg is not installed; download it in Settings.")?;
    let name = format!("describe {}", video.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
    let describe = move |endpoint: &Endpoint| kawarimi_llm::describe(endpoint, &ffmpeg, &video, &character, |_| {});
    match settings.mode {
        Mode::Api => {
            let endpoint = settings.remote(settings::key().map_err(shown)?).map_err(shown)?;
            tauri::async_runtime::spawn_blocking(move || describe(&endpoint)).await.map_err(|e| e.to_string())?.map_err(shown)
        }
        Mode::Local => {
            let _lease = state.gpu.lease(Work::Llm, name).await.map_err(shown)?;
            let llm = state.llm.clone();
            tauri::async_runtime::spawn_blocking(move || describe(&llm.ensure()?)).await.map_err(|e| e.to_string())?.map_err(shown)
        }
    }
}
