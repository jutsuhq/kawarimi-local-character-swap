"""TransNetV2: our ONNX (edgetools/transnetv2, the `transnetv2` asset) against the original TensorFlow model
(soCzech/TransNetV2, inference/transnetv2-weights) on the same 48×27 frames, windowed as upstream's predict_frames.

    uv run --python 3.12 --with tensorflow-cpu --with onnxruntime --with numpy \
        scripts/parity/transnetv2.py --tf <transnetv2-weights dir> --onnx .models/transnetv2/model.onnx \
        --ffmpeg .models/ffmpeg/ffmpeg.exe clip.mp4 [...]
"""

import argparse
import subprocess

import numpy as np


def frames(ffmpeg, clip):
    raw = subprocess.run(
        [ffmpeg, "-v", "error", "-i", clip, "-vf", "scale=48:27", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        check=True, capture_output=True,
    ).stdout
    return np.frombuffer(raw, np.uint8).reshape(-1, 27, 48, 3)


def windows(video):
    """Upstream's input_iterator: 25 copies of the first frame before, the last frame after, windows of 100 by 50."""
    tail = 25 + 50 - (len(video) % 50 if len(video) % 50 else 50)
    padded = np.concatenate([np.repeat(video[:1], 25, 0), video, np.repeat(video[-1:], tail, 0)])
    for at in range(0, len(padded) - 100 + 1, 50):
        yield padded[at:at + 100][None]


def predict(run, video):
    return np.concatenate([run(w)[0, 25:75] for w in windows(video)])[: len(video)]


def cuts(p, threshold=0.5):
    return [i for i in range(len(p)) if p[i] > threshold and (i + 1 == len(p) or p[i + 1] <= threshold)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tf", required=True)
    parser.add_argument("--onnx", required=True)
    parser.add_argument("--ffmpeg", default="ffmpeg")
    parser.add_argument("clips", nargs="+")
    args = parser.parse_args()

    import onnxruntime
    import tensorflow as tf

    model = tf.saved_model.load(args.tf)
    session = onnxruntime.InferenceSession(args.onnx, providers=["CPUExecutionProvider"])
    name, out = session.get_inputs()[0].name, session.get_outputs()[0].name

    def run_tf(w):
        single, _ = model(tf.cast(w, tf.float32))
        return tf.sigmoid(single).numpy()[..., 0]

    def run_onnx(w):
        y = session.run([out], {name: w.astype(np.float32)})[0]
        return y.reshape(y.shape[0], y.shape[1])

    for clip in args.clips:
        video = frames(args.ffmpeg, clip)
        a, b = predict(run_tf, video), predict(run_onnx, video)
        diff = np.abs(a - b)
        print(f"{clip}: {len(video)} frames, max |TF - ONNX| {diff.max():.2e}, mean {diff.mean():.2e}, "
              f"cuts TF {cuts(a)} ONNX {cuts(b)}")


if __name__ == "__main__":
    main()
