//! One GPU, one job at a time: prep and render take turns through a lease, and whatever the last holder left in
//! video memory (ComfyUI keeps its models between prompts) is evicted before a different kind of work starts. With a
//! lock file, the lease also waits for other processes on this machine (`kawarimi-mcp`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use kawarimi_comfy::locks::GpuLock;
use serde::Serialize;
use tokio::sync::{Mutex as Turn, OwnedMutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Work {
    /// The prep pipeline's ONNX Runtime sessions; they live only inside `kawarimi_prep::prepare`.
    Prep,
    /// ComfyUI.
    Render,
    /// llama-server, the local prompt writer; evicting it stops the process.
    Llm,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Holder {
    pub work: Work,
    /// What the holder is doing, for the UI ("prep bedroom.mp4").
    pub name: String,
}

/// Frees what `work` left in video memory. Blocking; called with the GPU held, before the next kind of work.
pub type Evict = dyn Fn(Work) -> Result<()> + Send + Sync;

/// Cheap to clone; every clone is the same GPU.
#[derive(Clone)]
pub struct Gpu {
    /// The kind of work that last held the GPU, and so may still have memory on it.
    turn: Arc<Turn<Option<Work>>>,
    holder: Arc<Mutex<Option<Holder>>>,
    evict: Arc<Evict>,
    lock: Option<PathBuf>,
}

/// The GPU, until dropped.
pub struct Lease {
    holder: Arc<Mutex<Option<Holder>>>,
    _turn: OwnedMutexGuard<Option<Work>>,
    _lock: Option<GpuLock>,
}

impl Gpu {
    pub fn new(evict: impl Fn(Work) -> Result<()> + Send + Sync + 'static) -> Self {
        Self { turn: Arc::new(Turn::new(None)), holder: Default::default(), evict: Arc::new(evict), lock: None }
    }

    /// Also takes `path` ([`GpuLock`]) for every lease, so other processes wait their turn too.
    pub fn with_lock(mut self, path: PathBuf) -> Self {
        self.lock = Some(path);
        self
    }

    /// Waits for the GPU, in the order asked, then evicts the previous kind of work if it differs. A failed
    /// eviction gives the GPU back and is returned, so the caller does not start on a full card.
    pub async fn lease(&self, work: Work, name: impl Into<String>) -> Result<Lease> {
        let name = name.into();
        let mut turn = self.turn.clone().lock_owned().await;
        let lock = match self.lock.clone() {
            Some(path) => {
                let holder = name.clone();
                Some(tokio::task::spawn_blocking(move || GpuLock::acquire(&path, &holder)).await??)
            }
            None => None,
        };
        if let Some(last) = *turn
            && last != work
        {
            let evict = self.evict.clone();
            tokio::task::spawn_blocking(move || evict(last)).await??;
        }
        *turn = Some(work);
        *self.holder.lock().unwrap() = Some(Holder { work, name });
        Ok(Lease { holder: self.holder.clone(), _turn: turn, _lock: lock })
    }

    /// Who has the GPU now.
    pub fn holder(&self) -> Option<Holder> {
        self.holder.lock().unwrap().clone()
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        *self.holder.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording() -> (Gpu, Arc<Mutex<Vec<Work>>>) {
        let evicted = Arc::new(Mutex::new(Vec::new()));
        let log = evicted.clone();
        let gpu = Gpu::new(move |w| {
            log.lock().unwrap().push(w);
            Ok(())
        });
        (gpu, evicted)
    }

    #[tokio::test]
    async fn evicts_only_when_the_kind_of_work_changes() {
        let (gpu, evicted) = recording();
        drop(gpu.lease(Work::Prep, "a").await.unwrap());
        drop(gpu.lease(Work::Prep, "b").await.unwrap());
        drop(gpu.lease(Work::Render, "c").await.unwrap());
        drop(gpu.lease(Work::Render, "d").await.unwrap());
        drop(gpu.lease(Work::Prep, "e").await.unwrap());
        assert_eq!(*evicted.lock().unwrap(), [Work::Prep, Work::Render]);
    }

    #[tokio::test]
    async fn the_prompt_writer_is_evicted_before_prep_and_render() {
        let (gpu, evicted) = recording();
        drop(gpu.lease(Work::Llm, "describe").await.unwrap());
        drop(gpu.lease(Work::Llm, "describe again").await.unwrap());
        drop(gpu.lease(Work::Render, "render").await.unwrap());
        drop(gpu.lease(Work::Llm, "describe").await.unwrap());
        drop(gpu.lease(Work::Prep, "prep").await.unwrap());
        assert_eq!(*evicted.lock().unwrap(), [Work::Llm, Work::Render, Work::Llm]);
    }

    #[tokio::test]
    async fn a_second_lease_waits_for_the_first() {
        let (gpu, _) = recording();
        let first = gpu.lease(Work::Prep, "prep").await.unwrap();
        assert_eq!(gpu.holder(), Some(Holder { work: Work::Prep, name: "prep".into() }));
        let other = gpu.clone();
        let waiting = tokio::spawn(async move { other.lease(Work::Render, "render").await.map(|_| ()) });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert!(!waiting.is_finished());
        drop(first);
        waiting.await.unwrap().unwrap();
        assert_eq!(gpu.holder(), None);
    }

    #[tokio::test]
    async fn a_failed_eviction_gives_the_gpu_back() {
        let gpu = Gpu::new(|_| anyhow::bail!("ComfyUI did not free"));
        drop(gpu.lease(Work::Render, "render").await.unwrap());
        assert!(gpu.lease(Work::Prep, "prep").await.is_err());
        assert_eq!(gpu.holder(), None);
        // Still Render's memory, so the next prep tries again.
        assert!(gpu.lease(Work::Prep, "prep").await.is_err());
        assert!(gpu.lease(Work::Render, "render").await.is_ok());
    }
}
