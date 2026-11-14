//! Upload store on the filesystem — a port of packages/infra/src/sync-store.ts.
//!
//! Chunks are appended to `.uploading-<id>` files in a staging directory, then
//! renamed into the shared directory. Ids and relative paths are validated by the
//! upload route before they reach here; the directories are injected so the whole
//! path runs against a temp dir in tests.
//!
//! The service runs as root and the shared tree lives in the vault, where the user (and anything
//! they run) can create symlinks. A validated relative path does not help against a symlink that
//! is already there: `shared/link -> /etc` makes `link/passwd` land in /etc. So nothing here
//! follows a symlink: the commit walks the path one directory at a time with openat(O_NOFOLLOW),
//! and the staging file is opened with O_NOFOLLOW. A planted link makes the call fail instead.
//!
//! Every function is synchronous `std::fs`; the async handlers call them through
//! `tokio::task::spawn_blocking`, so no blocking work runs on a runtime thread.

use rustix::fs::{mkdirat, openat, renameat, Mode, OFlags, CWD};
use rustix::io::Errno;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// `open(2)` flags that refuse a symlink in the last path position.
fn nofollow() -> i32 {
    OFlags::NOFOLLOW.bits() as i32
}

/// Open the directory `name` inside `parent` (or, with CWD as parent, at an absolute path),
/// failing rather than following a symlink standing in its place.
fn open_dir(parent: impl AsFd, name: impl rustix::path::Arg) -> std::io::Result<OwnedFd> {
    Ok(openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?)
}

#[derive(Clone)]
pub struct Dirs {
    pub shared: String,
    pub upload: String,
}

impl Dirs {
    fn temp(&self, id: &str) -> PathBuf {
        Path::new(&self.upload).join(Self::temp_name(id))
    }

    fn temp_name(id: &str) -> String {
        format!(".uploading-{id}")
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
        .custom_flags(nofollow())
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
    let mut f = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nofollow())
        .open(dirs.temp(id))?;
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

/// Move the staged upload to `relative_path` under the shared directory, creating the directories
/// on the way. Every step is relative to the directory opened before it and refuses a symlink, so
/// nothing that is (or becomes) a link can send the file elsewhere; a link in the final position
/// is replaced by the new file, not followed.
pub fn commit(dirs: &Dirs, id: &str, relative_path: &str) -> std::io::Result<()> {
    let mut names: Vec<&str> = relative_path.split('/').collect();
    let file = names.pop().unwrap_or("");
    let plain = |n: &str| !n.is_empty() && n != "." && n != ".." && !n.contains('\0');
    if !plain(file) || !names.iter().all(|n| plain(n)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a plain relative path",
        ));
    }
    let staging = open_dir(CWD, dirs.upload.as_str())?;
    let mut dir = open_dir(CWD, dirs.shared.as_str())?;
    for name in names {
        match mkdirat(&dir, name, Mode::from_bits_truncate(0o755)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(e) => return Err(e.into()),
        }
        dir = open_dir(&dir, name)?;
    }
    renameat(&staging, Dirs::temp_name(id), &dir, file)?;
    Ok(())
}

/// Outcome of verifying + committing an assembled upload.
#[derive(Debug, PartialEq, Eq)]
pub enum FinalizeOutcome {
    Committed,
    ChecksumMismatch,
    NoChunks,
}

/// Verify the staged upload's whole-file checksum and commit it. The route holds the per-upload
/// sequencer across this call, so an append for the same id cannot race the hash and rename. A
/// mismatch discards the staged file; a missing staged file is NoChunks, not an error.
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
    fn a_symlink_in_the_shared_tree_is_never_followed_by_a_commit() {
        use std::os::unix::fs::symlink;
        let (d, guard) = dirs();
        let outside = guard.path().join("outside");
        fs::create_dir(&outside).unwrap();
        append_chunk(&d, "x", b"data", 1 << 20).unwrap();

        // A link standing in for a directory on the way: the file must not land behind it.
        symlink(&outside, format!("{}/link", d.shared)).unwrap();
        assert!(commit(&d, "x", "link/escaped.txt").is_err());
        assert!(!outside.join("escaped.txt").exists(), "nothing escaped");
        assert!(
            has_chunks(&d, "x"),
            "a refused commit leaves the staged file where it was"
        );

        // A link in the file's own place is replaced, and what it pointed at is left alone.
        fs::write(outside.join("target.txt"), "keep").unwrap();
        symlink(
            outside.join("target.txt"),
            format!("{}/final.txt", d.shared),
        )
        .unwrap();
        commit(&d, "x", "final.txt").unwrap();
        assert_eq!(
            fs::read_to_string(outside.join("target.txt")).unwrap(),
            "keep"
        );
        let landed = format!("{}/final.txt", d.shared);
        assert!(!fs::symlink_metadata(&landed)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&landed).unwrap(), "data");

        // Directories that do not exist yet are still created.
        append_chunk(&d, "y", b"nested", 1 << 20).unwrap();
        commit(&d, "y", "a/b/c.txt").unwrap();
        assert_eq!(
            fs::read_to_string(format!("{}/a/b/c.txt", d.shared)).unwrap(),
            "nested"
        );
    }

    #[test]
    fn a_shared_directory_that_is_itself_a_link_is_refused() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir_all(real.join("uploads")).unwrap();
        symlink(&real, dir.path().join("shared")).unwrap();
        let d = Dirs {
            shared: dir.path().join("shared").to_string_lossy().into_owned(),
            upload: real.join("uploads").to_string_lossy().into_owned(),
        };
        append_chunk(&d, "x", b"data", 1 << 20).unwrap();
        assert!(commit(&d, "x", "f.txt").is_err());
        assert!(!real.join("f.txt").exists());
    }

    #[test]
    fn a_hostile_relative_path_is_refused_even_though_the_route_checks_it_too() {
        let (d, _guard) = dirs();
        append_chunk(&d, "x", b"data", 1 << 20).unwrap();
        for bad in ["", "..", "../x", "a/../x", "a//b", "a/./b", "/abs", "a/\0b"] {
            assert!(commit(&d, "x", bad).is_err(), "{bad:?}");
        }
        assert!(has_chunks(&d, "x"));
    }

    #[test]
    fn the_staging_file_is_not_followed_if_it_is_a_link() {
        use std::os::unix::fs::symlink;
        let (d, guard) = dirs();
        fs::create_dir_all(&d.upload).unwrap();
        let victim = guard.path().join("victim.txt");
        fs::write(&victim, "precious").unwrap();
        symlink(&victim, d.temp("planted")).unwrap();
        assert!(append_chunk(&d, "planted", b"evil", 1 << 20).is_err());
        assert!(sha256_hex(&d, "planted").is_err());
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
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
