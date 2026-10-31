# Render: Wan 2.2 Animate on a local ComfyUI

## What fits where (issue #37, 2026-10-20)

A research spike from published files and reports; nothing here is measured on our machine yet. Marked (?) where no
source confirmed it.

### Weights

| Variant | File and size | License | Memory reports |
|---|---|---|---|
| Comfy bf16 | `Comfy-Org/Wan_2.2_ComfyUI_Repackaged` `split_files/diffusion_models/wan2.2_animate_14B_bf16.safetensors`, 34.5 GB | Apache-2.0 | none for Animate; with ComfyUI's offload ~10 GB stays on the GPU, the rest in RAM (needs ≥ 48 GB RAM (?)) |
| Comfy int8 | same folder, `wan2.2_animate_14B_int8_convrot.safetensors`, 18.4 GB | Apache-2.0 | (?) |
| Kijai fp8 scaled | `Kijai/WanVideo_comfy_fp8_scaled` `Wan22Animate/Wan2_2-Animate-14B_fp8_e4m3fn_scaled_KJ.safetensors` 18.4 GB (the template's), `…_KJ_v2` 17.3 GB; e5m2 variants for RTX 30xx `torch.compile` | Apache-2.0 | native nodes, 16 GB card, 32 GB RAM: 8.1 GB at a 45-frame window ([wrapper #1272](https://github.com/kijai/ComfyUI-WanVideoWrapper/issues/1272)) |
| GGUF (QuantStack) | `QuantStack/Wan2.2-Animate-14B-GGUF`: Q2_K 6.5, Q3_K_S 8.0, Q3_K_M 8.6, Q4_K_S 10.6, Q4_K_M 11.5, Q5_K_M 13.0, Q6_K 14.6, Q8_0 18.7 GB; needs the ComfyUI-GGUF node | Apache-2.0 | native Q4 GGUF 9.9 GB at 77 frames (#1272); Q4_K_M at 720×960 without OOM on 16 GB ([#1805](https://github.com/kijai/ComfyUI-WanVideoWrapper/issues/1805), Wan 2.2 base (?)) |

Shared by every variant: text encoder `umt5_xxl_fp8_e4m3fn_scaled` 6.74 GB (or GGUF Q4_K_M 3.7 GB,
`city96/umt5-xxl-encoder-gguf`), run once per prompt; `wan_2.1_vae` 254 MB, where a tiled decode avoids the common
OOM; `clip_vision_h` 1.26 GB; the 4-step distilled LoRA `lightx2v_I2V_14B_480p_cfg_step_distill_rank64_bf16` 738 MB
(the template uses it at CFG 1; ~10× fewer model evaluations than 20 steps with CFG, not measured on Animate (?)).

### Choice per card

| VRAM | Weights | Notes |
|---|---|---|
| 8 GB | GGUF Q3_K_M (8.6 GB) or Q4_K_S (10.6 GB), umt5 GGUF Q4, 480p, 4 steps, tiled VAE | **No Animate run on 8 GB was found.** ComfyUI's native offload keeps ~8–10 GB resident at 77 frames, so it may fit with RAM taking the rest (32 GB is enough for a ~11 GB GGUF plus the encoder). To be measured here. |
| 12 GB | GGUF Q4_K_M (11.5 GB), 480p; 720p reported at ~12 GB peak (?) | |
| 16 GB | Kijai fp8 scaled v2 (17.3 GB), native nodes, 480p and 720p | Kijai's wrapper spikes past 15 GB at 720×1280 on the next window ([#1722](https://github.com/kijai/ComfyUI-WanVideoWrapper/issues/1722)); the native nodes do not use it. |
| 24 GB | Kijai fp8 scaled, or GGUF Q8_0 for quality, 720p | Native nodes again: a wrapper user saw ~30 GB spikes at 1280×720 ([#1267](https://github.com/kijai/ComfyUI-WanVideoWrapper/issues/1267)). |

Pinned (issue #40): `wan-animate-q3km` below 12 GB, `wan-animate-q4km` (Q4_K_M) at 12 GB, `wan-animate-fp8` (Kijai's
`…_fp8_scaled_e4m3fn_KJ_v2`, loaded by the core `UNETLoader` instead of `UnetLoaderGGUF`) from 16 GB, by the first
GPU's memory rounded to whole GB (`Variant::for_vram`). Only Q3_K_M is measured; the 12 and 16 GB picks follow the
table above. A 24 GB card gets the fp8 v2 as well; Q8_0 is not pinned. `kawarimi-comfy render --model` overrides it.

ComfyUI's native Animate nodes landed in September 2025 (first release with all of it: v0.3.60); we pin v0.39.0.
DWPose comes from `comfyui_controlnet_aux`. The template's KJNodes Points Editor and SAM 2 are replaced by prep's mask,
as in Swapjutsu.

Newer, and worth a look before the weights are fixed: **Wan Animate 2** (August 2026, `Comfy-Org/Wan-Animate-2`,
Apache-2.0, int8 16.7 GB, nodes `WanAnimate2ToVideo` and `WanAnimate2Cache`, which Comfy says halves the time at a
system-RAM cost). No memory reports yet.

No published seconds per window for Animate on a named GPU were found, hence the measurement below.

### Measured: RTX 3070 8 GB, 32 GB RAM (issues #40, #43, #44; 2026-10-20)

`bedroom.mp4` prepped with `--targets main`; one 77-frame window at 848×480, GGUF Q3_K_M, umt5 Q4_K_M, LightX2V,
4 steps at CFG 1. The desktop already held 1.9–2.7 GB of VRAM (wallpaper, browsers) and ~18 GB of RAM.

| | |
|---|---|
| ComfyUI start, inputs uploaded | 90 s (cold imports of the custom nodes once took > 180 s) |
| Graph, start to saved video | 15 min 44 s; 17 min 22 s with the start |
| Sampling | 4 steps in 5 min 15 s, 79–85 s per step |
| The rest | text encoder, CLIP vision, DWPose on 77 frames (~0.2 s per frame), VAE decode: ~10 min |
| Peak VRAM, whole GPU | 7.8 GB of 8 GB |
| Wan on the GPU | none: `loaded partially; 0.00 MB usable, 0.00 MB loaded, 8439.37 MB offloaded`; every step streams the weights from RAM |
| Peak RAM, ComfyUI's Python | 20.9 GB; free RAM fell to 43 MB |

It runs, but 8 GB is the floor, not a fit: at ~4.8 s of video per 17 minutes, a 30 s clip is close to two hours, and
the RAM it takes leaves the rest of the machine paging. Ways to try before calling 8 GB supported: Q2_K or Q3_K_S
(6.5 / 8.0 GB) so part of Wan stays on the GPU, freeing the text encoder after the prompt, fewer frames per window,
and Wan Animate 2.

The reference image: `WanAnimateToVideo` scales it to the frame with a centre crop, which cut a portrait character
(161×382) down to its torso, and the first render invented a different person. Kawarimi now fits the character
whole into the frame on grey (`wan::reference`) before uploading it. The second render, same settings, with the
fitted reference: the outfit came through (the blue swimsuit), the purple hair, bow and anime face did not; the
reference was a small cutout (161×382), and Q3 with 4 steps is the low end. That run took 10 min 23 s for the graph
(sampling 56.5 s per step, the rest warm from the first), peak VRAM 7.2 GB, ComfyUI 22.3 GB of RAM, 410 MB free.

### Quantisation, freeing the text encoder, reference size (2026-10-20)

Same clip and settings, seed 0, character 05 of `.cache/characters` (an anime girl with a beret, 1280×852, cut out
and cropped to 879×852). Likeness is `scripts/likeness.py`: CLIP vision H cosine between the character and the
masked person in every 4th frame; the untouched source scores 0.323 against the same character.

| Run | Wan on the GPU | Sampling, 4 steps | Graph | Peak VRAM | ComfyUI RAM | Free RAM, min | Likeness |
|---|---|---|---|---|---|---|---|
| Q3_K_M (8.6 GB) | 0 MB, 8439 MB offloaded | 84.6 s/step | 15:58 | 7.7 GB | 21.4 GB | 42 MB | 0.442 |
| Q3_K_M, `--cache-none` | 0 MB | 103.3 s/step | 15:07 | 7.8 GB | 20.5 GB | 232 MB | 0.442 |
| Q3_K_M, `--disable-smart-memory` | 0 MB | 82.8 s/step | 18:03 | 7.0 GB | 21.8 GB | 2 MB | 0.442 |
| Q3_K_S (8.0 GB) | 0 MB, 7809 MB offloaded | 56.8 s/step | 10:46 | 7.7 GB | 23.6 GB | 278 MB | 0.406 |
| Q2_K (6.5 GB) | 0 MB, 6367 MB offloaded | 71.3 s/step | 9:39 | 7.6 GB | 23.5 GB (7.1 GB another Python at start) | 108 MB | 0.330 |

- **Smaller quants do not put Wan on the GPU.** Even Q2_K gets 0 MB: what is left of 8 GB after the desktop goes to
  the activations of a 77-frame window at 848×480, not to weights. Times vary with the machine's load as much as
  with the quant (the same Q3_K_M ran at 56.5 s/step earlier), and the likeness drops clearly: Q3_K_S loses the
  beret, Q2_K the beret and the face. Q3_K_M stays; the others are not assets.
- **Freeing the text encoder changes nothing.** `--cache-none` keeps it on the GPU (3.4 GB "remains loaded"), and
  `--disable-smart-memory` frees it but Wan still gets 0 MB; the three videos are identical. Neither is a default.
  `kawarimi-comfy render --comfy-args "…"` passes such flags for experiments.
- **The reference matters most.** With a 1280-px character the likeness went from "an outfit" to the beret, twin
  tails and face; with the 161×382 one it stayed at the source's level. The app should ask for a character image
  of at least ~800 px and warn below it.
- **A shorter window puts part of Wan on the GPU.** One 33-frame window (Q3_K_M, same settings): Wan gets 2.9 GB
  usable and loads 2.7 GB (`loaded partially; 2858.75 MB usable, 2713.47 MB loaded, 5725.91 MB offloaded`), sampling
  takes 21.1 s/step against 84.6 at 77 frames: 2.6 s of sampling per frame instead of 4.4, and 3.0 s per new frame
  of a chained window (28 new frames) instead of 4.7 (72). The whole graph took 6:30 for 33 frames (11.8 s/frame
  against 12.4), because the text encoder, CLIP vision, DWPose and loading do not shrink with the window. Likeness
  0.455, on par with 77 frames. Peak VRAM 6.8 GB; RAM unchanged (22.0 GB, 352 MB free). So on 8 GB the window
  length should be 33, and the fixed costs per render are the next thing to cut.
- **RAM is the limit on 32 GB.** Every run left the machine with almost no free RAM; with ~18 GB taken by other
  programs, Windows pages. Lower resolution is untested.

### Grown masks (2026-10-30)

Prep grows every person mask by the frame's short side / 30 (18 px at 540, 16 at 480; a square max filter in
`write_mask`), and a frame inside a tracklet where the tracker lost the person takes the nearest frame's mask (the
manifest counts them as `tracking.filledFrames`; `bedroom` has none). The idea is Imagination Studio's Character Swap
(64 px at 768). The masked area doubles on `bedroom` (6.6% of the frame to 12.9%).

Same clip and character as above, one 33-frame window, Q3_K_M, 4 steps, seed 0; likeness scored on the same
ungrown mask for both:

| Mask | Likeness |
|---|---|
| as tracked (radius 0) | 0.453 ± 0.085 |
| grown 18 px | 0.591 ± 0.044 |
| the source, untouched | 0.323 |

With the tight mask Wan kept the girl's outline: the twin tails were cut off at the mask and the face turned away in
some frames. With the grown one the twin tails, beret and face stay on every frame. No halo of redrawn background
showed in any of the four frames checked (0, 12, 24, 32); the wall stripes and bed next to the grown edge match the
source. A larger radius was not tried.

### Anchor first frame (2026-10-30)

Imagination Studio's Character Swap (Apache-2.0, `spark/spark_swap.py` `anchor()`) first has an image-edit model
put the character into the clip's first frame, and the video model starts from it. Tried here with the smallest edit
model that runs in the pinned ComfyUI's core nodes (`TextEncodeQwenImageEditPlus`, `FluxKontextMultiReferenceLatentMethod`
`index_timestep_zero`, `CFGNorm`) plus ComfyUI-GGUF, all Apache-2.0:

| File | Size |
|---|---|
| `unsloth/Qwen-Image-Edit-2511-GGUF` `qwen-image-edit-2511-Q3_K_M.gguf` | 9.92 GB |
| `unsloth/Qwen2.5-VL-7B-Instruct-GGUF` `Qwen2.5-VL-7B-Instruct-Q4_K_M.gguf` + `mmproj-F16.gguf` (renamed `Qwen2.5-VL-7B-Instruct-mmproj-F16.gguf`: ComfyUI-GGUF finds it by the encoder's name) | 4.68 + 1.35 GB |
| `lightx2v/Qwen-Image-Edit-2511-Lightning` `…-4steps-V1.0-bf16.safetensors` | 0.85 GB |
| `Comfy-Org/Qwen-Image` `split_files/vae/qwen_image_vae.safetensors` | 0.25 GB |

17.1 GB in all. Edit: frame 0 of `bedroom` (848×480, scaled to 1360×768, the ~1 MP the node encodes references at)
as image 1, character 05 as image 2, Imagination Studio's prompt ("Image 1 with the person replaced by the character
of image 2, same pose, same framing, same background; the character wears the outfit of image 2."), 4 steps, CFG 1,
seed 0, scaled back to 848×480. The anchor goes into the first `WanAnimateToVideo` as `continue_motion` with
`continue_motion_max_frames` 1 (the input steps by 4 from 1; 1 and 5 both take the one frame): its latent is kept,
the pose and background still start at frame 0, and `trim_image` is 1, so the window's frame 0 is dropped and an
`ImageBatch` puts the anchor back in front, 33 frames out. Then one 33-frame window, Q3_K_M, 4 steps, seed 0, both
graphs on one ComfyUI with `/free` between.

| Run | Sampling, 4 steps | Graph | Peak VRAM, whole GPU | Free RAM, min | Likeness (every frame) |
|---|---|---|---|---|---|
| Anchor edit | 18:44 (281 s/step; 4.6 GB on the GPU, 5.0 GB offloaded; the encoder 5.6 GB loaded) | 26:09 | 7.6 GB | 77 MB | |
| Wan, no anchor | 2:24 | 10:34 | 7.5 GB | 17 MB | 0.441 |
| Wan, anchor | 2:33 | 9:35 | 7.5 GB | 84 MB | **0.530** |

(ComfyUI's own RAM was not caught: the sampler watched the venv's launcher, not the Python it starts. The free RAM
says the same as above: 32 GB is used up.)

- **The anchor holds the character.** Without it the window is a dark-haired girl in a navy dress; with it the beret,
  the long twin tails with their blue bow, the sailor collar and pleated skirt stay to frame 32, also turned away from
  the camera. +0.089 likeness, the largest change any setting made here.
- **Frame 0 is the edit, not the clip.** Qwen kept the framing and background but not the exact pose (arms out where
  the girl jumps), so frame 0 → 1 jumps; the motion is the clip's from frame 1 on. The person region of the anchor
  differs from the source by 71.6 grey levels on average (6.0 is Imagination Studio's no-op threshold).
- **It costs more than the render.** The edit at ~1 MP took 26 minutes on 8 GB, against 10 for a 33-frame window:
  Qwen-Image's 20 B transformer streams from RAM at every step like Wan does. It is worth it for a final; editing at
  the window's 848×480 instead of 1 MP is the first thing to try to cut it (not measured).

In the code: the optional asset `qwen-edit` (the five files above), `data/anchor-edit.api.json`, and
`kawarimi_comfy::anchor`: `make` takes frame 0 of `normalized.mp4` and `mask.mp4`, runs the edit, frees ComfyUI's
models for Wan, and fails when the grey-level change inside the mask is under 6.0 (Imagination Studio's threshold for an
edit that returned the frame unchanged); the caller then renders without an anchor. `wan::Inputs::anchor` feeds the
result to the first window as above. Not yet asked for by a job or the UI.

### Where the time goes, per node (2026-10-30)

One 33-frame window, Q3_K_M, 848×480, 4 steps, seed 0, character 05, from a cold ComfyUI (`kawarimi-comfy render
--preset draft --frames 33 --window 33 --steps 4 --short-side 480`). Node times come from ComfyUI's websocket
(`executing` messages, `kawarimi_comfy::timing`) and are kept in `job.json` (`nodes`). A node's time includes what
ComfyUI loads lazily for it.

| Node | Class | Seconds |
|---|---|---|
| 102 | KSampler: loading Wan, then 4 steps (82.8 s, 20.7 s/step) | 190.9 |
| 4 | CLIPLoaderGGUF (umt5 Q4_K_M, dequantises the token embeddings) | 57.9 |
| 15 | DWPreprocessor, body and hands | 30.6 |
| 5 | CLIPTextEncode, the prompt (the encoder moves to the GPU, 4.7 GB) | 24.7 |
| 14 | DWPreprocessor, face | 15.4 |
| 101 | WanAnimateToVideo | 12.7 |
| 104 | VAEDecode | 7.4 |
| | 25 other nodes | 19.3 |
| | Graph | 358.9 (5:59) |

RAM, from the watchdog (`job.json` `ram`): ComfyUI's process tree peaked at 22.3 GB; free physical RAM fell to
31 MB; commit headroom, counting the page file's room to grow, fell to 2.0 GB, 0.5 GB above the floor at which the
guard stops ComfyUI. The pre-check said the render would page (24 GB needed, 21.2 GB free) and let it run.

Outside the 82.8 s of sampling, ~276 s are fixed costs. Three ways to cut the largest:

1. **Keep the text encoder's output.** Loading umt5 and encoding take 85 s (nodes 4, 5, 6) for a prompt that
   rarely changes. Conditioning saved per prompt text (two 512×4096 tensors, a few MB) skips the 4.7 GB encoder on
   a repeat render: about −85 s, and 4.7 GB less to page. ComfyUI's own cache does this only while the same server
   stays up with the same prompt.
2. **One pose pass instead of two.** Nodes 14 and 15 run DWPose over the same 33 frames, face only and then body
   and hands. Inference is ~0.09 s per frame (37 ms bbox, ~50 ms pose, from the log); the other ~40 s are loading
   YOLOX and the pose model. One pass with body, hands and face, whose face crops feed `face_video`, saves about
   −15 s per render (node 14). Computing pose in prep, which already finds the person's boxes, would remove both
   nodes and YOLOX: about −46 s.
3. **Load Wan once per ComfyUI, not once per render.** Of KSampler's 190.9 s, 108 s are loading and offloading
   the 8.4 GB of weights before the first step. A ComfyUI kept warm between renders (the app keeps it; the CLI
   starts a fresh one) skips that when RAM still holds the weights. This needs RAM to spare, which 32 GB at a 22 GB
   peak does not have, so the saving is up to −108 s on a bigger machine and less here.

## ComfyUI on the user's machine (issues #38, #41)

`kawarimi-comfy` installs ComfyUI into a folder the app owns and runs it as a child process.

- **Pins.** `uv` 0.12.22 and ComfyUI's source at v0.39.0 are assets (`uv`, `comfyui`) with size and SHA-256, like
  every model. ComfyUI's Python packages are locked per platform with hashes
  (`crates/kawarimi-comfy/data/requirements-<platform>.txt`, from `requirements.in` by `scripts/comfy_lock.sh`):
  PyTorch 2.14.1 for CUDA 13.0 plus ComfyUI's own requirements, installed with `uv pip install --require-hashes`.
- **Install.** uv installs CPython 3.12 into the folder, makes a venv and installs the lock; a marker records the
  lock's hash and the source, so a changed pin counts as not installed. Models, custom nodes, input, output and user
  data live under `data/` (`--base-directory`).
- **Server.** A free local port, `--listen 127.0.0.1 --disable-auto-launch`, ready when `/system_stats` answers,
  output in `logs/server.log`, stopped with its whole process tree (taskkill /T on Windows, the process group on
  Linux), also when dropped. On Windows a job object kills the tree if the app itself is killed.
- **In the app** (`src-tauri/src/comfy.rs`). Installed from Settings into `<app data>/comfy` (`KAWARIMI_COMFY`
  overrides it), started on demand (`Comfy::ensure`, again if it died) and reused, stopped when the app exits. Its
  state goes to the UI as `comfy-state` events: not installed, installing, stopped, starting, ready, failed with the
  end of the log.
- **One GPU** (`src-tauri/src/gpu.rs`). Prep and render take turns through `Gpu::lease(work, name)`; when the kind
  of work changes, the last one's memory is evicted first: ComfyUI's models through `/free` (it is stopped if it
  does not answer). Prep's ONNX Runtime sessions end with `prepare()`, so a prep leaves nothing behind.

- **Custom nodes and weights.** `comfyui_controlnet_aux` (DWPose), `ComfyUI-GGUF` and the weights are assets.
  ComfyUI finds them where they are installed through an `extra_model_paths.yaml` written at start (`custom_nodes`,
  `diffusion_models`, `text_encoders`, `vae`, `clip_vision`, `loras`); DWPose's checkpoints through
  `AUX_ANNOTATOR_CKPTS_PATH`. `HF_HUB_OFFLINE=1`, so no node downloads a model of its own.
- **Workflow.** `data/wan-animate.api.json` is Swapjutsu's graph (MIT) with the GGUF loaders; `wan::make` fills it
  and chains windows (77 frames, or the preset's) with a 5-frame overlap for longer clips.
- **Draft and final** (issue #47, `wan::Preset`). Both use the same seed. The draft is the first window at 480p, 4 steps.
  The final is the whole clip. Under 11 GB of VRAM (as ComfyUI reports it) windows are 33 frames, the length measured
  above, and the final stays at 480p. With the same windows and size, the final's first window is the draft's
  frames, so it repeats the draft's motion. From 11 GB on, windows are 77 frames and the final is 720p; nothing
  above 8 GB has been measured, so 720p and that threshold are unmeasured guesses. `--short-side`, `--steps`,
  `--frames` and `--window` override the preset.
- **Finish** (issue #46, `Ffmpeg::finish`). ComfyUI's video, cut to the clip's frame count at 16 fps, is encoded again
  as H.264 (NVENC). The sound is taken from prep's `normalized.mp4`, which already has the clip's trim, and is
  padded or cut to the same length, so the duration is frames ÷ 16 s, as `normalized.mp4`'s is. The frame rate stays
  at 16 fps (there is no interpolation back to the source's rate). A render with fewer frames than asked for fails
  instead of coming out short.
- **Client.** Upload (`/upload/image`, video too), `/prompt` (rejections name the node), a prompt's state from
  `/queue` and `/history` (errors name the node), a prompt found by where it saves, cancel, `/view` to download,
  `/free`.
- **Jobs** (issue #45, `kawarimi_comfy::job`). A render is a job with its own folder (`<app data>/jobs/<id>`, the CLI's
  `.cache/jobs`): `job.json` holds the spec (clip or prep folder, character, settings), the step (prep → upload →
  queued → rendering → downloading → finishing) and an end (done with the result, failed with a reason, or
  cancelled). It is saved after every step, and every file a step makes is written as `<name>.part.<ext>` and
  renamed when whole, so a kill never leaves a half file under its final name.
  - **No second render.** The graph saves under `kawarimi/<job id>/render` in ComfyUI's output folder. Before it
    queues, a job takes a whole video already saved there (checked by its frame count, since a kill while ComfyUI
    saves leaves a short one), else a prompt of its own still in ComfyUI's queue or history; it queues only when
    ComfyUI has neither, which happens when ComfyUI was restarted (it dies with the app). It queues `graph.json`,
    saved the first time, so a second submission is the same graph and seed. More than three submissions fail the
    job.
  - **ComfyUI dying** mid-render is noticed when it stops answering: it is started again (up to three times per run)
    and the job carries on as after a restart.
  - **Cancel** ends the job first, then takes its prompt off ComfyUI's queue (`/queue` delete) or interrupts it if
    it runs (`/interrupt` with its prompt id), leaving other prompts alone; the GPU lease ends with the run. A job
    being prepared stops when prep returns.
  - **RAM** (`kawarimi_comfy::ram`). Before a job queues on a ComfyUI on this machine, the variant's RAM need
    (Q3_K_M measured at 24 GB; the others estimated from the difference in weights) is compared with free RAM and
    commit headroom: refused when RAM and page file together cannot hold it above the floor, a warning that it will
    page when only physical RAM is short. The render estimate shows the same check. While ComfyUI runs, a watchdog
    reads memory 4 times a second and stops ComfyUI's process tree when commit headroom (counting a system-managed
    page file's room to grow) falls under 1.5 GB, and the job fails with that reason instead of restarting ComfyUI.
    Free physical RAM is not the signal: it is near zero in every working render on 32 GB.
  - **In the app** (`src-tauri/src/jobs.rs`) one worker runs jobs oldest first under `Gpu::lease` (prep's lease for
    prep, render's from upload to finishing); at start it resumes every unfinished job. Commands `jobs_list`,
    `job_create`, `job_cancel`; each change is a `job-state` event.

```bash
cargo run -p kawarimi-assets --bin kawarimi-fetch -- uv comfyui comfyui-controlnet-aux comfyui-gguf dwpose   wan-animate-q3km umt5-q4km wan-vae clip-vision-h lightx2v
cargo run --release -p kawarimi-comfy -- install --root .cache/comfy
cargo run --release -p kawarimi-comfy -- check --root .cache/comfy
cargo run --release -p kawarimi-comfy -- render --prep .cache/prep/bedroom-main --character character.png   --out draft.mp4 --preset draft --seed 0
cargo run --release -p kawarimi-comfy -- render --prep .cache/prep/bedroom-main --character character.png   --out final.mp4 --preset final --seed 0
cargo run --release -p kawarimi-comfy -- resume                   # unfinished jobs in .cache/jobs, e.g. after a kill
cargo run --release -p kawarimi-comfy -- finish --prep .cache/prep/bedroom-main --video final.comfy.mp4 --out final.mp4
```
