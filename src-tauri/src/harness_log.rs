//! The per-run harness log: a frame-level JSONL record of what the shell
//! received from an adapter run, kept in the app's local data directory.
//!
//! The log exists so an operator can read the exact lines the product
//! parsed (an agent's `system/init`, `assistant`/`tool_use` and `result`
//! lines, or a PTY launch's text frames) after the run is over. It lives
//! at `<app local data dir>/harness-logs/<run id>.jsonl`, never under the
//! project: the outbox inventories its own run subdirectory and the
//! wrong-root scan flags stray files, and a log under the project would
//! land in both. The directory is mode `0700` and each log `0600` on Unix
//! (Windows keeps the default ACLs of the user's data directory).
//!
//! # Format
//!
//! One JSON object per line:
//!
//! - header, first: `{"kind":"header","schema":"omnifrons.harness-log/1",
//!   "runId":"run-...","adapter":"claude-code","startedAt":"<RFC 3339 UTC>"}`
//! - text frames: `{"kind":"stdout"|"stderr","text":"...","continued":false,
//!   "droppedBefore":0}`, the capture side's own fields, unmodified
//! - `{"kind":"truncated","framesDropped":<n>}`, only when the cap dropped
//!   text frames, immediately before the state line
//! - state, last: `{"kind":"state","state":"exited","code":0}`
//!
//! # Limits and privacy
//!
//! A log holds at most [`MAX_LOG_BYTES`]; past that, text frames are
//! counted instead of written, while the truncation marker and the final
//! state line are always written. Only the newest [`RETAINED_LOGS`] logs
//! are kept ([`prune`]). Nothing is redacted: the file is exactly what the
//! harness printed and is readable only by the user; whoever archives one
//! as evidence scrubs it first. The renderer never receives a path, only
//! the run id and [`HarnessLogDir::relative_name`].
//!
//! # Buffering
//!
//! Writes go through a buffered writer that is flushed at the state line
//! and after every [`FLUSH_EVERY_FRAMES`] text frames, so a crash loses at
//! most that many frames and the writer does not issue a syscall per line.
//! The header line is flushed when the log opens.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use omnifrons_app::{FramePayload, OutputFrame, OutputStream};
use omnifrons_domain::adapter::AdapterId;
use omnifrons_domain::outbox::RunId;
use serde::Serialize;

use crate::ipc::dto::ProcessTerminalStateDto;

/// The most bytes of text-frame lines one log holds.
pub const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;
/// How many logs are kept; older ones are deleted when a run starts.
pub const RETAINED_LOGS: usize = 64;
/// The header line's `schema` value.
pub const SCHEMA: &str = "omnifrons.harness-log/1";
/// The log directory's name under the app's local data directory.
pub const DIR_NAME: &str = "harness-logs";
/// Flush the buffered writer after this many text frames.
const FLUSH_EVERY_FRAMES: u32 = 256;
/// The writer's buffer: larger than 256 typical lines, so the frame
/// count, not the buffer filling, decides most flushes.
const BUFFER_BYTES: usize = 64 * 1024;
/// The most directory entries one [`prune`] scan looks at.
const MAX_SCANNED_ENTRIES: usize = 4096;

/// Why a log could not be created or written. Carries the I/O error kind
/// only: its text never names a path, so it is safe to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessLogError(io::ErrorKind);

impl std::fmt::Display for HarnessLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for HarnessLogError {}

impl From<io::Error> for HarnessLogError {
    fn from(error: io::Error) -> Self {
        Self(error.kind())
    }
}

/// The directory harness logs live in, opened once at startup.
#[derive(Debug, Clone)]
pub struct HarnessLogDir {
    path: PathBuf,
}

