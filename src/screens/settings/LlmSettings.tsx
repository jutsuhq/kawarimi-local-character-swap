import { useEffect, useState } from "react";
import { api, formatBytes, llm, type AssetProgress, type LlmInfo, type LlmSettings, type LlmState } from "../../lib/api";
import { Button, Card, Pill, Progress, SectionLabel } from "../../components/ui";

const STATE_LABEL: Record<LlmState["state"], [string, "neutral" | "good" | "warn" | "bad"]> = {
  notInstalled: ["Not downloaded", "neutral"],
  stopped: ["Stopped", "neutral"],
  starting: ["Loading", "warn"],
  ready: ["Running", "good"],
  failed: ["Failed", "bad"],
};

const field = "w-full rounded-field border border-stroke bg-well px-3 py-2 text-body outline-none focus:border-accent";

/** Settings › Prompt writer: on or off, local llama-server or an OpenAI-compatible API, and the consent to send frames. */
export function LlmSettings() {
  const [info, setInfo] = useState<LlmInfo | null>(null);
  const [draft, setDraft] = useState<LlmSettings | null>(null);
  const [key, setKey] = useState("");
  const [progress, setProgress] = useState<Record<string, AssetProgress>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = (next: LlmInfo) => {
    setInfo(next);
    setDraft(next.settings);
  };

  useEffect(() => {
    llm.info().then(load);
    const state = llm.onState((s) => setInfo((i) => (i ? { ...i, state: s } : i)));
    const bytes = api.onAssetProgress((p) => setProgress((all) => ({ ...all, [p.id]: p })));
    return () => {
      state.then((f) => f());
      bytes.then((f) => f());
    };
  }, []);

  if (!info || !draft) return null;

  const run = async (action: () => Promise<unknown>) => {
    setError(null);
    setBusy(true);
    try {
      await action();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setProgress({});
      load(await llm.info());
    }
  };

  const save = (next: LlmSettings) => {
    setDraft(next);
    run(() => llm.setSettings(next));
  };

  const download = async (again: boolean) => {
    await llm.stop();
    for (const id of info.assets) {
      if (again) await api.remove(id);
      await api.install(id);
    }
  };

  const [label, tone] = STATE_LABEL[info.state.state];
  const downloading = Object.values(progress).filter((p) => info.assets.includes(p.id));
  const loaded = info.state.state === "starting" || info.state.state === "ready";
  const local = draft.mode === "local";

  return (
    <div>
      <SectionLabel>Prompt writer</SectionLabel>
      <Card className="divide-y divide-stroke">
        <label className="flex cursor-pointer items-center gap-4 px-5 py-4">
          <div className="min-w-0 flex-1">
            <div className="text-row font-semibold">Describe clips with an LLM</div>
            <div className="mt-0.5 text-note text-muted">
              Writes the render prompt from the character and a few frames of the clip. Off: the prompt is the template.
            </div>
          </div>
          <input
            type="checkbox"
            className="size-5 accent-[var(--color-accent)]"
            checked={draft.enabled}
            onChange={(e) => save({ ...draft, enabled: e.target.checked })}
          />
        </label>

        <div className="flex gap-2 px-5 py-4">
          {(["local", "api"] as const).map((mode) => (
            <Button key={mode} variant={draft.mode === mode ? "primary" : "glass"} onClick={() => save({ ...draft, mode })}>
              {mode === "local" ? "On this GPU" : "OpenAI-compatible API"}
            </Button>
          ))}
        </div>

        {local ? (
          <div className="px-5 py-4">
            <div className="flex items-center gap-4">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-row font-semibold">llama-server</span>
                  <Pill tone={tone}>{label}</Pill>
                </div>
                <div className="mt-0.5 text-note text-muted">
                  {info.server} · {info.model}. Starts when a description is asked for and stops before prep or render, so
                  its video memory is free for them. Nothing leaves this computer.
                </div>
              </div>
              {info.state.state === "notInstalled" ? (
                <Button disabled={busy} onClick={() => run(() => download(false))}>
                  {busy ? "Downloading…" : "Download"}
                </Button>
              ) : (
                <div className="flex items-center gap-3">
                  {loaded ? (
                    <Button disabled={busy} onClick={() => run(llm.stop)}>Stop</Button>
                  ) : (
                    <Button disabled={busy} onClick={() => run(llm.start)}>Start</Button>
                  )}
                  <Button variant="link" disabled={busy} onClick={() => run(() => download(true))}>
                    Reinstall
                  </Button>
                </div>
              )}
            </div>
            {downloading.map((p) => (
              <div key={p.id} className="mt-2 flex items-center gap-3">
                <Progress value={p.total ? p.done / p.total : 0} />
                <span className="w-28 text-right text-meta text-faint tabular-nums">
                  {formatBytes(p.done)} / {formatBytes(p.total)}
                </span>
              </div>
            ))}
            {info.state.state === "failed" && (
              <>
                <div className="mt-2 text-note text-danger">{info.state.error}</div>
                {info.state.log && (
                  <pre className="mt-2 max-h-60 overflow-auto rounded-card bg-well p-3 text-meta whitespace-pre-wrap text-muted">
                    {info.state.log}
                  </pre>
                )}
              </>
            )}
          </div>
        ) : (
          <div className="flex flex-col gap-3 px-5 py-4">
            <div className="grid grid-cols-2 gap-3">
              <label className="text-note text-muted">
                API address
                <input
                  className={`${field} mt-1`}
                  placeholder="https://api.openai.com/v1"
                  value={draft.api.baseUrl}
                  onChange={(e) => setDraft({ ...draft, api: { ...draft.api, baseUrl: e.target.value } })}
                  onBlur={() => save(draft)}
                />
              </label>
              <label className="text-note text-muted">
                Model (with vision)
                <input
                  className={`${field} mt-1`}
                  placeholder="gpt-4o-mini"
                  value={draft.api.model}
                  onChange={(e) => setDraft({ ...draft, api: { ...draft.api, model: e.target.value } })}
                  onBlur={() => save(draft)}
                />
              </label>
            </div>
            <label className="text-note text-muted">
              API key {info.hasKey && <span className="text-accent">· stored in the system keyring</span>}
              <div className="mt-1 flex gap-2">
                <input
                  type="password"
                  autoComplete="off"
                  className={field}
                  placeholder={info.hasKey ? "Replace the stored key" : "sk-…"}
                  value={key}
                  onChange={(e) => setKey(e.target.value)}
                />
                <Button className="whitespace-nowrap" disabled={busy || !key.trim()} onClick={() => run(() => llm.setKey(key).then(() => setKey("")))}>
                  Save key
                </Button>
                {info.hasKey && (
                  <Button variant="link" disabled={busy} onClick={() => run(() => llm.setKey(""))}>
                    Remove
                  </Button>
                )}
              </div>
            </label>
            {info.consented ? (
              <div className="flex items-center gap-3 text-note text-muted">
                <Pill tone="good">Allowed</Pill>
                <span className="flex-1">Frames and character images may be sent to {draft.api.baseUrl}.</span>
                <Button variant="link" disabled={busy} onClick={() => run(() => llm.consent(false))}>
                  Withdraw
                </Button>
              </div>
            ) : (
              <div className="rounded-card border border-warning/40 bg-warning/10 p-4 text-note">
                <div className="text-text">
                  Describing a clip sends a few of its frames and your character image to{" "}
                  <b>{draft.api.baseUrl || "the API address"}</b>. Nothing is sent until you allow it.
                </div>
                <Button
                  className="mt-3"
                  disabled={busy || !draft.api.baseUrl.trim()}
                  onClick={() => run(() => llm.setSettings(draft).then(() => llm.consent(true)))}
                >
                  Allow sending frames to this address
                </Button>
              </div>
            )}
          </div>
        )}
      </Card>
      {error && <div className="mt-2 px-1 text-note text-danger">{error}</div>}
    </div>
  );
}
