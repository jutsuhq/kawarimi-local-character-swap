"""Kawarimi's pose pass: DWPose finds each frame's people once and draws them twice, faces only for Wan's
face_video and bodies with hands for its pose_video. Two DWPreprocessor nodes would run the detection twice over the
same frames. Written into ComfyUI's custom_nodes by kawarimi-comfy when it starts ComfyUI."""

import sys

import nodes


class KawarimiPose:
    @classmethod
    def INPUT_TYPES(cls):
        return {"required": {"image": ("IMAGE",), "resolution": ("INT", {"default": 512, "min": 64, "max": 16384})}}

    RETURN_TYPES = ("IMAGE", "IMAGE")
    RETURN_NAMES = ("faces", "bodies")
    FUNCTION = "run"
    CATEGORY = "Kawarimi"

    def run(self, image, resolution):
        node = nodes.NODE_CLASS_MAPPINGS["DWPreprocessor"]
        detector = sys.modules[node.__module__].DwposeDetector
        detect = detector.detect_poses
        found = []

        def record(self, frame):
            found.append(detect(self, frame))
            return found[-1]

        args = dict(resolution=resolution, bbox_detector="yolox_l.onnx",
                    pose_estimator="dw-ll_ucoco_384_bs5.torchscript.pt", scale_stick_for_xinsr_cn="disable")
        try:
            # The face pass detects and keeps what it found; the body pass, over the same frames in the same order,
            # is handed it back.
            detector.detect_poses = record
            faces = node().estimate_pose(image, detect_hand="disable", detect_body="disable", detect_face="enable", **args)
            replay = iter(found)
            detector.detect_poses = lambda self, frame: next(replay)
            bodies = node().estimate_pose(image, detect_hand="enable", detect_body="enable", detect_face="disable", **args)
        finally:
            detector.detect_poses = detect
        return (faces["result"][0], bodies["result"][0])


NODE_CLASS_MAPPINGS = {"KawarimiPose": KawarimiPose}
