//! What agents may spend, and what they spent: `limits.json` and `ledger.jsonl` in the server's own folder, outside
//! the jobs folder, so removing jobs does not reset the count. Only a person edits the limits; no tool changes them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

const DAY_MS: u64 = 24 * 3600 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Limits {
    /// Renders an agent may have waiting or running at once.
    pub queued_renders: usize,
    /// In the last 24 hours.
    pub renders_per_day: usize,
    pub preps_per_day: usize,
    /// GPU time an agent's preps, cutouts and renders held the card for, in the last 24 hours.
    pub gpu_minutes_per_day: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { queued_renders: 2, renders_per_day: 10, preps_per_day: 50, gpu_minutes_per_day: 120.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Prep,
    Cutout,
    /// A render job created; its GPU time comes as `Gpu` entries while it runs.
    Render,
    Gpu,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Milliseconds since the Unix epoch.
    pub at: u64,
    pub kind: Kind,
    pub id: String,
    #[serde(default)]
    pub gpu_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub preps: usize,
    pub renders: usize,
    pub gpu_minutes: f64,
    pub limits: Limits,
}

pub struct Ledger {
    path: PathBuf,
    pub limits: Limits,
}

impl Ledger {
    /// Reads `limits.json` in `dir`, writing the defaults there the first time.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let limits_path = dir.join("limits.json");
        let limits = match std::fs::read(&limits_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(_) => {
                let limits = Limits::default();
                std::fs::write(&limits_path, serde_json::to_vec_pretty(&limits)?)?;
                limits
            }
        };
        Ok(Self { path: dir.join("ledger.jsonl"), limits })
    }

    pub fn record(&self, kind: Kind, id: &str, gpu_seconds: f64) -> Result<()> {
        let entry = Entry { at: now(), kind, id: id.into(), gpu_seconds };
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(file, "{}", serde_json::to_string(&entry)?)?;
        Ok(())
    }

    pub fn entries(&self) -> Vec<Entry> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }

    /// Every render job an agent created, oldest first.
    pub fn renders(&self) -> Vec<String> {
        self.entries().into_iter().filter(|e| e.kind == Kind::Render).map(|e| e.id).collect()
    }

    pub fn usage(&self) -> Usage {
        let since = now().saturating_sub(DAY_MS);
        let recent: Vec<Entry> = self.entries().into_iter().filter(|e| e.at >= since).collect();
        let count = |kind| recent.iter().filter(|e| e.kind == kind).count();
        Usage {
            preps: count(Kind::Prep),
            renders: count(Kind::Render),
            gpu_minutes: recent.iter().map(|e| e.gpu_seconds).sum::<f64>() / 60.0,
            limits: self.limits.clone(),
        }
    }

    /// Fails with what ran out, before a prep or a cutout.
    pub fn check_prep(&self) -> Result<()> {
        let usage = self.usage();
        if usage.preps >= self.limits.preps_per_day {
            bail!("the limit of {} preps a day is reached", self.limits.preps_per_day);
        }
        self.check_gpu(&usage, 0.0)
    }

    /// Fails with what ran out, before a render estimated at `seconds` with `queued` of the agent's renders active.
    pub fn check_render(&self, seconds: f64, queued: usize) -> Result<()> {
        let usage = self.usage();
        if queued >= self.limits.queued_renders {
            bail!("{queued} renders are already waiting or running; the limit is {}", self.limits.queued_renders);
        }
        if usage.renders >= self.limits.renders_per_day {
            bail!("the limit of {} renders a day is reached", self.limits.renders_per_day);
        }
        self.check_gpu(&usage, seconds)
    }

    fn check_gpu(&self, usage: &Usage, seconds: f64) -> Result<()> {
        let left = self.limits.gpu_minutes_per_day - usage.gpu_minutes;
        if left * 60.0 < seconds.max(1.0) {
            bail!(
                "not enough GPU time left today: {:.1} of {} minutes used, this needs about {:.1}",
                usage.gpu_minutes,
                self.limits.gpu_minutes_per_day,
                seconds / 60.0
            );
        }
        Ok(())
    }
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_stop_preps_and_renders() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("limits.json"),
            r#"{"queuedRenders": 1, "rendersPerDay": 2, "prepsPerDay": 1, "gpuMinutesPerDay": 10}"#,
        )
        .unwrap();
        let ledger = Ledger::open(dir.path()).unwrap();
        ledger.check_prep().unwrap();
        ledger.record(Kind::Prep, "a", 30.0).unwrap();
        assert!(ledger.check_prep().unwrap_err().to_string().contains("preps a day"));

        ledger.check_render(60.0, 0).unwrap();
        assert!(ledger.check_render(60.0, 1).unwrap_err().to_string().contains("waiting or running"));
        // 0.5 minutes used, a 10-minute estimate no longer fits.
        assert!(ledger.check_render(600.0, 0).unwrap_err().to_string().contains("GPU time"));
        ledger.record(Kind::Render, "j1", 0.0).unwrap();
        ledger.record(Kind::Render, "j2", 0.0).unwrap();
        assert!(ledger.check_render(10.0, 0).unwrap_err().to_string().contains("renders a day"));
        assert_eq!(ledger.renders(), ["j1", "j2"]);
    }

    #[test]
    fn defaults_are_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let first = Ledger::open(dir.path()).unwrap();
        assert_eq!(first.limits, Limits::default());
        assert!(dir.path().join("limits.json").is_file());
    }
}
