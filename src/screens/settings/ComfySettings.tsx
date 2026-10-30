import { useEffect, useState, type ReactNode } from "react";
import { ExternalLink } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  api,
  comfyApi,
  formatBytes,
  storageApi,
  type ComfyInfo,
  type ComfyState,
  type OwnComfy,
  type OwnComfyCheck,
  type SystemInfo,
} from "../../lib/api";
import { Button, Card, Pill, SectionLabel } from "../../components/ui";

const COMFY_LABEL: Record<ComfyState["state"], [string, "neutral" | "good" | "warn" | "bad"]> = {
  notInstalled: ["Not installed", "neutral"],
  installing: ["Installing", "warn"],
  stopped: ["Stopped", "neutral"],
  starting: ["Starting", "warn"],
  ready: ["Running", "good"],
  failed: ["Failed", "bad"],
};

const LINKS: [string, string, string][] = [
  ["Tutorial", "docs.comfy.org · Wan 2.2 Animate", "https://docs.comfy.org/tutorials/video/wan/wan2-2-animate"],
  ["Models", "Hugging Face · Comfy-Org/Wan_2.2_ComfyUI_Repackaged", "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged"],
  ["GGUF weights and loader", "GitHub · city96/ComfyUI-GGUF", "https://github.com/city96/ComfyUI-GGUF"],
  ["Pose nodes", "GitHub · comfyui_controlnet_aux", "https://github.com/Fannovel16/comfyui_controlnet_aux"],
];

export function ComfySettings({ system }: { system: SystemInfo | null }) {
  const [own, setOwn] = useState<OwnComfy | null>(null);
  useEffect(() => {
    comfyApi.own().then(setOwn);
  }, []);
  const remote = !!own?.enabled && !!own.checked;
  return (
    <>
      <AppComfy system={system} remote={remote} />
      {own && <OwnComfySection own={own} onChange={setOwn} />}
    </>
  );
}

