//! Installs pinned assets for development and tests, the same way the app does.
//!
//!     cargo run -p kawarimi-assets --bin kawarimi-fetch -- [--dir .models] <asset id>... | --all | --list | --licenses

use std::io::Write;

use anyhow::{Result, bail};
use kawarimi_assets::{ALL, Status, Store};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = std::env::var("KAWARIMI_ASSETS").unwrap_or_else(|_| ".models".into());
    if let Some(i) = args.iter().position(|a| a == "--dir") {
        dir = args.get(i + 1).cloned().unwrap_or_default();
        args.drain(i..=i + 1);
    }
    if args.first().map(String::as_str) == Some("--licenses") {
        let list: Vec<_> = ALL
            .iter()
            .map(|a| serde_json::json!({ "id": a.id, "title": a.title, "license": a.license, "source": a.source }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    let store = Store::new(&dir);
    if args.is_empty() || args[0] == "--list" {
        for asset in ALL {
            let status = if store.status(asset) == Status::Installed { "installed" } else { "missing" };
            println!("{:<18} {:>8.1} MB  {:<9} {}", asset.id, asset.download_size() as f64 / 1e6, status, asset.title);
        }
        return Ok(());
    }
    let wanted: Vec<_> = if args[0] == "--all" {
        ALL.iter().collect()
    } else {
        args.iter().map(|id| kawarimi_assets::find(id).ok_or_else(|| anyhow::anyhow!("unknown asset {id}"))).collect::<Result<_>>()?
    };
    let mut failed = 0;
    for asset in wanted {
        if store.status(asset) == Status::Installed {
            println!("{}: installed", asset.id);
            continue;
        }
        let last = std::cell::Cell::new(u64::MAX);
        let result = store
            .install(asset, |done, total| {
                let percent = done * 100 / total.max(1);
                if percent != last.replace(percent) {
                    print!("\r{}: {percent:>3}%", asset.id);
                    let _ = std::io::stdout().flush();
                }
            })
            .await;
        match result {
            Ok(()) => println!("\r{}: installed in {}", asset.id, store.dir(asset).display()),
            Err(e) => {
                println!("\r{}: {e:#}", asset.id);
                failed += 1;
            }
        }
    }
    if failed > 0 {
        bail!("{failed} asset(s) failed");
    }
    Ok(())
}
