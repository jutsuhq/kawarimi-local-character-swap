//! The normalised clip's frames on disk as raw RGB, so every tracklet reads its shot without decoding again.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use kawarimi_media::Ffmpeg;

pub struct FrameCache {
    path: PathBuf,
    file: File,
    pub width: usize,
    pub height: usize,
    pub count: usize,
}

impl FrameCache {
    /// Decodes `video` once into `path` (width × height × 3 bytes per frame).
    pub fn build(ffmpeg: &Ffmpeg, video: &Path, width: u32, height: u32, path: &Path) -> Result<Self> {
        let mut out = BufWriter::new(File::create(path).with_context(|| format!("creating {}", path.display()))?);
        let mut count = 0;
        for frame in ffmpeg.frames(video, width, height, None)? {
            out.write_all(&frame?)?;
            count += 1;
        }
        out.flush()?;
        drop(out);
        Ok(Self { path: path.to_owned(), file: File::open(path)?, width: width as usize, height: height as usize, count })
    }

    pub fn frame_bytes(&self) -> usize {
        self.width * self.height * 3
    }

    pub fn read(&mut self, index: usize, into: &mut Vec<u8>) -> Result<()> {
        anyhow::ensure!(index < self.count, "frame {index} of {}", self.count);
        into.resize(self.frame_bytes(), 0);
        self.file.seek(SeekFrom::Start((index * self.frame_bytes()) as u64))?;
        self.file.read_exact(into)?;
        Ok(())
    }

    /// Deletes the file; the cache is only for one prep.
    pub fn remove(self) -> Result<()> {
        let FrameCache { path, file, .. } = self;
        drop(file);
        std::fs::remove_file(path)?;
        Ok(())
    }
}

/// A binary mask packed 8 pixels to a byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bits {
    pub data: Vec<u8>,
    pub area: u32,
    /// x0, y0, x1, y1 (exclusive) of the set pixels; all zero for an empty mask.
    pub bounds: [u32; 4],
}

impl Bits {
    pub fn pack(mask: &[u8], width: usize) -> Self {
        let mut data = vec![0u8; mask.len().div_ceil(8)];
        let (mut area, mut b) = (0u32, [u32::MAX, u32::MAX, 0, 0]);
        for (i, &v) in mask.iter().enumerate() {
            if v != 0 {
                data[i / 8] |= 1 << (i % 8);
                area += 1;
                let (x, y) = ((i % width) as u32, (i / width) as u32);
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
        if area == 0 {
            b = [0; 4];
        }
        Self { data, area, bounds: b }
    }

    pub fn get(&self, i: usize) -> bool {
        self.data[i / 8] & (1 << (i % 8)) != 0
    }

    /// Sets this mask's pixels to 255 in `into`.
    pub fn paint(&self, into: &mut [u8]) {
        for (i, px) in into.iter_mut().enumerate() {
            if self.get(i) {
                *px = 255;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_area_and_bounds() {
        let (w, h) = (10, 4);
        let mut mask = vec![0u8; w * h];
        mask[1 * w + 2] = 1;
        mask[2 * w + 7] = 1;
        let bits = Bits::pack(&mask, w);
        assert_eq!(bits.area, 2);
        assert_eq!(bits.bounds, [2, 1, 8, 3]);
        assert!(bits.get(12) && bits.get(27) && !bits.get(0));
        let mut painted = vec![0u8; w * h];
        bits.paint(&mut painted);
        assert_eq!(painted.iter().filter(|&&v| v == 255).count(), 2);
    }
}