impl HarnessLogDir {
    /// Create `<base>/harness-logs` (mode `0700` on Unix) if missing.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessLogError`] if the directory cannot be created or
    /// its mode cannot be set.
    pub fn open(base: &Path) -> Result<Self, HarnessLogError> {
        let path = base.join(DIR_NAME);
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&path)?;
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
        Ok(Self { path })
    }

    /// The fixed, pathless name the renderer shows for a run's log.
    #[must_use]
    pub fn relative_name(run_id: &RunId) -> String {
        format!("{DIR_NAME}/{}.jsonl", run_id.as_str())
    }

    fn file_for(&self, run_id: &RunId) -> PathBuf {
        self.path.join(format!("{}.jsonl", run_id.as_str()))
    }
}

/// The managed state commands read: the log directory as opened at
/// startup, or why it could not be. A failed open never stops the app; each
/// run then reports the log as unavailable and goes on without one.
#[derive(Debug)]
pub struct HarnessLogs(pub Result<HarnessLogDir, HarnessLogError>);

/// What one [`prune`] pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PruneReport {
    /// Log-named entries seen.
    pub scanned: usize,
    /// Old logs deleted.
    pub deleted: usize,
    /// Old logs that could not be deleted.
    pub failed: usize,
}

/// Delete the oldest logs beyond `keep`. Names sort chronologically
/// (`run-<seconds>-<nanoseconds>-<sequence>`), so the order is by name.
/// Only `run-*.jsonl` entries are considered and at most
/// [`MAX_SCANNED_ENTRIES`] are read. A delete that fails is counted and
/// logged by error kind at debug level, never returned as an error.
pub fn prune(dir: &HarnessLogDir, keep: usize) -> PruneReport {
    let Ok(entries) = std::fs::read_dir(&dir.path) else {
        return PruneReport::default();
    };
    let mut names: Vec<std::ffi::OsString> = entries
        .take(MAX_SCANNED_ENTRIES)
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| {
            name.to_str().is_some_and(|name| {
                name.starts_with("run-")
                    && Path::new(name)
                        .extension()
                        .is_some_and(|ext| ext == "jsonl")
            })
        })
        .collect();
    names.sort();
    let mut report = PruneReport {
        scanned: names.len(),
        ..PruneReport::default()
    };
    let excess = names.len().saturating_sub(keep);
    for name in names.into_iter().take(excess) {
        match std::fs::remove_file(dir.path.join(&name)) {
            Ok(()) => report.deleted += 1,
            Err(error) => {
                report.failed += 1;
                tracing::debug!(kind = ?error.kind(), "a harness log could not be pruned");
            }
        }
    }
    report
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HeaderLine<'a> {
    kind: &'static str,
    schema: &'static str,
    run_id: &'a str,
    adapter: &'a str,
    started_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextLine<'a> {
    kind: &'static str,
    text: &'a str,
    continued: bool,
    dropped_before: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TruncatedLine {
    kind: &'static str,
    frames_dropped: u64,
}

#[derive(Serialize)]
struct StateLine<'a> {
    kind: &'static str,
    #[serde(flatten)]
    terminal: &'a ProcessTerminalStateDto,
}

/// One run's open log.
pub struct HarnessLog {
    writer: BufWriter<File>,
    written: u64,
    cap: u64,
    frames_dropped: u64,
    frames_since_flush: u32,
}

impl HarnessLog {
    /// Create `<dir>/<run id>.jsonl` (exclusive, mode `0600` on Unix) and
    /// write the header line.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessLogError`] if the file exists or cannot be created
    /// or written.
    pub fn open(
        dir: &HarnessLogDir,
        run_id: &RunId,
        adapter: &AdapterId,
    ) -> Result<Self, HarnessLogError> {
        Self::open_with(dir, run_id, adapter, SystemTime::now(), MAX_LOG_BYTES)
    }

