//! Per-camera logging and filesystem helpers.
//!
//! Every `GStreamerBridge` gets its own log file under the configured
//! log directory. This makes it trivial to investigate a camera that
//! fails to produce frames: open `logs/<label>.log` and read the
//! GStreamer bus messages, state transitions, and metrics from day one.
//!
//! ### Weekly rotation
//!
//! Each logger tracks the *date* of the day it last wrote to. When the
//! date changes, the current file is renamed to
//! `<dir>/<label>-YYYY-MM-DD.log` and a fresh `current.log` is opened.
//! After every rotation the logger prunes rotated files older than
//! `retention_days` (default 7 — one week) from the log directory.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

/// Command sent from the (UI) callers to the logger's writer thread.
enum LogCmd {
    /// A fully formatted line (timestamp + label already applied).
    Line(String),
    /// Flush now and acknowledge on the channel so the caller can wait.
    Flush(SyncSender<()>),
    #[cfg(test)]
    /// Test hook: overwrite the tracked calendar day so the next `Line`
    /// triggers rotation without touching the system clock.
    Backdate(String),
}

/// A per-camera logger that appends timestamped lines to a file.
///
/// Writes happen on a **dedicated background thread**, never on the caller.
/// Callers `try_send` into a bounded channel and drop the line if the disk
/// can't keep up — logging must never stall the frame tick (this used to
/// `flush()` a file per GStreamer bus message on the UI thread and froze the
/// whole grid).
pub struct CameraLogger {
    tx: Option<SyncSender<LogCmd>>,
    label: String,
    handle: Option<JoinHandle<()>>,
}

struct LoggerInner {
    writer: BufWriter<File>,
    /// The absolute path of the current log file. Stored alongside the
    /// writer so `rotate_inner` can rename it after flushing.
    current_path: PathBuf,
    /// The `YYYY-MM-DD` string the last line was written on.
    /// `None` = the logger was just created and has not written yet.
    current_date: Option<String>,
    log_dir: PathBuf,
    label: String,
    retention_days: u32,
}

impl CameraLogger {
    /// Create a new logger at `path`. The parent directory must already
    /// exist — the caller is responsible for `ensure_log_dir`.
    ///
    /// `retention_days` controls how many days of rotated logs are kept
    /// before pruning (default 7).
    pub fn new(path: PathBuf, label: &str, retention_days: u32) -> Result<Self, String> {
        let log_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("cannot open log file {}: {e}", path.display()))?;

        let mut inner = LoggerInner {
            writer: BufWriter::new(file),
            current_path: path,
            current_date: None,
            log_dir,
            label: label.to_string(),
            retention_days: retention_days.max(1),
        };

        // Bounded: a slow disk drops log lines instead of back-pressuring the
        // UI thread.
        let (tx, rx) = sync_channel::<LogCmd>(4096);
        let handle = std::thread::Builder::new()
            .name(format!("camlog-{label}"))
            .spawn(move || Self::writer_loop(&mut inner, rx))
            .map_err(|e| format!("cannot spawn logger thread: {e}"))?;

