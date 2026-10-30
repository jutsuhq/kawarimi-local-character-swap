//! Prepares a clip for rendering: `normalized.mp4`, `mask.mp4`, `people/<id>.png` and `manifest.json` in one folder.
//!
//!     kawarimi-prep <clip> [--out <folder>] [--models .models] [--start s] [--duration s] [--targets main|all|1,3]

use std::path::PathBuf;

use anyhow::{Context, Result};
use kawarimi_assets::{Status, Store, manifest};
use kawarimi_media::{Ffmpeg, Normalize};
use kawarimi_prep::{Models, Options, Targets};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let clip = PathBuf::from(args.first().context("usage: kawarimi-prep <clip> [--out <folder>]")?);
    let out = PathBuf::from(flag("--out").unwrap_or_else(|| format!("{}.prep", clip.display())));
    let store = Store::new(flag("--models").unwrap_or_else(|| ".models".into()));
    anyhow::ensure!(store.status(&manifest::FFMPEG) == Status::Installed, "FFmpeg is not installed (kawarimi-fetch ffmpeg)");
    let ffmpeg = Ffmpeg::locate(Some(&store.dir(&manifest::FFMPEG))).context("no FFmpeg")?;
    let models = Models::installed(&store)?;
    let targets = match flag("--targets").as_deref() {
        None | Some("main") => Targets::Main,
        Some("all") => Targets::Everyone,
        Some(ids) => Targets::Ids(ids.split(',').map(str::parse).collect::<Result<_, _>>().context("--targets")?),
    };
    let options = Options {
        normalize: Normalize {
            start: flag("--start").map(|s| s.parse()).transpose()?.unwrap_or(0.0),
            duration: flag("--duration").map(|s| s.parse()).transpose()?,
            ..Default::default()
        },
        targets,
        ..Default::default()
    };
    let manifest = kawarimi_prep::prepare(&ffmpeg, &models, &clip, &out, &options, |line| eprintln!("{line}"))?;
    eprintln!(
        "{} shots, {} tracklets, {} people; targets {:?} → {}",
        manifest.shots.len(),
        manifest.tracklets.len(),
        manifest.identities.len(),
        manifest.targets,
        out.display()
    );
    Ok(())
}
