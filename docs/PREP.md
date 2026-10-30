# Prep: from a clip to the renderer's inputs

`kawarimi-prep <clip>` (crate `kawarimi-prep`) writes one folder per clip:

| File | What it is |
|---|---|
| `normalized.mp4` | The clip upright, at 16 fps (Wan 2.2 Animate's rate), long side at most 960 px, with its audio |
| `mask.mp4` | The targets' masks, white on black, lossless (NVENC) |
| `people/<id>.png` | Each person's largest view, for choosing targets |
| `manifest.json` | Shots, tracklets, identities, targets, appearance distances, timings |
| `cache.json` | What a second run reuses (below) |

```bash
cargo run -p kawarimi-assets --bin kawarimi-fetch -- ffmpeg cuda-runtime transnetv2 rtdetr-v2-r18 edgetam-video
scripts/clips.sh                                   # the test clips, from pinned sources
cargo run --release -p kawarimi-prep -- .cache/clips/cuts.mp4 --out .cache/prep/cuts --targets all
```

## Stages

| Stage | How | Where |
|---|---|---|
| Normalise | FFmpeg: rotation applied, `fps=16`, Lanczos to ≤ 960 px, H.264 on NVENC (OpenH264 without it) | `kawarimi-media` |
| Shots | TransNetV2 on the GPU over 48×27 frames, upstream's windows (100, step 50, middle 50) and scene rule (threshold 0.5) | `shots.rs` |
| Decode | Every frame once into a raw RGB file, read back as each shot is tracked | `frames.rs` |
| People | RT-DETRv2 R18 every 8 frames, score ≥ 0.5, duplicates above IoU 0.7 dropped | `people.rs` |
| Tracking | One pass per shot, each frame encoded once for everyone on it. Each person not yet covered (IoU < 0.3 with a kept tracklet's mask box) seeds EdgeTAM from its box; the tracklet ends after 12 frames without the object; tracklets seen in under 20 % of the shot (and under 8 frames) are dropped | `track.rs` |
| Identities | HSV histogram inside the masks (12×4×4 bins, 8 frames), Bhattacharyya distance, average linkage below 0.40, tracklets seen together never merge; pairs within 0.05 of the threshold listed as close calls; a person made only of tracklets under 8 frames (a glimpse across a cut) joins the closest person it is never seen with instead of becoming one | `identity.rs` |
| Targets | The most screen presence (mask pixels over all frames), everyone, or ids | `pipeline.rs` |
| Mask video | The union of the targets' masks per frame | `pipeline.rs` |

A second run on the same folder reuses the normalised clip and the shots while the source (size, modified time)
and the normalising options are unchanged, and the tracklets too while the tracking options and the model pins are.
Changing only `--targets` then redoes the identities and the mask video: 2.5 s instead of 21 s on `cuts.mp4`.

## Test clips

Swapjutsu's benchmark annotations (`benchmarks/data/annotations`) are masks for clips that are not in its
repository (they come from Higgsfield's catalogue), so prep is checked on clips anyone can rebuild with
`scripts/clips.sh`:

- `cuts.mp4`: EdgeTAM's bedroom (two children), 3 s of a CC0 surfing clip, the bedroom again from frame 120. Cuts
  after frames 47 and 95 at 16 fps; both children on either side of the surfing shot.
- `surf30b.mp4`: 30 s of the surfing clip (it loops once, which is a real cut).

## Results (2026-10-19, RTX 3070 8 GB, Windows 11, every CPU core busy with other processes)

| | `cuts.mp4` (9.3 s, 149 frames) | `surf30b.mp4` (30 s, 484 frames) |
|---|---|---|
| Shots found | 3: cuts exactly after frames 47 and 95 | 2: the loop point |
| People | 4: each child joined across the cut, 2 surfers | 2 |
| Tracklets kept / dropped | 6 / 0 | 4 / 11 |
| Normalise | 1.0 s | 4.1 s |
| Shots | 1.2 s | 1.5 s |
| Decode to the frame cache | 0.8 s | 1.9 s |
| People and tracking | 17.6 s (298 tracked frames, ~59 ms each) | 33.0 s (537 tracked frames, ~61 ms each) |
| Mask video | 0.6 s | 1.0 s |
| Peak working set | 1817 MB | 1818 MB |

Peak memory does not grow with the clip: the frames live on disk, the masks bit-packed. Most of the 1.8 GB is the
CUDA and cuDNN libraries and ONNX Runtime's arenas, loaded whatever the clip.

The identity distances on `cuts.mp4`: the girl with herself across the cut 0.106, the boy with himself 0.36 (listed as
a close call), the two children 0.456–0.498, the surfers 0.75 and more from everyone. The threshold sits between.

### Several people in one pass (issue #29)

Prep used to follow each person with a full EdgeTAM pass over the shot, so every frame was encoded again per person.
Now a shot is gone through once: each frame is encoded once, then every person followed on it is stepped (memory
attention, decoder, memory encoder on that person's own bank). The tracklets are the same as the per-person passes'.
Coverage counts only kept tracklets, and whether one is kept (long enough) is only known later, so a detection covered
only by undecided tracklets seeds one that exists only if none of them is kept; it is dropped as soon as one is. On
`surf30b.mp4` 6 seeds went this way (3 turned out to exist, 3 were discarded), on `cuts.mp4` 2 (both discarded);
counting undecided tracklets as covering would have missed those 3.

Checked against the per-person passes (branch `issue-26-decoder`): `manifest.json` tracklets, identities, targets,
shots, appearance distances and close calls are equal on `cuts.mp4` and `surf30b.mp4`, and `mask.mp4` decodes to the
same bytes. `track-parity` ([edgetam-onnx](https://github.com/jutsuhq/edgetam-onnx)) gives byte-identical masks on every
sequence of `.cache/reference/edgetam` and `.cache/boxref`, alone and with `--together` (several sequences' objects in one pass).

Per frame on EdgeTAM's bedroom (`track-parity --together`, 40 frames, two runs; the bank is full from frame 16, the
rest of a longer shot runs at the faster full-bank rate). RTX 3070, 2026-10-20, other processes on the CPU:

| Objects | Per-person passes (before) | One pass (now) | Now, against one object |
|---|---|---|---|
| 1 (girl) | 46.6–48.3 ms | 41.4–41.5 ms | 1.00× |
| 2 (girl, boy) | ~93 ms | 62.4–62.9 ms | 1.51× (1.34× the old one-object time) |
| 3 (girl, boy, pillow) | ~139 ms | 82.3–88.8 ms | 2.0–2.1× |

Each extra object costs ~21 ms against ~20 ms of shared work (encoder 15 ms, upload and preprocessing, layout), so
two objects sit at the 1.5× line: the memory attention (11.6 ms per object) is GPU-bound, not launch-bound.

Prep's tracking loop (`people and tracking`, now timed apart from loading the models, `people and tracking models`,
4–5.5 s either way), three alternating runs each:

| Clip | Before | Now | Encoder passes now |
|---|---|---|---|
| `cuts.mp4`: 149 frames, 2 people on every frame, 298 tracked frames | 13.2–16.2 s | 9.6–11.1 s | 149 |
| `surf30b.mp4`: 484 frames, 511 tracked frames | 31.5–60.1 s | 22.6–23.3 s | 196 |

`manifest.json`'s `tracking.encodedFrames` counts the encoder passes.

### Box seeds against PyTorch (issue #26)

Prep seeds EdgeTAM from RT-DETRv2's box (corners with labels 2 and 3). Against transformers' EdgeTAM seeded by the same
box on the bedroom (40 frames):

| Seed | IoU min / mean | Seed frame |
|---|---|---|
| girl's box | 0.795 / 0.890 | 0.907 |
| boy's box | 0.897 / 0.978 | 0.953 |
| a single click at the girl's torso | 0.090 / 0.128 | — (only her top) |

The ONNX export's mask decoder is traced with multimask on: it always returns the best of three masks. For two or
more points (a box is two), PyTorch takes the single-mask output instead. On the girl the export's choice includes the
cushion behind her legs. A click is worse: one point picks the best of three nested masks, here her top. A box stays
the seed.

With our own export of the decoder (edgetam-onnx's `scripts/export/edgetam_decoder.py`, picked up from
`.models/edgetam-decoder/`; see [PARITY.md](https://github.com/jutsuhq/edgetam-onnx/blob/main/docs/PARITY.md#our-edgetam-decoder)) the seed takes the
single mask as PyTorch does:

| Seed | Old refs: IoU min / mean | Seed frame | Fixed refs: IoU min / mean | Against strict-fp32 PyTorch |
|---|---|---|---|---|
| girl's box | 0.934 / 0.974 | 0.993 | **0.969** / 0.992 | 0.932 / 0.973 |
| boy's box | 0.932 / 0.985 | 0.992 | 0.900 / 0.985 | **0.993** / 0.997 |

The cushion is gone from the girl's seed. Neither box reaches 0.95 on every frame against every PyTorch run, because
PyTorch does not either: with only cuDNN's TF32 toggled it scores 0.931 (girl box) and 0.899 (boy box) against itself.
Near-tied candidates on a few frames flip, and the memory carries the flip forward. Each box is within 0.95 of one of
PyTorch's two runs (0.969 and 0.993).

### GPU decoding (issue #22)

Decoding the 1080p source to RGB: 4.1 s wall and 16.9 s of CPU on the CPU, 4.0 s and 15.1 s with `-hwaccel cuda`. The
CPU time is the conversion to RGB and the copy back from the GPU, not the decoding; NVDEC alone saves little. Prep
decodes on the GPU when NVENC is there (it is the same driver), and the real saving would be the colour conversion on
the GPU too (`scale_cuda` and a YUV→RGB glue graph).

### Shot model check (issue #24)

`scripts/parity/transnetv2.py` runs our ONNX (`edgetools/transnetv2`) and the original TensorFlow model
(soCzech/TransNetV2 at 85cef72, `inference/transnetv2-weights`) on the same 48×27 frames with upstream's windows. The
ONNX graph's first output is already the probability (upstream applies the sigmoid itself).

| Clip | Frames | max \|TF − ONNX\| | mean | Cuts, TF = ONNX |
|---|---|---|---|---|
| cuts.mp4 | 224 | 1.02e-7 | 2.41e-8 | 71, 143 |
| bedroom.mp4 | 200 | 7.54e-8 | 2.26e-8 | none |
| surf30.mp4 | 527 | 1.05e-7 | 2.60e-8 | none |

## Characters (stage 3)

### Faces (issue #35)

YuNet (`face_detection_yunet_2023mar`, MIT) on the GPU, decoded from its three strides as OpenCV's `FaceDetectorYN`
does, score ≥ 0.75, NMS at 0.3. Prep looks at up to 16 frames where the person is at least half their largest size
(the largest is often a close-up out of focus), finds the face in the person's own crop (so a small face is scaled up
for the detector) whose centre is on that person's mask, and takes the frame with the sharpest face (variance of the
Laplacian at a fixed height); without any face, the sharpest body, measured on the person's own pixels. The preview
and the face come from that frame. It writes `people/<id>-face.png` (the face with 40 % around it) and `face` in the manifest (frame, box, the five
landmarks, score).

On `cuts.mp4`: both children's faces are found (0.87 and 0.80; the source video blurs them); the two surfers have none
(far away, mostly from behind). All four people's previews and faces take 0.4 s.

### Character cutout (issue #36)

`kawarimi-cutout <image>...` lifts a character off its background with BiRefNet lite (`onnx-community/BiRefNet_lite-ONNX`,
fp16, MIT): 1024² input, ImageNet mean and std, a sigmoid per pixel, scaled back to the image and composited on grey
128, as Swapjutsu's stage 10. It writes `<name>.png` (on grey) and `<name>.alpha.png`.

![Eight test images and their cutouts, left to right in pairs](assets/cutouts.png)

Seven images, all public domain on Wikimedia Commons (`scripts/characters.py` fetches them and records each page), plus
a frame of a test clip:

| Image | Cutout |
|---|---|
| Vermeer, *Girl with a Pearl Earring*; Leonardo, *Mona Lisa*; Van Gogh, self-portrait | clean |
| An anime girl at a computer | clean; the computer and desk are gone |
| A 1916 newspaper cartoon with a photo inset | clean |
| *Hound Dawg* character model sheet (1938) | the drawings kept, the paper gone |
| A sketchbook spread | fails: the whole book is kept (not a character image) |
| A frame of the surfing clip | fails: nothing kept (the people are tiny) |

On the RTX 3070 the first image takes ~1.9 s (cuDNN's algorithm search) and each further one 0.34–0.57 s. With ONNX
Runtime's defaults (largest cuDNN workspace, an arena growing by powers of two) BiRefNet filled the 8 GB card and ran
2.8–3 s per image; it now runs on a lean session (`onnx::lean_session`). The fp32 model is slower still on 8 GB
(7–55 s), so the pinned fp16 one stays.
