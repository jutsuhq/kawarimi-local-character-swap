import { useEffect, useState } from "react";
import { jobsApi, type Job } from "./api";

/** Every job on disk, oldest first, kept current by `job-state` events. `reload` reads them again. */
export function useJobs() {
  const [jobs, setJobs] = useState<Job[] | null>(null);

  const reload = () => jobsApi.list().then(setJobs);

  useEffect(() => {
    reload();
    const unlisten = jobsApi.onJob((job) =>
      setJobs((all) => {
        const list = all ?? [];
        return list.some((j) => j.id === job.id) ? list.map((j) => (j.id === job.id ? job : j)) : [...list, job];
      }),
    );
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  return { jobs, reload };
}
