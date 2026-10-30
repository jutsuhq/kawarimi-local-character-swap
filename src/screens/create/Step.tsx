import type { ReactNode } from "react";
import { Check } from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";

export type StepState = "done" | "now" | "later";

/** One numbered step: a ring that fills once done, the title, a detail on the right, the step's controls under it. */
export function Step({
  number,
  title,
  detail,
  state = "now",
  children,
}: {
  number: number;
  title: string;
  detail?: ReactNode;
  state?: StepState;
  children: ReactNode;
}) {
  return (
    <section aria-label={`Step ${number}, ${title}`} className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span
          className={`grid size-5 shrink-0 place-items-center rounded-full border-[1.5px] text-[10px] font-bold ${
            state === "later" ? "border-stroke text-faint" : "border-accent text-accent"
          } ${state === "done" ? "bg-accent text-ink" : ""}`}
        >
          {state === "done" ? <Check size={11} strokeWidth={3.5} /> : number}
        </span>
        <span className="text-meta font-semibold uppercase tracking-[0.12em] text-faint">{title}</span>
        {detail && <span className="ml-auto text-meta text-faint tabular-nums">{detail}</span>}
      </div>
      <div className={state === "later" ? "opacity-55" : ""}>{children}</div>
    </section>
  );
}

/** The player beside a column of steps; the column scrolls on its own. */
export function StageAndSide({ stage, side }: { stage: ReactNode; side: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 gap-5">
      <div className="min-w-0 flex-1">{stage}</div>
      <div className="w-[360px] shrink-0 overflow-y-auto pr-1 pb-4">{side}</div>
    </div>
  );
}

export function Note({ children, tone = "muted" }: { children: ReactNode; tone?: "muted" | "faint" | "warn" | "bad" }) {
  const color = { muted: "text-muted", faint: "text-faint", warn: "text-warning", bad: "text-danger" }[tone];
  return <p className={`text-note ${color}`}>{children}</p>;
}

/** A compact segmented control, as Swapjutsu's. */
export function Segmented<T extends string | number>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: ReactNode; disabled?: boolean }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="inline-flex w-fit rounded-full border border-stroke bg-well p-0.5">
      {options.map((o) => (
        <button
          key={String(o.value)}
          disabled={o.disabled}
          onClick={() => onChange(o.value)}
          className={`rounded-full px-3 py-1 text-note font-semibold whitespace-nowrap transition-colors disabled:opacity-40 ${
            o.value === value ? "bg-glass-pressed text-text" : "text-muted hover:text-text"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Toggle({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={() => onChange(!on)}
      className={`relative h-[18px] w-8 shrink-0 rounded-full transition-colors ${on ? "bg-accent" : "bg-glass-pressed"}`}
    >
      <span
        className={`absolute top-0.5 size-3.5 rounded-full transition-[left] ${on ? "left-4 bg-ink" : "left-0.5 bg-text"}`}
      />
    </button>
  );
}

/** A local file as the WebView loads it. */
export const fileUrl = (path: string) => convertFileSrc(path);
