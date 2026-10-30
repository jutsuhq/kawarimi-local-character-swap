//! The folder the assets live in. A file is written next to its destination as `.part`, checked for size and SHA-256
//! while it streams, and moved into place only when both match; a mismatch is deleted. An asset counts as installed
//! only when its `.complete` marker names the exact pins of the current manifest.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use futures_util::{Stream, StreamExt};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::manifest::{Asset, File};

const MARKER: &str = ".complete";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Installed,
    Missing,
}

pub struct Store {
    root: PathBuf,
    client: reqwest::Client,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("kawarimi/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("TLS backend");
        Self { root: root.into(), client }
    }

    pub fn dir(&self, asset: &Asset) -> PathBuf {
        self.root.join(asset.id)
    }

    pub fn status(&self, asset: &Asset) -> Status {
        let marker = std::fs::read_to_string(self.dir(asset).join(MARKER)).unwrap_or_default();
        if marker == pins(asset) { Status::Installed } else { Status::Missing }
    }

    /// The path of a file the asset provides, once installed.
    pub fn path(&self, asset: &Asset, file: &str) -> Result<PathBuf> {
        if self.status(asset) != Status::Installed {
            bail!("{} is not installed", asset.title);
        }
        Ok(self.dir(asset).join(file))
    }

    /// Downloads, verifies and unpacks every file. `progress` gets (bytes done, bytes total) across the asset.
    pub async fn install(&self, asset: &Asset, progress: impl Fn(u64, u64)) -> Result<()> {
        let dir = self.dir(asset);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let _ = std::fs::remove_file(dir.join(MARKER));
        let total = asset.download_size();
        let mut done = 0;
        for file in asset.files {
            let dest = dir.join(file.path);
            if file.unpack.is_empty() && verify_file(&dest, file).is_ok() {
                done += file.size;
                progress(done, total);
                continue;
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let response = self.client.get(file.url).send().await.and_then(|r| r.error_for_status());
            let response = response.with_context(|| format!("downloading {}", file.url))?;
            let base = done;
            write_verified(response.bytes_stream(), &dest, file, |n| progress(base + n, total)).await?;
            done += file.size;
            if !file.unpack.is_empty() {
                unpack(&dest, &dir, file.unpack)?;
                std::fs::remove_file(&dest)?;
            }
        }
        std::fs::write(dir.join(MARKER), pins(asset))?;
        Ok(())
    }

    pub fn remove(&self, asset: &Asset) -> Result<()> {
        let dir = self.dir(asset);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        Ok(())
    }
}

/// What the marker records: every file's pin, so a changed manifest makes the old install count as missing.
fn pins(asset: &Asset) -> String {
    asset.files.iter().map(|f| format!("{} {}\n", f.sha256, f.path)).collect()
}

/// Streams `chunks` into `dest.part`, then moves it to `dest` if the size and SHA-256 match `file`.
pub async fn write_verified<S, B, E>(chunks: S, dest: &Path, file: &File, progress: impl Fn(u64)) -> Result<()>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
    E: std::error::Error + Send + Sync + 'static,
{
    let part = dest.with_extension(match dest.extension() {
        Some(ext) => format!("{}.part", ext.to_string_lossy()),
        None => "part".into(),
    });
    let result = async {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&part)?);
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        futures_util::pin_mut!(chunks);
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk?;
            let bytes = chunk.as_ref();
            written += bytes.len() as u64;
            if written > file.size {
                bail!("{} is larger than the pinned {} bytes", file.path, file.size);
            }
            hasher.update(bytes);
            out.write_all(bytes)?;
            progress(written);
        }
        out.flush()?;
        drop(out);
        if written != file.size {
            bail!("{} is {written} bytes, pinned {}", file.path, file.size);
        }
        let digest = hex::encode(hasher.finalize());
        if digest != file.sha256 {
            bail!("{} has SHA-256 {digest}, pinned {}", file.path, file.sha256);
        }
        std::fs::rename(&part, dest)?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// Checks a file already on disk against its pin.
pub fn verify_file(path: &Path, file: &File) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    if meta.len() != file.size {
        bail!("{} is {} bytes, pinned {}", path.display(), meta.len(), file.size);
    }
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    let digest = hex::encode(hasher.finalize());
    if digest != file.sha256 {
        bail!("{} has SHA-256 {digest}, pinned {}", path.display(), file.sha256);
    }
    Ok(())
}

