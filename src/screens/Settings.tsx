import { useEffect, useState } from "react";
import { ExternalLink } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, formatBytes, type AssetInfo, type AssetProgress, type SystemInfo } from "../lib/api";
import { Button, Card, Notice, PageHeader, Pill, Progress, SectionLabel } from "../components/ui";
import { ComfySettings } from "./settings/ComfySettings";
import { LlmSettings } from "./settings/LlmSettings";
import { StorageSettings } from "./settings/StorageSettings";
import { Acknowledgements } from "./settings/Acknowledgements";

const GROUP_LABEL: Record<AssetInfo["group"], string> = {
  tool: "Tool",
  tracking: "Tracking",
  people: "People",
  faces: "Faces",
  shots: "Shot cuts",
  cutout: "Cutout",
  render: "Render",
  llm: "Prompt writer",
};

type Tab = "general" | "comfy" | "llm" | "storage";
const TABS: [Tab, string][] = [
  ["general", "GPU and models"],
  ["comfy", "ComfyUI"],
  ["llm", "LLM"],
  ["storage", "Storage"],
];

/** Settings, as in Swapjutsu: a tab switcher over cards of rows. */
export function Settings({ system, onChanged }: { system: SystemInfo | null; onChanged: () => void }) {
  const [tab, setTab] = useState<Tab>("general");
  return (
    <div className="flex h-full flex-col">
      <PageHeader title="Settings" subtitle="Your GPU, the models and tools Kawarimi downloads only when you ask, ComfyUI, and what is kept on disk." />
      <div className="flex pb-6">
        <div className="flex rounded-full border border-stroke bg-well p-1">
          {TABS.map(([id, label]) => (
            <button
              key={id}
              onClick={() => setTab(id)}
              className={`rounded-full px-3.5 py-1.5 text-body font-semibold transition-colors ${
                tab === id ? "bg-selection text-text" : "text-muted hover:text-text"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto pr-2">
        <div className="flex max-w-[860px] flex-col gap-6 pb-8">
          {tab === "general" && <General system={system} onChanged={onChanged} />}
          {tab === "comfy" && <ComfySettings system={system} />}
          {tab === "llm" && <LlmSettings />}
          {tab === "storage" && <StorageSettings />}
        </div>
      </div>
    </div>
  );
}

function General({ system, onChanged }: { system: SystemInfo | null; onChanged: () => void }) {
  const [assets, setAssets] = useState<AssetInfo[]>([]);
  const [progress, setProgress] = useState<Record<string, AssetProgress>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});

  const refresh = () => api.assets().then(setAssets);

  useEffect(() => {
    refresh();
    const unlisten = api.onAssetProgress((p) => setProgress((all) => ({ ...all, [p.id]: p })));
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const run = async (id: string, action: (id: string) => Promise<void>) => {
    setErrors(({ [id]: _, ...rest }) => rest);
    setProgress((all) => ({ ...all, [id]: { id, done: 0, total: 1 } }));
    try {
      await action(id);
    } catch (e) {
      setErrors((all) => ({ ...all, [id]: String(e) }));
    } finally {
      setProgress(({ [id]: _, ...rest }) => rest);
      await refresh();
      onChanged();
    }
  };

  const missing = assets.filter((a) => a.status === "missing");

  const problem = system?.gpuProblem;
  return (
    <>
      <div>
        <SectionLabel>GPU</SectionLabel>
        <Card className="p-5">
          {system?.gpus.length ? (
            system.gpus.map((gpu) => (
              <div key={gpu.name} className="flex items-center justify-between">
                <div>
                  <div className="text-row font-semibold">{gpu.name}</div>
                  <div className="text-note text-muted">
                    {Math.round(gpu.memoryMb / 1024)} GB · driver {gpu.driver}
                  </div>
                </div>
                <Pill tone={problem?.blocks ? "bad" : problem ? "warn" : "good"}>{problem?.blocks ? "Not usable" : "CUDA"}</Pill>
              </div>
            ))
          ) : null}
          {problem && (
            <div className={system?.gpus.length ? "mt-3" : ""}>
              <Notice tone={problem.blocks ? "bad" : "warn"} title={problem.message}>
                {problem.advice}
              </Notice>
            </div>
          )}
          {system?.renderModel && (
            <div className="mt-3 text-note text-muted">Renders with {system.renderModel}</div>
          )}
        </Card>
      </div>

      <div>
        <SectionLabel
          trailing={missing.length ? `${formatBytes(missing.reduce((s, a) => s + a.size, 0))} to download` : "All installed"}
        >
          Models and tools
        </SectionLabel>
        <Card className="divide-y divide-stroke">
          {assets.map((asset) => {
            const p = progress[asset.id];
            return (
              <div key={asset.id} className="flex items-center gap-4 px-5 py-4">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-row font-semibold">{asset.title}</span>
                    <Pill>{GROUP_LABEL[asset.group]}</Pill>
                  </div>
                  <div className="mt-0.5 flex items-center gap-1 text-note text-muted">
                    {formatBytes(asset.size)} · {asset.license} ·
                    <button className="inline-flex items-center gap-1 hover:text-text" onClick={() => openUrl(asset.source)}>
                      source <ExternalLink size={11} />
                    </button>
                  </div>
                  {p && (
                    <div className="mt-2 flex items-center gap-3">
                      <Progress value={p.total ? p.done / p.total : 0} />
                      <span className="w-24 text-right text-meta text-faint tabular-nums">
                        {formatBytes(p.done)} / {formatBytes(p.total)}
                      </span>
                    </div>
                  )}
                  {errors[asset.id] && <div className="mt-1 text-note text-danger">{errors[asset.id]}</div>}
                </div>
                {asset.status === "installed" ? (
                  <div className="flex items-center gap-3">
                    <Pill tone="good">Installed</Pill>
                    <Button variant="link" onClick={() => run(asset.id, api.remove)}>
                      Remove
                    </Button>
                  </div>
                ) : (
                  <Button disabled={!!p} onClick={() => run(asset.id, api.install)}>
                    {p ? "Downloading…" : "Download"}
                  </Button>
                )}
              </div>
            );
          })}
        </Card>
        <p className="mt-2 px-1 text-note text-faint">
          Every file is pinned to an exact revision and checked against its SHA-256 before it is used. Stored in{" "}
          {system?.assetsDir ?? "…"}.
        </p>
      </div>

      <div>
        <SectionLabel>About</SectionLabel>
        <Card className="p-5 text-note text-muted">
          Kawarimi {system?.appVersion} · {system?.os} · {system?.ffmpeg ?? "FFmpeg not installed"}
          <Acknowledgements />
        </Card>
      </div>
    </>
  );
}
