import { useEffect, useState } from "react";
import { CircleStop, Film, Sparkles } from "lucide-react";
import { clipApi, type ClipView, type Estimate, type Quality, type RenderJob, type RenderProgress } from "../../lib/api";
import { Button, Progress } from "../../components/ui";
import { Note, Segmented } from "./Step";

const STEP_TITLE: Record<RenderJob["step"], string> = {
  prep: "Preparing",
  upload: "Sending to ComfyUI",
  queued: "Queued",
  rendering: "Rendering",
  downloading: "Downloading",
  finishing: "Adding the sound",
};

/** The people who have a character, so can be rendered. */
export function renderable(clip: ClipView): number[] {
  return clip.targets.filter((p) => clip.characters[String(p)]);
}

/** Why nothing can be rendered yet, if so. */
export function blocker(clip: ClipView, person: number | null): string | null {
  if (clip.running) return "Renders once prep is done.";
  if (!clip.manifest) return "Prepare the clip first.";
  if (person === null) return "Add a character for a person first.";
  if (clip.jobs.some((j) => !j.end)) return "One render of this clip at a time; this one is on its way.";
  return null;
}

function minutes(seconds: number): string {
  return seconds < 90 ? `${Math.round(seconds)} s` : `${Math.round(seconds / 60)} min`;
}

/** Step 5: who, draft or final, and what it takes. The button is in the bar under the steps. */
export function RenderStep({
  clip,
  person,
  quality,
  onPerson,
  onQuality,
}: {
  clip: ClipView;
  person: number | null;
  quality: Quality;
  onPerson: (p: number) => void;
  onQuality: (q: Quality) => void;
}) {
  const [estimate, setEstimate] = useState<Estimate | null>(null);
  useEffect(() => {
    if (!clip.manifest) return;
    clipApi.estimate(clip.id, quality).then(setEstimate, () => setEstimate(null));
  }, [clip.id, clip.prep, quality]);
  const people = renderable(clip);
  const why = blocker(clip, person);

  return (
    <div className="flex flex-col gap-3">
      {people.length > 1 && (
        <Segmented value={person ?? people[0]} options={people.map((p) => ({ value: p, label: `Person ${p}` }))} onChange={onPerson} />
      )}
      <Segmented
        value={quality}
        options={[
          { value: "draft", label: "Draft" },
          { value: "final", label: "Final" },
        ]}
        onChange={onQuality}
      />
      <Note>
        {quality === "draft"
          ? "The first seconds at 480p, to check the motion and the likeness. The final keeps its seed."
          : "The whole clip. Wan 2.2 Animate replaces the person and keeps the light, the background and the sound."}
      </Note>
      {estimate && (
        <div className="grid grid-cols-2 gap-x-3 gap-y-1 rounded-field border border-stroke bg-well p-3 text-note">
          <span className="text-faint">Size</span>
          <span className="tabular-nums">
            {estimate.width}×{estimate.height}, {estimate.frames} frames
          </span>
          <span className="text-faint">Windows</span>
          <span className="tabular-nums">
            {estimate.windows.length} × up to {Math.max(...estimate.windows)} frames, {estimate.steps} steps
          </span>
          <span className="text-faint">Time</span>
          <span className="tabular-nums">about {minutes(estimate.seconds)}</span>
          <span className="text-faint">Video memory</span>
          <span className="tabular-nums">about {estimate.vramGb.toFixed(1)} GB</span>
          <span className="text-faint">Model</span>
          <span>{estimate.model}</span>
        </div>
      )}
      {estimate?.ram?.message && <Note tone={estimate.ram.verdict === "short" ? "bad" : "warn"}>{estimate.ram.message}</Note>}
      <Note tone={why ? "warn" : "faint"}>
        {why ?? "Runs on this GPU through ComfyUI; prep waits while it does. It shows in Jobs, the result here and in Library."}
      </Note>
    </div>
  );
}

/** The render of this clip on its way, with its windows; or the last one that stopped. */
export function RenderStatus({ clip, onOpenLibrary }: { clip: ClipView; onOpenLibrary?: () => void }) {
  const job = clip.jobs[clip.jobs.length - 1];
  const [progress, setProgress] = useState<RenderProgress | null>(null);
  const active = job && !job.end;
  useEffect(() => {
    if (!active) return;
    const poll = () => clipApi.renderProgress(job.id).then(setProgress, () => {});
    poll();
    const timer = setInterval(poll, 2000);
    return () => clearInterval(timer);
  }, [job?.id, active]);
  if (!job) return null;
  const person = clip.renders.find((r) => r.job === job.id)?.person;
  const kind = job.spec.settings.quality === "draft" ? "Draft" : "Final";

  if (active) {
    const sampling = job.step === "rendering" && progress && progress.windows > 0;
    const fraction = sampling ? Math.min(1, (progress.done + progress.step / progress.steps) / progress.windows) : 0;
    return (
      <div className="flex flex-col gap-2 rounded-field border border-accent/40 bg-well p-3">
        <div className="flex items-center gap-2 text-body font-semibold">
          <Sparkles size={14} className="text-accent" /> {kind} of Person {person} · {STEP_TITLE[job.step]}
        </div>
        {sampling && <Progress value={fraction} />}
        <Note>
          {sampling
            ? `Window ${Math.min(progress.done + 1, progress.windows)} of ${progress.windows} · step ${progress.step} of ${progress.steps}`
            : job.detail || "Waiting for its turn"}
        </Note>
        <div>
          <Button onClick={() => clipApi.cancelRender(job.id)}>
            <span className="flex items-center gap-2">
              <CircleStop size={14} /> Cancel Render
            </span>
          </Button>
        </div>
      </div>
    );
  }
  if (job.end?.status === "failed") return <Note tone="bad">The {kind.toLowerCase()} failed: {job.end.reason}</Note>;
  if (job.end?.status === "done" && onOpenLibrary) {
    return (
      <div className="flex items-center gap-2 rounded-field border border-stroke bg-well p-3">
        <Note>
          {kind} of Person {person} is ready, in the player and in Library.
        </Note>
        <Button className="ml-auto shrink-0" onClick={onOpenLibrary}>
          <span className="flex items-center gap-2">
            <Film size={14} /> Library
          </span>
        </Button>
      </div>
    );
  }
  return null;
}

/** Always in sight under the steps: what step 5 picked and Generate. */
export function RenderBar({ clip, person, quality }: { clip: ClipView; person: number | null; quality: Quality }) {
  const [failure, setFailure] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const why = blocker(clip, person);
  const go = async () => {
    if (person === null) return;
    setSending(true);
    setFailure(null);
    try {
      await clipApi.render(clip.id, person, quality);
    } catch (e) {
      setFailure(String(e));
    } finally {
      setSending(false);
    }
  };
  return (
    <footer className="flex items-center gap-3 border-t border-stroke pt-4">
      <span className={`text-note ${failure ? "text-danger" : "text-muted"}`}>
        {failure ?? why ?? `${quality === "draft" ? "Draft" : "Final"} of Person ${person} · Wan 2.2 Animate on this GPU · free`}
      </span>
      <Button variant="primary" className="ml-auto shrink-0" disabled={!!why || sending} onClick={go}>
        <span className="flex items-center gap-2">
          <Sparkles size={16} /> {quality === "draft" ? "Generate Draft" : "Generate Final"}
        </span>
      </Button>
    </footer>
  );
}
