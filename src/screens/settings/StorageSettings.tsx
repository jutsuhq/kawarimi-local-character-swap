import { useEffect, useState } from "react";
import { formatBytes, storageApi, type Folder, type StorageInstalled, type StorageUsage } from "../../lib/api";
import { Button, Card, SectionLabel } from "../../components/ui";

function bytes(n: number | undefined): string {
  return n === undefined ? "Measuring…" : n === 0 ? "Empty" : formatBytes(n);
}

/** Where jobs and results live, what the app keeps on disk, and the caches it can clear. */
export function StorageSettings() {
  const [usage, setUsage] = useState<StorageUsage | null>(null);
  const [installed, setInstalled] = useState<StorageInstalled | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [message, setMessage] = useState<{ text: string; bad?: boolean } | null>(null);
  const [cleaning, setCleaning] = useState(false);

  const measure = () =>
    storageApi
      .usage()
      .then(setUsage)
      .catch((e) => setMessage({ text: String(e), bad: true }));
  useEffect(() => {
    measure();
    storageApi.installed().then(setInstalled).catch(() => {});
  }, []);

  const reveal = (which: Folder) => storageApi.reveal(which).catch((e) => setMessage({ text: String(e), bad: true }));

  const clean = async () => {
    setConfirming(false);
    setCleaning(true);
    try {
      const freed = await storageApi.clean();
      setMessage({ text: freed ? `Freed ${formatBytes(freed)}.` : "Nothing to clear." });
    } catch (e) {
      setMessage({ text: `Couldn't clear the caches: ${e}`, bad: true });
    } finally {
      setCleaning(false);
      measure();
    }
  };

  return (
    <>
      <div>
        <SectionLabel>Jobs and results</SectionLabel>
        <Card className="divide-y divide-stroke">
          <div className="flex items-center gap-4 px-5 py-4">
            <div className="min-w-0 flex-1">
              <div className="text-row font-semibold">Folder</div>
              <div className="mt-0.5 text-note break-all text-muted select-text">{usage?.jobsDir ?? "…"}</div>
            </div>
            <Button onClick={() => reveal("jobs")}>Open</Button>
          </div>
          <Line label="Finished videos" value={usage ? `${usage.resultCount} · ${bytes(usage.results)}` : bytes(undefined)} />
          <Line label="Job files" detail="Each job's prep, inputs and ComfyUI's video before the finish step" value={bytes(usage?.jobFiles)} />
        </Card>
        <p className="mt-2 px-1 text-note text-faint">
          Every job has its own folder with its character, prep and the finished video. Deleting a result in Library moves
          its folder to the Recycle Bin.
        </p>
      </div>

      <div>
        <SectionLabel>Caches</SectionLabel>
        <Card>
          <div className="flex items-center gap-4 px-5 py-4">
            <div className="min-w-0 flex-1">
              <div className="text-row font-semibold">Render leftovers</div>
              <div className="mt-0.5 text-note text-muted">
                {confirming
                  ? "Deletes them for good. Finished videos, and jobs still running or that can be retried, stay."
                  : "Finished jobs' prep and intermediate videos, and the copies ComfyUI keeps of ended jobs."}
              </div>
            </div>
            <span className="text-body text-muted tabular-nums">{bytes(usage?.cache)}</span>
            {confirming ? (
              <>
                <Button onClick={clean}>Clear</Button>
                <Button variant="link" onClick={() => setConfirming(false)}>
                  Keep
                </Button>
              </>
            ) : (
              <Button disabled={!usage?.cache || cleaning} onClick={() => setConfirming(true)}>
                {cleaning ? "Clearing…" : "Clear…"}
              </Button>
            )}
          </div>
        </Card>
        {message && <div className={`mt-2 px-1 text-note ${message.bad ? "text-danger" : "text-muted"}`}>{message.text}</div>}
      </div>

      <div>
        <SectionLabel>Installed</SectionLabel>
        <Card className="divide-y divide-stroke">
          <Line label="Models and tools" detail={installed?.modelsDir} value={bytes(installed?.models)} onOpen={() => reveal("models")} />
          <Line label="ComfyUI" detail={installed?.comfyDir} value={bytes(installed?.comfy)} onOpen={() => reveal("comfy")} />
        </Card>
        <p className="mt-2 px-1 text-note text-faint">Remove single models in GPU and models; reinstall ComfyUI in its tab.</p>
      </div>
    </>
  );
}

function Line({ label, detail, value, onOpen }: { label: string; detail?: string; value: string; onOpen?: () => void }) {
  return (
    <div className="flex items-center gap-4 px-5 py-4">
      <div className="min-w-0 flex-1">
        <div className="text-row font-semibold">{label}</div>
        {detail && <div className="mt-0.5 text-note break-all text-muted">{detail}</div>}
      </div>
      <span className="text-body text-muted tabular-nums">{value}</span>
      {onOpen && <Button onClick={onOpen}>Open</Button>}
    </div>
  );
}
