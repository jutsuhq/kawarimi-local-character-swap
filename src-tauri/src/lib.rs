pub mod clips;
pub mod comfy;
pub mod gpu;
pub mod jobs;
mod llm;
pub mod own_comfy;
pub mod storage;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kawarimi_assets::{Asset, Group, Status, Store};
use kawarimi_comfy::job::{Job, Jobs, Spec};
use kawarimi_comfy::nvidia::Problem;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

use crate::comfy::{Comfy, ComfyState};
use crate::gpu::{Gpu, Holder, Work};
use crate::jobs::Renders;
use crate::llm::Llm;
use crate::own_comfy::OwnComfy;

pub struct AppState {
    store: Store,
    assets_dir: PathBuf,
    installing: Mutex<HashSet<&'static str>>,
    comfy: Arc<Comfy>,
    /// Prep and render lease the GPU from here.
    gpu: Gpu,
    renders: Arc<Renders>,
    /// The optional prompt writer; its llama-server holds the GPU as `Work::Llm`.
    llm: Arc<Llm>,
    own_comfy: Arc<OwnComfy>,
    jobs_dir: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemInfo {
    app_version: &'static str,
    os: String,
    gpus: Vec<kawarimi_comfy::nvidia::Gpu>,
    /// The Wan weights the first GPU renders with.
    render_model: Option<&'static str>,
    /// What keeps the first GPU from prep and render, if anything.
    gpu_problem: Option<GpuProblem>,
    ffmpeg: Option<String>,
    assets_dir: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetInfo {
    id: &'static str,
    title: &'static str,
    group: Group,
    license: &'static str,
    source: &'static str,
    size: u64,
    status: Status,
}

#[derive(Clone, Serialize)]
struct AssetProgress {
    id: &'static str,
    done: u64,
    total: u64,
}

fn ffmpeg(store: &Store) -> Option<kawarimi_media::Ffmpeg> {
    let installed = store.status(&kawarimi_assets::manifest::FFMPEG) == Status::Installed;
    let dir = installed.then(|| store.dir(&kawarimi_assets::manifest::FFMPEG));
    kawarimi_media::Ffmpeg::locate(dir.as_deref())
}

/// The third-party licenses file the installer ships next to the app.
#[tauri::command]
fn licenses(app: AppHandle) -> Result<String, String> {
    let path = app.path().resolve("THIRD_PARTY_LICENSES.md", tauri::path::BaseDirectory::Resource).map_err(|e| e.to_string())?;
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[tauri::command]
async fn system_info(state: State<'_, AppState>) -> Result<SystemInfo, String> {
    let gpus = tauri::async_runtime::spawn_blocking(kawarimi_comfy::nvidia::gpus).await.map_err(|e| e.to_string())?;
    let render_model = gpus.first().map(|g| kawarimi_comfy::Variant::for_vram(Some(g.memory_mb)).asset.title);
    let gpu_problem = Problem::of(&gpus).map(GpuProblem::from);
    Ok(SystemInfo {
        app_version: env!("CARGO_PKG_VERSION"),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        gpus,
        render_model,
        gpu_problem,
        ffmpeg: ffmpeg(&state.store).and_then(|f| f.version().ok()),
        assets_dir: state.assets_dir.clone(),
    })
}

#[tauri::command]
fn assets_list(state: State<'_, AppState>) -> Vec<AssetInfo> {
    kawarimi_assets::ALL
        .iter()
        .map(|a| AssetInfo {
            id: a.id,
            title: a.title,
            group: a.group,
            license: a.license,
            source: a.source,
            size: a.download_size(),
            status: state.store.status(a),
        })
        .collect()
}

fn asset(id: &str) -> Result<&'static Asset, String> {
    kawarimi_assets::find(id).ok_or_else(|| format!("unknown asset {id}"))
}

#[tauri::command]
async fn asset_install(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let asset = asset(&id)?;
    if !state.installing.lock().unwrap().insert(asset.id) {
        return Err(format!("{} is already downloading", asset.title));
    }
    // Progress goes out at most once per percent, so a large file does not flood the webview.
    let last = Mutex::new(u64::MAX);
    let result = state
        .store
        .install(asset, |done, total| {
            let percent = done * 100 / total.max(1);
            let mut last = last.lock().unwrap();
            if *last != percent {
                *last = percent;
                let _ = app.emit("asset-progress", AssetProgress { id: asset.id, done, total });
            }
        })
        .await;
    state.installing.lock().unwrap().remove(asset.id);
    result.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn asset_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let asset = asset(&id)?;
    if state.installing.lock().unwrap().contains(asset.id) {
        return Err(format!("{} is downloading", asset.title));
    }
    state.store.remove(asset).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn gpu_holder(state: State<'_, AppState>) -> Option<Holder> {
    state.gpu.holder()
}

#[tauri::command]
fn comfy_state(state: State<'_, AppState>) -> ComfyState {
    state.comfy.state()
}

#[tauri::command]
async fn comfy_install(state: State<'_, AppState>) -> Result<(), String> {
    let comfy = state.comfy.clone();
    tauri::async_runtime::spawn_blocking(move || comfy.install_blocking()).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn comfy_start(state: State<'_, AppState>) -> Result<(), String> {
    gpu_ready().map_err(error)?;
    state.comfy.start().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn comfy_stop(state: State<'_, AppState>) -> Result<(), String> {
    let comfy = state.comfy.clone();
    tauri::async_runtime::spawn_blocking(move || comfy.stop()).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn jobs_list(state: State<'_, AppState>) -> Vec<Job> {
    state.renders.list()
}

#[tauri::command]
fn job_create(app: AppHandle, state: State<'_, AppState>, spec: Spec) -> Result<Job, String> {
    // A prepared clip can render on the person's own ComfyUI without a GPU here.
    if matches!(spec.source, kawarimi_comfy::job::Source::Clip { .. }) || state.own_comfy.server().is_none() {
        gpu_ready().map_err(error)?;
    }
    let job = state.renders.create(spec).map_err(|e| format!("{e:#}"))?;
    allow_source(&app, &job);
    Ok(job)
}

#[tauri::command]
fn job_cancel(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.renders.cancel(&id).map_err(|e| format!("{e:#}"))
}

// --- Jobs, Library, storage, ComfyUI settings, own ComfyUI ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GpuProblem {
    #[serde(flatten)]
    problem: Problem,
    message: String,
    advice: &'static str,
    blocks: bool,
}

impl From<Problem> for GpuProblem {
    fn from(problem: Problem) -> Self {
        Self { message: problem.message(), advice: problem.advice(), blocks: problem.blocks(), problem }
    }
}

/// Fails, saying why and what to do, when this machine cannot prep or render.
fn gpu_ready() -> anyhow::Result<()> {
    match Problem::of(&kawarimi_comfy::nvidia::gpus()) {
        Some(p) if p.blocks() => anyhow::bail!("{} {}", p.message(), p.advice()),
        _ => Ok(()),
    }
}

/// Lets the webview show a job's source clip (its own folder is allowed whole at start).
fn allow_source(app: &AppHandle, job: &Job) {
    let scope = app.asset_protocol_scope();
    let _ = match &job.spec.source {
        kawarimi_comfy::job::Source::Clip { path, .. } => scope.allow_file(path),
        kawarimi_comfy::job::Source::Prepared { folder } => scope.allow_directory(folder, false),
    };
}

fn error(e: anyhow::Error) -> String {
    format!("{e:#}")
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(error)
}

#[tauri::command]
fn job_retry(state: State<'_, AppState>, id: String) -> Result<Job, String> {
    state.renders.retry(&id).map_err(error)
}

#[tauri::command]
async fn job_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let renders = state.renders.clone();
    blocking(move || renders.remove(&id)).await
}

fn job_result(state: &AppState, id: &str) -> anyhow::Result<PathBuf> {
    match state.renders.jobs().load(id)?.end {
        Some(kawarimi_comfy::job::End::Done { result }) if result.is_file() => Ok(result),
        _ => anyhow::bail!("the job has no finished video"),
    }
}

/// Plays a finished video in the system's player.
#[tauri::command]
fn job_open(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let result = job_result(&state, &id).map_err(error)?;
    tauri_plugin_opener::open_path(result, None::<&str>).map_err(|e| e.to_string())
}

/// Shows a job's finished video, or else its folder, in the file manager.
#[tauri::command]
fn job_reveal(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let jobs = state.renders.jobs();
    jobs.load(&id).map_err(error)?;
    let path = job_result(&state, &id).unwrap_or_else(|_| jobs.dir(&id));
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn folder_reveal(state: State<'_, AppState>, which: String) -> Result<(), String> {
    let path = match which.as_str() {
        "jobs" => state.jobs_dir.clone(),
        "models" => state.assets_dir.clone(),
        "comfy" => state.comfy.root().to_path_buf(),
        "comfyLogs" => state.comfy.root().join("logs"),
        other => return Err(format!("unknown folder {other}")),
    };
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
}

fn comfy_data(state: &AppState) -> Option<PathBuf> {
    state.comfy.output().ok().and_then(|o| o.parent().map(PathBuf::from))
}

#[tauri::command]
async fn storage_usage(state: State<'_, AppState>) -> Result<storage::Usage, String> {
    let (jobs, jobs_dir, data) = (state.renders.jobs().clone(), state.jobs_dir.clone(), comfy_data(&state));
    blocking(move || Ok(storage::usage(&jobs, &jobs_dir, data.as_deref()))).await
}

#[tauri::command]
async fn storage_installed(state: State<'_, AppState>) -> Result<storage::Installed, String> {
    let (assets, comfy) = (state.assets_dir.clone(), state.comfy.root().to_path_buf());
    blocking(move || Ok(storage::installed(&assets, &comfy))).await
}

#[tauri::command]
async fn storage_clean(state: State<'_, AppState>) -> Result<u64, String> {
    let (jobs, data) = (state.renders.jobs().clone(), comfy_data(&state));
    blocking(move || storage::clean(&jobs, data.as_deref())).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ComfyInfo {
    comfyui: &'static str,
    python: &'static str,
    torch: Option<&'static str>,
    frontend: Option<&'static str>,
    root: PathBuf,
}

#[tauri::command]
fn comfy_info(state: State<'_, AppState>) -> ComfyInfo {
    ComfyInfo {
        comfyui: kawarimi_assets::manifest::COMFYUI.title,
        python: kawarimi_comfy::PYTHON,
        torch: kawarimi_comfy::locked("torch"),
        frontend: kawarimi_comfy::locked("comfyui-frontend-package"),
        root: state.comfy.root().to_path_buf(),
    }
}

#[tauri::command]
async fn comfy_reinstall(state: State<'_, AppState>) -> Result<(), String> {
    let comfy = state.comfy.clone();
    blocking(move || comfy.reinstall_blocking()).await
}

#[tauri::command]
fn own_comfy_get(state: State<'_, AppState>) -> own_comfy::View {
    state.own_comfy.view()
}

#[tauri::command]
fn own_comfy_set(state: State<'_, AppState>, enabled: bool, url: String) -> Result<own_comfy::View, String> {
    state.own_comfy.set(enabled, url).map_err(error)
}

#[tauri::command]
async fn own_comfy_token(state: State<'_, AppState>, token: String) -> Result<own_comfy::View, String> {
    let own = state.own_comfy.clone();
    blocking(move || own.set_token(&token)).await
}

#[tauri::command]
async fn own_comfy_check(state: State<'_, AppState>) -> Result<kawarimi_comfy::check::Check, String> {
    let own = state.own_comfy.clone();
    blocking(move || own.check()).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let dir = |var, name| std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| data.join(name));
            let assets_dir = dir("KAWARIMI_ASSETS", "assets");
            let comfy = Comfy::new(Store::new(&assets_dir), dir("KAWARIMI_COMFY", "comfy"));
            let mut changes = comfy.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while changes.changed().await.is_ok() {
                    let state = changes.borrow_and_update().clone();
                    let _ = handle.emit("comfy-state", state);
                }
            });
            let llm = Arc::new(Llm::new(Store::new(&assets_dir), data.join("llm")));
            let mut llm_changes = llm.subscribe();
            let llm_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while llm_changes.changed().await.is_ok() {
                    let state = llm_changes.borrow_and_update().clone();
                    let _ = llm_handle.emit("llm-state", state);
                }
            });
            let freed = comfy.clone();
            let stopped = llm.clone();
            let gpu = Gpu::new(move |last| match last {
                Work::Render => freed.free(),
                // Prep's sessions are gone once prepare() returns.
                Work::Prep => Ok(()),
                Work::Llm => {
                    stopped.stop();
                    Ok(())
                }
            })
            .with_lock(kawarimi_comfy::locks::GpuLock::path(&data));
            let emitter = app.handle().clone();
            let jobs_dir = dir("KAWARIMI_JOBS", "jobs");
            let own_comfy = Arc::new(OwnComfy::load(data.join("own-comfy.json")));
            // Create screen: render progress by windows reads ComfyUI's log from where each job reached it.
            let progress = Arc::new(clips::Progress::new(dir("KAWARIMI_COMFY", "comfy").join("logs").join("server.log")));
            let observer = progress.clone();
            let renders = Renders::start(
                Jobs::new(&jobs_dir),
                jobs::Backend { store: Store::new(&assets_dir), gpu: gpu.clone(), comfy: comfy.clone(), own: own_comfy.clone() },
                move |job| {
                    observer.observe(job);
                    let _ = emitter.emit("job-state", job);
                },
            );
            std::fs::create_dir_all(&jobs_dir)?;
            app.asset_protocol_scope().allow_directory(&jobs_dir, true)?;
            for job in renders.list() {
                allow_source(app.handle(), &job);
            }
            // Create screen: clips, their preps and characters.
            let clips = clips::Clips::new(dir("KAWARIMI_CLIPS", "clips"), Store::new(&assets_dir), gpu.clone(), renders.clone(), app.handle().clone());
            app.manage(clips);
            app.manage(progress);
            app.manage(AppState { store: Store::new(&assets_dir), assets_dir, installing: Default::default(), comfy, gpu, renders, own_comfy, jobs_dir, llm });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            licenses,
            system_info,
            assets_list,
            asset_install,
            asset_remove,
            comfy_state,
            comfy_install,
            comfy_start,
            comfy_stop,
            gpu_holder,
            jobs_list,
            job_create,
            job_cancel,
            llm::llm_info,
            llm::llm_settings_set,
            llm::llm_consent,
            llm::llm_key_set,
            llm::llm_start,
            llm::llm_stop,
            llm::llm_describe,
            // Jobs, Library, storage, ComfyUI settings, own ComfyUI
            job_retry,
            job_remove,
            job_open,
            job_reveal,
            folder_reveal,
            storage_usage,
            storage_installed,
            storage_clean,
            comfy_info,
            comfy_reinstall,
            own_comfy_get,
            own_comfy_set,
            own_comfy_token,
            own_comfy_check,
            // Create screen
            clips::clip_probe,
            clips::clip_playable,
            clips::clip_strip,
            clips::clips_list,
            clips::clip_create,
            clips::clip_remove,
            clips::clip_prepare,
            clips::clip_cancel,
            clips::clip_fix,
            clips::clip_set_character,
            clips::clip_change_character,
            clips::clip_template,
            clips::clip_estimate,
            clips::clip_render,
            clips::render_progress
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<AppState>().comfy.stop();
                app.state::<AppState>().llm.stop();
            }
        });
}