        Ok(Self {
            tx: Some(tx),
            label: label.to_string(),
            handle: Some(handle),
        })
    }

    /// The background thread: drain commands, batch writes, flush at most
    /// once a second (and on demand / shutdown).
    fn writer_loop(inner: &mut LoggerInner, rx: Receiver<LogCmd>) {
        let mut dirty = false;
        loop {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(LogCmd::Line(line)) => {
                    Self::write_one(inner, &line);
                    dirty = true;
                }
                Ok(LogCmd::Flush(ack)) => {
                    let _ = inner.writer.flush();
                    dirty = false;
                    let _ = ack.send(());
                }
                #[cfg(test)]
                Ok(LogCmd::Backdate(d)) => {
                    inner.current_date = Some(d);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if dirty {
                        let _ = inner.writer.flush();
                        dirty = false;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = inner.writer.flush();
                    return;
                }
            }
        }
    }

    /// Rotate if the calendar day changed, then append `line`. Runs on the
    /// writer thread only.
    fn write_one(inner: &mut LoggerInner, line: &str) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        match &inner.current_date {
            None => inner.current_date = Some(today),
            Some(prev) if prev != &today => {
                let log_dir = inner.log_dir.clone();
                let label = inner.label.clone();
                let retention_days = inner.retention_days;
                if let Err(e) = Self::rotate_inner(inner, &today, &log_dir, &label, retention_days)
                {
                    eprintln!("camera logger rotation failed: {e}");
                }
            }
            _ => {}
        }
        if let Err(e) = inner.writer.write_all(line.as_bytes()) {
            eprintln!("camera logger write failed: {e}");
        }
    }

    /// Queue a line for the writer thread. Timestamp + label are applied
    /// here (caller thread) so they reflect when the event happened, not
    /// when the disk caught up. Non-blocking: drops the line if the queue
    /// is full.
    pub fn write_line(&self, msg: &str) {
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f");
        let line = format!("[{}] [{}] {}\n", now, self.label, msg);
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(LogCmd::Line(line));
        }
    }

    /// Convenience for severity levels.
    pub fn error(&self, msg: &str) {
        self.write_line(&format!("ERROR: {msg}"));
    }

    pub fn warn(&self, msg: &str) {
        self.write_line(&format!("WARN:  {msg}"));
    }

    pub fn info(&self, msg: &str) {
        self.write_line(&format!("INFO:  {msg}"));
    }

    pub fn debug(&self, msg: &str) {
        self.write_line(&format!("DEBUG: {msg}"));
    }

    /// Block until everything queued so far is on disk. Called at shutdown
    /// and by tests — never on the frame tick.
    pub fn flush(&self) {
        let Some(tx) = &self.tx else { return };
        let (ack_tx, ack_rx) = sync_channel::<()>(1);
        if tx.send(LogCmd::Flush(ack_tx)).is_ok() {
            let _ = ack_rx.recv_timeout(Duration::from_secs(2));
        }
    }

    #[cfg(test)]
    pub(crate) fn test_backdate(&self, date: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(LogCmd::Backdate(date.to_string()));
        }
        self.flush();
    }

    /// Rename the current log file to `<label>-<today>.log`, open a fresh
    /// one, update the tracked date, and prune rotated files older than
    /// `retention_days`.
    fn rotate_inner(
        guard: &mut LoggerInner,
        today: &str,
        log_dir: &Path,
        label: &str,
        retention_days: u32,
    ) -> std::io::Result<()> {
        // Flush so the rename captures everything written so far.
        guard.writer.flush()?;

        let current_path = guard.current_path.clone();
        let safe = safe_filename(label);

        // The rotated file name mirrors the current file stem, e.g.
        // `rio_cubatão_ponte_quiriri-2026-08-31.log`.
        let rotated_name = format!("{}-{}.log", safe, today);
        let rotated_path = log_dir.join(&rotated_name);

        if current_path.exists() {
            let _ = std::fs::remove_file(&rotated_path);
            std::fs::rename(&current_path, &rotated_path)?;
        }

        // Open a fresh append file at the original path.
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current_path)?;
        guard.writer = BufWriter::new(file);
        guard.current_path = current_path;
        guard.current_date = Some(today.to_string());

        // Prune rotated files older than retention_days, excluding the
        // one we just wrote (its date may legitimately be older than the
        // cutoff when tests backdate the clock — in production it is
        // always today).
        Self::prune(log_dir, retention_days, today, &rotated_name);

        Ok(())
    }

    /// Remove rotated log files (`<stem>-YYYY-MM-DD.log`) whose date is
    /// older than `retention_days` days before `today`, skipping
    /// `just_rotated` (the file we just wrote — it may be older than the
    /// cutoff when tests backdate the clock; in production it is always
    /// today so this guard has no effect).
    ///
    /// The rotated filename pattern is `<safe>-YYYY-MM-DD.log` where
    /// `<safe>` may itself contain dashes (e.g. `rio_cubatão_ponte_quiriri`
    /// → `rio_cubatão_ponte_quiriri-2026-08-31.log`). We therefore look
    /// for the date at the *end* of the stem, not just after the last
    /// dash.
    fn prune(log_dir: &Path, retention_days: u32, today: &str, just_rotated: &str) {
        let today_date = match chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => return,
        };
        let cutoff = today_date - chrono::Duration::days(retention_days as i64);

        let entries = match std::fs::read_dir(log_dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy().to_string();
            if name == just_rotated {
                continue;
            }
            // Only touch rotated files: the stem must end in `-YYYY-MM-DD`.
            // Parse the trailing date by splitting on '-' from the right so a
            // non-ASCII byte in the stem can't land mid-slice (a byte-offset
            // `stem.get(len-10..)` returns `None` on a char boundary miss and
            // the file is never pruned).
            let stem = match name.strip_suffix(".log") {
                Some(s) => s,
                None => continue,
            };
            let mut parts = stem.rsplitn(4, '-');
            let (Some(dd), Some(mm), Some(yyyy)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let is_date = yyyy.len() == 4
                && mm.len() == 2
                && dd.len() == 2
                && yyyy
                    .bytes()
                    .chain(mm.bytes())
                    .chain(dd.bytes())
                    .all(|b| b.is_ascii_digit());
            if !is_date {
                continue;
            }
            let date_str = format!("{yyyy}-{mm}-{dd}");
            let Ok(file_date) = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") else {
                continue;
            };
            if file_date < cutoff {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

impl Drop for CameraLogger {
    fn drop(&mut self) {
        // Drop the sender so the writer thread's `recv` disconnects and it
        // flushes + exits, then join it so buffered lines reach disk before
        // the process ends.
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Ensure a directory exists, creating it (and parents) if necessary.
///
/// Returns the expanded path.
pub fn ensure_dir(dir: &Path) -> Result<PathBuf, String> {
    let dir = expand_tilde(dir);
    if dir.exists() {
        if !dir.is_dir() {
            return Err(format!("{} exists and is not a directory", dir.display()));
        }
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Ensure the recording directory exists. Same as `ensure_dir` but
/// named for the recording use case (existing call sites rely on it).
pub fn ensure_recording_dir(dir: &Path) -> Result<PathBuf, String> {
    ensure_dir(dir)
}

/// Ensure the log directory exists, creating it if necessary.
///
/// Returns the expanded path.
pub fn ensure_log_dir(dir: &Path) -> Result<PathBuf, String> {
    let dir = expand_tilde(dir);
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// Build a safe filename from a camera label: lowercase, replace
/// spaces and most punctuation with underscores, strip anything that
/// isn't alphanumeric/underscore/dash/dot.
pub fn safe_filename(label: &str) -> String {
    let cleaned: String = label
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    // Collapse consecutive underscores and trim leading/trailing ones.
    let mut collapsed = String::with_capacity(cleaned.len());
    let mut last_was_underscore = false;
    for c in cleaned.chars() {
        if c == '_' {
            if !last_was_underscore {
                collapsed.push('_');
                last_was_underscore = true;
            }
        } else {
            collapsed.push(c);
            last_was_underscore = false;
        }
    }
    collapsed.trim_matches('_').to_string()
}

/// Build the full log path for a camera given the log directory and label.
pub fn log_path(log_dir: &std::path::Path, label: &str) -> PathBuf {
    log_dir.join(format!("{}.log", safe_filename(label)))
}

fn expand_tilde(path: &std::path::Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rrv-logger-test-{}-{}",
            std::process::id(),
            test_name
        ))
    }

    #[test]
    fn logger_writes_timestamped_lines() {
        let dir = tmp_dir("timestamped");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = log_path(&dir, "Test Camera");
        let logger = CameraLogger::new(path.clone(), "Test Camera", 7).unwrap();
        logger.info("hello");
        logger.warn("be careful");
        logger.error("boom");
        logger.flush();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("INFO:  hello"));
        assert!(content.contains("WARN:  be careful"));
        assert!(content.contains("ERROR: boom"));
        assert!(content.contains("Test Camera"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_filename_replaces_spaces_and_special_chars() {
        assert_eq!(safe_filename("Centro: Avenida JK"), "centro_avenida_jk");
        assert_eq!(safe_filename("Rio Águas Vermelhas"), "rio_águas_vermelhas");
        assert_eq!(safe_filename("simple-name.log"), "simple-name.log");
        assert_eq!(safe_filename("a/b\\c"), "a_b_c");
    }

    #[test]
    fn log_path_uses_extension() {
        let p = log_path(std::path::Path::new("/tmp"), "Cam 1");
        assert_eq!(p.extension().unwrap(), "log");
        assert!(p.file_name().unwrap().to_string_lossy().contains("cam_1"));
    }

    #[test]
    fn ensure_dir_creates_missing_dir() {
        let dir = std::env::temp_dir().join(format!("rrv-rec-paths-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!dir.exists());
        ensure_dir(&dir).unwrap();
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_dir_rejects_existing_file() {
        let path =
            std::env::temp_dir().join(format!("rrv-rec-paths-test-file-{}", std::process::id()));
        std::fs::write(&path, b"x").unwrap();
        let err = ensure_dir(&path).unwrap_err();
        assert!(err.contains("not a directory"));
        let _ = std::fs::remove_file(&path);
    }

    /// Writing on the same day must NOT rotate — only one file exists
    /// and it contains every line.
    #[test]
    fn same_day_writes_do_not_rotate() {
        let dir = tmp_dir("sameday");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let safe = safe_filename("CamRot");
        let path = dir.join(format!("{safe}.log"));
        let logger = CameraLogger::new(path.clone(), "CamRot", 7).unwrap();
        logger.info("day-one-line");
        logger.flush();

        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1, "only the current file should exist");
        assert_eq!(
            entries[0].file_name(),
            std::ffi::OsString::from(format!("{safe}.log"))
        );

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("day-one-line"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// Writing on a different calendar day renames the current file to
    /// `<stem>-YYYY-MM-DD.log` and opens a fresh one. We can't advance
    /// the system clock, so we lock the inner mutex, flip the tracked
    /// date, and write again — this is exactly the path `write_line`
    /// takes when midnight passes. The rotated file uses *today's* real
    /// date (because `chrono::Local::now()` is the source of truth in
    /// `write_line`), so the test verifies the rename happened without
    /// asserting on the exact rotated filename.
    #[test]
    fn day_change_triggers_rotation() {
        let dir = tmp_dir("rotate");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let safe = safe_filename("CamRot2");
        let path = dir.join(format!("{safe}.log"));
        let logger = CameraLogger::new(path.clone(), "CamRot2", 7).unwrap();

        // First write: seeds current_date = today.
        logger.info("day-one");
        logger.flush();

        // Lock the inner state, backdate the tracked day, drop the lock.
        logger.test_backdate("2000-01-01");

        // Second write: date differs → rotation runs.
        logger.info("day-two");
        logger.flush();

        // The original path should now hold only the second write.
        let current = fs::read_to_string(&path).unwrap();
        assert!(
            current.contains("day-two"),
            "current file holds post-rotation writes"
        );
        assert!(
            !current.contains("day-one"),
            "pre-rotation content was moved away"
        );

        // A rotated file should exist with today's date stamp.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let rotated = dir.join(format!("{safe}-{today}.log"));
        assert!(rotated.exists(), "rotated file should exist at {rotated:?}");
        let rotated_content = fs::read_to_string(&rotated).unwrap();
        assert!(
            rotated_content.contains("day-one"),
            "rotated file holds pre-rotation content"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// Rotation prunes files older than `retention_days`. We exercise
    /// `rotate_inner` directly with a prunable setup: create a stale
    /// rotated file, then force a rotation and confirm the stale one is
    /// removed and the fresh one kept.
    #[test]
    fn rotation_prunes_old_rotated_files() {
        let dir = tmp_dir("prune");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let safe = safe_filename("CamPrune");
        let path = dir.join(format!("{safe}.log"));
        let logger = CameraLogger::new(path.clone(), "CamPrune", 2).unwrap();

        // Write a seed line so the current file has content.
        logger.info("seed");
        logger.flush();

        // Create a fake 10-day-old rotated file that should be pruned.
        let old_date =
            chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap() - chrono::Duration::days(10);
        let old_name = format!("{}-{}.log", safe, old_date.format("%Y-%m-%d"));
        let old_path = dir.join(&old_name);
        std::fs::write(&old_path, b"stale").unwrap();

        // Force a rotation by backdating the tracked date.
        logger.test_backdate("2000-01-01");
        logger.info("after-rotation");
        logger.flush();

        assert!(
            !old_path.exists(),
            "stale rotated file should have been pruned"
        );

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let fresh = dir.join(format!("{safe}-{today}.log"));
        assert!(fresh.exists(), "fresh rotated file should be kept");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_prunes_old_files_with_accented_label() {
        // A label whose `safe_filename` keeps multibyte chars used to defeat
        // the byte-offset date parse, so old logs were never pruned.
        let dir = tmp_dir("prune-utf8");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let safe = safe_filename("Rio Águas Vermelhas");
        assert!(
            safe.contains('á'),
            "test relies on a multibyte stem: {safe}"
        );
        let path = dir.join(format!("{safe}.log"));
        let logger = CameraLogger::new(path.clone(), "Rio Águas Vermelhas", 2).unwrap();
        logger.info("seed");
        logger.flush();

        let old_date =
            chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap() - chrono::Duration::days(10);
        let old_name = format!("{}-{}.log", safe, old_date.format("%Y-%m-%d"));
        let old_path = dir.join(&old_name);
        std::fs::write(&old_path, b"stale").unwrap();

        logger.test_backdate("2000-01-01");
        logger.info("after-rotation");
        logger.flush();

        assert!(
            !old_path.exists(),
            "stale rotated file should have been pruned"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