/// Takes the named members out of a zip, .tar.xz or .tar.gz archive into `dir`. A member ending in `/` is a folder:
/// everything under it goes to the target folder, the prefix taken off.
fn unpack(archive: &Path, dir: &Path, members: &[(&str, &str)]) -> Result<()> {
    let name = archive.to_string_lossy();
    // Which target a path in the archive goes to, if any.
    let target = |path: &str| -> Option<(usize, PathBuf)> {
        members.iter().enumerate().find_map(|(i, (member, to))| {
            if member.ends_with('/') {
                let rest = path.strip_prefix(member)?;
                (!rest.is_empty()).then(|| (i, dir.join(to).join(rest)))
            } else {
                (*member == path).then(|| (i, dir.join(to)))
            }
        })
    };
    let mut found = vec![false; members.len()];
    let mut take = |path: &str, is_dir: bool, reader: &mut dyn std::io::Read| -> Result<()> {
        let Some((i, to)) = target(path) else { return Ok(()) };
        anyhow::ensure!(!path.split('/').any(|c| c == ".."), "{path} leaves the archive");
        found[i] = true;
        if is_dir {
            std::fs::create_dir_all(&to)?;
        } else {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_member(reader, &to)?;
        }
        Ok(())
    };
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let path = entry.name().to_owned();
            let is_dir = entry.is_dir();
            take(&path, is_dir, &mut entry)?;
        }
    } else if name.ends_with(".tar.xz") || name.ends_with(".tar.gz") {
        let file = std::fs::File::open(archive)?;
        let reader: Box<dyn std::io::Read> = if name.ends_with(".xz") {
            Box::new(xz2::read::XzDecoder::new(file))
        } else {
            Box::new(flate2::read::GzDecoder::new(file))
        };
        let mut tar = tar::Archive::new(reader);
        for entry in tar.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().replace('\\', "/");
            let is_dir = entry.header().entry_type().is_dir();
            take(&path, is_dir, &mut entry)?;
        }
    } else {
        bail!("unknown archive type: {name}");
    }
    if let Some(i) = found.iter().position(|f| !f) {
        bail!("{} not in {name}", members[i].0);
    }
    Ok(())
}

fn write_member<R: std::io::Read + ?Sized>(reader: &mut R, to: &Path) -> Result<()> {
    let mut out = std::fs::File::create(to)?;
    std::io::copy(reader, &mut out)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned(data: &[u8], path: &'static str) -> File {
        let sha: &'static str = Box::leak(hex::encode(Sha256::digest(data)).into_boxed_str());
        File { url: "https://example.invalid/x", path, size: data.len() as u64, sha256: sha, unpack: &[] }
    }

    fn chunks(data: &[u8]) -> impl Stream<Item = Result<Vec<u8>, std::io::Error>> {
        futures_util::stream::iter(data.chunks(3).map(|c| Ok(c.to_vec())).collect::<Vec<_>>())
    }

    #[tokio::test]
    async fn a_matching_download_lands_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"kawarimi model bytes";
        let file = pinned(data, "m.onnx");
        let dest = dir.path().join("m.onnx");
        write_verified(chunks(data), &dest, &file, |_| {}).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!dir.path().join("m.onnx.part").exists());
        verify_file(&dest, &file).unwrap();
    }

    #[tokio::test]
    async fn a_tampered_download_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let file = pinned(b"the real bytes", "m.onnx");
        let dest = dir.path().join("m.onnx");
        let err = write_verified(chunks(b"the fake bytes"), &dest, &file, |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("SHA-256"), "{err}");
        assert!(!dest.exists());
        assert!(!dir.path().join("m.onnx.part").exists());
    }

    #[tokio::test]
    async fn an_oversized_download_stops_early() {
        let dir = tempfile::tempdir().unwrap();
        let file = pinned(b"short", "m.onnx");
        let dest = dir.path().join("m.onnx");
        let err = write_verified(chunks(b"much longer than pinned"), &dest, &file, |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("larger"), "{err}");
        assert!(!dest.exists());
    }

    #[test]
    fn a_marker_from_other_pins_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let asset = &crate::manifest::YUNET;
        std::fs::create_dir_all(store.dir(asset)).unwrap();
        std::fs::write(store.dir(asset).join(MARKER), "0000 model.onnx\n").unwrap();
        assert_eq!(store.status(asset), Status::Missing);
        std::fs::write(store.dir(asset).join(MARKER), pins(asset)).unwrap();
        assert_eq!(store.status(asset), Status::Installed);
    }

    #[test]
    fn zip_members_are_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("tool.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        zip.start_file("tool-1.0/bin/tool.exe", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"binary").unwrap();
        zip.finish().unwrap();
        unpack(&archive, dir.path(), &[("tool-1.0/bin/tool.exe", "tool.exe")]).unwrap();
        assert_eq!(std::fs::read(dir.path().join("tool.exe")).unwrap(), b"binary");
        assert!(unpack(&archive, dir.path(), &[("missing", "x")]).is_err());
    }

    #[test]
    fn a_folder_member_unpacks_its_tree() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("src.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        zip.add_directory("app-1.0/", options).unwrap();
        zip.start_file("app-1.0/main.py", options).unwrap();
        zip.write_all(b"print()").unwrap();
        zip.start_file("app-1.0/pkg/mod.py", options).unwrap();
        zip.write_all(b"x = 1").unwrap();
        zip.finish().unwrap();
        unpack(&archive, dir.path(), &[("app-1.0/", "app")]).unwrap();
        assert_eq!(std::fs::read(dir.path().join("app/main.py")).unwrap(), b"print()");
        assert_eq!(std::fs::read(dir.path().join("app/pkg/mod.py")).unwrap(), b"x = 1");
    }
}
