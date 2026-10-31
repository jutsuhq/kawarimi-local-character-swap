import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Gpu = { name: string; memoryMb: number; driver: string };

export type SystemInfo = {
  appVersion: string;
  os: string;
  gpus: Gpu[];
  renderModel: string | null;
  gpuProblem: GpuProblem | null;
  ffmpeg: string | null;
  assetsDir: string;
};

export type AssetGroup = "tool" | "tracking" | "people" | "faces" | "shots" | "cutout" | "render" | "llm";

export type AssetInfo = {
  id: string;
  title: string;
  group: AssetGroup;
  license: string;
  source: string;
  size: number;
  status: "installed" | "missing";
};

export type AssetProgress = { id: string; done: number; total: number };

export type ComfyState =
  | { state: "notInstalled" }
  | { state: "installing"; step: string }
  | { state: "stopped" }
  | { state: "starting" }
  | { state: "ready"; url: string }
  | { state: "failed"; error: string; log: string };

export const api = {
  licenses: () => invoke<string>("licenses"),
  systemInfo: () => invoke<SystemInfo>("system_info"),
  assets: () => invoke<AssetInfo[]>("assets_list"),
  install: (id: string) => invoke<void>("asset_install", { id }),
  remove: (id: string) => invoke<void>("asset_remove", { id }),
  onAssetProgress: (handler: (p: AssetProgress) => void): Promise<UnlistenFn> =>
    listen<AssetProgress>("asset-progress", (e) => handler(e.payload)),
  comfyState: () => invoke<ComfyState>("comfy_state"),
  comfyInstall: () => invoke<void>("comfy_install"),
  comfyStart: () => invoke<void>("comfy_start"),
  comfyStop: () => invoke<void>("comfy_stop"),
  onComfyState: (handler: (s: ComfyState) => void): Promise<UnlistenFn> =>
    listen<ComfyState>("comfy-state", (e) => handler(e.payload)),
};

// Prompt writer (LLM)

export type LlmState =
  | { state: "notInstalled" }
  | { state: "stopped" }
  | { state: "starting" }
  | { state: "ready" }
  | { state: "failed"; error: string; log: string };

export type LlmSettings = {
  enabled: boolean;
  mode: "local" | "api";
  api: { baseUrl: string; model: string; consented: string | null };
};

export type LlmInfo = {
  settings: LlmSettings;
  state: LlmState;
  hasKey: boolean;
  consented: boolean;
  server: string;
  model: string;
  assets: [string, string];
};

export type LlmDescription = {
  character: { noun: string; pronoun: string; appearance: string; wardrobe: string; traits: string; moving: string };
  motion: { camera: string; shot: string; beats: string; gaze: string; scene: string; props: string } | null;
  prompt: string;
  source: string;
};

export const llm = {
  info: () => invoke<LlmInfo>("llm_info"),
  setSettings: (settings: LlmSettings) => invoke<LlmInfo>("llm_settings_set", { settings }),
  consent: (granted: boolean) => invoke<LlmInfo>("llm_consent", { granted }),
  setKey: (key: string) => invoke<LlmInfo>("llm_key_set", { key }),
  start: () => invoke<void>("llm_start"),
  stop: () => invoke<void>("llm_stop"),
  describe: (video: string, character: string) => invoke<LlmDescription>("llm_describe", { video, character }),
  onState: (handler: (s: LlmState) => void): Promise<UnlistenFn> =>
    listen<LlmState>("llm-state", (e) => handler(e.payload)),
};

export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

// --- Jobs, Library, storage, ComfyUI settings, own ComfyUI ---

export type GpuProblem = (
  | { kind: "noGpu" }
  | { kind: "oldDriver"; driver: string }
  | { kind: "lowVram"; memoryMb: number }
) & { message: string; advice: string; blocks: boolean };

export type JobSource =
  | { kind: "clip"; path: string; start: number; duration: number | null; targets: number[] | null }
  | { kind: "prepared"; folder: string };

