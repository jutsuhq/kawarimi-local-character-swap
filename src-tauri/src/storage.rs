//! What the app keeps on disk, and the caches it can clear. Finished videos (`result.mp4`), `job.json` and the
//! character copy are never caches; neither is anything of a job that is still running or could be retried.

use std::path::{Path, PathBuf};

use anyhow::Result;
use kawarimi_comfy::job::{End, Job, Jobs, Source};
use serde::Serialize;

/// Files a finished job no longer needs: ComfyUI's video before the finish step, the graph, the reference image.
const DONE_SCRAP: &[&str] = &["comfy.mp4", "graph.json", "reference.png"];

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub jobs_dir: PathBuf,
    /// Finished videos.
    pub results: u64,
    pub result_count: usize,
    /// Everything else in the jobs folder.
    pub job_files: u64,
    /// What [`clean`] would free.
    pub cache: u64,
}

/// The installs, slow to measure (ComfyUI's packages are tens of thousands of files).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    /// The downloaded models and tools.
    pub models: u64,
    pub models_dir: PathBuf,
    /// ComfyUI's install (Python, its packages, its data).
    pub comfy: u64,
    pub comfy_dir: PathBuf,
}

pub fn installed(models_dir: &Path, comfy_dir: &Path) -> Installed {
    Installed { models: size(models_dir), models_dir: models_dir.into(), comfy: size(comfy_dir), comfy_dir: comfy_dir.into() }
}

/// Bytes under `path`, a file or a folder; links are not followed.
pub fn size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    std::fs::read_dir(path).into_iter().flatten().flatten().map(|e| size(&e.path())).sum()
}

fn is_part(name: &str) -> bool {
    name.contains(".part.") || name.ends_with(".part")
}

/// Every cache path: scrap of finished jobs, half-written files of ended ones, and what ComfyUI keeps of jobs that
/// ended (in its `input` and `output` folders under `comfy_data`) or that are gone.
pub fn caches(jobs: &Jobs, all: &[Job], comfy_data: Option<&Path>) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for job in all.iter().filter(|j| !j.is_active()) {
        let dir = jobs.dir(&job.id);
        let entries = std::fs::read_dir(&dir).into_iter().flatten().flatten();
        found.extend(entries.map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| is_part(&n.to_string_lossy()))));
        if matches!(job.end, Some(End::Done { .. })) {
            found.extend(DONE_SCRAP.iter().map(|f| dir.join(f)).filter(|p| p.exists()));
            if matches!(job.spec.source, Source::Clip { .. }) {
                found.extend(Some(dir.join("prep")).filter(|p| p.exists()));
            }
        }
    }
    let active: Vec<&str> = all.iter().filter(|j| j.is_active()).map(|j| j.id.as_str()).collect();
    if let Some(data) = comfy_data {
        let ours = std::fs::read_dir(data.join("output").join("kawarimi")).into_iter().flatten().flatten();
        found.extend(ours.filter(|e| !active.contains(&&*e.file_name().to_string_lossy())).map(|e| e.path()));
        let inputs = std::fs::read_dir(data.join("input")).into_iter().flatten().flatten();
        found.extend(
            inputs
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let id = name.strip_prefix("kawarimi-").and_then(|n| n.split('-').next());
                    id.is_some_and(|id| !active.contains(&id))
                })
                .map(|e| e.path()),
        );
    }
    found
}

pub fn usage(jobs: &Jobs, jobs_dir: &Path, comfy_data: Option<&Path>) -> Usage {
    let all = jobs.all();
    let results: Vec<PathBuf> = all
        .iter()
        .filter_map(|j| match &j.end {
            Some(End::Done { result }) => Some(result.clone()),
            _ => None,
        })
        .filter(|r| r.is_file())
        .collect();
    let results_size = results.iter().map(|r| size(r)).sum();
    Usage {
        jobs_dir: jobs_dir.into(),
        results: results_size,
        result_count: results.len(),
        job_files: size(jobs_dir).saturating_sub(results_size),
        cache: caches(jobs, &all, comfy_data).iter().map(|p| size(p)).sum(),
    }
}

