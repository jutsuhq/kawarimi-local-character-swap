import { useEffect, useRef } from "react";
import { joinPath, type ClipView, type RenderJob } from "../../lib/api";
import { Segmented, fileUrl } from "./Step";

export type Version = "original" | "mask" | "review" | `render:${string}`;

/** The renders of the clip that finished, newest last. */
export function finished(clip: ClipView): RenderJob[] {
  return clip.jobs.filter((j) => j.end?.status === "done");
}

/** The clip's versions in one player: the trimmed clip, the mask, the mask over the clip, and every finished render.
 *  Switching keeps the time, and the mask over the clip follows the clip's seeking and playing. */
export function Player({ clip, version, onVersion }: { clip: ClipView; version: Version; onVersion: (v: Version) => void }) {
  const time = useRef(0);
  const main = useRef<HTMLVideoElement>(null);
  const overlay = useRef<HTMLVideoElement>(null);
  const prep = clip.prepDir;
  const manifest = clip.manifest;
  const done = finished(clip);

  const options: { value: Version; label: string }[] = [{ value: "original", label: "Original" }];
  if (prep) options.push({ value: "mask", label: "Mask" }, { value: "review", label: "Review" });
  done.forEach((job) => {
    const person = clip.renders.find((r) => r.job === job.id)?.person;
    const kind = job.spec.settings.quality === "draft" ? "Draft" : "Final";
    options.push({ value: `render:${job.id}`, label: `${kind} · P${person ?? "?"}` });
  });

  const job = version.startsWith("render:") ? done.find((j) => `render:${j.id}` === version) : undefined;
  const src = (() => {
    if (job?.end?.status === "done") return fileUrl(job.end.result);
    if (prep && version === "mask") return fileUrl(joinPath(prep, "mask-view.mp4"));
    if (prep) return fileUrl(joinPath(prep, "normalized.mp4"));
    return fileUrl(clip.source);
  })();
  // Before prep the original is the source file itself, from the start of the range.
  const offset = prep ? 0 : clip.start;

  useEffect(() => {
    const v = main.current;
    const o = overlay.current;
    if (!v) return;
    let frame = 0;
    const follow = () => {
      time.current = v.currentTime - offset;
      if (o) {
        if (Math.abs(o.currentTime - v.currentTime) > 0.08) o.currentTime = v.currentTime;
        if (v.paused !== o.paused) (v.paused ? o.pause() : o.play().catch(() => {}));
      }
      if (!prep && v.currentTime >= clip.start + clip.duration) v.currentTime = clip.start;
      frame = requestAnimationFrame(follow);
    };
    frame = requestAnimationFrame(follow);
    return () => cancelAnimationFrame(frame);
  }, [src, version, offset, prep, clip.start, clip.duration]);

  const aspect = manifest ? `${manifest.video.width} / ${manifest.video.height}` : "16 / 9";
  const caption = {
    original: prep ? "The clip as prep normalised it: 16 fps, with its sound." : "The range as cut from the source.",
    mask: "The mask the renderer gets: the people being replaced, in white.",
    review: "The mask over the clip: check that it covers the person, and only them, in every shot.",
  }[version as "original"] ?? (job ? `${job.spec.settings.quality === "draft" ? "Draft" : "Final"} by Wan 2.2 Animate, with the clip's sound.` : "");

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="overflow-x-auto">
        <Segmented value={version} options={options} onChange={onVersion} />
      </div>
      <div className="relative min-h-0 flex-1">
        <div className="relative mx-auto max-h-full max-w-full overflow-hidden rounded-media bg-black" style={{ aspectRatio: aspect }}>
          <video
            key={src}
            ref={main}
            src={src}
            controls
            onLoadedMetadata={(e) => (e.currentTarget.currentTime = time.current + offset)}
            className="absolute inset-0 h-full w-full object-contain"
          />
          {version === "review" && prep && (
            // White mask × accent, then screened over the frame: the person glows, the rest is untouched.
            <div className="pointer-events-none absolute inset-0 isolate opacity-70 mix-blend-screen">
              <video
                ref={overlay}
                src={fileUrl(joinPath(prep, "mask-view.mp4"))}
                muted
                onLoadedMetadata={(e) => (e.currentTarget.currentTime = time.current)}
                className="absolute inset-0 h-full w-full object-contain"
              />
              <div className="absolute inset-0 bg-accent mix-blend-multiply" />
            </div>
          )}
        </div>
      </div>
      <p className="text-note text-faint">{caption}</p>
    </div>
  );
}
