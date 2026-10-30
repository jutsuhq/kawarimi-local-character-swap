import { useEffect, useState } from "react";
import { Sidebar, type Screen } from "./components/Sidebar";
import { Create } from "./screens/Create";
import { Jobs } from "./screens/Jobs";
import { Library } from "./screens/Library";
import { Settings } from "./screens/Settings";
import { api, type SystemInfo } from "./lib/api";

export default function App() {
  const [screen, setScreen] = useState<Screen>("create");
  const [system, setSystem] = useState<SystemInfo | null>(null);

  const refresh = () => api.systemInfo().then(setSystem).catch(() => setSystem(null));
  useEffect(() => {
    refresh();
  }, []);

  return (
    <div className="flex h-full">
      <Sidebar screen={screen} onSelect={setScreen} system={system} />
      <main className="min-w-0 flex-1 px-7 pt-8 pb-7">
        {screen === "create" && <Create onOpenLibrary={() => setScreen("library")} />}
        {screen === "jobs" && <Jobs system={system} onCreate={() => setScreen("create")} />}
        {screen === "library" && <Library onCreate={() => setScreen("create")} />}
        {screen === "settings" && <Settings system={system} onChanged={refresh} />}
      </main>
    </div>
  );
}
