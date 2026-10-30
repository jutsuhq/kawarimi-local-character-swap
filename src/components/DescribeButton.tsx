import { useEffect, useState } from "react";
import { Sparkles } from "lucide-react";
import { llm, type LlmDescription } from "../lib/api";
import { Button } from "./ui";

/**
 * Create's «описание от LLM»: describes `character` and the moves in `video` (paths on disk) and hands back the
 * prompt written from it. Hidden while the prompt writer is off in Settings.
 */
export function DescribeButton({
  video,
  character,
  onPrompt,
}: {
  video: string;
  character: string;
  onPrompt: (prompt: string, description: LlmDescription) => void;
}) {
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    llm.info().then((i) => setEnabled(i.settings.enabled));
  }, []);

  if (!enabled) return null;

  const describe = async () => {
    setError(null);
    setBusy(true);
    try {
      const description = await llm.describe(video, character);
      onPrompt(description.prompt, description);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col items-start gap-1">
      <Button disabled={busy || !video || !character} onClick={describe} className="inline-flex items-center gap-2">
        <Sparkles size={14} />
        {busy ? "Describing…" : "Describe with LLM"}
      </Button>
      {error && <div className="text-note text-danger">{error}</div>}
    </div>
  );
}
