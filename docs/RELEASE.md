# Releasing Kawarimi

GitHub Actions is not available on the repository's account, so releases are built on a developer machine:

```bash
scripts/release.sh            # Windows NSIS here (Git Bash), Linux AppImage and deb in Docker
```

It writes `release/<version>/` with the installers and `SHA256SUMS`, and prints the `gh release create` command.
Nothing is uploaded until that command is run.

## What the installers hold

- The app, `THIRD_PARTY_LICENSES.md` (from `scripts/licenses.py`: every shipped crate and npm package with its
  license text, every downloadable model and tool with its license and source, the font, the FFmpeg LGPL note), and
  ONNX Runtime's CUDA provider (`onnxruntime_providers_cuda` and `_shared`, about 94 MB), which ONNX Runtime loads
  from the executable's folder. The release configs `src-tauri/tauri.release.{windows,linux}.json` add them; dev
  builds don't need them because Cargo leaves them next to the debug binary.
- No models. FFmpeg, the CUDA 13 / cuDNN 9 runtime, ComfyUI and its Python packages, the Wan weights and the
  optional LLM download from Settings, each pinned to an exact revision and SHA-256.
- Linux needs only the NVIDIA driver (580 or newer): the CUDA and cuDNN libraries come from the `cuda-runtime`
  asset and are loaded from the app's data folder, not from a system CUDA install.

## Signing

There is no code-signing certificate yet, so the Windows installer is unsigned and SmartScreen warns on first run.
To sign, set `KAWARIMI_SIGN_COMMAND` to a command Tauri runs for every binary (`%1` is the file), for example
`signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 /a %1`, then run `scripts/release.sh`.

## Clean-machine check (#70)

Run on a fresh Windows 11 and a fresh Ubuntu 24.04, each with an NVIDIA GPU and a current driver, before
publishing. Note the GPU, driver and times.

1. `sha256sum -c SHA256SUMS` passes for the downloaded installers.
2. Install: NSIS on Windows; `.deb` (`sudo apt install ./kawarimi_*.deb`) or the AppImage (`chmod +x`, run) on
   Linux. The app starts; Settings shows the GPU and the driver.
3. Settings → About → Acknowledgements opens the licenses.
4. Settings → Models and tools: download FFmpeg, the CUDA runtime, the prep models; each ends as Installed.
5. Settings → ComfyUI: install, start, stop; no `python` process is left after stop or after closing the app.
6. Create: import a 5–10 s clip, prep finishes, pick the person, add a character image (≥ 800 px), write or
   generate the prompt.
7. Render a draft; it appears in Jobs with progress; close the app mid-render, reopen: the job resumes and ends
   with one result.
8. The result plays in Create's player and appears in Library; delete it: it lands in the recycle bin / trash.
9. Uninstall: the app is gone; the data folder (models, jobs) stays unless removed by hand.
10. On a machine without an NVIDIA GPU the app starts and explains why prep and render are unavailable.

## Publishing on jutsuhq

The repository lives at `jutsuhq/kawarimi-local-character-swap`, next to Swapjutsu, private until the first release.

Description, topics and visibility (the last step makes it public):

```bash
gh repo edit jutsuhq/kawarimi-local-character-swap \
  --description "Free open-source local AI character swap for video on Windows and Linux. Wan 2.2 Animate on your own NVIDIA GPU through ComfyUI, prep in Rust and ONNX Runtime (EdgeTAM, RT-DETRv2, BiRefNet). No cloud, no account, no per-video cost. A Viggle alternative with an MCP server. Unofficial." \
  --add-topic character-swap,video-to-video,wan-animate,wan-2-2,comfyui,tauri,rust,onnxruntime,cuda,nvidia \
  --add-topic local-ai,offline,edgetam,sam2,ai-video,generative-ai,mcp,windows,linux,viggle-alternative
gh repo edit jutsuhq/kawarimi-local-character-swap --enable-wiki=false --enable-discussions
gh repo edit jutsuhq/kawarimi-local-character-swap --visibility public --accept-visibility-change-consequences
```

Then the first release: `scripts/release.sh`, the clean-machine check above, and the printed `gh release create`
with `--prerelease` while it is an alpha.
