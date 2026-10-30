//! FFmpeg and ffprobe run as child processes; frames come back as raw RGB on stdout.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub has_audio: bool,
}

impl Ffmpeg {
    /// The binaries in `dir` (the installed asset), else the ones on PATH.
    pub fn locate(dir: Option<&Path>) -> Option<Self> {
        let exe = |name: &str| format!("{name}{}", std::env::consts::EXE_SUFFIX);
        if let Some(dir) = dir {
            let (ffmpeg, ffprobe) = (dir.join(exe("ffmpeg")), dir.join(exe("ffprobe")));
            if ffmpeg.is_file() && ffprobe.is_file() {
                return Some(Self { ffmpeg, ffprobe });
            }
        }
        let on_path = |name: &str| {
            std::env::var_os("PATH")
                .and_then(|paths| std::env::split_paths(&paths).map(|p| p.join(exe(name))).find(|p| p.is_file()))
        };
        Some(Self { ffmpeg: on_path("ffmpeg")?, ffprobe: on_path("ffprobe")? })
    }

    /// The first line of `ffmpeg -version`.
    pub fn version(&self) -> Result<String> {
        let out = quiet(Command::new(&self.ffmpeg).arg("-version")).output().context("running ffmpeg")?;
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text.lines().next().unwrap_or_default().to_owned())
    }

    pub fn probe(&self, input: &Path) -> Result<Probe> {
        let out = quiet(Command::new(&self.ffprobe).args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,width,height,avg_frame_rate:format=duration",
            "-of",
            "json",
        ]))
        .arg(input)
        .output()
        .context("running ffprobe")?;
        if !out.status.success() {
            bail!("ffprobe failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        parse_probe(&out.stdout)
    }

    /// NVENC when the GPU and driver provide it (checked once by encoding a frame), else OpenH264.
    pub fn encoder(&self) -> Encoder {
        static FOUND: std::sync::OnceLock<Encoder> = std::sync::OnceLock::new();
        *FOUND.get_or_init(|| {
            let nvenc = quiet(Command::new(&self.ffmpeg).args(["-v", "error", "-f", "lavfi", "-i", "color=size=256x256"]))
                .args(["-frames:v", "1", "-c:v", "h264_nvenc", "-f", "null", "-"])
                .status();
            if matches!(nvenc, Ok(s) if s.success()) { Encoder::Nvenc } else { Encoder::OpenH264 }
        })
    }

    /// The clip as the pipeline works on it: upright (FFmpeg applies the rotation), at `fps`, the long side at most
    /// `max_side` with even sides, from `start` for `duration` seconds, H.264 with the original audio as AAC.
    pub fn normalize(&self, input: &Path, output: &Path, n: &Normalize) -> Result<Probe> {
        let side = n.max_side;
        // Rotation is applied before the filters, so iw and ih are the upright size here.
        let filter = format!(
            "fps={fps},scale='if(gte(iw,ih),min({side},iw),-2)':'if(gte(iw,ih),-2,min({side},ih))':flags=lanczos,\
             setsar=1,format=yuv420p",
            fps = n.fps
        );
        let mut cmd = command(&self.ffmpeg);
        cmd.args(["-v", "error", "-nostdin", "-y"]);
        if n.start > 0.0 {
            cmd.args(["-ss", &n.start.to_string()]);
        }
        if let Some(d) = n.duration {
            cmd.args(["-t", &d.to_string()]);
        }
        cmd.arg("-i").arg(input).args(["-vf", &filter]).args(self.encoder().quality_args());
        cmd.args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"]).arg(output);
        let out = cmd.output().context("running ffmpeg")?;
        if !out.status.success() {
            bail!("ffmpeg could not normalise {}: {}", input.display(), String::from_utf8_lossy(&out.stderr).trim());
        }
        self.probe(output)
    }

    /// The final file: the first `frames` frames of `render` at `fps`, H.264, with the soundtrack of `sound` (prep's
    /// normalised clip, already trimmed like the frames) as AAC, padded or cut to the same length.
    pub fn finish(&self, render: &Path, sound: &Path, output: &Path, frames: u32, fps: f64) -> Result<Probe> {
        let rendered = self.probe(render)?;
        let have = (rendered.duration * rendered.fps).round() as u32;
        anyhow::ensure!(have >= frames, "{} has {have} frames, the clip needs {frames}", render.display());
        let length = (frames as f64 / fps).to_string();
        let mut cmd = command(&self.ffmpeg);
        cmd.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(render);
        let with_sound = self.probe(sound)?.has_audio;
        if with_sound {
            cmd.arg("-i").arg(sound).args(["-map", "0:v:0", "-map", "1:a:0", "-af", "apad", "-c:a", "aac", "-b:a", "192k"]);
        }
        cmd.args(["-vf", &format!("setpts=N/({fps}*TB),setsar=1,format=yuv420p"), "-r", &fps.to_string()]);
        cmd.args(["-frames:v", &frames.to_string(), "-t", &length]).args(self.encoder().quality_args());
        cmd.args(["-movflags", "+faststart"]).arg(output);
        let out = cmd.output().context("running ffmpeg")?;
        if !out.status.success() {
            bail!("ffmpeg could not finish {}: {}", render.display(), String::from_utf8_lossy(&out.stderr).trim());
        }
        self.probe(output)
    }

    /// A copy a browser plays (H.264 4:2:0, AAC, long side at most `max_side`), for files the WebView cannot decode:
    /// prep's lossless masks are High 4:4:4, and sources come in any codec FFmpeg reads.
    pub fn preview(&self, input: &Path, output: &Path, max_side: u32) -> Result<()> {
        let filter = format!(
            "scale='if(gte(iw,ih),min({max_side},iw),-2)':'if(gte(iw,ih),-2,min({max_side},ih))',setsar=1,format=yuv420p"
        );
        let mut cmd = command(&self.ffmpeg);
        cmd.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(input).args(["-vf", &filter]);
        cmd.args(self.encoder().quality_args()).args(["-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart"]).arg(output);
        let out = cmd.output().context("running ffmpeg")?;
        if !out.status.success() {
            bail!("ffmpeg could not convert {}: {}", input.display(), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    /// `count` frames spread over the first `duration` seconds, side by side at `height` px: a trim strip, as PNG.
    pub fn strip(&self, input: &Path, output: &Path, duration: f64, count: u32, height: u32) -> Result<()> {
        let filter = format!("fps={}/{},scale=-2:{height},tile={count}x1", count, duration.max(0.1));
        let mut cmd = command(&self.ffmpeg);
        cmd.args(["-v", "error", "-nostdin", "-y", "-i"]).arg(input);
        cmd.args(["-vf", &filter, "-frames:v", "1", "-update", "1"]).arg(output);
        let out = cmd.output().context("running ffmpeg")?;
        if !out.status.success() {
            bail!("ffmpeg could not read frames of {}: {}", input.display(), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }

    /// Writes single-channel frames (0 or 255) to an MP4 the renderers read as a mask; lossless on NVENC, which needs
    /// frames of at least about 145×49.
    pub fn mask_writer(&self, output: &Path, width: u32, height: u32, fps: f64) -> Result<MaskWriter> {
        let mut cmd = command(&self.ffmpeg);
        cmd.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "gray"]);
        cmd.args(["-s", &format!("{width}x{height}"), "-r", &fps.to_string(), "-i", "-"]);
        cmd.args(match self.encoder() {
            Encoder::Nvenc => &["-c:v", "h264_nvenc", "-tune", "lossless", "-preset", "p7"][..],
            Encoder::OpenH264 => &["-c:v", "libopenh264", "-b:v", "20M"][..],
        });
        cmd.args(["-pix_fmt", "yuv420p"]).arg(output);
        let mut child = cmd.stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().context("starting ffmpeg")?;
        let stdin = child.stdin.take().expect("piped stdin");
        Ok(MaskWriter { child, stdin: Some(stdin), frame_bytes: (width * height) as usize })
    }

    /// Decodes `input` to RGB frames of exactly `width`×`height` (stretched, as the trackers expect a square input),
    /// at `fps` when given. H.264 and HEVC decode on the GPU when FFmpeg can (`-hwaccel cuda`), else on the CPU.
    pub fn frames(&self, input: &Path, width: u32, height: u32, fps: Option<f64>) -> Result<Frames> {
        let mut filter = format!("scale={width}:{height}:flags=bilinear");
        if let Some(fps) = fps {
            filter = format!("fps={fps},{filter}");
        }
        let mut cmd = command(&self.ffmpeg);
        cmd.args(["-v", "error", "-nostdin"]);
        if self.encoder() == Encoder::Nvenc {
            cmd.args(["-hwaccel", "cuda"]);
        }
        let mut child = cmd
            .arg("-i")
            .arg(input)
            .args(["-vf", &filter, "-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting ffmpeg")?;
        let stdout = child.stdout.take().expect("piped stdout");
        Ok(Frames { child, stdout, frame_bytes: (width * height * 3) as usize })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Encoder {
    /// NVIDIA's hardware H.264 encoder.
    Nvenc,
    /// Cisco's OpenH264, in the LGPL build; the fallback without NVENC.
    OpenH264,
}

impl Encoder {
    fn quality_args(self) -> &'static [&'static str] {
        match self {
            Encoder::Nvenc => &["-c:v", "h264_nvenc", "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", "18"],
            Encoder::OpenH264 => &["-c:v", "libopenh264", "-b:v", "12M"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Normalize {
    pub fps: f64,
    pub max_side: u32,
    pub start: f64,
    pub duration: Option<f64>,
}

impl Default for Normalize {
    /// Wan 2.2 Animate's frame rate, and Swapjutsu's 960 px long side.
    fn default() -> Self {
        Self { fps: 16.0, max_side: 960, start: 0.0, duration: None }
    }
}

/// Mask frames going into FFmpeg; `finish` waits for the file.
pub struct MaskWriter {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    frame_bytes: usize,
}

impl MaskWriter {
    pub fn write(&mut self, mask: &[u8]) -> Result<()> {
        use std::io::Write;
        anyhow::ensure!(mask.len() == self.frame_bytes, "mask frame is {} bytes, expected {}", mask.len(), self.frame_bytes);
        self.stdin.as_mut().context("writer finished")?.write_all(mask).context("writing a mask frame to ffmpeg")
    }

    pub fn finish(self) -> Result<()> {
        let MaskWriter { child, stdin, .. } = self;
        drop(stdin);
        let out = child.wait_with_output().context("waiting for ffmpeg")?;
        if !out.status.success() {
            bail!("ffmpeg could not write the mask: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }
}

/// Raw RGB frames streaming out of ffmpeg.
pub struct Frames {
    child: Child,
    stdout: ChildStdout,
    frame_bytes: usize,
}

impl Iterator for Frames {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut frame = vec![0; self.frame_bytes];
        let mut filled = 0;
        while filled < frame.len() {
            match self.stdout.read(&mut frame[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) => return Some(Err(e.into())),
            }
        }
        if filled == frame.len() {
            return Some(Ok(frame));
        }
        let status = self.child.wait();
        let mut stderr = String::new();
        if let Some(mut err) = self.child.stderr.take() {
            let _ = err.read_to_string(&mut stderr);
        }
        match status {
            Ok(s) if s.success() && filled == 0 => None,
            Ok(s) if s.success() => Some(Err(anyhow::anyhow!("ffmpeg ended mid-frame"))),
            Ok(_) => Some(Err(anyhow::anyhow!("ffmpeg failed: {}", stderr.trim()))),
            Err(e) => Some(Err(e.into())),
        }
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    quiet(&mut command);
    command
}

/// No console window flashes up for a child process of the GUI app on Windows.
pub fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn parse_probe(json: &[u8]) -> Result<Probe> {
    #[derive(Deserialize)]
    struct Out {
        streams: Vec<Stream>,
        format: Format,
    }
    #[derive(Deserialize)]
    struct Stream {
        codec_type: String,
        width: Option<u32>,
        height: Option<u32>,
        avg_frame_rate: Option<String>,
    }
    #[derive(Deserialize)]
    struct Format {
        duration: Option<String>,
    }
    let out: Out = serde_json::from_slice(json).context("parsing ffprobe output")?;
    let video = out.streams.iter().find(|s| s.codec_type == "video").context("no video stream")?;
    let fps = video.avg_frame_rate.as_deref().and_then(|r| {
        let (n, d) = r.split_once('/')?;
        let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
        (d > 0.0).then_some(n / d)
    });
    Ok(Probe {
        width: video.width.context("no width")?,
        height: video.height.context("no height")?,
        fps: fps.context("no frame rate")?,
        duration: out.format.duration.and_then(|d| d.parse().ok()).unwrap_or(0.0),
        has_audio: out.streams.iter().any(|s| s.codec_type == "audio"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reads_video_and_audio() {
        let json = br#"{"streams":[{"codec_type":"video","width":1920,"height":1080,"avg_frame_rate":"30000/1001"},
            {"codec_type":"audio"}],"format":{"duration":"7.007"}}"#;
        let p = parse_probe(json).unwrap();
        assert_eq!((p.width, p.height, p.has_audio), (1920, 1080, true));
        assert!((p.fps - 29.97).abs() < 0.01);
        assert!((p.duration - 7.007).abs() < 1e-9);
    }

    #[test]
    fn probe_without_video_fails() {
        let json = br#"{"streams":[{"codec_type":"audio"}],"format":{}}"#;
        assert!(parse_probe(json).is_err());
    }

    /// Needs FFmpeg: KAWARIMI_FFMPEG_DIR (the installed asset, which scripts/ci.sh sets) or PATH. With the variable set,
    /// a missing FFmpeg fails the test instead of skipping it.
    #[test]
    fn decodes_a_generated_clip() {
        let dir = std::env::var_os("KAWARIMI_FFMPEG_DIR").map(PathBuf::from);
        let Some(ff) = Ffmpeg::locate(dir.as_deref()) else {
            assert!(dir.is_none(), "no ffmpeg in KAWARIMI_FFMPEG_DIR");
            eprintln!("skipped: no ffmpeg (set KAWARIMI_FFMPEG_DIR)");
            return;
        };
        let tmp = std::env::temp_dir().join(format!("kawarimi-test-{}.mp4", std::process::id()));
        let made = quiet(Command::new(&ff.ffmpeg).args(["-v", "error", "-y", "-f", "lavfi", "-i"]))
            .args(["testsrc=size=320x240:rate=10:duration=1", "-pix_fmt", "yuv420p"])
            .arg(&tmp)
            .status()
            .unwrap();
        assert!(made.success());
        let probe = ff.probe(&tmp).unwrap();
        assert_eq!((probe.width, probe.height), (320, 240));
        let frames: Vec<_> = ff.frames(&tmp, 64, 64, None).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(frames.len(), 10);
        assert!(frames.iter().all(|f| f.len() == 64 * 64 * 3));
        let _ = std::fs::remove_file(&tmp);
    }

    /// Normalises a sideways 10 fps clip to the pipeline's size and rate, and writes a mask video that decodes back
    /// to the same pixels.
    #[test]
    fn normalises_and_writes_masks() {
        let dir = std::env::var_os("KAWARIMI_FFMPEG_DIR").map(PathBuf::from);
        let Some(ff) = Ffmpeg::locate(dir.as_deref()) else {
            assert!(dir.is_none(), "no ffmpeg in KAWARIMI_FFMPEG_DIR");
            eprintln!("skipped: no ffmpeg (set KAWARIMI_FFMPEG_DIR)");
            return;
        };
        let tmp = tempdir();
        let source = tmp.join("portrait.mp4");
        let made = quiet(Command::new(&ff.ffmpeg).args(["-v", "error", "-y", "-f", "lavfi", "-i"]))
            .args(["testsrc=size=1080x1920:rate=10:duration=2", "-pix_fmt", "yuv420p", "-c:v", "libopenh264"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(made.success());
        let out = tmp.join("normalized.mp4");
        let probe = ff.normalize(&source, &out, &Normalize { duration: Some(1.5), ..Default::default() }).unwrap();
        assert_eq!((probe.width, probe.height), (540, 960), "{probe:?}");
        assert!((probe.fps - 16.0).abs() < 0.01, "{probe:?}");
        assert!((probe.duration - 1.5).abs() < 0.1, "{probe:?}");

        let mask_path = tmp.join("mask.mp4");
        let mut writer = ff.mask_writer(&mask_path, 256, 192, 16.0).unwrap();
        let mut mask = vec![0u8; 256 * 192];
        mask[..256 * 96].fill(255);
        for _ in 0..4 {
            writer.write(&mask).unwrap();
        }
        writer.finish().unwrap();
        let frames: Vec<_> = ff.frames(&mask_path, 256, 192, None).unwrap().collect::<Result<_>>().unwrap();
        assert_eq!(frames.len(), 4);
        let red: Vec<u8> = frames[0].chunks_exact(3).map(|p| p[0]).collect();
        let wrong = red.iter().zip(&mask).filter(|(a, b)| (**a > 127) != (**b > 127)).count();
        assert_eq!(wrong, 0, "mask pixels changed through the video");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The mask's preview plays in a browser (no 4:4:4 profile) and the strip is one row of frames.
    #[test]
    fn previews_and_strips() {
        let dir = std::env::var_os("KAWARIMI_FFMPEG_DIR").map(PathBuf::from);
        let Some(ff) = Ffmpeg::locate(dir.as_deref()) else {
            assert!(dir.is_none(), "no ffmpeg in KAWARIMI_FFMPEG_DIR");
            eprintln!("skipped: no ffmpeg (set KAWARIMI_FFMPEG_DIR)");
            return;
        };
        let tmp = tempdir();
        let mask_path = tmp.join("mask.mp4");
        let mut writer = ff.mask_writer(&mask_path, 320, 192, 16.0).unwrap();
        for _ in 0..16 {
            writer.write(&vec![255u8; 320 * 192]).unwrap();
        }
        writer.finish().unwrap();
        let view = tmp.join("mask-view.mp4");
        ff.preview(&mask_path, &view, 960).unwrap();
        let profile = quiet(Command::new(&ff.ffprobe).args(["-v", "error", "-show_entries", "stream=profile", "-of", "csv=p=0"]))
            .arg(&view)
            .output()
            .unwrap();
        let profile = String::from_utf8_lossy(&profile.stdout);
        assert!(!profile.contains("4:4:4"), "{profile}");
        assert_eq!(ff.probe(&view).unwrap().width, 320);
        let strip = tmp.join("strip.png");
        ff.strip(&view, &strip, 1.0, 4, 48).unwrap();
        let (w, h) = image_size(&strip);
        assert_eq!((w, h), (4 * 80, 48));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A PNG's width and height from its header.
    fn image_size(png: &Path) -> (u32, u32) {
        let bytes = std::fs::read(png).unwrap();
        let be = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        (be(16), be(20))
    }

    /// A phone clip stored sideways with a rotation tag, at a variable frame rate, comes out upright at 16 fps.
    #[test]
    fn normalises_rotated_variable_rate_clips() {
        let dir = std::env::var_os("KAWARIMI_FFMPEG_DIR").map(PathBuf::from);
        let Some(ff) = Ffmpeg::locate(dir.as_deref()) else {
            assert!(dir.is_none(), "no ffmpeg in KAWARIMI_FFMPEG_DIR");
            eprintln!("skipped: no ffmpeg (set KAWARIMI_FFMPEG_DIR)");
            return;
        };
        let tmp = tempdir();
        let plain = tmp.join("plain.mp4");
        // 30 frames whose spacing alternates between 1/30 s and 1/10 s: 2 s in all.
        let made = command(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=640x360:rate=30:duration=1"])
            .args(["-vf", "setpts='(N*2+if(mod(N,2),1,0))/(30*TB)'", "-fps_mode", "passthrough"])
            .args(["-pix_fmt", "yuv420p", "-c:v", "libopenh264"])
            .arg(&plain)
            .status()
            .unwrap();
        assert!(made.success());
        let rotated = tmp.join("rotated.mp4");
        let tagged = command(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-display_rotation", "90", "-i"])
            .arg(&plain)
            .args(["-c", "copy"])
            .arg(&rotated)
            .status()
            .unwrap();
        assert!(tagged.success());
        let out = tmp.join("normalized.mp4");
        let probe = ff.normalize(&rotated, &out, &Normalize::default()).unwrap();
        assert_eq!((probe.width, probe.height), (360, 640), "upright: {probe:?}");
        assert!((probe.fps - 16.0).abs() < 0.01, "{probe:?}");
        let frames = ff.frames(&out, 36, 64, None).unwrap().count();
        assert!((frames as f64 - probe.duration * 16.0).abs() <= 2.0, "{frames} frames for {probe:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A render with no sound of its own gets the trimmed clip's sound, as long as the frames it is cut to.
    #[test]
    fn finishes_a_render_with_the_clips_sound() {
        let dir = std::env::var_os("KAWARIMI_FFMPEG_DIR").map(PathBuf::from);
        let Some(ff) = Ffmpeg::locate(dir.as_deref()) else {
            assert!(dir.is_none(), "no ffmpeg in KAWARIMI_FFMPEG_DIR");
            eprintln!("skipped: no ffmpeg (set KAWARIMI_FFMPEG_DIR)");
            return;
        };
        let tmp = tempdir();
        let (clip, render) = (tmp.join("clip.mp4"), tmp.join("render.mp4"));
        let made = command(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=24:duration=3"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=3", "-pix_fmt", "yuv420p", "-c:v", "libopenh264"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(made.success());
        let normalized = tmp.join("normalized.mp4");
        let n = Normalize { start: 0.5, duration: Some(2.0), ..Default::default() };
        assert!(ff.normalize(&clip, &normalized, &n).unwrap().has_audio);
        let made = command(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=256x192:rate=16", "-frames:v", "36"])
            .args(["-pix_fmt", "yuv420p", "-c:v", "libopenh264"])
            .arg(&render)
            .status()
            .unwrap();
        assert!(made.success());

        for frames in [32, 20] {
            let out = tmp.join(format!("final-{frames}.mp4"));
            let probe = ff.finish(&render, &normalized, &out, frames, 16.0).unwrap();
            assert!(probe.has_audio, "{probe:?}");
            assert_eq!((probe.width, probe.height), (256, 192));
            assert!((probe.fps - 16.0).abs() < 0.01, "{probe:?}");
            assert!((probe.duration - frames as f64 / 16.0).abs() < 0.03, "{frames}: {probe:?}");
            assert_eq!(ff.frames(&out, 32, 32, None).unwrap().count(), frames as usize);
        }
        let silent = ff.finish(&render, &render, &tmp.join("silent.mp4"), 32, 16.0).unwrap();
        assert!(!silent.has_audio);
        assert!(ff.finish(&render, &normalized, &tmp.join("long.mp4"), 40, 16.0).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kawarimi-media-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
