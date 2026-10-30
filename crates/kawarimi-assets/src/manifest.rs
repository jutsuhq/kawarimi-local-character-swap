//! Every model and tool the app downloads, pinned by revision, size and SHA-256.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    Tool,
    Tracking,
    People,
    Faces,
    Shots,
    Cutout,
    /// ComfyUI's custom nodes and the video model's weights.
    Render,
    /// The optional prompt writer: llama-server and its vision model.
    Llm,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: &'static str,
    pub title: &'static str,
    pub group: Group,
    pub license: &'static str,
    /// Where the files come from, for people to check.
    pub source: &'static str,
    pub files: &'static [File],
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub url: &'static str,
    /// Path inside the asset's folder. For an archive, where the archive is kept until it is unpacked.
    pub path: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// Members to take out of a verified archive: (path in the archive, path inside the asset's folder).
    pub unpack: &'static [(&'static str, &'static str)],
}

impl Asset {
    pub fn download_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

const fn file(url: &'static str, path: &'static str, size: u64, sha256: &'static str) -> File {
    File { url, path, size, sha256, unpack: &[] }
}

macro_rules! hf {
    ($repo:literal, $rev:literal, $file:literal) => {
        concat!("https://huggingface.co/", $repo, "/resolve/", $rev, "/", $file)
    };
}

pub const EDGETAM: Asset = Asset {
    id: "edgetam-video",
    title: "EdgeTAM video tracker",
    group: Group::Tracking,
    license: "Apache-2.0",
    source: "https://huggingface.co/jax-image-tools/edgetam-video-onnx",
    files: &[
        file(
            hf!("jax-image-tools/edgetam-video-onnx", "8ca3d3e4169938e65b552cdf14542fde25e8badb", "vision_encoder.onnx"),
            "vision_encoder.onnx",
            19_826_101,
            "57fe1a2b3d500813fe4987c4209b856920f187ab0b7723d81586b607ea2cffc1",
        ),
        file(
            hf!("jax-image-tools/edgetam-video-onnx", "8ca3d3e4169938e65b552cdf14542fde25e8badb", "mask_decoder.onnx"),
            "mask_decoder.onnx",
            17_800_677,
            "ee24ee1cae6ecc71889912c1893e08b09f645bb63875108ccd180f7218a3647d",
        ),
        file(
            hf!("jax-image-tools/edgetam-video-onnx", "8ca3d3e4169938e65b552cdf14542fde25e8badb", "memory_attention.onnx"),
            "memory_attention.onnx",
            20_914_958,
            "6477d775639905945949508c08a2c652d01e5933c0eb00a02225ae5f6aa1602c",
        ),
        file(
            hf!("jax-image-tools/edgetam-video-onnx", "8ca3d3e4169938e65b552cdf14542fde25e8badb", "memory_encoder.onnx"),
            "memory_encoder.onnx",
            6_692_568,
            "035b30f8fb1f7c99211ada6eb9d72f0819de819ef94310b14e8147dc97f120ec",
        ),
    ],
};

pub const SAM21_TINY: Asset = Asset {
    id: "sam21-tiny-video",
    title: "SAM 2.1 tiny video tracker",
    group: Group::Tracking,
    license: "Apache-2.0",
    source: "https://huggingface.co/square-zero-labs/sam2.1-tiny-video-onnx",
    files: &[
        file(
            hf!("square-zero-labs/sam2.1-tiny-video-onnx", "3b2984dd865f6e9d2cc6aed0be6a5a5c2eb352ce", "onnx/vision_encoder.onnx"),
            "vision_encoder.onnx",
            134_335_567,
            "aa7a8542942f042e235a993a1ab0ccf5f049918500577802a7f10ec1b39bb873",
        ),
        file(
            hf!("square-zero-labs/sam2.1-tiny-video-onnx", "3b2984dd865f6e9d2cc6aed0be6a5a5c2eb352ce", "onnx/mask_decoder.onnx"),
            "mask_decoder.onnx",
            17_794_355,
            "0461896de3db00936fe1643506f129d71cb6d5d2ae15754811756b7ea1b070c6",
        ),
        file(
            hf!("square-zero-labs/sam2.1-tiny-video-onnx", "3b2984dd865f6e9d2cc6aed0be6a5a5c2eb352ce", "onnx/memory_attention.onnx"),
            "memory_attention.onnx",
            32_259_165,
            "791e648ce8f5ef91ad00ba06e83066ff261ae5a645f0b042b4f46a89fd054baf",
        ),
        file(
            hf!("square-zero-labs/sam2.1-tiny-video-onnx", "3b2984dd865f6e9d2cc6aed0be6a5a5c2eb352ce", "onnx/memory_encoder.onnx"),
            "memory_encoder.onnx",
            5_616_496,
            "580d246c109de88838f600ba7c1c0d03d1fe267f7641b06f8c88c5f0dc5834cd",
        ),
        file(
            hf!("square-zero-labs/sam2.1-tiny-video-onnx", "3b2984dd865f6e9d2cc6aed0be6a5a5c2eb352ce", "onnx/pointer_tpos.onnx"),
            "pointer_tpos.onnx",
            67_289,
            "7e71df1d75dba09bc18dd4ae745c2a2cceebdd14d84eadea2fe65d0205f101fc",
        ),
        file(
            hf!("square-zero-labs/sam2.1-tiny-video-onnx", "3b2984dd865f6e9d2cc6aed0be6a5a5c2eb352ce", "constants.json"),
            "constants.json",
            9_922,
            "172edc70e892aab0aef6caedf97b8e4091fbf6c5d26dac5fd9ea69d093d927d3",
        ),
    ],
};

pub const RTDETR: Asset = Asset {
    id: "rtdetr-v2-r18",
    title: "RT-DETRv2 person detector",
    group: Group::People,
    license: "Apache-2.0",
    source: "https://huggingface.co/onnx-community/rtdetr_v2_r18vd-ONNX",
    files: &[file(
        hf!("onnx-community/rtdetr_v2_r18vd-ONNX", "936f90b6a476c6da4dfe053fc521af55285976ba", "onnx/model.onnx"),
        "model.onnx",
        81_057_510,
        "583a236ac21c95a7fd94f284fc21485e42355bfef82c27011ba78fbc09ee87e2",
    )],
};

pub const YUNET: Asset = Asset {
    id: "yunet",
    title: "YuNet face detector",
    group: Group::Faces,
    license: "MIT",
    source: "https://huggingface.co/opencv/face_detection_yunet",
    files: &[file(
        hf!("opencv/face_detection_yunet", "3cc26e7f1014a5ee5d74a42acee58bafc9d0a310", "face_detection_yunet_2023mar.onnx"),
        "model.onnx",
        232_589,
        "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
    )],
};

pub const TRANSNETV2: Asset = Asset {
    id: "transnetv2",
    title: "TransNetV2 shot detector",
    group: Group::Shots,
    license: "MIT",
    source: "https://huggingface.co/edgetools/transnetv2",
    files: &[file(
        hf!("edgetools/transnetv2", "4b2cef0e4f246904228785e9f0a693a2363e0a6b", "transnetv2.onnx"),
        "model.onnx",
        31_250_929,
        "c4d54a682bace32f25136ef83ca2c9d403e8f8193775efeb995172a0d95a8e0c",
    )],
};

pub const BIREFNET_LITE: Asset = Asset {
    id: "birefnet-lite",
    title: "BiRefNet lite cutout",
    group: Group::Cutout,
    license: "MIT",
    source: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX",
    files: &[file(
        hf!("onnx-community/BiRefNet_lite-ONNX", "de15b22ba131738a16dff04aab8bdf8dc32e3ac1", "onnx/model_fp16.onnx"),
        "model.onnx",
        114_538_221,
        "d39b897ceb16ae654c1731f3dba0cf9b368d9cae74b5a57459b455cc8bfec402",
    )],
};

// note: BtbN deletes old autobuild releases after a while; when this URL goes away, move the pin to a newer
// n9.0 build (same naming) or a mirror under our own releases.
#[cfg(windows)]
pub const FFMPEG: Asset = Asset {
    id: "ffmpeg",
    title: "FFmpeg 9.0 (LGPL)",
    group: Group::Tool,
    license: "LGPL-2.1-or-later",
    source: "https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-10-18-13-01",
    files: &[File {
        url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-18-13-01/ffmpeg-n9.0.2-28-g669f5ab083-win64-lgpl-9.0.zip",
        path: "ffmpeg.zip",
        size: 171_480_233,
        sha256: "7f4cbda0ece7721488eeebf2fa69762d7388bca69450fd13045b408744457b14",
        unpack: &[
            ("ffmpeg-n9.0.2-28-g669f5ab083-win64-lgpl-9.0/bin/ffmpeg.exe", "ffmpeg.exe"),
            ("ffmpeg-n9.0.2-28-g669f5ab083-win64-lgpl-9.0/bin/ffprobe.exe", "ffprobe.exe"),
            ("ffmpeg-n9.0.2-28-g669f5ab083-win64-lgpl-9.0/LICENSE.txt", "LICENSE.txt"),
        ],
    }],
};

#[cfg(not(windows))]
pub const FFMPEG: Asset = Asset {
    id: "ffmpeg",
    title: "FFmpeg 9.0 (LGPL)",
    group: Group::Tool,
    license: "LGPL-2.1-or-later",
    source: "https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-10-18-13-01",
    files: &[File {
        url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-18-13-01/ffmpeg-n9.0.2-28-g669f5ab083-linux64-lgpl-9.0.tar.xz",
        path: "ffmpeg.tar.xz",
        size: 137_946_364,
        sha256: "b2ee4166aa1f068ac261cbdb637667f789c58f3556b463c41221ca8d971ac793",
        unpack: &[
            ("ffmpeg-n9.0.2-28-g669f5ab083-linux64-lgpl-9.0/bin/ffmpeg", "ffmpeg"),
            ("ffmpeg-n9.0.2-28-g669f5ab083-linux64-lgpl-9.0/bin/ffprobe", "ffprobe"),
            ("ffmpeg-n9.0.2-28-g669f5ab083-linux64-lgpl-9.0/LICENSE.txt", "LICENSE.txt"),
        ],
    }],
};

/// What ONNX Runtime's CUDA provider loads: NVIDIA's CUDA 13 runtime, cuBLAS, cuFFT, cuRAND, NVRTC, nvJitLink and
/// cuDNN 9, from NVIDIA's own PyPI wheels, so no CUDA toolkit is needed. Only the shared libraries are kept.
pub const CUDA_RUNTIME: Asset = Asset {
    id: "cuda-runtime",
    title: "CUDA 13 and cuDNN 9 runtime",
    group: Group::Tool,
    license: "NVIDIA redistributable",
    source: "https://pypi.org/project/nvidia-cudnn-cu13/",
    files: crate::cuda_runtime::FILES,
};

/// uv, which installs ComfyUI's Python and its locked packages.
#[cfg(windows)]
pub const UV: Asset = Asset {
    id: "uv",
    title: "uv 0.12.22 (Python installer)",
    group: Group::Tool,
    license: "MIT OR Apache-2.0",
    source: "https://github.com/astral-sh/uv/releases/tag/0.12.22",
    files: &[File {
        url: "https://github.com/astral-sh/uv/releases/download/0.12.22/uv-x86_64-pc-windows-msvc.zip",
        path: "uv.zip",
        size: 18_019_334,
        sha256: "ea1397797a0ca15f63516dd0f49c2dde9776db9be5861cab152ebe8ad199894d",
        unpack: &[("uv.exe", "uv.exe")],
    }],
};

#[cfg(not(windows))]
pub const UV: Asset = Asset {
    id: "uv",
    title: "uv 0.12.22 (Python installer)",
    group: Group::Tool,
    license: "MIT OR Apache-2.0",
    source: "https://github.com/astral-sh/uv/releases/tag/0.12.22",
    files: &[File {
        url: "https://github.com/astral-sh/uv/releases/download/0.12.22/uv-x86_64-unknown-linux-gnu.tar.gz",
        path: "uv.tar.gz",
        size: 19_916_278,
        sha256: "b9980552309f09c15172b8be828555e375097f16deb459795ce7bfd200380f0b",
        unpack: &[("uv-x86_64-unknown-linux-gnu/uv", "uv")],
    }],
};

/// ComfyUI's source at v0.39.0, the renderer's host. Its Python packages are locked in kawarimi-comfy.
pub const COMFYUI: Asset = Asset {
    id: "comfyui",
    title: "ComfyUI v0.39.0",
    group: Group::Tool,
    license: "GPL-3.0",
    source: "https://github.com/comfyanonymous/ComfyUI/tree/b0b743566f65daafc423b4fea8a2fbda94b3384a",
    files: &[File {
        url: "https://codeload.github.com/comfyanonymous/ComfyUI/zip/b0b743566f65daafc423b4fea8a2fbda94b3384a",
        path: "comfyui.zip",
        size: 13_350_160,
        sha256: "c8997f6781e2b53861697fab23b77a7fdb556ec0640f21653a89689556816b54",
        unpack: &[("ComfyUI-b0b743566f65daafc423b4fea8a2fbda94b3384a/", "ComfyUI")],
    }],
};

macro_rules! hf_file {
    ($repo:literal, $rev:literal, $file:literal, $path:literal, $size:literal, $sha:literal) => {
        File { url: hf!($repo, $rev, $file), path: $path, size: $size, sha256: $sha, unpack: &[] }
    };
}

/// DWPose for Wan 2.2 Animate's pose and face videos, through `comfyui_controlnet_aux`.
pub const NODES_CONTROLNET_AUX: Asset = Asset {
    id: "comfyui-controlnet-aux",
    title: "ComfyUI controlnet_aux nodes (DWPose)",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://github.com/Fannovel16/comfyui_controlnet_aux/tree/a587a4c1594524c1d2de5cd1a86cd868ff448e1c",
    files: &[File {
        url: "https://codeload.github.com/Fannovel16/comfyui_controlnet_aux/zip/a587a4c1594524c1d2de5cd1a86cd868ff448e1c",
        path: "nodes.zip",
        size: 39_937_384,
        sha256: "3d717b853303c3c030effbb5ee23fd461ba427917b161b9340a01ea3c0341868",
        unpack: &[("comfyui_controlnet_aux-a587a4c1594524c1d2de5cd1a86cd868ff448e1c/", "comfyui_controlnet_aux")],
    }],
};

/// GGUF model loading for ComfyUI, for the quantised Wan weights.
pub const NODES_GGUF: Asset = Asset {
    id: "comfyui-gguf",
    title: "ComfyUI-GGUF nodes",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://github.com/city96/ComfyUI-GGUF/tree/6ea2651e7df66d7585f6ffee804b20e92fb38b8a",
    files: &[File {
        url: "https://codeload.github.com/city96/ComfyUI-GGUF/zip/6ea2651e7df66d7585f6ffee804b20e92fb38b8a",
        path: "nodes.zip",
        size: 38_051,
        sha256: "aad273a0b774684285b4496f6edff7e2293bd29d71ef89926d6861bfdf4947ac",
        unpack: &[("ComfyUI-GGUF-6ea2651e7df66d7585f6ffee804b20e92fb38b8a/", "ComfyUI-GGUF")],
    }],
};

/// DWPose's person detector and pose model, where `comfyui_controlnet_aux` looks for them
/// (`AUX_ANNOTATOR_CKPTS_PATH`/<repo>/<file>), so it never downloads them itself.
pub const DWPOSE: Asset = Asset {
    id: "dwpose",
    title: "DWPose (YOLOX-L and DW-LL)",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://huggingface.co/yzd-v/DWPose",
    files: &[
        hf_file!("yzd-v/DWPose", "1a7144101628d69ee7a3768d1ee3a094070dc388", "yolox_l.onnx", "yzd-v/DWPose/yolox_l.onnx",
            216_746_733, "7860ae79de6c89a3c1eb72ae9a2756c0ccfbe04b7791bb5880afabd97855a411"),
        hf_file!("hr16/DWPose-TorchScript-BatchSize5", "359d662a9b33b73f6d0f21732baf8845f17bb4be",
            "dw-ll_ucoco_384_bs5.torchscript.pt", "hr16/DWPose-TorchScript-BatchSize5/dw-ll_ucoco_384_bs5.torchscript.pt",
            135_059_124, "d86a0b2b59fddc0901a7076e9f59c9f8602602133ed72511c693fd11eea23d91"),
    ],
};

/// Wan 2.2 Animate 14B quantised to Q3_K_M, the weights for 8 GB cards (docs/RENDER.md).
pub const WAN_ANIMATE_Q3: Asset = Asset {
    id: "wan-animate-q3km",
    title: "Wan 2.2 Animate 14B (GGUF Q3_K_M)",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://huggingface.co/QuantStack/Wan2.2-Animate-14B-GGUF",
    files: &[hf_file!("QuantStack/Wan2.2-Animate-14B-GGUF", "33c51bb84d4e70ffc0d088aeb6068d40d9446fa3",
        "Wan2.2-Animate-14B-Q3_K_M.gguf", "Wan2.2-Animate-14B-Q3_K_M.gguf",
        8_630_769_472, "62cd6067c96599ade5bdde54a49a5ef1a7fd4de449eaae56ff4fd61ccbb16678")],
};

/// Wan 2.2 Animate 14B quantised to Q4_K_M, the weights for 12 GB cards (docs/RENDER.md).
pub const WAN_ANIMATE_Q4: Asset = Asset {
    id: "wan-animate-q4km",
    title: "Wan 2.2 Animate 14B (GGUF Q4_K_M)",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://huggingface.co/QuantStack/Wan2.2-Animate-14B-GGUF",
    files: &[hf_file!("QuantStack/Wan2.2-Animate-14B-GGUF", "33c51bb84d4e70ffc0d088aeb6068d40d9446fa3",
        "Wan2.2-Animate-14B-Q4_K_M.gguf", "Wan2.2-Animate-14B-Q4_K_M.gguf",
        11_496_331_072, "43d720f243c3cdf5346ca05b525ec662f89bc5ebeb8e998dc347b840406cdfa6")],
};

/// Wan 2.2 Animate 14B in Kijai's scaled fp8 (v2), the weights for cards with 16 GB or more; ComfyUI's own
/// `UNETLoader` reads it.
pub const WAN_ANIMATE_FP8: Asset = Asset {
    id: "wan-animate-fp8",
    title: "Wan 2.2 Animate 14B (fp8 scaled v2)",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://huggingface.co/Kijai/WanVideo_comfy_fp8_scaled",
    files: &[hf_file!("Kijai/WanVideo_comfy_fp8_scaled", "033a4e487f60220b3d6e469599a6aebc46e13cee",
        "Wan22Animate/Wan2_2-Animate-14B_fp8_scaled_e4m3fn_KJ_v2.safetensors",
        "Wan2_2-Animate-14B_fp8_scaled_e4m3fn_KJ_v2.safetensors",
        17_317_143_060, "b1cd5a67e968e5bb38fcf84dab58f2fb551e0889fd463acb2a657259a70bbbf6")],
};

/// Wan's text encoder (UMT5-XXL) quantised to Q4_K_M.
pub const UMT5_Q4: Asset = Asset {
    id: "umt5-q4km",
    title: "UMT5-XXL text encoder (GGUF Q4_K_M)",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://huggingface.co/city96/umt5-xxl-encoder-gguf",
    files: &[hf_file!("city96/umt5-xxl-encoder-gguf", "b535255bee98c2b0a59ea7c0ae2dcd0c6657b3b7",
        "umt5-xxl-encoder-Q4_K_M.gguf", "umt5-xxl-encoder-Q4_K_M.gguf",
        3_655_145_312, "17cf97a5bbbc60a646d6105b832b6f657ce904a8a1ad970e4b59df0c67584a40")],
};

pub const WAN_VAE: Asset = Asset {
    id: "wan-vae",
    title: "Wan 2.1 VAE",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://huggingface.co/Comfy-Org/Wan_2.2_ComfyUI_Repackaged",
    files: &[hf_file!("Comfy-Org/Wan_2.2_ComfyUI_Repackaged", "ee6f4a40737a995bf5818954cfce6d59443b0f04",
        "split_files/vae/wan_2.1_vae.safetensors", "wan_2.1_vae.safetensors",
        253_815_318, "2fc39d31359a4b0a64f55876d8ff7fa8d780956ae2cb13463b0223e15148976b")],
};

pub const CLIP_VISION_H: Asset = Asset {
    id: "clip-vision-h",
    title: "CLIP vision H (the character image's encoder)",
    group: Group::Render,
    license: "Apache-2.0",
    source: "https://huggingface.co/Comfy-Org/Wan_2.1_ComfyUI_repackaged",
    files: &[hf_file!("Comfy-Org/Wan_2.1_ComfyUI_repackaged", "123acf1cc74bccbb9bfff8ac1ee72edc08c2341d",
        "split_files/clip_vision/clip_vision_h.safetensors", "clip_vision_h.safetensors",
        1_264_219_396, "64a7ef761bfccbadbaa3da77366aac4185a6c58fa5de5f589b42a65bcc21f161")],
};

/// The step- and CFG-distilled LoRA the Animate template uses: 4–6 steps at CFG 1.
pub const LIGHTX2V: Asset = Asset {
    id: "lightx2v",
    title: "LightX2V step-distill LoRA (rank 64)",
    group: Group::Render,
    // Kijai's extraction carries no license tag; the LightX2V model it comes from is Apache-2.0.
    license: "Apache-2.0 (LightX2V)",
    source: "https://huggingface.co/Kijai/WanVideo_comfy",
    files: &[hf_file!("Kijai/WanVideo_comfy", "8260d429d19fd7a72304cad059160b95d843913f",
        "Lightx2v/lightx2v_I2V_14B_480p_cfg_step_distill_rank64_bf16.safetensors",
        "lightx2v_I2V_14B_480p_cfg_step_distill_rank64_bf16.safetensors",
        738_005_744, "85c4a61c30e0497aa44b91d93a893b624708461a56fe5485183b28fa07e2dfb3")],
};

// llama.cpp's own CUDA 13.4 build without its cudart bundle: cuBLAS and the CUDA runtime come from `cuda-runtime`
// (the server gets that folder on PATH / LD_LIBRARY_PATH); the driver provides libcuda and nvcudart_hybrid.
#[cfg(windows)]
pub const LLAMA_SERVER: Asset = Asset {
    id: "llama-server",
    title: "llama.cpp b11669 server (CUDA 13)",
    group: Group::Llm,
    license: "MIT",
    source: "https://github.com/ggml-org/llama.cpp/releases/tag/b11669",
    files: &[File {
        url: "https://github.com/ggml-org/llama.cpp/releases/download/b11669/llama-b11669-bin-win-cuda-13.4-x64.zip",
        path: "llama.zip",
        size: 154_582_227,
        sha256: "3b92d5c064f5032e603e531d3ec8961f3bd6613e5dfad047399a92dec8e95dbc",
        unpack: &[
            ("ggml.dll", "ggml.dll"),
            ("ggml-base.dll", "ggml-base.dll"),
            ("ggml-cpu-alderlake.dll", "ggml-cpu-alderlake.dll"),
            ("ggml-cpu-cannonlake.dll", "ggml-cpu-cannonlake.dll"),
            ("ggml-cpu-cascadelake.dll", "ggml-cpu-cascadelake.dll"),
            ("ggml-cpu-cooperlake.dll", "ggml-cpu-cooperlake.dll"),
            ("ggml-cpu-haswell.dll", "ggml-cpu-haswell.dll"),
            ("ggml-cpu-icelake.dll", "ggml-cpu-icelake.dll"),
            ("ggml-cpu-ivybridge.dll", "ggml-cpu-ivybridge.dll"),
            ("ggml-cpu-piledriver.dll", "ggml-cpu-piledriver.dll"),
            ("ggml-cpu-sandybridge.dll", "ggml-cpu-sandybridge.dll"),
            ("ggml-cpu-sapphirerapids.dll", "ggml-cpu-sapphirerapids.dll"),
            ("ggml-cpu-skylakex.dll", "ggml-cpu-skylakex.dll"),
            ("ggml-cpu-sse42.dll", "ggml-cpu-sse42.dll"),
            ("ggml-cpu-x64.dll", "ggml-cpu-x64.dll"),
            ("ggml-cpu-zen4.dll", "ggml-cpu-zen4.dll"),
            ("ggml-cuda.dll", "ggml-cuda.dll"),
            ("libomp.dll", "libomp.dll"),
            ("LICENSE-LLVM-OpenMP", "LICENSE-LLVM-OpenMP"),
            ("llama.dll", "llama.dll"),
            ("llama-common.dll", "llama-common.dll"),
            ("llama-server.exe", "llama-server.exe"),
            ("llama-server-impl.dll", "llama-server-impl.dll"),
            ("mtmd.dll", "mtmd.dll"),
        ],
    }],
};

/// The shared libraries go in under their sonames, which the archive has as symlinks.
#[cfg(not(windows))]
pub const LLAMA_SERVER: Asset = Asset {
    id: "llama-server",
    title: "llama.cpp b11669 server (CUDA 13)",
    group: Group::Llm,
    license: "MIT",
    source: "https://github.com/ggml-org/llama.cpp/releases/tag/b11669",
    files: &[File {
        url: "https://github.com/ggml-org/llama.cpp/releases/download/b11669/llama-b11669-bin-ubuntu-cuda-13.4-x64.tar.gz",
        path: "llama.tar.gz",
        size: 153_679_828,
        sha256: "80878473d27278dbe9c38fb7e704f292a51e889d3c28a1e863137cdee1e2ba4e",
        unpack: &[
            ("llama-b11669/libggml.so.0.27.0", "libggml.so.0"),
            ("llama-b11669/libggml-base.so.0.27.0", "libggml-base.so.0"),
            ("llama-b11669/libggml-cpu-alderlake.so", "libggml-cpu-alderlake.so"),
            ("llama-b11669/libggml-cpu-cannonlake.so", "libggml-cpu-cannonlake.so"),
            ("llama-b11669/libggml-cpu-cascadelake.so", "libggml-cpu-cascadelake.so"),
            ("llama-b11669/libggml-cpu-cooperlake.so", "libggml-cpu-cooperlake.so"),
            ("llama-b11669/libggml-cpu-haswell.so", "libggml-cpu-haswell.so"),
            ("llama-b11669/libggml-cpu-icelake.so", "libggml-cpu-icelake.so"),
            ("llama-b11669/libggml-cpu-ivybridge.so", "libggml-cpu-ivybridge.so"),
            ("llama-b11669/libggml-cpu-piledriver.so", "libggml-cpu-piledriver.so"),
            ("llama-b11669/libggml-cpu-sandybridge.so", "libggml-cpu-sandybridge.so"),
            ("llama-b11669/libggml-cpu-sapphirerapids.so", "libggml-cpu-sapphirerapids.so"),
            ("llama-b11669/libggml-cpu-skylakex.so", "libggml-cpu-skylakex.so"),
            ("llama-b11669/libggml-cpu-sse42.so", "libggml-cpu-sse42.so"),
            ("llama-b11669/libggml-cpu-x64.so", "libggml-cpu-x64.so"),
            ("llama-b11669/libggml-cpu-zen4.so", "libggml-cpu-zen4.so"),
            ("llama-b11669/libggml-cuda.so", "libggml-cuda.so"),
            ("llama-b11669/libllama.so.0.7.0", "libllama.so.0"),
            ("llama-b11669/libllama-common.so.0.7.0", "libllama-common.so.0"),
            ("llama-b11669/libllama-server-impl.so", "libllama-server-impl.so"),
            ("llama-b11669/libmtmd.so.0.7.0", "libmtmd.so.0"),
            ("llama-b11669/LICENSE", "LICENSE"),
            ("llama-b11669/llama-server", "llama-server"),
        ],
    }],
};

/// The prompt writer's vision model: Qwen3.5-9B in Unsloth's Q4_K_M with its F16 vision projector.
pub const QWEN35_9B: Asset = Asset {
    id: "qwen35-9b-q4km",
    title: "Qwen3.5-9B (GGUF Q4_K_M, vision)",
    group: Group::Llm,
    license: "Apache-2.0",
    source: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF",
    files: &[
        hf_file!("unsloth/Qwen3.5-9B-GGUF", "3885219b6810b007914f3a7950a8d1b469d598a5", "Qwen3.5-9B-Q4_K_M.gguf",
            "Qwen3.5-9B-Q4_K_M.gguf", 5_680_522_464, "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8"),
        hf_file!("unsloth/Qwen3.5-9B-GGUF", "3885219b6810b007914f3a7950a8d1b469d598a5", "mmproj-F16.gguf",
            "mmproj-F16.gguf", 918_166_080, "f70dc3509053962b0d0d3ee8a7eacebf5d60aa560cad78254ae8698516ae029f"),
    ],
};

pub const ALL: &[Asset] = &[FFMPEG, CUDA_RUNTIME, UV, COMFYUI, NODES_CONTROLNET_AUX, NODES_GGUF, DWPOSE, WAN_ANIMATE_Q3, WAN_ANIMATE_Q4, WAN_ANIMATE_FP8, UMT5_Q4, WAN_VAE, CLIP_VISION_H, LIGHTX2V, EDGETAM, SAM21_TINY, RTDETR, YUNET, TRANSNETV2, BIREFNET_LITE, LLAMA_SERVER, QWEN35_9B];

pub fn find(id: &str) -> Option<&'static Asset> {
    ALL.iter().find(|a| a.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for asset in ALL {
            assert!(ids.insert(asset.id), "duplicate id {}", asset.id);
            assert!(!asset.files.is_empty());
            for f in asset.files {
                assert_eq!(f.sha256.len(), 64, "{}", f.path);
                assert!(f.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "{}", f.path);
                assert!(f.size > 0 && f.url.starts_with("https://"), "{}", f.path);
                assert!(!f.url.contains("/resolve/main/"), "unpinned revision: {}", f.url);
            }
        }
    }
}