function Row({ label, detail, children, first }: { label: string; detail?: ReactNode; children?: ReactNode; first?: boolean }) {
  return (
    <div className={`flex items-center gap-4 px-5 py-4 ${first ? "" : "border-t border-stroke"}`}>
      <div className="min-w-0 flex-1">
        <div className="text-row font-semibold">{label}</div>
        {detail && <div className="mt-0.5 text-note break-words text-muted">{detail}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

function AppComfy({ system, remote }: { system: SystemInfo | null; remote: boolean }) {
  const [comfy, setComfy] = useState<ComfyState | null>(null);
  const [info, setInfo] = useState<ComfyInfo | null>(null);
  const [size, setSize] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    api.comfyState().then(setComfy);
    comfyApi.info().then(setInfo);
    storageApi.installed().then((u) => setSize(u.comfy)).catch(() => {});
    const unlisten = api.onComfyState(setComfy);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const run = (action: () => Promise<void>) => {
    setError(null);
    setConfirming(false);
    action().catch((e) => setError(String(e)));
  };

  if (!comfy) return null;
  const [label, tone] = COMFY_LABEL[comfy.state];
  const running = comfy.state === "starting" || comfy.state === "ready";
  const installed = comfy.state !== "notInstalled";
  const noGpu = !!system?.gpuProblem?.blocks;
  return (
    <div>
      <SectionLabel trailing={size !== null && installed ? formatBytes(size) : undefined}>ComfyUI on this PC</SectionLabel>
      <Card>
        <Row
          first
          label="Render server"
          detail={
            <>
              {comfy.state === "notInstalled" && "Download uv and ComfyUI in GPU and models, then install its Python packages (about 5 GB)."}
              {comfy.state === "installing" && comfy.step}
              {comfy.state === "stopped" &&
                (remote ? "Not used while renders go to your own ComfyUI." : "Starts by itself when a render needs it, and stops when Kawarimi closes.")}
              {comfy.state === "starting" && "Loading ComfyUI and its custom nodes; the first start takes a few minutes."}
              {comfy.state === "ready" && comfy.url}
              {comfy.state === "failed" && <span className="text-danger">{comfy.error}</span>}
            </>
          }
        >
          <Pill tone={tone}>{label}</Pill>
          {comfy.state === "notInstalled" && <Button onClick={() => run(api.comfyInstall)}>Install</Button>}
          {(comfy.state === "stopped" || comfy.state === "failed") && (
            <Button disabled={noGpu} title={noGpu ? system?.gpuProblem?.message : undefined} onClick={() => run(api.comfyStart)}>
              Start
            </Button>
          )}
          {running && <Button onClick={() => run(api.comfyStop)}>Stop</Button>}
        </Row>
        {comfy.state === "failed" && comfy.log && (
          <pre className="mx-5 mb-4 max-h-60 overflow-auto rounded-card bg-well p-3 text-meta whitespace-pre-wrap text-muted select-text">{comfy.log}</pre>
        )}
        {info && (
          <Row
            label="Versions"
            detail={[info.comfyui, `Python ${info.python}`, info.torch && `PyTorch ${info.torch}`, info.frontend && `frontend ${info.frontend}`]
              .filter(Boolean)
              .join(" · ")}
          />
        )}
        {info && (
          <Row label="Folder" detail={info.root}>
            <Button onClick={() => run(() => storageApi.reveal("comfy"))}>Open</Button>
            <Button onClick={() => run(() => storageApi.reveal("comfyLogs"))}>Logs</Button>
          </Row>
        )}
        {installed && (
          <Row
            label="Reinstall"
            detail={
              confirming
                ? "Stops ComfyUI (a render on it starts again after) and installs its Python packages afresh. Models and outputs stay."
                : "Installs ComfyUI's Python packages afresh, for a broken install. Models and outputs stay."
            }
          >
            {confirming ? (
              <>
                <Button onClick={() => run(comfyApi.reinstall)}>Reinstall</Button>
                <Button variant="link" onClick={() => setConfirming(false)}>
                  Keep
                </Button>
              </>
            ) : (
              <Button disabled={comfy.state === "installing"} onClick={() => setConfirming(true)}>
                Reinstall…
              </Button>
            )}
          </Row>
        )}
        {error && <div className="px-5 pb-4 text-note text-danger">{error}</div>}
      </Card>
    </div>
  );
}

const field =
  "w-64 rounded-field border border-stroke bg-well px-3 py-2 text-body text-text outline-none placeholder:text-faint focus:border-accent select-text";

function OwnComfySection({ own, onChange }: { own: OwnComfy; onChange: (o: OwnComfy) => void }) {
  const [address, setAddress] = useState(own.url);
  const [token, setToken] = useState("");
  const [check, setCheck] = useState<"idle" | "checking" | OwnComfyCheck | string>("idle");
  const [error, setError] = useState<string | null>(null);
  const valid = parse(address) !== null;

  const save = (enabled: boolean, url: string) => {
    setError(null);
    comfyApi
      .setOwn(enabled, url)
      .then(onChange)
      .catch((e) => setError(String(e)));
  };

  const runCheck = async () => {
    setError(null);
    setCheck("checking");
    try {
      onChange(await comfyApi.setOwn(own.enabled, address));
      setCheck(await comfyApi.check());
    } catch (e) {
      setCheck(String(e));
    } finally {
      comfyApi.own().then(onChange);
    }
  };

  const saveToken = (value: string) => {
    setError(null);
    comfyApi
      .setToken(value)
      .then((o) => {
        onChange(o);
        setToken("");
        setCheck("idle");
      })
      .catch((e) => setError(String(e)));
  };

  const passed = own.enabled && own.checked !== null && own.checked === parse(address);
  const checkDetail =
    check === "idle"
      ? passed
        ? `Passed: renders go to ${own.checked}`
        : "Asks ComfyUI for its GPU and for the nodes and models the Wan graph needs"
      : check === "checking"
        ? "Reading /system_stats and /object_info…"
        : typeof check === "string"
          ? check
          : check.missingNodes.length + check.missingModels.length === 0
            ? `Ready: ${Math.round(check.vram / 2 ** 30)} GB GPU, renders with ${check.model}`
            : `${check.missingNodes.length + check.missingModels.length} missing, listed below`;

  return (
    <>
      <div>
        <SectionLabel>Own ComfyUI</SectionLabel>
        <Card>
          <Row first label="Use my ComfyUI" detail="Renders go to a ComfyUI you run (another PC, a GPU box on your network) once Check passes. Prep stays on this PC.">
            <button
              role="switch"
              aria-checked={own.enabled}
              aria-label="Use my ComfyUI"
              onClick={() => save(!own.enabled, address)}
              className={`relative h-6 w-11 rounded-full transition-colors ${own.enabled ? "bg-accent" : "bg-glass-stroke"}`}
            >
              <span className={`absolute top-0.5 size-5 rounded-full bg-text transition-[left] ${own.enabled ? "left-5.5 bg-ink" : "left-0.5"}`} />
            </button>
          </Row>
          <Row
            label="Address"
            detail={valid || !address.trim() ? "ComfyUI's address, port included; start it with --listen to reach it from here" : "Not a valid http(s) address"}
          >
            <input
              className={field}
              value={address}
              placeholder="192.168.1.20:8188"
              spellCheck={false}
              onChange={(e) => {
                setAddress(e.target.value);
                setCheck("idle");
              }}
              onBlur={() => address !== own.url && save(own.enabled, address)}
              onKeyDown={(e) => e.key === "Enter" && valid && runCheck()}
            />
          </Row>
          <Row
            label={own.hasToken ? "Replace token" : "Token"}
            detail={own.hasToken ? "Saved in the system keyring, sent as a bearer token" : "Optional, for a ComfyUI behind a proxy"}
          >
            <input
              className={`${field} w-44`}
              type="password"
              value={token}
              placeholder="none"
              onChange={(e) => setToken(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && token.trim() && saveToken(token)}
            />
            <Button disabled={!token.trim()} onClick={() => saveToken(token)}>
              Save
            </Button>
            {own.hasToken && (
              <Button variant="link" className="text-danger" onClick={() => saveToken("")}>
                Remove
              </Button>
            )}
          </Row>
          <Row label="Check" detail={<span className={typeof check === "string" && check !== "idle" && check !== "checking" ? "text-danger" : ""}>{checkDetail}</span>}>
            <Button variant="primary" disabled={check === "checking" || !valid} onClick={runCheck}>
              {check === "checking" ? "Checking…" : "Check"}
            </Button>
          </Row>
        </Card>
        {error && <div className="mt-2 px-1 text-note text-danger">{error}</div>}
      </div>
      {typeof check === "object" && check.missingNodes.length + check.missingModels.length > 0 && (
        <div>
          <SectionLabel>Missing on your ComfyUI</SectionLabel>
          <Card>
            {check.missingNodes.map((node, i) => (
              <Row key={node} first={i === 0} label={node} detail="Node · install its custom node pack" />
            ))}
            {check.missingModels.map((file, i) => (
              <Row key={file} first={i === 0 && check.missingNodes.length === 0} label={file.split("/").pop() ?? file} detail={`Model · models/${file}`} />
            ))}
          </Card>
        </div>
      )}
      <div>
        <SectionLabel>Setting it up</SectionLabel>
        <Card>
          {LINKS.map(([label, detail, url], i) => (
            <Row key={label} first={i === 0} label={label} detail={detail}>
              <Button onClick={() => openUrl(url)}>
                <span className="inline-flex items-center gap-1.5">
                  Open <ExternalLink size={12} />
                </span>
              </Button>
            </Row>
          ))}
        </Card>
        <p className="mt-2 px-1 text-note text-faint">
          Kawarimi only talks to your ComfyUI over its API. It needs the GGUF loader and comfyui_controlnet_aux (DWPose
          downloads its pose models on the first run), and the models in ComfyUI's models folder as Check lists them.
        </p>
      </div>
    </>
  );
}

/** The same rule as the app's: http(s) with a host, no trailing slash. */
function parse(text: string): string | null {
  let t = text.trim();
  if (!t) return null;
  if (!t.includes("://")) t = `http://${t}`;
  const [scheme, rest] = [t.slice(0, t.indexOf("://")).toLowerCase(), t.slice(t.indexOf("://") + 3)];
  const host = rest.split(/[/?#]/)[0];
  if (!["http", "https"].includes(scheme) || !host || host.startsWith(":") || /\s/.test(host)) return null;
  return `${scheme}://${rest.split(/[?#]/)[0].replace(/\/+$/, "")}`;
}
