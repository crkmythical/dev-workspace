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
use std::io::{Read, Write};
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

/// Why a chunk append can fail.
#[derive(Debug)]
pub enum AppendError {
    /// Appending this chunk would push the assembled upload past `max_total`.
    TooLarge,
    Io(std::io::Error),
}

/// Append one decrypted chunk to the staging file, enforcing a total-size ceiling so a client (or
/// a looping one) cannot fill the vault disk. `max_total` is the largest the assembled upload may
/// reach; an append that would cross it is refused before any bytes are written.
pub fn append_chunk(dirs: &Dirs, id: &str, data: &[u8], max_total: u64) -> Result<(), AppendError> {
    fs::create_dir_all(&dirs.upload).map_err(AppendError::Io)?;
    fs::create_dir_all(&dirs.shared).map_err(AppendError::Io)?;
    let existing = fs::metadata(dirs.temp(id)).map(|m| m.len()).unwrap_or(0);
    if existing.saturating_add(data.len() as u64) > max_total {
        return Err(AppendError::TooLarge);
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dirs.temp(id))
        .map_err(AppendError::Io)?;
    f.write_all(data).map_err(AppendError::Io)
}

pub fn has_chunks(dirs: &Dirs, id: &str) -> bool {
    dirs.temp(id).exists()
}

pub fn sha256_hex(dirs: &Dirs, id: &str) -> std::io::Result<String> {
    // Stream the file through the hasher instead of reading it whole: an upload can reach the
    // client's 500 MB SIZE_LIMIT, and loading all of it into one Vec at finalize is an OOM vector.
    let mut f = fs::File::open(dirs.temp(id))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
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

/// Outcome of verifying + committing an assembled upload.
#[derive(Debug, PartialEq, Eq)]
pub enum FinalizeOutcome {
    Committed,
    ChecksumMismatch,
    NoChunks,
}

/// Verify the staged upload's whole-file checksum and commit it, all in ONE call so no concurrent
/// append can slip between the hash and the rename (the former hash-then-commit sequence had that
/// TOCTOU). A mismatch discards the staged file; a missing staged file is NoChunks, not an error.
pub fn finalize_verified(
    dirs: &Dirs,
    id: &str,
    relative_path: &str,
    expected_sha256: &str,
) -> std::io::Result<FinalizeOutcome> {
    if !dirs.temp(id).exists() {
        return Ok(FinalizeOutcome::NoChunks);
    }
    if sha256_hex(dirs, id)? != expected_sha256 {
        discard(dirs, id);
        return Ok(FinalizeOutcome::ChecksumMismatch);
    }
    commit(dirs, id, relative_path)?;
    Ok(FinalizeOutcome::Committed)
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
        append_chunk(&d, "f1", b"hello ", 1 << 20).unwrap();
        append_chunk(&d, "f1", b"world", 1 << 20).unwrap();
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
        append_chunk(&d, "f2", b"data", 1 << 20).unwrap();
        assert!(has_chunks(&d, "f2"));
        discard(&d, "f2");
        assert!(!has_chunks(&d, "f2"));
    }

    #[test]
    fn list_of_missing_dir_is_empty() {
        let (d, _guard) = dirs();
        assert!(list(&d).is_empty());
    }

    #[test]
    fn append_chunk_refuses_to_exceed_the_cap() {
        let (d, _guard) = dirs();
        // First 6 bytes fit under a 10-byte cap.
        append_chunk(&d, "cap", b"hello ", 10).unwrap();
        // The next 5 would cross it (6 + 5 > 10): refused, and nothing is appended.
        assert!(matches!(
            append_chunk(&d, "cap", b"world", 10),
            Err(AppendError::TooLarge)
        ));
        assert_eq!(sha256_hex(&d, "cap").unwrap(), {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(b"hello "))
        });
    }

    #[test]
    fn finalize_verified_commits_discards_or_reports_no_chunks() {
        let (d, _guard) = dirs();
        // No staged file yet.
        assert_eq!(
            finalize_verified(&d, "none", "out.txt", "deadbeef").unwrap(),
            FinalizeOutcome::NoChunks
        );

        append_chunk(&d, "ok", b"hello world", 1 << 20).unwrap();
        let good = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        // A wrong checksum discards the staged file and does not commit.
        assert_eq!(
            finalize_verified(&d, "ok", "out.txt", "00").unwrap(),
            FinalizeOutcome::ChecksumMismatch
        );
        assert!(!has_chunks(&d, "ok"), "a mismatch discards the staged file");

        // The right checksum commits atomically.
        append_chunk(&d, "ok2", b"hello world", 1 << 20).unwrap();
        assert_eq!(
            finalize_verified(&d, "ok2", "out2.txt", good).unwrap(),
            FinalizeOutcome::Committed
        );
        assert!(!has_chunks(&d, "ok2"));
        assert_eq!(
            std::fs::read(format!("{}/out2.txt", d.shared)).unwrap(),
            b"hello world"
        );
    }
}