    fn open_with(
        dir: &HarnessLogDir,
        run_id: &RunId,
        adapter: &AdapterId,
        now: SystemTime,
        cap: u64,
    ) -> Result<Self, HarnessLogError> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(dir.file_for(run_id))?;
        let mut log = Self {
            writer: BufWriter::with_capacity(BUFFER_BYTES, file),
            written: 0,
            cap,
            frames_dropped: 0,
            frames_since_flush: 0,
        };
        let header = HeaderLine {
            kind: "header",
            schema: SCHEMA,
            run_id: run_id.as_str(),
            adapter: adapter.as_str(),
            started_at: rfc3339_utc(now),
        };
        log.write_line(&header)?;
        // The header reaches the file at once: a log that exists is never
        // empty, even for a run that is still starting.
        log.writer.flush()?;
        Ok(log)
    }

    /// Write one text frame, or count it as dropped once the cap would be
    /// exceeded. A `State` payload is ignored here: use
    /// [`Self::write_state`], which also adds the truncation marker.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessLogError`] if the write or a periodic flush fails.
    pub fn write_frame(&mut self, frame: &OutputFrame) -> Result<(), HarnessLogError> {
        let FramePayload::Text {
            stream,
            text,
            continued,
        } = &frame.payload
        else {
            return Ok(());
        };
        let line = TextLine {
            kind: match stream {
                OutputStream::Stdout => "stdout",
                OutputStream::Stderr => "stderr",
            },
            text,
            continued: *continued,
            dropped_before: frame.dropped_before,
        };
        let mut bytes =
            serde_json::to_vec(&line).map_err(|_| HarnessLogError(io::ErrorKind::InvalidData))?;
        bytes.push(b'\n');
        if self.frames_dropped > 0 || self.written + bytes.len() as u64 > self.cap {
            self.frames_dropped += 1;
            return Ok(());
        }
        self.writer.write_all(&bytes)?;
        self.written += bytes.len() as u64;
        self.frames_since_flush += 1;
        if self.frames_since_flush >= FLUSH_EVERY_FRAMES {
            self.frames_since_flush = 0;
            self.writer.flush()?;
        }
        Ok(())
    }

    /// Write the final state line, preceded by the truncation marker when
    /// the cap dropped frames, and flush. Neither line counts against the
    /// cap: they are always written.
    ///
    /// # Errors
    ///
    /// Returns [`HarnessLogError`] if the write or the flush fails.
    pub fn write_state(&mut self, state: &ProcessTerminalStateDto) -> Result<(), HarnessLogError> {
        if self.frames_dropped > 0 {
            let marker = TruncatedLine {
                kind: "truncated",
                frames_dropped: self.frames_dropped,
            };
            self.write_line(&marker)?;
        }
        self.write_line(&StateLine {
            kind: "state",
            terminal: state,
        })?;
        self.writer.flush()?;
        Ok(())
    }

    fn write_line(&mut self, line: &impl Serialize) -> Result<(), HarnessLogError> {
        let mut bytes =
            serde_json::to_vec(line).map_err(|_| HarnessLogError(io::ErrorKind::InvalidData))?;
        bytes.push(b'\n');
        self.writer.write_all(&bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }
}