/// Deletes the caches for good (they are made again when needed); returns the bytes freed.
pub fn clean(jobs: &Jobs, comfy_data: Option<&Path>) -> Result<u64> {
    let mut freed = 0;
    for path in caches(jobs, &jobs.all(), comfy_data) {
        let bytes = size(&path);
        let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match removed {
            Ok(()) => freed += bytes,
            Err(e) => eprintln!("cleaning {}: {e}", path.display()),
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use kawarimi_comfy::job::Spec;
    use kawarimi_comfy::render::Settings;
    use kawarimi_comfy::wan::Quality;

    use super::*;

    fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![7u8; bytes]).unwrap();
    }

    /// A clip, a character, and three jobs made from them: done, failed, running.
    fn world(root: &Path) -> (Jobs, Vec<Job>, PathBuf) {
        let clip = root.join("clip.mp4");
        let character = root.join("me.png");
        write(&clip, 10);
        write(&character, 20);
        let jobs = Jobs::new(root.join("jobs"));
        let settings = Settings { quality: Quality::Draft, seed: 0, prompt: String::new(), model: None, frames: None, steps: None, short_side: None, window: None };
        let spec = Spec { source: Source::Clip { path: clip, start: 0.0, duration: None, targets: None }, character, settings };
        let mut made = Vec::new();
        for end in ["done", "failed", "running"] {
            let mut job = jobs.create(spec.clone()).unwrap();
            let dir = jobs.dir(&job.id);
            write(&dir.join("prep").join("normalized.mp4"), 1000);
            write(&dir.join("comfy.mp4"), 300);
            write(&dir.join("graph.json"), 40);
            write(&dir.join("reference.png"), 50);
            write(&dir.join("result.part.mp4"), 7);
            job.end = match end {
                "done" => {
                    write(&dir.join("result.mp4"), 500);
                    Some(End::Done { result: dir.join("result.mp4") })
                }
                "failed" => Some(End::Failed { reason: "x".into() }),
                _ => None,
            };
            jobs.save(&job).unwrap();
            made.push(job);
        }
        let data = root.join("comfy").join("data");
        for job in &made {
            write(&data.join("output").join("kawarimi").join(&job.id).join("render_00001_.mp4"), 200);
            write(&data.join("input").join(format!("kawarimi-{}-source.mp4", job.id)), 100);
        }
        write(&data.join("output").join("someone-elses.png"), 9);
        write(&data.join("input").join("example.png"), 9);
        (jobs, made, data)
    }

    #[test]
    fn sizes_add_up() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a/b/c.bin"), 1234);
        write(&dir.path().join("a/d.bin"), 66);
        assert_eq!(size(&dir.path().join("a")), 1300);
        assert_eq!(size(&dir.path().join("a/d.bin")), 66);
        assert_eq!(size(&dir.path().join("nothing")), 0);
    }

    #[test]
    fn usage_counts_results_apart_from_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let (jobs, made, data) = world(dir.path());
        let usage = usage(&jobs, &dir.path().join("jobs"), Some(&data));
        assert_eq!((usage.results, usage.result_count), (500, 1));
        assert_eq!(usage.job_files, size(&dir.path().join("jobs")) - 500);
        // Done: prep, comfy.mp4, graph, reference, part. Failed: its part. ComfyUI: both ended jobs' files.
        assert_eq!(usage.cache, 1000 + 300 + 40 + 50 + 7 + 7 + 2 * (200 + 100));
        assert_eq!(made.len(), 3);
    }

    #[test]
    fn cleaning_never_touches_results_or_running_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let (jobs, made, data) = world(dir.path());
        let before: Vec<(PathBuf, u64)> = [&made[1], &made[2]].iter().map(|j| (jobs.dir(&j.id), size(&jobs.dir(&j.id)))).collect();
        let freed = clean(&jobs, Some(&data)).unwrap();
        assert_eq!(freed, 1000 + 300 + 40 + 50 + 7 + 7 + 2 * (200 + 100));

        let done = jobs.dir(&made[0].id);
        assert_eq!(std::fs::read(done.join("result.mp4")).unwrap().len(), 500);
        assert!(done.join("job.json").is_file() && done.join("character.png").is_file());
        assert!(!done.join("prep").exists() && !done.join("comfy.mp4").exists());
        // The failed job keeps what a retry needs, the running one everything.
        assert_eq!(size(&before[0].0), before[0].1 - 7);
        assert_eq!(size(&before[1].0), before[1].1);
        let running = &made[2].id;
        assert!(data.join("output").join("kawarimi").join(running).exists());
        assert!(data.join("input").join(format!("kawarimi-{running}-source.mp4")).exists());
        assert!(data.join("output").join("someone-elses.png").exists() && data.join("input").join("example.png").exists());
        assert_eq!(clean(&jobs, Some(&data)).unwrap(), 0);
    }
}
