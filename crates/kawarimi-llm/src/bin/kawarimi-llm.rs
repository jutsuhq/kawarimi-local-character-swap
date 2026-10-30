//! Runs the local prompt writer once, for checks: starts llama-server, describes a clip and a character, stops it,
//! and prints the GPU's used memory at each step.
//!
//!     cargo run -p kawarimi-llm --bin kawarimi-llm -- [--dir .models] <video> <character image>

use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use kawarimi_assets::{Store, manifest};
use kawarimi_llm::{Paths, START_TIMEOUT, Server};

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = std::env::var("KAWARIMI_ASSETS").unwrap_or_else(|_| ".models".into());
    if let Some(i) = args.iter().position(|a| a == "--dir") {
        dir = args.get(i + 1).cloned().unwrap_or_default();
        args.drain(i..=i + 1);
    }
    let [video, character] = args.as_slice() else { anyhow::bail!("usage: kawarimi-llm [--dir .models] <video> <character image>") };
    let store = Store::new(&dir);
    let paths = Paths::from_store(&store)?;
    let ffmpeg = kawarimi_media::Ffmpeg::locate(Some(&store.dir(&manifest::FFMPEG))).context("FFmpeg is not installed")?;
    let log = std::env::temp_dir().join("kawarimi-llama-server.log");

    println!("before start: {} MiB used", used_mib()?);
    let started = Instant::now();
    let mut server = Server::start(&paths, &log, START_TIMEOUT)?;
    println!("ready in {:.1} s on port {}: {} MiB used", started.elapsed().as_secs_f64(), server.port, used_mib()?);
    let asked = Instant::now();
    let description = kawarimi_llm::describe(&server.endpoint(), &ffmpeg, video.as_ref(), character.as_ref(), |line| println!("  {line}"));
    println!("described in {:.1} s: {} MiB used", asked.elapsed().as_secs_f64(), used_mib()?);
    server.stop();
    std::thread::sleep(Duration::from_secs(1));
    println!("after stop: {} MiB used", used_mib()?);
    println!("{}", serde_json::to_string_pretty(&description?)?);
    Ok(())
}

fn used_mib() -> Result<String> {
    let out = Command::new("nvidia-smi").args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"]).output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