/// `time` as `YYYY-MM-DDTHH:MM:SSZ` (UTC, whole seconds), computed from
/// the Unix epoch with the civil-from-days algorithm so the shell needs no
/// date crate for one header field.
fn rfc3339_utc(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::dto::ProcessTerminalStateTag;
    use omnifrons_app::{FramePayload, OutputStream};
    use std::time::Duration;

    struct Temp(PathBuf);

    impl Temp {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-shell-harness-log-{}-{label}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("fixture dir");
            Self(dir)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(token: &str) -> RunId {
        RunId::new(token).expect("valid run id")
    }

    fn text(
        seq: u64,
        stream: OutputStream,
        text: &str,
        continued: bool,
        dropped: u64,
    ) -> OutputFrame {
        OutputFrame::with_dropped_before(
            seq,
            dropped,
            FramePayload::Text {
                stream,
                text: text.to_string(),
                continued,
            },
        )
    }

    fn exited() -> ProcessTerminalStateDto {
        ProcessTerminalStateDto {
            state: ProcessTerminalStateTag::Exited,
            code: Some(0),
        }
    }

    fn lines_of(dir: &HarnessLogDir, id: &str) -> Vec<serde_json::Value> {
        std::fs::read_to_string(dir.path.join(format!("{id}.jsonl")))
            .expect("log readable")
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line is JSON"))
            .collect()
    }

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn the_first_line_is_the_header() {
        let temp = Temp::new("header");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        let adapter = AdapterId::claude_code();
        let mut log = HarnessLog::open_with(
            &dir,
            &run("run-1-000000000-0"),
            &adapter,
            at(1_700_000_000),
            MAX_LOG_BYTES,
        )
        .expect("open");
        log.write_state(&exited()).expect("state");
        let lines = lines_of(&dir, "run-1-000000000-0");
        assert_eq!(
            lines[0],
            serde_json::json!({
                "kind": "header",
                "schema": "omnifrons.harness-log/1",
                "runId": "run-1-000000000-0",
                "adapter": "claude-code",
                "startedAt": "2023-11-14T22:13:20Z",
            })
        );
    }

    #[test]
    fn text_frames_keep_stream_text_continued_and_dropped_before() {
        let temp = Temp::new("frames");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        let mut log =
            HarnessLog::open(&dir, &run("run-2-000000000-0"), &AdapterId::pty_cli()).expect("open");
        log.write_frame(&text(
            0,
            OutputStream::Stdout,
            "{\"type\":\"system\"}",
            false,
            0,
        ))
        .expect("stdout");
        log.write_frame(&text(1, OutputStream::Stderr, "warn \"x\"\u{1b}", true, 3))
            .expect("stderr");
        log.write_state(&exited()).expect("state");
        let lines = lines_of(&dir, "run-2-000000000-0");
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[1],
            serde_json::json!({"kind":"stdout","text":"{\"type\":\"system\"}","continued":false,"droppedBefore":0})
        );
        assert_eq!(
            lines[2],
            serde_json::json!({"kind":"stderr","text":"warn \"x\"\u{1b}","continued":true,"droppedBefore":3})
        );
        assert_eq!(
            lines[3],
            serde_json::json!({"kind":"state","state":"exited","code":0})
        );
    }

    #[test]
    fn past_the_cap_text_is_dropped_and_a_marker_precedes_the_state_line() {
        let temp = Temp::new("cap");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        let mut log = HarnessLog::open_with(
            &dir,
            &run("run-3-000000000-0"),
            &AdapterId::claude_code(),
            at(0),
            400,
        )
        .expect("open");
        for seq in 0..10 {
            log.write_frame(&text(seq, OutputStream::Stdout, &"y".repeat(60), false, 0))
                .expect("frame");
        }
        log.write_state(&exited()).expect("state");
        let lines = lines_of(&dir, "run-3-000000000-0");
        let kept = lines.iter().filter(|line| line["kind"] == "stdout").count();
        assert!(
            kept > 0 && kept < 10,
            "some frames kept, some dropped: {kept}"
        );
        let n = lines.len();
        assert_eq!(
            lines[n - 2],
            serde_json::json!({"kind":"truncated","framesDropped": 10 - kept})
        );
        assert_eq!(lines[n - 1]["kind"], "state");
    }

    #[test]
    fn no_marker_is_written_when_nothing_was_dropped() {
        let temp = Temp::new("nomarker");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        let mut log = HarnessLog::open(&dir, &run("run-4-000000000-0"), &AdapterId::claude_code())
            .expect("open");
        log.write_state(&exited()).expect("state");
        let lines = lines_of(&dir, "run-4-000000000-0");
        assert!(lines.iter().all(|line| line["kind"] != "truncated"));
    }

    #[test]
    fn a_log_is_created_exclusively() {
        let temp = Temp::new("exclusive");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        let adapter = AdapterId::claude_code();
        let _first = HarnessLog::open(&dir, &run("run-5-000000000-0"), &adapter).expect("first");
        assert!(HarnessLog::open(&dir, &run("run-5-000000000-0"), &adapter).is_err());
    }

    #[test]
    fn buffered_frames_reach_the_file_every_256_frames() {
        let temp = Temp::new("flush");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        let mut log = HarnessLog::open(&dir, &run("run-6-000000000-0"), &AdapterId::claude_code())
            .expect("open");
        let path = dir.path.join("run-6-000000000-0.jsonl");
        let len = || std::fs::metadata(&path).expect("meta").len();
        let header_len = len();
        assert!(header_len > 0, "the header is flushed at open");
        for seq in 0..255 {
            log.write_frame(&text(seq, OutputStream::Stdout, "a", false, 0))
                .expect("frame");
        }
        assert_eq!(len(), header_len, "still buffered");
        log.write_frame(&text(255, OutputStream::Stdout, "a", false, 0))
            .expect("frame");
        assert!(len() > header_len, "flushed at 256");
    }

    #[test]
    fn prune_deletes_the_oldest_names_beyond_keep() {
        let temp = Temp::new("prune");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        for name in [
            "run-30-000000000-0",
            "run-10-000000000-1",
            "run-20-000000000-0",
            "run-40-000000000-2",
        ] {
            std::fs::write(dir.path.join(format!("{name}.jsonl")), b"{}\n").expect("seed");
        }
        std::fs::write(dir.path.join("notes.txt"), b"keep me").expect("foreign");
        let report = prune(&dir, 2);
        assert_eq!(
            report,
            PruneReport {
                scanned: 4,
                deleted: 2,
                failed: 0
            }
        );
        assert!(!dir.path.join("run-10-000000000-1.jsonl").exists());
        assert!(!dir.path.join("run-20-000000000-0.jsonl").exists());
        assert!(dir.path.join("run-30-000000000-0.jsonl").exists());
        assert!(dir.path.join("run-40-000000000-2.jsonl").exists());
        assert!(
            dir.path.join("notes.txt").exists(),
            "foreign files are never touched"
        );
    }

    #[test]
    fn prune_counts_a_failed_delete_and_goes_on() {
        let temp = Temp::new("prunefail");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        // A directory with a log name cannot be removed by `remove_file`.
        std::fs::create_dir(dir.path.join("run-1-000000000-0.jsonl")).expect("stuck entry");
        std::fs::write(dir.path.join("run-2-000000000-0.jsonl"), b"{}\n").expect("seed");
        std::fs::write(dir.path.join("run-3-000000000-0.jsonl"), b"{}\n").expect("seed");
        let report = prune(&dir, 1);
        assert_eq!(
            report,
            PruneReport {
                scanned: 3,
                deleted: 1,
                failed: 1
            }
        );
        assert!(!dir.path.join("run-2-000000000-0.jsonl").exists());
        assert!(dir.path.join("run-3-000000000-0.jsonl").exists());
    }

    #[test]
    fn the_relative_name_is_fixed_and_pathless() {
        assert_eq!(
            HarnessLogDir::relative_name(&run("run-7-000000000-0")),
            "harness-logs/run-7-000000000-0.jsonl"
        );
    }

    #[test]
    fn timestamps_are_rfc_3339_utc() {
        assert_eq!(rfc3339_utc(at(0)), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(at(1_700_000_000)), "2023-11-14T22:13:20Z");
        assert_eq!(rfc3339_utc(at(951_782_400)), "2000-02-29T00:00:00Z");
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_is_0700_and_a_log_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let temp = Temp::new("modes");
        let dir = HarnessLogDir::open(&temp.0).expect("dir");
        let _log = HarnessLog::open(&dir, &run("run-8-000000000-0"), &AdapterId::claude_code())
            .expect("open");
        let mode =
            |path: &Path| std::fs::metadata(path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path), 0o700);
        assert_eq!(mode(&dir.path.join("run-8-000000000-0.jsonl")), 0o600);
    }
}
