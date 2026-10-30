import { useEffect, useRef, useState, type MouseEvent } from "react";
import { CircleStop, Crosshair, RotateCcw, TriangleAlert } from "lucide-react";
import { clipApi, joinPath, type ClipView, type Identity } from "../../lib/api";
import { Button, Progress } from "../../components/ui";
import { Note, Segmented, fileUrl } from "./Step";

/** Step 2: prep on its way with Cancel, stopped with its reason, or done with the people it found to pick from and
 *  the way to fix one. */
export function PrepStep({ clip }: { clip: ClipView }) {
  const [failure, setFailure] = useState<string | null>(null);
  const [picked, setPicked] = useState<number[]>(clip.targets);
  const [fixing, setFixing] = useState(false);
  useEffect(() => setPicked(clip.targets), [clip.targets.join()]);

  const act = (action: Promise<unknown>) => {
    setFailure(null);
    action.catch((e) => setFailure(String(e)));
  };

  if (clip.running) {
    return (
      <div className="flex flex-col gap-2">
        <Progress value={clip.running.fraction} />
        <Note>
          {clip.running.stage} · {Math.round(clip.running.fraction * 100)}%
        </Note>
        <div>
          <Button onClick={() => act(clipApi.cancel(clip.id))}>
            <span className="flex items-center gap-2">
              <CircleStop size={14} /> Cancel Prep
            </span>
          </Button>
        </div>
        <Note tone="faint">
          {clip.prep ? "Until it finishes, the clip keeps its last prep." : "On this GPU. Add the characters meanwhile."}
        </Note>
      </div>
    );
  }

  const manifest = clip.manifest;
  const error = failure ?? clip.error;
  if (!manifest || !clip.prepDir) {
    return (
      <div className="flex flex-col gap-2">
        <Note tone={error ? "bad" : "muted"}>{error ?? "Prep was cancelled before it finished."}</Note>
        <div>
          <Button variant="primary" onClick={() => act(clipApi.prepare(clip.id))}>
            <span className="flex items-center gap-2">
              <RotateCcw size={14} /> Prepare Again
            </span>
          </Button>
        </div>
      </div>
    );
  }

  const people = manifest.identities;
  const changed = picked.slice().sort().join() !== clip.targets.slice().sort().join();
  const toggle = (id: number) =>
    setPicked((p) => (p.includes(id) ? (p.length > 1 ? p.filter((x) => x !== id) : p) : [...p, id]));
  const presence = Math.max(0, ...people.filter((p) => clip.targets.includes(p.id)).map((p) => p.onScreen));

  return (
    <div className="flex flex-col gap-3">
      <Note>
        {people.length} {people.length === 1 ? "person" : "people"} found · {clip.targets.length} replaced · on screen{" "}
        {Math.round(presence * 100)}% of the time · {manifest.shots.length}{" "}
        {manifest.shots.length === 1 ? "shot" : "shots"}
      </Note>
      {error && <Note tone="bad">{error}</Note>}
      {manifest.closeCalls.length > 0 && (
        <p className="flex items-start gap-1.5 text-note text-warning">
          <TriangleAlert size={13} className="mt-0.5 shrink-0" />
          Someone looks alike across a cut; check Review that each person is one person.
        </p>
      )}
      <div className="grid grid-cols-4 gap-2">
        {people.map((p) => (
          <PersonTile key={p.id} person={p} prep={clip.prepDir!} picked={picked.includes(p.id)} onClick={() => toggle(p.id)} />
        ))}
      </div>
      <Note tone="faint">Click the people to replace. Each gets their own character; a render does one at a time.</Note>
      <div className="flex flex-wrap items-center gap-3">
        {changed && (
          <Button variant="primary" onClick={() => act(clipApi.prepare(clip.id, picked))}>
            Replace {picked.map((id) => `Person ${id}`).join(", ")}
          </Button>
        )}
        <button className="text-note font-semibold text-accent hover:underline" onClick={() => setFixing(!fixing)}>
          {fixing ? "Close Fix" : "Fix a Person…"}
        </button>
        <button className="text-note font-semibold text-accent hover:underline" onClick={() => act(clipApi.prepare(clip.id))}>
          Prepare Again
        </button>
      </div>
      {fixing && <FixPanel clip={clip} onError={setFailure} />}
    </div>
  );
}

function PersonTile({ person, prep, picked, onClick }: { person: Identity; prep: string; picked: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      aria-pressed={picked}
      title={`Person ${person.id}, on screen ${Math.round(person.onScreen * 100)}% of the clip`}
      className={`relative overflow-hidden rounded-small border-2 bg-well ${picked ? "border-accent" : "border-transparent opacity-70 hover:opacity-100"}`}
    >
      <img src={fileUrl(joinPath(prep, "people", `${person.id}.png`))} alt="" className="aspect-square w-full object-cover" />
      <span
        className={`absolute bottom-1 left-1 rounded-full px-1.5 text-meta font-bold ${picked ? "bg-accent text-ink" : "bg-black/60 text-text"}`}
      >
        {person.id}
      </span>
    </button>
  );
}

