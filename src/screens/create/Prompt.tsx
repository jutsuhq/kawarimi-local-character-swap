import { useEffect, useState } from "react";
import { clipApi, joinPath, type ClipView } from "../../lib/api";
import { DescribeButton } from "../../components/DescribeButton";
import { Note } from "./Step";

/** Step 4: the prompt of the person being rendered, written from the clip and the character until it is edited;
 *  an edit is saved with the clip and goes into the render's job. */
export function PromptStep({ clip, person }: { clip: ClipView; person: number | null }) {
  const character = person === null ? undefined : clip.characters[String(person)];
  const [template, setTemplate] = useState("");
  const [text, setText] = useState("");
  const [focused, setFocused] = useState(false);

  useEffect(() => {
    if (person === null || !clip.manifest) return;
    clipApi.template(clip.id, person).then(setTemplate, () => setTemplate(""));
  }, [clip.id, clip.prep, person, character?.image]);

  const saved = character?.prompt ?? null;
  useEffect(() => {
    if (!focused) setText(saved ?? template);
  }, [saved, template, focused]);

  if (person === null || !character) {
    return <Note tone="faint">Written for you from the clip and the character once a person has one; edit it if you like.</Note>;
  }

  const save = (value: string) =>
    clipApi.changeCharacter(clip.id, person, value.trim() === template.trim() ? { resetPrompt: true } : { prompt: value });

  return (
    <div className="flex flex-col gap-2">
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => {
          setFocused(false);
          if (text !== (saved ?? template)) save(text);
        }}
        rows={5}
        aria-label={`Prompt for Person ${person}`}
        className="w-full resize-none rounded-field border border-stroke bg-well p-2.5 text-body outline-none focus:border-accent"
      />
      <div className="flex items-center gap-3">
        <span className="text-meta text-faint">{saved === null ? "From the template" : "Edited; saved with the clip"}</span>
        <span className="ml-auto flex items-center gap-3">
          {clip.prepDir && (
            <DescribeButton
              video={joinPath(clip.prepDir, "normalized.mp4")}
              character={character.image}
              onPrompt={(p) => (setText(p), save(p))}
            />
          )}
          {saved !== null && (
            <button
              className="text-note font-semibold text-accent hover:underline"
              onClick={() => {
                setText(template);
                clipApi.changeCharacter(clip.id, person, { resetPrompt: true });
              }}
            >
              Reset to Template
            </button>
          )}
        </span>
      </div>
    </div>
  );
}
