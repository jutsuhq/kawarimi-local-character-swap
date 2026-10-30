# Contributing

For anything larger than a small fix, open an issue first.

## Setup

Windows 10/11 (Git Bash) or Linux, x86_64, with an NVIDIA GPU and a current driver. Rust (stable), Node 24 with pnpm,
and on Linux `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`.

```bash
pnpm install
pnpm tauri dev
```

Models and tools are not committed. Download them from the app's Settings, or for the command-line tools:

```bash
cargo run -p kawarimi-assets --bin kawarimi-fetch -- --list
cargo run -p kawarimi-assets --bin kawarimi-fetch -- --dir .models ffmpeg cuda-runtime
```

## Layout

| Crate | What it does |
|---|---|
| `src-tauri` | The app: Tauri commands, jobs, storage, settings |
| `crates/kawarimi-prep` | Clip preparation: shots, people, tracking, identities, masks, cutouts |
| `crates/kawarimi-comfy` | ComfyUI install and server, Wan 2.2 Animate workflow, render jobs |
| `crates/kawarimi-llm` | The optional prompt writer |
| `crates/kawarimi-assets` | Pinned downloads, checked by SHA-256 |
| `crates/kawarimi-media` | FFmpeg as a child process |
| `crates/kawarimi-mcp` | The MCP server for agents |

The tracker lives in its own repository, [edgetam-onnx](https://github.com/jutsuhq/edgetam-onnx).

## Tests

```bash
scripts/ci.sh            # frontend build, FFmpeg, cargo test, app build
scripts/ci.sh --docker   # the same on Ubuntu 24.04 in Docker
```

GPU-bound tests skip themselves without CUDA and the models.

## Releases

Installers are built on a developer machine with `scripts/release.sh`; the steps and the clean-machine checklist are
in [docs/RELEASE.md](docs/RELEASE.md).
