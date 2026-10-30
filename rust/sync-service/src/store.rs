//! Upload store on the filesystem — a port of packages/infra/src/sync-store.ts.
//!
//! Chunks are appended to `.uploading-<id>` files in a staging directory, then
//! renamed into the shared directory. Ids and relative paths are validated by the
//! upload route before they reach here; the directories are injected so the whole
//! path runs against a temp dir in tests.
//!
//! Every function is synchronous `std::fs`; the async handlers call them through
//! `tokio::task::spawn_blocking`, so no blocking work runs on a runtime thread.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone)]
pub struct Dirs {
    pub shared: String,
    pub upload: String,
}

impl Dirs {
    fn temp(&self, id: &str) -> PathBuf {
        Path::new(&self.upload).join(format!(".uploading-{id}"))
    }
}

#[derive(Serialize)]
pub struct SharedFile {
    pub name: String,
    pub size: u64,
    /// Modified time in whole milliseconds since the epoch (the TS side reports
    /// sub-millisecond `mtimeMs`; the difference is not wire-significant).
    pub mtime: u64,
}

pub fn append_chunk(dirs: &Dirs, id: &str, data: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(&dirs.upload)?;
    fs::create_dir_all(&dirs.shared)?;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dirs.temp(id))?;
    f.write_all(data)
}

pub fn has_chunks(dirs: &Dirs, id: &str) -> bool {
    dirs.temp(id).exists()
}

pub fn sha256_hex(dirs: &Dirs, id: &str) -> std::io::Result<String> {
    let bytes = fs::read(dirs.temp(id))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize()))
}

pub fn discard(dirs: &Dirs, id: &str) {
    let _ = fs::remove_file(dirs.temp(id));
}

pub fn commit(dirs: &Dirs, id: &str, relative_path: &str) -> std::io::Result<()> {
    let final_path = Path::new(&dirs.shared).join(relative_path);
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(dirs.temp(id), final_path)
}

/// Files in the shared directory, hidden entries skipped (same as the TS store).
pub fn list(dirs: &Dirs) -> Vec<SharedFile> {
    let mut files = Vec::new();
    let Ok(rd) = fs::read_dir(&dirs.shared) else {
        return files;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let Ok(md) = entry.metadata() else { continue };
        if !md.is_file() {
            continue;
        }
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        files.push(SharedFile {
            name,
            size: md.len(),
            mtime,
        });
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> (Dirs, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let d = Dirs {
            shared: dir.path().join("shared").to_string_lossy().into_owned(),
            upload: dir.path().join("uploads").to_string_lossy().into_owned(),
        };
        (d, dir)
    }

    #[test]
    fn append_commit_and_list() {
        let (d, _guard) = dirs();
        append_chunk(&d, "f1", b"hello ").unwrap();
        append_chunk(&d, "f1", b"world").unwrap();
        assert!(has_chunks(&d, "f1"));
        // sha256("hello world")
        assert_eq!(
            sha256_hex(&d, "f1").unwrap(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        commit(&d, "f1", "sub/dir/out.txt").unwrap();
        assert!(
            !has_chunks(&d, "f1"),
            "the staged file is gone after commit"
        );

        let listed = list(&d);
        // The committed file lives under a subdir, so the top level shows the dir,
        // not a file: list reports only regular files at the top of shared.
        assert!(listed.iter().all(|f| f.name != "out.txt"));

        // A file directly in shared shows up with its size; a dotfile does not.
        std::fs::write(format!("{}/visible.bin", d.shared), b"1234").unwrap();
        std::fs::write(format!("{}/.hidden", d.shared), b"x").unwrap();
        let listed = list(&d);
        let visible: Vec<_> = listed.iter().map(|f| f.name.as_str()).collect();
        assert!(visible.contains(&"visible.bin"));
        assert!(!visible.iter().any(|n| n.starts_with('.')));
        let f = listed.iter().find(|f| f.name == "visible.bin").unwrap();
        assert_eq!(f.size, 4);
    }

    #[test]
    fn discard_removes_the_staged_file() {
        let (d, _guard) = dirs();
        append_chunk(&d, "f2", b"data").unwrap();
        assert!(has_chunks(&d, "f2"));
        discard(&d, "f2");
        assert!(!has_chunks(&d, "f2"));
    }

    #[test]
    fn list_of_missing_dir_is_empty() {
        let (d, _guard) = dirs();
        assert!(list(&d).is_empty());
    }
}
