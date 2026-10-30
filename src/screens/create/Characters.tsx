import { useState } from "react";
import { ArrowRight, LoaderCircle, Plus, TriangleAlert, X } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { clipApi, joinPath, type ClipView } from "../../lib/api";
import { Note, Toggle, fileUrl } from "./Step";

export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg"];
/** Below this the renderer loses the face and details (docs/RENDER.md). */
const SMALL_IMAGE = 800;

export async function pickImage(): Promise<string | null> {
  const path = await open({ multiple: false, filters: [{ name: "Images", extensions: IMAGE_EXTENSIONS }] });
  return typeof path === "string" ? path : null;
}

/** Step 3: for every person being replaced, who they are now → the character they become, cut out on grey. */
export function CharactersStep({ clip, busy, onPick }: { clip: ClipView; busy: number[]; onPick: (person: number, image?: string) => void }) {
  if (!clip.manifest || !clip.prepDir) {
    return <Note tone="faint">One image per person you replace; you see who is who once prep is done.</Note>;
  }
  return (
    <div className="flex flex-col gap-2">
      {clip.targets.map((person) => (
        <PersonRow key={person} clip={clip} person={person} busy={busy.includes(person)} onPick={onPick} />
      ))}
      <Note tone="faint">
        Each person gets their own character; Wan renders one person per render. Drop an image on the window to give it
        to the first person without one.
      </Note>
    </div>
  );
}

function PersonRow({ clip, person, busy, onPick }: { clip: ClipView; person: number; busy: boolean; onPick: (person: number) => void }) {
  const character = clip.characters[String(person)];
  const identity = clip.manifest!.identities.find((i) => i.id === person);
  const face = identity?.face ? `${person}-face.png` : `${person}.png`;
  const [size, setSize] = useState<number | null>(null);
  const shown = character ? (character.useCutout && character.cutout ? character.cutout : character.image) : null;
  const change = (c: Parameters<typeof clipApi.changeCharacter>[2]) => clipApi.changeCharacter(clip.id, person, c);

  return (
    <div className="flex flex-col gap-2 rounded-field border border-stroke bg-well p-2">
      <div className="flex items-center gap-2">
        <img
          src={fileUrl(joinPath(clip.prepDir!, "people", face))}
          alt={`Person ${person} as prep found them`}
          className="size-12 rounded-small object-cover"
        />
        <ArrowRight size={13} className={character ? "text-accent" : "text-faint"} />
        <button
          onClick={() => onPick(person)}
          disabled={busy}
          title={character ? `Pick another image for Person ${person}` : `Pick the image for Person ${person}`}
          className={`grid size-12 place-items-center overflow-hidden rounded-small border ${
            character ? "border-stroke" : "border-dashed border-accent"
          } bg-well`}
        >
          {busy ? (
            <LoaderCircle size={16} className="animate-spin text-accent" />
          ) : shown ? (
            <img src={fileUrl(shown)} alt="" className="h-full w-full object-cover" onLoad={(e) => setSize(Math.min(e.currentTarget.naturalWidth, e.currentTarget.naturalHeight))} />
          ) : (
            <Plus size={16} className="text-accent" />
          )}
        </button>
        <div className="min-w-0">
          <div className="text-body font-semibold">Person {person}</div>
          <div className={`text-meta ${character ? "text-muted" : "text-warning"}`}>
            {busy
              ? "Cutting the character out…"
              : character
                ? `Ready${character.useCutout ? " · cut out" : ""}`
                : "Click or drop the image they become"}
          </div>
          {character && size !== null && size < SMALL_IMAGE && (
            <div className="flex items-center gap-1 text-meta text-warning" title="Wan keeps the face and details better from a larger image">
              <TriangleAlert size={11} /> Small image ({size} px); 800 px or more works better
            </div>
          )}
        </div>
        {character && (
          <button className="ml-auto rounded-full p-1.5 text-muted hover:bg-glass hover:text-text" title="Take this image off" onClick={() => change({ remove: true })}>
            <X size={14} />
          </button>
        )}
      </div>
      {character?.cutout && (
        <div className="flex items-center gap-2 border-t border-stroke pt-2">
          <img src={fileUrl(character.image)} alt="Before the cutout" className="size-9 rounded-small object-cover" />
          <ArrowRight size={11} className="text-faint" />
          <img src={fileUrl(character.cutout)} alt="The cutout on grey" className="size-9 rounded-small object-cover" />
          <div className="min-w-0 flex-1">
            <div className="text-note font-semibold">Cut out</div>
            <div className="text-meta text-faint">On plain grey, so its background stays out of the video</div>
          </div>
          <Toggle label="Cut out" on={character.useCutout} onChange={(v) => change({ useCutout: v })} />
        </div>
      )}
    </div>
  );
}