export type JobStep = "prep" | "upload" | "queued" | "rendering" | "downloading" | "finishing";

export type JobEnd = { status: "done"; result: string } | { status: "failed"; reason: string } | { status: "cancelled" };

export type Job = {
  id: string;
  created: number;
  spec: { source: JobSource; character: string; settings: { quality: "draft" | "final"; seed: number; prompt: string } };
  step: JobStep;
  end: JobEnd | null;
  submissions: number;
  frames: number | null;
  detail: string;
};

export type GpuHolder = { work: "prep" | "render"; name: string };

export type StorageUsage = {
  jobsDir: string;
  results: number;
  resultCount: number;
  jobFiles: number;
  cache: number;
};

export type StorageInstalled = { models: number; modelsDir: string; comfy: number; comfyDir: string };

export type ComfyInfo = { comfyui: string; python: string; torch: string | null; frontend: string | null; root: string };

export type OwnComfy = { enabled: boolean; url: string; checked: string | null; hasToken: boolean };

export type OwnComfyCheck = { vram: number; model: string; missingNodes: string[]; missingModels: string[] };

export type Folder = "jobs" | "models" | "comfy" | "comfyLogs";

export const jobsApi = {
  list: () => invoke<Job[]>("jobs_list"),
  cancel: (id: string) => invoke<void>("job_cancel", { id }),
  retry: (id: string) => invoke<Job>("job_retry", { id }),
  remove: (id: string) => invoke<void>("job_remove", { id }),
  open: (id: string) => invoke<void>("job_open", { id }),
  reveal: (id: string) => invoke<void>("job_reveal", { id }),
  gpuHolder: () => invoke<GpuHolder | null>("gpu_holder"),
  onJob: (handler: (job: Job) => void): Promise<UnlistenFn> => listen<Job>("job-state", (e) => handler(e.payload)),
};

export const storageApi = {
  usage: () => invoke<StorageUsage>("storage_usage"),
  installed: () => invoke<StorageInstalled>("storage_installed"),
  clean: () => invoke<number>("storage_clean"),
  reveal: (which: Folder) => invoke<void>("folder_reveal", { which }),
};

export const comfyApi = {
  info: () => invoke<ComfyInfo>("comfy_info"),
  reinstall: () => invoke<void>("comfy_reinstall"),
  own: () => invoke<OwnComfy>("own_comfy_get"),
  setOwn: (enabled: boolean, url: string) => invoke<OwnComfy>("own_comfy_set", { enabled, url }),
  setToken: (token: string) => invoke<OwnComfy>("own_comfy_token", { token }),
  check: () => invoke<OwnComfyCheck>("own_comfy_check"),
};

/** The clip a job renders, as a file the webview can play. */
export function sourceVideo(job: Job): string {
  return job.spec.source.kind === "clip" ? job.spec.source.path : `${job.spec.source.folder}/normalized.mp4`;
}

export function jobName(job: Job): string {
  const path = job.spec.source.kind === "clip" ? job.spec.source.path : job.spec.source.folder;
  return path.split(/[\\/]/).filter(Boolean).pop() ?? job.id;
}


// Create: clips, their prep, characters, prompts and renders (src-tauri/src/clips.rs).

export type Quality = "draft" | "final";

export type Box = { x0: number; y0: number; x1: number; y1: number };

export type Identity = {
  id: number;
  tracklets: number[];
  framesSeen: number;
  onScreen: number;
  thumbnail: string;
  face: { frame: number; bounds: Box; score: number; thumbnail: string } | null;
};

export type Manifest = {
  video: { width: number; height: number; fps: number; duration: number; hasAudio: boolean };
  frames: number;
  shots: { start: number; end: number }[];
  tracklets: { id: number; shot: number; first: number; last: number; framesSeen: number; seed: Box }[];
  identities: Identity[];
  closeCalls: [number, number, number][];
  targets: number[];
};

export type Character = { image: string; cutout: string | null; useCutout: boolean; prompt: string | null };

