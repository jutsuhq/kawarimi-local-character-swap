import { useEffect, useState } from "react";
import { FolderOpen, Trash } from "lucide-react";
import { openPath } from "@tauri-apps/plugin-opener";
import { clipApi, timecode, type ClipView, type Quality } from "../../lib/api";
import { CharactersStep } from "./Characters";
import { Player, finished, type Version } from "./Player";
import { PrepStep } from "./Prep";
import { PromptStep } from "./Prompt";
import { RenderBar, RenderStatus, RenderStep, renderable } from "./Render";
import { Note, StageAndSide, Step } from "./Step";

export function title(clip: ClipView): string {
  return clip.source.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "Clip";
}

/** One clip: its versions in the player on the left, the steps from prep to a render on the right, the render bar
 *  under both. */
export function ClipWorkspace({
  clip,
  busy,
  onPick,
  onRemoved,
  onOpenLibrary,
}: {
  clip: ClipView;
  busy: number[];
  onPick: (person: number) => void;
  onRemoved: () => void;
  onOpenLibrary?: () => void;
}) {
  const [version, setVersion] = useState<Version>(clip.prep ? "review" : "original");
  const [quality, setQuality] = useState<Quality>("draft");
  const [chosen, setChosen] = useState<number | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const people = renderable(clip);
  const person = chosen !== null && people.includes(chosen) ? chosen : (people[0] ?? null);

  // A render that just finished is what the person wants to see; a new prep, its review.
  const done = finished(clip);
  const last = done[done.length - 1]?.id;
  useEffect(() => {
    if (last) setVersion(`render:${last}`);
  }, [last]);
  useEffect(() => {
    if (clip.prep && version === "original") setVersion("review");
  }, [clip.prep]);

  const manifest = clip.manifest;
  const subtitle = [
    `${timecode(clip.start)}–${timecode(clip.start + clip.duration)}`,
    manifest && `${manifest.video.width}×${manifest.video.height}`,
    manifest && `${manifest.identities.length} ${manifest.identities.length === 1 ? "person" : "people"}`,
  ]
    .filter(Boolean)
    .join(" · ");
  const missing = clip.targets.filter((p) => !clip.characters[String(p)]).length;

  const remove = async () => {
    if (!confirm(`Remove ${title(clip)}? Its prep and characters go; finished renders stay in Library.`)) return;
    try {
      await clipApi.remove(clip.id);
      onRemoved();
    } catch (e) {
      setFailure(String(e));
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-5 rounded-card border border-stroke bg-card p-6">
      <header className="flex items-end gap-2">
        <div className="min-w-0">
          <h2 className="truncate font-display text-[30px] leading-none font-extrabold uppercase">{title(clip)}</h2>
          <p className="mt-1 text-muted">{failure ?? subtitle}</p>
        </div>
        <span className="ml-auto flex gap-1">
          <button className="rounded-full p-2 text-muted hover:bg-glass hover:text-text" title="Open the clip's folder" onClick={() => openPath(clip.dir)}>
            <FolderOpen size={16} />
          </button>
          <button className="rounded-full p-2 text-muted hover:bg-glass hover:text-danger" title="Remove the clip" onClick={remove}>
            <Trash size={16} />
          </button>
        </span>
      </header>
      <StageAndSide
        stage={<Player clip={clip} version={version} onVersion={setVersion} />}
        side={
          <div className="flex flex-col gap-6">
            <RenderStatus clip={clip} onOpenLibrary={onOpenLibrary} />
            <Step number={1} title="Video" detail={`${clip.duration.toFixed(1)} s`} state="done">
              <Note>
                {timecode(clip.start)}–{timecode(clip.start + clip.duration)} of the source
              </Note>
            </Step>
            <Step
              number={2}
              title="Prep"
              detail={clip.running ? clip.running.stage : clip.manifest ? "Done" : clip.error ? "Failed" : "Stopped"}
              state={clip.manifest && !clip.running ? "done" : "now"}
            >
              <PrepStep clip={clip} />
            </Step>
            <Step
              number={3}
              title="Characters"
              detail={manifest ? `${clip.targets.length - missing} of ${clip.targets.length}` : undefined}
              state={!manifest ? "later" : missing ? "now" : "done"}
            >
              <CharactersStep clip={clip} busy={busy} onPick={onPick} />
            </Step>
            <Step number={4} title="Prompt" detail={person !== null ? `Person ${person}` : "Optional"} state={person === null ? "later" : "now"}>
              <PromptStep clip={clip} person={person} />
            </Step>
            <Step number={5} title="Render" state={person === null || !manifest ? "later" : "now"}>
              <RenderStep clip={clip} person={person} quality={quality} onPerson={setChosen} onQuality={setQuality} />
            </Step>
          </div>
        }
      />
      <RenderBar clip={clip} person={person} quality={quality} />
    </div>
  );
}
