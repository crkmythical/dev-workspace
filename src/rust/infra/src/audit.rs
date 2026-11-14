//! Appends audit events to a file, one JSON object per line (the format and the meaning are in
//! `sdw_core::audit`). The file is mode 0600 in a 0700 directory and is rotated by size, so a flood
//! of events cannot fill the disk. Every event is flushed to disk before `record` returns: the
//! self-destruct path kills the container right after it is logged.

use sdw_core::audit::{AuditEvent, AuditSink};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_KEEP: u32 = 3;

pub struct FileAuditSink {
    path: PathBuf,
    max_bytes: u64,
    keep: u32,
    echo_stdout: bool,
    lock: Mutex<()>,
}

impl FileAuditSink {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            max_bytes: DEFAULT_MAX_BYTES,
            keep: DEFAULT_KEEP,
            echo_stdout: false,
            lock: Mutex::new(()),
        }
    }

    /// Rotate once the file reaches `max_bytes`, keeping `keep` files in all (`audit.jsonl`,
    /// `audit.jsonl.1`, ...).
    pub fn with_limits(mut self, max_bytes: u64, keep: u32) -> Self {
        self.max_bytes = max_bytes;
        self.keep = keep.max(1);
        self
    }

    /// Also print each line on stdout, so `docker logs` (and whatever collects it) holds a copy
    /// outside the container's own file system.
    pub fn echo_stdout(mut self, on: bool) -> Self {
        self.echo_stdout = on;
        self
    }

    fn rotated(&self, n: u32) -> PathBuf {
        let mut s = self.path.clone().into_os_string();
        s.push(format!(".{n}"));
        PathBuf::from(s)
    }

    fn rotate(&self) {
        // audit.jsonl.(keep-1) is overwritten, each older file moves one up, the live file becomes .1.
        for n in (1..self.keep).rev() {
            let from = if n == 1 {
                self.path.clone()
            } else {
                self.rotated(n - 1)
            };
            let _ = fs::rename(from, self.rotated(n));
        }
        if self.keep == 1 {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn append(&self, line: &str) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        if fs::metadata(&self.path).is_ok_and(|m| m.len() >= self.max_bytes) {
            self.rotate();
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_data()
    }
}

impl AuditSink for FileAuditSink {
    fn record(&self, event: &AuditEvent) {
        let line = event.to_json_line();
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        if let Err(e) = self.append(&line) {
            eprintln!("audit: cannot write {}: {e}", self.path.display());
        }
        if self.echo_stdout {
            // A closed stdout must not take the service down (println! would panic).
            let _ = writeln!(std::io::stdout().lock(), "{line}");
        }
    }
}

/// The last `max_bytes` of a file as lines (a first line cut in half by the window is dropped).
/// Empty when the file is missing or unreadable.
fn tail_lines(path: &Path, max_bytes: u64) -> Vec<String> {
    let Ok(mut f) = fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max_bytes);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.take(max_bytes).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines
}

/// The newest audit lines, oldest first: the tail of the rotated file (if the live one is short)
/// followed by the tail of the live file.
pub fn read_recent_lines(path: &Path, max_bytes: u64) -> Vec<String> {
    let mut older = {
        let mut s = path.to_path_buf().into_os_string();
        s.push(".1");
        tail_lines(Path::new(&s), max_bytes)
    };
    older.extend(tail_lines(path, max_bytes));
    older
}

/// Append one event to the default audit log. The entrypoint and the egress monitor are not the
/// sync-service and have no sink of their own; they write to the same file it does.
pub fn record_default(event: &AuditEvent) {
    FileAuditSink::new(sdw_core::audit::AUDIT_LOG_PATH).record(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdw_core::audit::{event, outcome};

    fn ev(n: i64) -> AuditEvent {
        AuditEvent::new(n, event::LOGIN, outcome::FAILURE).with("ip", format!("10.0.0.{n}"))
    }

    #[test]
    fn events_are_appended_as_lines_with_private_modes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/audit.jsonl");
        let sink = FileAuditSink::new(&path);
        sink.record(&ev(1));
        sink.record(&ev(2));
        let lines = read_recent_lines(&path, 1 << 20);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("10.0.0.2"));
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
    }

    #[test]
    fn rotation_keeps_a_bounded_number_of_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        // ~100 bytes per line, rotate at 250: a few lines per file, three files kept.
        let sink = FileAuditSink::new(&path).with_limits(250, 3);
        for n in 0..40 {
            sink.record(&ev(n));
        }
        assert!(path.exists());
        assert!(dir.path().join("audit.jsonl.1").exists());
        assert!(dir.path().join("audit.jsonl.2").exists());
        assert!(!dir.path().join("audit.jsonl.3").exists());
        // The newest line is never lost to rotation.
        let last = read_recent_lines(&path, 1 << 20).pop().unwrap();
        assert!(last.contains("10.0.0.39"));
    }

    #[test]
    fn a_window_that_cuts_a_line_drops_the_partial_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let sink = FileAuditSink::new(&path);
        for n in 0..10 {
            sink.record(&ev(n));
        }
        let all = read_recent_lines(&path, 1 << 20);
        let tail = read_recent_lines(&path, 150);
        assert!(tail.len() < all.len());
        for l in &tail {
            assert!(
                sdw_core::audit::parse_line(l).is_some(),
                "whole lines only: {l}"
            );
        }
    }

    #[test]
    fn a_missing_file_reads_as_empty() {
        assert!(read_recent_lines(Path::new("/nonexistent/audit.jsonl"), 1024).is_empty());
    }
}