type Point = { x: number; y: number; on: boolean };

/** Points on a frame where a person's mask is wrong (+ on them, − off them), or on someone prep missed; the shot is
 *  followed again from there and the masks redone, the other shots kept. */
function FixPanel({ clip, onError }: { clip: ClipView; onError: (e: string) => void }) {
  const manifest = clip.manifest!;
  const prep = clip.prepDir!;
  const { width, height, fps } = manifest.video;
  const [frame, setFrame] = useState(0);
  const [points, setPoints] = useState<Point[]>([]);
  const [on, setOn] = useState(true);
  const [person, setPerson] = useState<number>(clip.targets[0] ?? 0);
  const video = useRef<HTMLVideoElement>(null);
  const mask = useRef<HTMLVideoElement>(null);

  useEffect(() => setPoints([]), [clip.prep]);
  useEffect(() => {
    // The middle of the frame, so rounding never lands on its neighbour.
    const t = (frame + 0.5) / fps;
    if (video.current) video.current.currentTime = t;
    if (mask.current) mask.current.currentTime = t;
  }, [frame, fps]);

  const shot = manifest.shots.findIndex((s) => frame >= s.start && frame < s.end);
  const click = (e: MouseEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const x = ((e.clientX - r.left) / r.width) * width;
    const y = ((e.clientY - r.top) / r.height) * height;
    setPoints((p) => [...p, { x, y, on }]);
  };

  const retrack = () => {
    const replaces = manifest.identities.find((i) => i.id === person)?.tracklets ?? [];
    const fix = {
      frame,
      points: points.map((p) => [p.x, p.y] as [number, number]),
      labels: points.map((p) => (p.on ? 1 : 0)),
      replaces,
    };
    clipApi.fix(clip.id, fix, person || null).catch((e) => onError(String(e)));
  };

  return (
    <div className="flex flex-col gap-2 rounded-field border border-stroke bg-well p-3">
      <div className="flex items-center gap-2">
        <span className="w-24 text-note font-semibold tabular-nums">Frame {frame}</span>
        <input
          type="range"
          min={0}
          max={manifest.frames - 1}
          value={frame}
          onChange={(e) => {
            setFrame(Number(e.target.value));
            setPoints([]);
          }}
          className="flex-1 accent-[var(--color-accent)]"
          aria-label="Frame"
        />
      </div>
      <div
        onClick={click}
        className="relative cursor-crosshair overflow-hidden rounded-media bg-black"
        style={{ aspectRatio: `${width} / ${height}` }}
      >
        <video ref={video} src={fileUrl(joinPath(prep, "normalized.mp4"))} muted preload="auto" className="absolute inset-0 h-full w-full" />
        <div className="pointer-events-none absolute inset-0 isolate opacity-60 mix-blend-screen">
          <video ref={mask} src={fileUrl(joinPath(prep, "mask-view.mp4"))} muted preload="auto" className="absolute inset-0 h-full w-full" />
          <div className="absolute inset-0 bg-accent mix-blend-multiply" />
        </div>
        {points.map((p, i) => (
          <span
            key={i}
            className={`pointer-events-none absolute -mt-1.5 -ml-1.5 grid size-3 place-items-center rounded-full border-2 border-text ${p.on ? "bg-accent" : "bg-danger"}`}
            style={{ left: `${(p.x / width) * 100}%`, top: `${(p.y / height) * 100}%` }}
          />
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Segmented
          value={on ? "on" : "off"}
          options={[
            { value: "on", label: "+ On them" },
            { value: "off", label: "− Off them" },
          ]}
          onChange={(v) => setOn(v === "on")}
        />
        <Segmented
          value={person}
          options={[
            ...manifest.identities.map((i) => ({ value: i.id, label: `P${i.id}` })),
            { value: 0, label: "Someone new" },
          ]}
          onChange={setPerson}
        />
      </div>
      <Note tone="faint">
        {points.length
          ? `Shot ${shot + 1}: ${person ? `Person ${person}` : "someone new"} is followed again from frame ${frame}, the other shots stay.`
          : "Pick a frame where the mask is wrong, then click on the person (+) or on what is not them (−)."}
      </Note>
      <div className="flex items-center gap-2">
        <Button variant="primary" onClick={retrack} disabled={!points.some((p) => p.on)}>
          <span className="flex items-center gap-2">
            <Crosshair size={14} /> Re-track Shot
          </span>
        </Button>
        {points.length > 0 && <Button onClick={() => setPoints([])}>Clear Points</Button>}
      </div>
    </div>
  );
}
