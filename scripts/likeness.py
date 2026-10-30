"""How much a render's swapped person looks like the character: cosine similarity of CLIP vision H embeddings
between the character and the masked person in every 4th frame, each padded square on grey first.

Runs in ComfyUI's venv, with ComfyUI's own CLIP vision loader:

    .cache/comfy/venv/Scripts/python.exe scripts/likeness.py --comfy .models/comfyui/ComfyUI \
        --clip .models/clip-vision-h/clip_vision_h.safetensors --character c.png \
        --mask .cache/prep/x/mask.mp4 render.mp4 [more.mp4 …]
"""

import argparse
import sys

import av
import numpy as np
import torch
from PIL import Image


def frames(path):
    with av.open(path) as video:
        for frame in video.decode(video=0):
            yield frame.to_ndarray(format="rgb24")


def square(rgb, box=None):
    """The box of `rgb` (or all of it) padded square on grey 128, as a 1×H×W×3 float tensor."""
    if box is not None:
        rgb = rgb[box[1]:box[3], box[0]:box[2]]
    h, w = rgb.shape[:2]
    side = max(h, w)
    canvas = np.full((side, side, 3), 128, np.uint8)
    canvas[(side - h) // 2:(side - h) // 2 + h, (side - w) // 2:(side - w) // 2 + w] = rgb
    return torch.from_numpy(canvas).float().div(255).unsqueeze(0)


def bbox(mask, margin=0.05):
    ys, xs = np.nonzero(mask[..., 0] > 127)
    if len(xs) < 64:
        return None
    h, w = mask.shape[:2]
    mx, my = int(w * margin), int(h * margin)
    return max(0, xs.min() - mx), max(0, ys.min() - my), min(w, xs.max() + mx), min(h, ys.max() + my)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--comfy", required=True)
    parser.add_argument("--clip", required=True)
    parser.add_argument("--character", required=True)
    parser.add_argument("--mask", required=True)
    parser.add_argument("--every", type=int, default=4)
    parser.add_argument("videos", nargs="+")
    args = parser.parse_args()
    sys.path.insert(0, args.comfy)
    import comfy.clip_vision

    model = comfy.clip_vision.load(args.clip)

    def embed(image):
        out = model.encode_image(image, crop=False).image_embeds[0].float()
        return out / out.norm()

    character = embed(square(np.asarray(Image.open(args.character).convert("RGB"))))
    masks = list(frames(args.mask))
    for path in args.videos:
        scores = []
        for i, rgb in enumerate(frames(path)):
            if i % args.every or i >= len(masks):
                continue
            mask = np.asarray(Image.fromarray(masks[i]).resize((rgb.shape[1], rgb.shape[0]), Image.NEAREST))
            box = bbox(mask)
            if box is not None:
                scores.append(float(embed(square(rgb, box)) @ character))
        print(f"{path}\t{np.mean(scores):.3f} +- {np.std(scores):.3f}\t({len(scores)} frames)")


if __name__ == "__main__":
    with torch.inference_mode():
        main()
