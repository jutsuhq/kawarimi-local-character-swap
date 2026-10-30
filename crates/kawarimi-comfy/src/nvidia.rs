//! The machine's NVIDIA GPUs, for the settings screen and for picking the Wan weights.

use std::process::Command;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    pub memory_mb: u64,
    pub driver: String,
}

/// The oldest driver branch with CUDA 13.0, which the pinned PyTorch (cu130) needs.
pub const MIN_DRIVER: u32 = 580;
/// Below this, rounded, even the smallest Wan weights were never seen to fit.
pub const MIN_VRAM_GB: u64 = 8;

/// NVIDIA GPUs as `nvidia-smi` reports them; empty without an NVIDIA driver. `KAWARIMI_GPUS` stands in for
/// `nvidia-smi`'s CSV (`;` between GPUs, `none` for no GPU), to see the app on other machines.
pub fn gpus() -> Vec<Gpu> {
    if let Ok(fake) = std::env::var("KAWARIMI_GPUS") {
        return parse(&fake.replace(';', "\n"));
    }
    let mut command = Command::new("nvidia-smi");
    command.args(["--query-gpu=name,memory.total,driver_version", "--format=csv,noheader,nounits"]);
    crate::install::quiet(&mut command);
    let output = command.output();
    match output {
        Ok(out) if out.status.success() => parse(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

/// What keeps the first GPU from prep and render, with what to do about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Problem {
    NoGpu,
    OldDriver { driver: String },
    LowVram { memory_mb: u64 },
}

impl Problem {
    pub fn of(gpus: &[Gpu]) -> Option<Self> {
        let Some(gpu) = gpus.first() else { return Some(Self::NoGpu) };
        let major = gpu.driver.split('.').next().and_then(|m| m.parse::<u32>().ok());
        if major.is_none_or(|m| m < MIN_DRIVER) {
            return Some(Self::OldDriver { driver: gpu.driver.clone() });
        }
        ((gpu.memory_mb + 512) / 1024 < MIN_VRAM_GB).then_some(Self::LowVram { memory_mb: gpu.memory_mb })
    }

    /// Prep and render cannot run here at all; low memory only warns.
    pub fn blocks(&self) -> bool {
        !matches!(self, Self::LowVram { .. })
    }

    pub fn message(&self) -> String {
        match self {
            Self::NoGpu => "No NVIDIA GPU found. Prep and render run on an NVIDIA GPU with CUDA; this computer has none, or its driver is not installed.".into(),
            Self::OldDriver { driver } => format!("The NVIDIA driver {driver} is too old. Kawarimi needs driver {MIN_DRIVER} or newer (CUDA 13)."),
            Self::LowVram { memory_mb } => format!("The GPU has {} GB of video memory; Wan 2.2 Animate needs about {MIN_VRAM_GB} GB, so renders may run out of memory.", (memory_mb + 512) / 1024),
        }
    }

    pub fn advice(&self) -> &'static str {
        match self {
            Self::NoGpu => "Install the latest NVIDIA driver if the computer has an NVIDIA card. Without one, render on your own ComfyUI elsewhere (Settings › ComfyUI) from clips already prepared.",
            Self::OldDriver { .. } => "Update the driver from nvidia.com/drivers or GeForce Experience / the NVIDIA app, then restart Kawarimi.",
            Self::LowVram { .. } => "Close other apps that use the GPU, keep clips short, or render on your own ComfyUI on a bigger GPU (Settings › ComfyUI).",
        }
    }
}

fn parse(csv: &str) -> Vec<Gpu> {
    csv.lines()
        .filter_map(|line| {
            let mut fields = line.split(',').map(str::trim);
            let name = fields.next()?.to_owned();
            let memory_mb = fields.next()?.parse().ok()?;
            let driver = fields.next()?.to_owned();
            Some(Gpu { name, memory_mb, driver })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvidia_smi_csv() {
        let gpus = parse("NVIDIA GeForce RTX 3070, 8192, 610.88\nNVIDIA RTX A6000, 49140, 610.88\n");
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0], Gpu { name: "NVIDIA GeForce RTX 3070".into(), memory_mb: 8192, driver: "610.88".into() });
        assert!(parse("garbage").is_empty());
    }

    #[test]
    fn problems_are_found_in_order() {
        let gpu = |mb, driver: &str| Gpu { name: "GPU".into(), memory_mb: mb, driver: driver.into() };
        assert_eq!(Problem::of(&[]), Some(Problem::NoGpu));
        assert_eq!(Problem::of(&[gpu(4096, "472.12")]), Some(Problem::OldDriver { driver: "472.12".into() }));
        assert_eq!(Problem::of(&[gpu(6144, "610.88")]), Some(Problem::LowVram { memory_mb: 6144 }));
        assert_eq!(Problem::of(&[gpu(8192, "580.88")]), None);
        assert!(Problem::NoGpu.blocks() && !Problem::LowVram { memory_mb: 0 }.blocks());
        let json = serde_json::to_value(Problem::LowVram { memory_mb: 6144 }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "lowVram", "memoryMb": 6144 }));
    }
}
