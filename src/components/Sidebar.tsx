import { Film, Layers, Settings, Sparkles } from "lucide-react";
import type { SystemInfo } from "../lib/api";

export type Screen = "create" | "jobs" | "library" | "settings";

const NAV: { id: Screen; label: string; icon: typeof Sparkles }[] = [
  { id: "create", label: "Create", icon: Sparkles },
  { id: "jobs", label: "Jobs", icon: Layers },
  { id: "library", label: "Library", icon: Film },
];

export function Sidebar({
  screen,
  onSelect,
  system,
}: {
  screen: Screen;
  onSelect: (s: Screen) => void;
  system: SystemInfo | null;
}) {
  const gpu = system?.gpus[0];
  const problem = system?.gpuProblem;
  return (
    <aside className="flex w-[264px] shrink-0 flex-col border-r border-stroke bg-sidebar px-3 pt-6 pb-4">
      <div className="flex items-center gap-2.5 px-3 pb-7">
        <div className="grid size-8 place-items-center rounded-small bg-accent text-ink">
          <Sparkles size={18} strokeWidth={2.5} />
        </div>
        <span className="font-display text-[24px] font-extrabold uppercase tracking-tight">Kawarimi</span>
      </div>
      <nav className="flex flex-col gap-1">
        {NAV.map((item) => (
          <NavRow key={item.id} {...item} active={screen === item.id} onClick={() => onSelect(item.id)} />
        ))}
      </nav>
      <div className="mt-auto flex flex-col gap-3">
        <div className="rounded-field border border-stroke bg-card px-4 py-3">
          <div className="flex items-center gap-2 text-body font-semibold">
            <span className={`size-1.5 rounded-full ${problem?.blocks ? "bg-danger" : problem ? "bg-warning" : "bg-accent"}`} />
            {gpu ? gpu.name.replace(/^NVIDIA (GeForce )?/, "") : "No NVIDIA GPU found"}
          </div>
          <div className="mt-1 text-note text-muted">
            {problem?.kind === "oldDriver"
              ? `Driver ${problem.driver} is too old`
              : problem?.kind === "lowVram"
                ? `${Math.round(problem.memoryMb / 1024)} GB · may run out of memory`
                : gpu
                  ? `${Math.round(gpu.memoryMb / 1024)} GB · everything runs here`
                  : "Prep and render need CUDA"}
          </div>
        </div>
        <NavRow id="settings" label="Settings" icon={Settings} active={screen === "settings"} onClick={() => onSelect("settings")} />
      </div>
    </aside>
  );
}

function NavRow({
  label,
  icon: Icon,
  active,
  onClick,
}: {
  id: Screen;
  label: string;
  icon: typeof Sparkles;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex items-center gap-3 rounded-row px-3 py-2.5 text-left text-[15px] font-medium transition-colors ${
        active ? "bg-selection text-text" : "text-muted hover:bg-well hover:text-text"
      }`}
    >
      <Icon size={18} className={active ? "text-accent" : ""} />
      {label}
    </button>
  );
}
