import { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ArrowLeft, ExternalLink, Film, FolderOpen, Trash2 } from "lucide-react";
import { jobName, jobsApi, sourceVideo, type Job } from "../lib/api";
import { useJobs } from "../lib/useJobs";
import { Button, Card, EmptyState, PageHeader, SectionLabel } from "../components/ui";

type Filter = "all" | "draft" | "final";
const FILTERS: [Filter, string][] = [
  ["all", "All"],
  ["draft", "Drafts"],
  ["final", "Finals"],
];

function result(job: Job): string {
  return job.end?.status === "done" ? job.end.result : "";
}

function stamp(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** Every finished render, drafts and finals, newest first; a click opens it with its source and character. */
export function Library({ onCreate }: { onCreate: () => void }) {
  const { jobs, reload } = useJobs();
  const [filter, setFilter] = useState<Filter>("all");
  const [open, setOpen] = useState<string | null>(null);

  const all = (jobs ?? []).filter((j) => j.end?.status === "done").sort((a, b) => b.created - a.created);
  const shown = all.filter((j) => filter === "all" || j.spec.settings.quality === filter);
  const entry = all.find((j) => j.id === open);

  useEffect(() => {
    const back = (e: KeyboardEvent) => e.key === "Escape" && setOpen(null);
    window.addEventListener("keydown", back);
    return () => window.removeEventListener("keydown", back);
  }, []);

  if (entry) {
    return (
      <Detail
        job={entry}
        onBack={() => setOpen(null)}
        onRemoved={() => {
          setOpen(null);
          reload();
        }}
      />
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-start justify-between gap-4">
        <PageHeader title="Library" subtitle="Every draft and final you've rendered, with the clip and character it came from." />
        <div className="flex shrink-0 rounded-full border border-stroke bg-well p-1">
          {FILTERS.map(([id, label]) => (
            <button
              key={id}
              onClick={() => setFilter(id)}
              className={`rounded-full px-3.5 py-1.5 text-body font-semibold transition-colors ${
                filter === id ? "bg-selection text-text" : "text-muted hover:text-text"
              }`}
            >
              {label} {all.filter((j) => id === "all" || j.spec.settings.quality === id).length}
            </button>
          ))}
        </div>
      </div>
      {jobs && shown.length === 0 ? (
        <div className="min-h-0 flex-1">
          <EmptyState
            icon={<Film size={36} />}
            title={all.length ? "Nothing here" : "No results yet"}
            note={all.length ? `No ${filter === "draft" ? "drafts" : "finals"} yet.` : "Drafts and finals you render in Create show up here."}
            action={all.length ? { label: "Show All", onClick: () => setFilter("all") } : { label: "Go to Create", onClick: onCreate }}
          />
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 auto-rows-min grid-cols-[repeat(auto-fill,minmax(230px,1fr))] gap-3 overflow-y-auto pr-2 pb-6">
          {shown.map((job) => (
            <Tile key={job.id} job={job} onOpen={() => setOpen(job.id)} />
          ))}
        </div>
      )}
    </div>
  );
}

/** A frame of the result that plays while the pointer is over it, with the clip's name and the version. */
function Tile({ job, onOpen }: { job: Job; onOpen: () => void }) {
  const final = job.spec.settings.quality === "final";
  return (
    <button
      onClick={onOpen}
      aria-label={`${jobName(job)}, ${final ? "final" : "draft"}`}
      className="group relative aspect-video overflow-hidden rounded-field border border-stroke bg-well text-left hover:border-glass-stroke"
    >
      <video
        src={convertFileSrc(result(job))}
        preload="metadata"
        muted
        loop
        className="size-full object-cover"
        onMouseEnter={(e) => e.currentTarget.play().catch(() => {})}
        onMouseLeave={(e) => e.currentTarget.pause()}
      />
      <span className="absolute bottom-1.5 left-1.5 max-w-[80%] truncate rounded-small bg-black/60 px-2 py-0.5 text-meta font-semibold">
        {jobName(job)}
      </span>
      <span
        className={`absolute top-1.5 right-1.5 rounded-full px-2 py-0.5 text-[9px] font-extrabold tracking-wider uppercase ${
          final ? "bg-accent text-ink" : "bg-black/60 text-text"
        }`}
      >
        {final ? "Final" : "Draft"}
      </span>
    </button>
  );
}

function Detail({ job, onBack, onRemoved }: { job: Job; onBack: () => void; onRemoved: () => void }) {
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = (action: () => Promise<unknown>, then?: () => void) => {
    setError(null);
    action()
      .then(then)
      .catch((e) => setError(String(e)));
  };
  const final = job.spec.settings.quality === "final";
  return (
    <div className="flex h-full flex-col overflow-y-auto pr-2 pb-6">
      <div className="pb-4">
        <Button variant="link" onClick={onBack}>
          <span className="inline-flex items-center gap-1.5">
            <ArrowLeft size={14} /> Library
          </span>
        </Button>
      </div>
      <PageHeader title={jobName(job)} subtitle={`${final ? "Final" : "Draft"} · rendered ${stamp(job.created)} · seed ${job.spec.settings.seed}`} />
      <div className="flex max-w-[1100px] flex-col gap-5">
        <video src={convertFileSrc(result(job))} controls className="max-h-[52vh] w-full rounded-card bg-black" />
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="primary" onClick={() => run(() => jobsApi.open(job.id))}>
            <span className="inline-flex items-center gap-1.5">
              <ExternalLink size={14} /> Open
            </span>
          </Button>
          <Button onClick={() => run(() => jobsApi.reveal(job.id))}>
            <span className="inline-flex items-center gap-1.5">
              <FolderOpen size={14} /> Show in folder
            </span>
          </Button>
          <div className="ml-auto flex items-center gap-2">
            {confirming ? (
              <>
                <span className="text-note text-muted">The video and its job folder go to the Recycle Bin.</span>
                <Button className="text-danger" onClick={() => run(() => jobsApi.remove(job.id), onRemoved)}>
                  Move to Recycle Bin
                </Button>
                <Button variant="link" onClick={() => setConfirming(false)}>
                  Keep
                </Button>
              </>
            ) : (
              <Button onClick={() => setConfirming(true)}>
                <span className="inline-flex items-center gap-1.5">
                  <Trash2 size={14} /> Delete
                </span>
              </Button>
            )}
          </div>
        </div>
        {error && <div className="text-note text-danger">{error}</div>}
        <div className="grid grid-cols-[2fr_1fr] gap-4">
          <div>
            <SectionLabel>Source</SectionLabel>
            <Card className="overflow-hidden">
              <video src={convertFileSrc(sourceVideo(job))} controls muted preload="metadata" className="aspect-video w-full bg-black object-contain" />
            </Card>
          </div>
          <div>
            <SectionLabel>Character</SectionLabel>
            <Card className="grid aspect-square place-items-center overflow-hidden p-2">
              <img src={convertFileSrc(job.spec.character)} alt="Character" className="max-h-full max-w-full rounded-field object-contain" />
            </Card>
          </div>
        </div>
        {job.spec.settings.prompt && (
          <div>
            <SectionLabel>Prompt</SectionLabel>
            <Card className="p-4 text-body text-muted select-text">{job.spec.settings.prompt}</Card>
          </div>
        )}
      </div>
    </div>
  );
}
