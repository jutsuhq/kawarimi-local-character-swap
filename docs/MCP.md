# Kawarimi for AI agents (MCP)

`kawarimi-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server over stdio. An agent prepares a
clip, looks at the people in it, cuts out a character and queues the render, on this computer's GPU, with the app's
models, ComfyUI and jobs folder. What an agent renders shows up in the app's Jobs and Library.

```bash
cargo build --release -p kawarimi-mcp
claude mcp add kawarimi -- /path/to/target/release/kawarimi-mcp
```

Any MCP client that starts stdio servers works the same way (command: the binary, no arguments). The server reads
the app's folders (`%APPDATA%\dev.kawarimi.app` on Windows, `~/.local/share/dev.kawarimi.app` on Linux) and honours
the app's overrides: `KAWARIMI_DATA`, `KAWARIMI_ASSETS`, `KAWARIMI_COMFY`, `KAWARIMI_JOBS`. Logs go to stderr.

## Tools

| Tool | What it does |
|---|---|
| `status` | GPU, driver problems, the Wan weights for this card, what prep and render still miss, ComfyUI installed, installs in progress, who holds the GPU, this agent's usage and limits |
| `install` | Downloads `"prep"` (~2 GB), `"render"` (~15 GB), `"comfyui"` (Python packages, ~5 GB) or listed asset ids, in the background; `status` shows progress |
| `probe_clip` | Duration, size, frame rate, sound |
| `prepare_clip` | Shots, people and their masks for 4–30 s of a video (~15–40 s on the GPU); returns a prep id, the people and their previews as images |
| `view_person` | A person's preview and face |
| `view_frame` | A frame, with a person's mask tinted red |
| `cut_character` | The character off its background, cropped to it, as the render wants it |
| `prompt_template` | Kawarimi's prompt for replacing a person with a character |
| `estimate_render` | Size, frames, windows, steps, model, seconds and VRAM on this card |
| `start_render` | Queues a draft (first window) or final; returns a job id at once |
| `job_status`, `list_jobs`, `cancel_job` | Follow, list and cancel the agent's renders (`list_jobs` with `all` shows the app's too) |

Failures come back as tool results with `isError` and a sentence on what to do (install what is missing, wait for
the GPU, the limit reached).

## Limits and accounting

`<app data>/mcp/limits.json`, written with the defaults on the first start; only a person edits it:

```json
{ "queuedRenders": 2, "rendersPerDay": 10, "prepsPerDay": 50, "gpuMinutesPerDay": 120.0 }
```

Every prep, cutout, render created and the GPU time each held the card for go into `<app data>/mcp/ledger.jsonl`,
outside the jobs folder, so removing jobs does not reset the count. Days are the last 24 hours. A render is refused
when the agent already has `queuedRenders` waiting or running, when `rendersPerDay` is reached, or when its estimate
does not fit the GPU minutes left.

## Sharing the GPU with the app

Both take `<app data>/gpu.lock` for every prep, cutout and render, so they never use the card at once; `status`
says who holds it (`gpu.holder`). A job is run by one process at a time (`run.lock` in its folder). The server starts
its own ComfyUI when a render needs it and stops it when its queue is empty; the app frees its ComfyUI's models when
its queue is empty, so neither sits on video memory the other needs. The agent's unfinished renders resume the next
time the server starts.

## Files

Preps, character cutouts and scratch frames live in `<app data>/mcp/` (`preps/<id>`, `characters/<id>`, `scratch`);
renders go to the shared jobs folder.
