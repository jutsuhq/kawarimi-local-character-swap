import { useEffect, useRef, useState } from "react";
import { Clapperboard, Film, Scissors, X } from "lucide-react";
import { clipApi, timecode, type SourceInfo } from "../../lib/api";
import { Button } from "../../components/ui";
import { MAX_SECONDS, TrimStrip, type Range } from "./TrimStrip";
import { Note, StageAndSide, Step, fileUrl } from "./Step";

/** The new clip: a drop target until there is a video, then the video with its trim and the steps ahead, dimmed. */
export function NewClip({
  source,
  reading,
  error,
  onChoose,
  onClear,
  onCreated,
}: {
  source: SourceInfo | null;
  reading: boolean;
  error: string | null;
  onChoose: () => void;
  onClear: () => void;
  onCreated: (id: string) => void;
}) {
  if (!source) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-4 rounded-card border border-dashed border-glass-stroke bg-well p-10 text-center">
        <Clapperboard size={36} className="text-faint" />
        <div className="text-row font-semibold">{reading ? "Reading the video…" : "Drop a video to start"}</div>
        <p className="max-w-sm text-note text-muted">
          Any video FFmpeg reads. Kawarimi finds the people, follows the one you pick through every cut and renders your
          character in their place. Nothing leaves this computer.
        </p>
        {error && <Note tone="bad">{error}</Note>}
        <Button variant="primary" onClick={onChoose} disabled={reading}>
          <span className="flex items-center gap-2">
            <Film size={16} /> Choose Video…
          </span>
        </Button>
      </div>
    );
  }
  return <Trim key={source.path} source={source} onChoose={onChoose} onClear={onClear} onCreated={onCreated} />;
}

function Trim({
  source,
  onChoose,
  onClear,
  onCreated,
}: {
  source: SourceInfo;
  onChoose: () => void;
  onClear: () => void;
  onCreated: (id: string) => void;
}) {
  const [range, setRange] = useState<Range>({ start: 0, end: Math.min(source.duration, MAX_SECONDS) });
  const [url, setUrl] = useState(fileUrl(source.path));
  const [converting, setConverting] = useState(false);
  const [strip, setStrip] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const video = useRef<HTMLVideoElement>(null);
  const name = source.path.split(/[\\/]/).pop() ?? "Video";

  useEffect(() => {
    clipApi.strip(source.path, source.duration).then(setStrip, () => setStrip(null));
  }, [source]);

  // A codec the WebView cannot play gets a converted copy for the preview; prep still reads the original.
  const unplayable = () => {
    if (converting) return;
    setConverting(true);
    clipApi.playable(source.path).then(
      (p) => setUrl(fileUrl(p)),
      (e) => setFailure(String(e)),
    );
  };

  const scrub = (t: number) => {
    if (video.current) video.current.currentTime = t;
  };

  // Playing stays inside the range.
  const keepInRange = () => {
    const v = video.current;
    if (v && !v.paused && (v.currentTime >= range.end || v.currentTime < range.start - 0.1)) {
      v.currentTime = range.start;
    }
  };

  const prepare = async () => {
    setBusy(true);
    setFailure(null);
    try {
      const clip = await clipApi.create(source.path, range.start, range.end - range.start);
      onCreated(clip.id);
    } catch (e) {
      setFailure(String(e));
    } finally {
      setBusy(false);
    }
  };

  const seconds = `${(range.end - range.start).toFixed(1)} s`;
  return (
    <div className="flex h-full flex-col gap-5 rounded-card border border-stroke bg-card p-6">
      <header className="flex items-end gap-2">
        <div className="min-w-0">
          <h2 className="truncate font-display text-[30px] leading-none font-extrabold uppercase">{name}</h2>
          <p className="mt-1 text-muted">
            New clip · {timecode(source.duration)} · {source.width}×{source.height}
            {source.hasAudio ? "" : " · no sound"}
          </p>
        </div>
        <span className="ml-auto flex gap-2">
          <Button onClick={onChoose} title="Choose another video for this new clip">
            <span className="flex items-center gap-2">
              <Film size={14} /> Replace Video…
            </span>
          </Button>
          <Button onClick={onClear} title="Drop this video">
            <span className="flex items-center gap-2">
              <X size={14} /> Clear
            </span>
          </Button>
        </span>
      </header>
      <StageAndSide
        stage={
          <div className="flex flex-col gap-4">
            <video
              ref={video}
              src={url}
              controls
              onError={unplayable}
              onTimeUpdate={keepInRange}
              onLoadedMetadata={() => scrub(range.start)}
              className="aspect-video w-full rounded-media bg-black object-contain"
            />
            {converting && url === fileUrl(source.path) && (
              <Note tone="faint">Converting a preview this view can play…</Note>
            )}
            <TrimStrip duration={source.duration} strip={strip} range={range} onChange={setRange} onScrub={scrub} />
          </div>
        }
        side={
          <div className="flex flex-col gap-6">
            <Step number={1} title="Video" detail={seconds} state="done">
              <Note>Trim under the player to the part with the person, 4–30 s.</Note>
            </Step>
            <Step number={2} title="Prep" detail="On this GPU">
              <Note>
                Finds the cuts and the people, follows them through every shot and writes their masks. A few seconds
                to a minute; nothing is sent anywhere.
              </Note>
            </Step>
            <Step number={3} title="Characters" state="later">
              <Note tone="faint">One image per person you replace; you see who is who once prep is done.</Note>
            </Step>
            <Step number={4} title="Prompt" detail="Optional" state="later">
              <Note tone="faint">Written for you from the clip and the character; edit it if you like.</Note>
            </Step>
            <Step number={5} title="Render" state="later">
              <Note tone="faint">A quick draft of the first seconds to check, then the whole clip, on your GPU.</Note>
            </Step>
          </div>
        }
      />
      <footer className="flex items-center gap-3 border-t border-stroke pt-4">
        <span className={`text-note ${failure ? "text-danger" : "text-muted"}`}>
          {failure ?? `${timecode(range.start)}–${timecode(range.end)} · ${seconds}; characters and the render come next.`}
        </span>
        <Button variant="primary" className="ml-auto" onClick={prepare} disabled={busy}>
          <span className="flex items-center gap-2">
            <Scissors size={16} /> Prepare Clip
          </span>
        </Button>
      </footer>
    </div>
  );
}
