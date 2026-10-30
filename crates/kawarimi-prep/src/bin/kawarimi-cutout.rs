//! Lifts characters off their backgrounds: `<name>.png` on grey, cropped to the character as the app hands it to Wan,
//! and `<name>.alpha.png` at the image's size, in the output folder.
//!
//!     kawarimi-cutout <image>... [--out <folder>] [--models .models]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use kawarimi_assets::{Status, Store, manifest};
use kawarimi_prep::cutout::Cutter;
use edgetam_onnx::Device;

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |name: &str| {
        let i = args.iter().position(|a| a == name)?;
        let value = args.get(i + 1).cloned();
        args.drain(i..(i + 2).min(args.len()));
        value
    };
    let out = PathBuf::from(take("--out").unwrap_or_else(|| "cutouts".into()));
    let store = Store::new(take("--models").unwrap_or_else(|| ".models".into()));
    anyhow::ensure!(!args.is_empty(), "usage: kawarimi-cutout <image>... [--out <folder>]");
    for asset in [&manifest::CUDA_RUNTIME, &manifest::BIREFNET_LITE] {
        anyhow::ensure!(store.status(asset) == Status::Installed, "{} is not installed (kawarimi-fetch {})", asset.title, asset.id);
    }
    edgetam_onnx::cuda::use_runtime(&store.dir(&manifest::CUDA_RUNTIME))?;
    let mut cutter = Cutter::load(&store.dir(&manifest::BIREFNET_LITE).join("model.onnx"), Device::Cuda)?;
    std::fs::create_dir_all(&out)?;
    for path in &args {
        let img = image::open(path).with_context(|| format!("opening {path}"))?.into_rgb8();
        let (w, h) = img.dimensions();
        let t = Instant::now();
        let cut = cutter.cut(img.as_raw(), w as usize, h as usize)?;
        let stem = PathBuf::from(path).file_stem().unwrap_or_default().to_string_lossy().into_owned();
        cut.crop(w, h).save(out.join(format!("{stem}.png")))?;
        image::GrayImage::from_raw(w, h, cut.alpha).unwrap().save(out.join(format!("{stem}.alpha.png")))?;
        eprintln!("{stem}: {w}×{h}, {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(())
}
