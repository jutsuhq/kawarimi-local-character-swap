import { useState } from "react";
import { api } from "../../lib/api";
import { Button } from "../../components/ui";

/** The licenses file, loaded when asked for: it is close to a megabyte. */
export function Acknowledgements() {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (text === null) {
    return (
      <div className="mt-3 flex items-center gap-3">
        <Button variant="link" onClick={() => api.licenses().then(setText, (e) => setError(String(e)))}>
          Acknowledgements
        </Button>
        {error && <span className="text-danger">{error}</span>}
      </div>
    );
  }
  return (
    <div className="mt-3">
      <Button variant="link" onClick={() => setText(null)}>
        Hide acknowledgements
      </Button>
      <pre className="mt-2 max-h-[480px] overflow-auto whitespace-pre-wrap rounded-lg bg-page p-4 text-meta text-faint">
        {text}
      </pre>
    </div>
  );
}