export type RenderJob = {
  id: string;
  created: number;
  spec: { source: { kind: string; folder?: string }; character: string; settings: { quality: Quality; prompt: string } };
  step: "prep" | "upload" | "queued" | "rendering" | "downloading" | "finishing";
  end: { status: "done"; result: string } | { status: "failed"; reason: string } | { status: "cancelled" } | null;
  frames: number | null;
  detail: string;
};

export type ClipView = {
  id: string;
  created: number;
  source: string;
  start: number;
  duration: number;
  targets: number[];
  prep: string | null;
  error: string | null;
  characters: Record<string, Character>;
  renders: { job: string; person: number }[];
  dir: string;
  prepDir: string | null;
  manifest: Manifest | null;
  running: { stage: string; fraction: number } | null;
  jobs: RenderJob[];
};

export type SourceInfo = { path: string; duration: number; width: number; height: number; fps: number; hasAudio: boolean };

export type Estimate = {
  width: number;
  height: number;
  frames: number;
  windows: number[];
  steps: number;
  model: string;
  seconds: number;
  vramGb: number;
  /** This machine's RAM against the render's; null when renders go to the person's own ComfyUI. */
  ram: { verdict: "fits" | "pages" | "short"; need: number; measured: boolean; free: number; headroom: number; message: string | null } | null;
};

export type RenderProgress = { windows: number; done: number; step: number; steps: number };

/** Points on one frame, in the normalised clip's pixels; labels 1 on the person, 0 off them. */
export type Fix = { frame: number; points: [number, number][]; labels: number[]; replaces: number[] };

export type CharacterChange = { useCutout?: boolean; prompt?: string; resetPrompt?: boolean; remove?: boolean };

export const clipApi = {
  probe: (path: string) => invoke<SourceInfo>("clip_probe", { path }),
  playable: (path: string) => invoke<string>("clip_playable", { path }),
  strip: (path: string, duration: number) => invoke<string>("clip_strip", { path, duration }),
  list: () => invoke<ClipView[]>("clips_list"),
  create: (source: string, start: number, duration: number) => invoke<ClipView>("clip_create", { source, start, duration }),
  remove: (id: string) => invoke<void>("clip_remove", { id }),
  prepare: (id: string, targets?: number[]) => invoke<ClipView>("clip_prepare", { id, targets }),
  cancel: (id: string) => invoke<void>("clip_cancel", { id }),
  fix: (id: string, fix: Fix, person: number | null) => invoke<ClipView>("clip_fix", { id, fix, person }),
  setCharacter: (id: string, person: number, image: string) =>
    invoke<ClipView>("clip_set_character", { id, person, image }),
  changeCharacter: (id: string, person: number, change: CharacterChange) =>
    invoke<ClipView>("clip_change_character", { id, person, change }),
  template: (id: string, person: number) => invoke<string>("clip_template", { id, person }),
  estimate: (id: string, quality: Quality) => invoke<Estimate>("clip_estimate", { id, quality }),
  render: (id: string, person: number, quality: Quality) => invoke<ClipView>("clip_render", { id, person, quality }),
  cancelRender: (id: string) => invoke<void>("job_cancel", { id }),
  renderProgress: (job: string) => invoke<RenderProgress | null>("render_progress", { job }),
  onClip: (handler: (c: ClipView) => void): Promise<UnlistenFn> =>
    listen<ClipView>("clip-state", (e) => handler(e.payload)),
  onJob: (handler: (j: RenderJob) => void): Promise<UnlistenFn> =>
    listen<RenderJob>("job-state", (e) => handler(e.payload)),
};

/** A file under `dir`, with the separator `dir` uses, so the asset scope matches it. */
export function joinPath(dir: string, ...parts: string[]): string {
  const sep = dir.includes("\\") ? "\\" : "/";
  return [dir.replace(/[\/]+$/, ""), ...parts.map((p) => p.replace(/[\/]/g, sep))].join(sep);
}

/** "0:04.5" */
export function timecode(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  return `${m}:${s.toFixed(1).padStart(4, "0")}`;
}
