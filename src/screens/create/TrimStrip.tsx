import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import { timecode } from "../../lib/api";
import { fileUrl } from "./Step";

export const MIN_SECONDS = 4;
export const MAX_SECONDS = 30;

export type Range = { start: number; end: number };
type Part = "start" | "end" | "range";

/** The range `part` moved to `to`, kept 4–30 s long and inside the video. */
export function moveRange(range: Range, part: Part, to: number, duration: number): Range {
  const length = range.end - range.start;
  if (part === "range") {
    const start = Math.min(Math.max(0, to), duration - length);
    return { start, end: start + length };
  }
  if (part === "start") {
    const start = Math.min(Math.max(to, range.end - MAX_SECONDS, 0), range.end - MIN_SECONDS);
    return { start, end: range.end };
  }
  return { start: range.start, end: Math.max(Math.min(to, range.start + MAX_SECONDS, duration), range.start + MIN_SECONDS) };
}

/** Thumbnails of the whole video with two handles on the range; outside it is dimmed. */
export function TrimStrip({
  duration,
  strip,
  range,
  onChange,
  onScrub,
}: {
  duration: number;
  strip: string | null;
  range: Range;
  onChange: (r: Range) => void;
  onScrub: (t: number) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const x = (t: number) => `${(t / duration) * 100}%`;

  const drag = (part: Part) => (e: PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const width = box.current?.getBoundingClientRect().width ?? 1;
    const origin = range;
    const from = e.clientX;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const move = (m: globalThis.PointerEvent) => {
      const shift = ((m.clientX - from) / width) * duration;
      const at = part === "end" ? origin.end + shift : origin.start + shift;
      const next = moveRange(origin, part, at, duration);
      onChange(next);
      onScrub(part === "end" ? next.end : next.start);
    };
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
  };

  const nudge = (part: Part) => (e: KeyboardEvent) => {
    const step = e.key === "ArrowLeft" ? -1 / 16 : e.key === "ArrowRight" ? 1 / 16 : 0;
    if (!step) return;
    e.preventDefault();
    const next = moveRange(range, part, (part === "end" ? range.end : range.start) + step, duration);
    onChange(next);
    onScrub(part === "end" ? next.end : next.start);
  };

  const length = range.end - range.start;
  return (
    <div className="rounded-field border border-stroke bg-well p-4">
      <div className="flex items-baseline gap-2 pb-2">
        <span className="text-meta font-semibold uppercase tracking-[0.12em] text-faint">Trim</span>
        <span className="text-body font-semibold tabular-nums">
          {timecode(range.start)} – {timecode(range.end)}
        </span>
        <span className="ml-auto text-meta text-muted tabular-nums">
          {length.toFixed(1)} s of {duration.toFixed(1)} s
        </span>
      </div>
      <div ref={box} className="relative h-14 select-none">
        <div className="absolute inset-0 overflow-hidden rounded-small bg-track">
          {strip && <img src={fileUrl(strip)} alt="" className="h-full w-full object-fill" draggable={false} />}
        </div>
        <div className="absolute inset-y-0 left-0 bg-black/60" style={{ width: x(range.start) }} />
        <div className="absolute inset-y-0 right-0 bg-black/60" style={{ left: x(range.end) }} />
        <div
          role="slider"
          aria-label="Range"
          aria-valuetext={`${timecode(range.start)} to ${timecode(range.end)}`}
          tabIndex={0}
          onKeyDown={nudge("range")}
          onPointerDown={drag("range")}
          className="absolute inset-y-0 cursor-grab rounded-[4px] border-2 border-accent active:cursor-grabbing"
          style={{ left: x(range.start), width: x(length) }}
        />
        {(["start", "end"] as const).map((part) => (
          <div
            key={part}
            role="slider"
            aria-label={part === "start" ? "Range start" : "Range end"}
            aria-valuetext={timecode(range[part])}
            tabIndex={0}
            onKeyDown={nudge(part)}
            onPointerDown={drag(part)}
            className="absolute inset-y-0 -ml-[5px] grid w-2.5 cursor-ew-resize place-items-center rounded-[3px] bg-accent"
            style={{ left: x(range[part]) }}
          >
            <span className="h-4 w-0.5 rounded-full bg-ink" />
          </div>
        ))}
      </div>
      <p className="pt-2 text-meta text-faint">Drag the handles or the range; ← → nudge a frame. A clip is 4–30 s.</p>
    </div>
  );
}
