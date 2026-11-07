//! Output capture: pipes a spawned child's stdout/stderr (or reads its
//! pseudo-terminal master), frames the bytes into [`OutputFrame`]s bounded
//! by [`MAX_LINE_BYTES`], and delivers them through a bounded per-child
//! channel with best-effort backpressure for text (`try_send` only -- a
//! full channel drops the newest frame and counts it, never blocks the
//! reader). The one exception is the end of the run ([`try_finalize`]):
//! its diagnostics and the final `State` frame are guaranteed-delivered via
//! a dedicated blocking-send thread rather than dropped alongside text when
//! the channel is full, since the `State` frame is the only signal a
//! subscriber has that the run is truly done.
//!
//! Line-framing contract ([`drain_stream`]): a line ends at a bare `\n` or
//! at `\r\n` (that `\r` is stripped, never delivered as content); a final
//! line with no trailing newline at all is still delivered at EOF, exactly
//! as read (a trailing bare `\r` there is content). A line longer than
//! [`MAX_LINE_BYTES`] is delivered as consecutive frames split at a UTF-8
//! character boundary, every frame but the last flagged `continued`; a
//! line of exactly [`MAX_LINE_BYTES`] is one un-continued frame whichever
//! terminator follows it (bare `\n`, `\r\n`, or EOF) and however its bytes
//! fall across reads -- including a `\r` and its `\n` arriving in
//! different reads. Only the *next* byte after a full buffer, never the
//! buffer's length, decides whether a split happens.
//!
//! ## When the final `State` frame is sent
//!
//! Deliberately independent of this crate's own `Tracked` bookkeeping
//! (`docs/spike-log.md` § IPC contract): `Tracked`'s Running/Terminal
//! transition is the `ProcessSupervisor` port's own contract and must keep
//! reporting a confirmed reap immediately, regardless of whether stdout/
//! stderr have finished draining. The final `State` frame this module
//! sends therefore waits for *both* the confirmed terminal state
//! ([`record_final_state`], from `observe`, `stop`, or the self-exit
//! reaper) and every reader task finishing ([`drain_stream`]'s own
//! bookkeeping) -- whichever happens second performs the send, so the state
//! frame is never emitted ahead of output that was already read but not yet
//! delivered.
//!
//! One case bounds that wait. A descendant that inherited the child's
//! output (`sh -c 'sleep 3600 & exit 0'`) holds it open after the child has
//! exited, and its reader then never ends. The self-exit reaper (which polls
//! for the child's exit from spawn, not from the readers' end) gives the
//! readers a grace of 2 s from that exit; if one is still running after it,
//! the reaper *abandons* them: the run ends with `orphan-risk/uncertain`,
//! preceded by a diagnostic `stderr` frame saying a descendant still holds
//! the output open. The same grace applies when `stop` reaped the child.
//! From then on the `State` frame stays the last frame on the channel: the
//! readers' sending slot is closed under the same lock that assigns the
//! `State` frame's `seq` ([`ReaderHandles`]), so whatever a lingering
//! reader reads afterwards is discarded, never delivered. Each such reader
//! counts what it discarded and logs that count once, in a `tracing`
//! warning, when it finally ends. Closing the slot also drops the readers'
//! last sender, so a subscriber's iterator ends after the `State` frame even
//! while a reader lingers; the descendant itself is left running.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use omnifrons_app::{FramePayload, OutputFrame, OutputStream, ProcessId, ProcessTerminalState};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

/// The maximum text bytes one frame carries. A line longer than this is
/// split into consecutive frames instead of growing one frame unbounded,
/// every frame but the last of the split flagged `continued: true`
/// ([`drain_stream`]).
///
/// Re-exports `omnifrons_domain::output::MAX_TEXT_FRAME_BYTES` rather than
/// redefining the number here: it is one shared framing contract with
/// `omnifrons-app`'s `harness_adapter::LineAssembler` (which bounds its
/// own reassembly buffer in terms of it), so the two crates must never be
/// able to drift onto different values (`docs/spike-log.md` § Slice 3).
/// The value is a size bound only -- a consumer never infers continuation
/// from a frame's length, only from its `continued` flag.
pub(crate) const MAX_LINE_BYTES: usize = omnifrons_domain::output::MAX_TEXT_FRAME_BYTES;

/// The per-child delivery channel's bound.
const CHANNEL_CAPACITY: usize = 1024;

/// The shared map of every spawned child's output-capture bookkeeping.
pub(crate) type OutputTable = Arc<Mutex<HashMap<ProcessId, OutputChannelState>>>;

/// The readers' sending slot: `Some` until [`try_finalize`] closes it.
type Gate = Arc<Mutex<Option<SyncSender<OutputFrame>>>>;

/// Per-child output-capture bookkeeping, independent of this crate's own
/// `Tracked` entry for the same id.
///
/// Removed from the shared [`OutputTable`] entirely once *both* finalized
/// (`sender` is `None`: every frame this process will ever produce,
/// including the terminal one, has been handed off) and subscribed-out
/// (`receiver` is `None`: a caller already took it via `subscribe`, and
/// from then on owns it independently of this table) -- see
/// [`try_finalize`] and [`take_receiver`]. An entry nobody ever subscribes
/// to is deliberately retained forever instead (so a late `subscribe`
/// still finds an already-terminal process's buffered output, per this
/// module's own doc comment); that is a known, narrower residual growth
/// path than the one this cleanup closes, bounded in practice by
/// `TokioProcessSupervisor`'s own running/retained-terminal caps on how
/// many processes can exist at all.
pub(crate) struct OutputChannelState {
    /// Taken by `ProcessOutput::subscribe`, exactly once.
    receiver: Option<Receiver<OutputFrame>>,
    /// The sender the final frames go out on. `None` once they have been
    /// handed to the delivery thread (or, on a platform where reap is never
    /// confirmed, forever `Some` -- see this module's own doc comment):
    /// dropping every sender is what lets a subscriber's `Receiver`
    /// iterator end.
    sender: Option<SyncSender<OutputFrame>>,
    /// The readers' own sender, closed by [`try_finalize`]; shared with
    /// every reader task via its [`ReaderHandles`].
    gate: Gate,
    /// Shared with this process's reader tasks via their own
    /// [`ReaderHandles`] clone. See [`ReaderHandles`]'s own doc comment for
    /// why a frame's `seq` stays unique although its read-then-write
    /// sequence (`try_send`) is not itself one atomic operation.
    seq: Arc<AtomicU64>,
    pending_drops: Arc<AtomicU64>,
    /// Starts at the number of reader tasks actually spawned for this
    /// child (two for piped stdout/stderr, one for a pseudo-terminal
    /// master -- `docs/spike-log.md` § Slice 4); each reader task
    /// decrements it once, on EOF or a read error.
    active_readers: Arc<AtomicU8>,
    /// Notified once by the last reader task to finish: the self-exit
    /// reaper waits on it.
    readers_done: Arc<Notify>,
    /// Set once `observe`/`stop`/the self-exit reaper confirms the
    /// process's terminal state, independent of whether the readers are
    /// done yet.
    confirmed_state: Option<ProcessTerminalState>,
    /// Diagnostic `stderr` texts to deliver just before the `State` frame
    /// ([`record_final_state`], [`abandon_readers`]).
    final_notes: Vec<String>,
    /// Set when the self-exit reaper gave up waiting for a reader that
    /// outlasted the exit grace: the final frames no longer wait for it.
    readers_abandoned: bool,
    /// Set by the self-exit reaper when it starts, at spawn
    /// ([`claim_reap`]).
    reaper_claimed: bool,
    /// When [`claim_reap`] took the claim; cleared by [`release_reap`].
    claimed_at: Option<std::time::Instant>,
    /// How many times the claiming reaper has polled ([`note_reap_poll`]).
    reap_polls: u32,
    /// When the claiming reaper last polled.
    last_reap_poll_at: Option<std::time::Instant>,
    /// Set once `observe` logged that it outlasted the reaper's claim
    /// ([`note_deferral_expired`]), so the warning appears once per child.
    deferral_expired_logged: bool,
}

/// What each reader task holds its own clone of.
///
/// ## Why each frame's `seq` stays unique, and none follows the `State`
/// frame
///
/// [`try_send`] reads `seq`/`pending_drops`, builds a frame from what it
/// read, attempts delivery, and only *then* writes `seq`/`pending_drops`
/// back -- three separate atomic operations, not one. Two callers
/// interleaving between that read and that write would capture the same
/// `seq` and emit two frames claiming it, silently breaking the
/// contiguous-per-process-`seq` guarantee `docs/spike-log.md` § IPC
/// contract makes.
///
/// `try_send` therefore holds `gate`'s lock for that whole sequence, and
/// [`try_finalize`] holds the same lock while it assigns the final frames'
/// `seq` and closes the slot. That matters since a reader can be abandoned
/// ([`abandon_readers`]): the final frames are then assigned on whichever
/// thread confirmed the state while that reader still runs on the
/// supervisor's runtime thread. Every reader frame sent before the slot
/// closed has a lower `seq` and is already in the channel; every one after
/// it is discarded. (The two reader tasks of one process still share one
/// current-thread runtime -- `tests/runtime_drives_tasks.rs`'s
/// `the_supervisors_runtime_is_current_thread_flavor` -- but the lock, not
/// that, is what the guarantee now rests on.)
///
/// `discarded` is the one per-reader field: a clone starts its own count.
#[derive(Clone)]
pub(crate) struct ReaderHandles {
    gate: Gate,
    seq: Arc<AtomicU64>,
    pending_drops: Arc<AtomicU64>,
    active_readers: Arc<AtomicU8>,
    readers_done: Arc<Notify>,
    /// Bytes of text this reader read after the slot closed, discarded.
    discarded: Cell<u64>,
}

impl ReaderHandles {
    /// What the self-exit reaper waits on: notified once, by the last
    /// reader task to finish.
    pub(crate) fn readers_done(&self) -> Arc<Notify> {
        Arc::clone(&self.readers_done)
    }

    /// Whether the readers' slot is still open (the final frames are not
    /// assigned yet).
    fn slot_open(&self) -> bool {
        self.gate
            .lock()
            .expect("output gate mutex poisoned by a prior panic")
            .is_some()
    }
}

impl OutputChannelState {
    /// A fresh [`ReaderHandles`] onto this channel, for a frame sent from
    /// outside the reader tasks.
    fn handles(&self) -> ReaderHandles {
        ReaderHandles {
            gate: Arc::clone(&self.gate),
            seq: Arc::clone(&self.seq),
            pending_drops: Arc::clone(&self.pending_drops),
            active_readers: Arc::clone(&self.active_readers),
            readers_done: Arc::clone(&self.readers_done),
            discarded: Cell::new(0),
        }
    }

    /// The next frame in this channel's `seq` order, carrying the drops
    /// pending before it. Called only with `gate` locked.
    fn next_frame(&self, payload: FramePayload) -> OutputFrame {
        let seq = self.seq.fetch_add(1, Ordering::AcqRel);
        let dropped_before = self.pending_drops.swap(0, Ordering::AcqRel);
        OutputFrame::with_dropped_before(seq, dropped_before, payload)
    }
}

/// Build a fresh channel and bookkeeping for a newly spawned child, and the
/// [`ReaderHandles`] template to clone once per stream. `reader_count` is
/// the number of reader tasks the caller will actually spawn: the final
/// `State` frame is gated on exactly that many finishing, so a launch
/// with one master reader never waits for a second reader that does not
/// exist.
pub(crate) fn new_channel(reader_count: u8) -> (OutputChannelState, ReaderHandles) {
    let (sender, receiver) = sync_channel(CHANNEL_CAPACITY);
    let state = OutputChannelState {
        receiver: Some(receiver),
        gate: Arc::new(Mutex::new(Some(sender.clone()))),
        sender: Some(sender),
        seq: Arc::new(AtomicU64::new(0)),
        pending_drops: Arc::new(AtomicU64::new(0)),
        active_readers: Arc::new(AtomicU8::new(reader_count)),
        readers_done: Arc::new(Notify::new()),
        confirmed_state: None,
        final_notes: Vec::new(),
        readers_abandoned: false,
        reaper_claimed: false,
        claimed_at: None,
        reap_polls: 0,
        last_reap_poll_at: None,
        deferral_expired_logged: false,
    };
    let handles = state.handles();
    (state, handles)
}

/// Take the one-shot receiver for `id`, or the appropriate error: unknown
/// id, or a receiver already taken by an earlier subscribe.
///
/// If `id`'s channel was already finalized before this call (the process
/// finished and was drained of everything it will ever produce, before
/// anyone subscribed), taking the receiver here also makes this the moment
/// both cleanup conditions are met -- finalized and now subscribed-out --
/// so the whole entry is removed immediately, symmetric with
/// [`try_finalize`]'s own removal for the opposite ordering (subscribe
/// first, finalize later).
pub(crate) fn take_receiver(
    outputs: &OutputTable,
    id: ProcessId,
) -> Result<Receiver<OutputFrame>, omnifrons_app::SupervisorError> {
    let mut outputs = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    let entry = outputs
        .get_mut(&id)
        .ok_or(omnifrons_app::SupervisorError::UnknownProcess)?;
    let receiver = entry
        .receiver
        .take()
        .ok_or(omnifrons_app::SupervisorError::AlreadySubscribed)?;
    if entry.sender.is_none() {
        outputs.remove(&id);
    }
    Ok(receiver)
}

/// Drain `reader` (a child's stdout or stderr) into framed [`OutputFrame`]s:
/// lossy UTF-8, split at [`MAX_LINE_BYTES`] on a UTF-8 character boundary
/// (never mid-character), `\r\n` treated as a line end with the `\r`
/// stripped, delivered best-effort via `handles`. Runs until EOF or a read
/// error -- a final line with no trailing newline is still delivered once
/// EOF is reached, not dropped for lacking a terminator.
///
/// `Ok(0)` ends capture silently: the normal way a stream ends, including
/// on the pty path where [`crate::pty::MasterReader`] already maps `EIO` to
/// EOF before it reaches here. An `Err` also ends capture, but flushes any
/// buffered partial line first, then delivers exactly one synthetic
/// diagnostic frame on `OutputStream::Stderr` naming the stream, the
/// error's kind, and its raw OS error number ([`read_error_diagnostic`]) --
/// never the error's own free-form, OS-supplied (and sometimes localized)
/// message -- and logs the error via `tracing::warn!`.
///
/// Either way, once the loop ends this stream marks itself done and, if it
/// was the last of its process's readers, wakes the self-exit reaper and
/// asks [`try_finalize`] to send the final state frame (a no-op if the
/// terminal state is not confirmed yet). A reader that ends after its run's
/// final frames were already assigned -- one the reaper abandoned -- first
/// logs, once, how many bytes it read and discarded since.
///
/// A split is decided *lazily*: a full buffer is never emitted the moment
/// it fills, only once the next byte of input proves the line goes on --
/// so `continued: true` is set exactly when more of the same line really
/// follows, and a genuine line of exactly [`MAX_LINE_BYTES`] (followed by
/// a newline, or by EOF) is emitted as one ordinary `continued: false`
/// frame, never as a continuation plus an empty terminator (R3-003).
pub(crate) async fn drain_stream<R>(
    mut reader: R,
    stream: OutputStream,
    handles: ReaderHandles,
    outputs: OutputTable,
    id: ProcessId,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; MAX_LINE_BYTES];
    // A `\r` read right after `buf` filled to the cap, still unresolved:
    // if the very next byte is `\n` it was a CRLF line end (the line is
    // complete, not continued, and the CR is not content); anything else,
    // or EOF, makes the CR content -- and then the line continues past the
    // cap (R3-012).
    let mut pending_cr = false;
    let mut read_error: Option<std::io::Error> = None;

    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => {
                read_error = Some(error);
                break;
            }
        };
        let mut start = 0;
        while start < read {
            if pending_cr {
                pending_cr = false;
                if chunk[start] == b'\n' {
                    emit_text(&handles, stream, &buf, false);
                    buf.clear();
                    start += 1;
                    continue;
                }
                split_at_cap(&handles, stream, &mut buf);
                buf.push(b'\r');
            }
            if buf.len() == MAX_LINE_BYTES {
                // The buffer is full and more input follows in this chunk:
                // only the next byte(s) decide whether the line ends here.
                match chunk[start] {
                    b'\n' => {
                        // An exactly-cap line, LF-terminated (a trailing
                        // CR inside the buffer belongs to a CRLF):
                        // complete, not continued -- no split at all.
                        strip_trailing_cr(&mut buf);
                        emit_text(&handles, stream, &buf, false);
                        buf.clear();
                        start += 1;
                        continue;
                    }
                    b'\r' if start + 1 == read => {
                        // The read ends on this CR: whether an LF follows
                        // is only known once the next read arrives.
                        pending_cr = true;
                        start += 1;
                        continue;
                    }
                    b'\r' if chunk[start + 1] == b'\n' => {
                        // An exactly-cap line, CRLF-terminated: complete,
                        // not continued, and the CRLF is not content.
                        emit_text(&handles, stream, &buf, false);
                        buf.clear();
                        start += 2;
                        continue;
                    }
                    _ => split_at_cap(&handles, stream, &mut buf),
                }
            }
            let space = MAX_LINE_BYTES - buf.len();
            let window_end = start + space.min(read - start);
            if let Some(relative_newline) =
                chunk[start..window_end].iter().position(|&b| b == b'\n')
            {
                buf.extend_from_slice(&chunk[start..start + relative_newline]);
                strip_trailing_cr(&mut buf);
                emit_text(&handles, stream, &buf, false);
                buf.clear();
                start += relative_newline + 1;
            } else {
                buf.extend_from_slice(&chunk[start..window_end]);
                start = window_end;
            }
        }
    }
    if pending_cr {
        // EOF right after that CR: it is content, and the line it ends
        // runs past the cap.
        split_at_cap(&handles, stream, &mut buf);
        buf.push(b'\r');
    }
    if !buf.is_empty() {
        emit_text(&handles, stream, &buf, false);
    }

    finish_stream(&handles, stream, read_error, &outputs, id);
}

/// [`drain_stream`]'s end, after its last line: report a read error, log a
/// reader that outlived its run's final frame, then mark this stream done.
fn finish_stream(
    handles: &ReaderHandles,
    stream: OutputStream,
    read_error: Option<std::io::Error>,
    outputs: &OutputTable,
    id: ProcessId,
) {
    if let Some(error) = read_error {
        tracing::warn!(
            pid = id.0,
            ?stream,
            kind = ?error.kind(),
            os_error = error.raw_os_error(),
            "output capture ended by a read error"
        );
        let diagnostic = read_error_diagnostic(stream, &error);
        // The supervisor's own text, not the child's: never counted as
        // discarded output.
        let _ = try_send(
            handles,
            FramePayload::Text {
                stream: OutputStream::Stderr,
                text: diagnostic,
                continued: false,
            },
        );
    }

    // Still counted among the active readers here, so a closed slot can
    // only mean the run's final frames went out without waiting for this
    // reader: it was abandoned.
    if !handles.slot_open() {
        tracing::warn!(
            pid = id.0,
            ?stream,
            discarded_bytes = handles.discarded.get(),
            "a reader outlived its run's final state frame; what it read after that frame \
             was discarded"
        );
    }

    if handles.active_readers.fetch_sub(1, Ordering::AcqRel) == 1 {
        handles.readers_done.notify_one();
        try_finalize(outputs, id);
    }
}

/// Render [`drain_stream`]'s read-error diagnostic text: which stream
/// ended, the error's `Debug`-formatted [`std::io::ErrorKind`], and its raw
/// OS error number (or `(no os error)`) -- never `error.to_string()`, whose
/// OS-supplied message is sometimes localized and does not belong in a
/// delivered frame.
fn read_error_diagnostic(stream: OutputStream, error: &std::io::Error) -> String {
    let stream_name = match stream {
        OutputStream::Stdout => "stdout",
        OutputStream::Stderr => "stderr",
    };
    let os_error = match error.raw_os_error() {
        Some(code) => format!("os error {code}"),
        None => "no os error".to_string(),
    };
    format!(
        "output capture ended by a read error on {stream_name}: {:?} ({os_error})",
        error.kind()
    )
}

/// Split a buffer that reached [`MAX_LINE_BYTES`] whose line goes on: emit
/// everything up to the last complete UTF-8 character boundary as a
/// `continued` frame, and keep any trailing partial-character bytes for
/// the next frame instead of emitting (and thereby corrupting into a
/// replacement character) them now.
fn split_at_cap(handles: &ReaderHandles, stream: OutputStream, buf: &mut Vec<u8>) {
    let boundary = utf8_safe_boundary(buf);
    let carry_over = buf.split_off(boundary);
    emit_text(handles, stream, buf, true);
    *buf = carry_over;
}

/// Strip a single trailing `\r`, if present: `\r\n` is treated as a line
/// end like a bare `\n`, not as a `\n`-terminated line whose text happens
/// to carry a dangling carriage return.
fn strip_trailing_cr(buf: &mut Vec<u8>) {
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
}

/// The largest prefix of `bytes` that is valid UTF-8, for splitting a
/// buffer at [`MAX_LINE_BYTES`] without ever cutting a multibyte character
/// in half.
///
/// Falls back to `bytes.len()` (accept the whole, invalid, prefix) if not
/// even one complete character can be recovered -- i.e. the very first byte
/// is definitively invalid, not merely the start of an as-yet-incomplete
/// sequence waiting on more bytes. Holding those bytes back to await bytes
/// that will never resolve them into anything valid would stall forward
/// progress for no benefit; lossy decoding (`emit_text`) already turns them
/// into a replacement character exactly as it does for any other invalid
/// byte, matching this module's existing invalid-UTF-8 policy.
fn utf8_safe_boundary(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(error) => {
            let valid_up_to = error.valid_up_to();
            if valid_up_to == 0 {
                bytes.len()
            } else {
                valid_up_to
            }
        }
    }
}

/// Frame and best-effort-deliver one decoded text chunk of the child's
/// output. `continued` is `true` only for a chunk that the per-frame cap
/// forced to end early with more of the same logical line still to come
/// (see [`drain_stream`]). A chunk read after the readers' slot closed is
/// discarded and its bytes counted (`ReaderHandles::discarded`); returns
/// whether it was accepted.
fn emit_text(handles: &ReaderHandles, stream: OutputStream, bytes: &[u8], continued: bool) -> bool {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let accepted = try_send(
        handles,
        FramePayload::Text {
            stream,
            text,
            continued,
        },
    );
    if !accepted {
        let discarded = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        handles
            .discarded
            .set(handles.discarded.get().saturating_add(discarded));
    }
    accepted
}

/// Best-effort delivery of one text frame through the readers' slot:
/// `try_send` only. A full channel increments the pending-drop count,
/// surfaced as `dropped_before` on the next frame that does get through,
/// rather than blocking the caller. Holds the slot's lock throughout (see
/// [`ReaderHandles`]); returns `false` only when the slot is already closed
/// -- the run's final frames are assigned -- and the frame was discarded
/// rather than delivered after them.
fn try_send(handles: &ReaderHandles, payload: FramePayload) -> bool {
    let slot = handles
        .gate
        .lock()
        .expect("output gate mutex poisoned by a prior panic");
    let Some(sender) = slot.as_ref() else {
        return false;
    };
    let seq = handles.seq.load(Ordering::Acquire);
    let dropped_before = handles.pending_drops.load(Ordering::Acquire);
    let frame = OutputFrame::with_dropped_before(seq, dropped_before, payload);
    match sender.try_send(frame) {
        Ok(()) => {
            handles.seq.fetch_add(1, Ordering::AcqRel);
            handles.pending_drops.store(0, Ordering::Release);
        }
        Err(TrySendError::Full(_)) => {
            handles.pending_drops.fetch_add(1, Ordering::AcqRel);
        }
        Err(TrySendError::Disconnected(_)) => {
            // The subscriber dropped its receiver; nothing left to do.
        }
    }
    true
}

/// Emit a synthetic stderr text frame reporting that writing an adapter
/// launch's prompt to its child's stdin failed, best-effort (a full or
/// already-disconnected channel is not itself an error here -- there is
/// nothing more actionable to do).
///
/// Routed as a `stderr` frame deliberately, not invented as some new
/// cross-layer "state observation" concept: the raw `OutputFrame`/
/// `FramePayload` vocabulary this module speaks has no notion of an
/// adapter-level `AdapterEvent` or its `observations` (that concept lives
/// only in `omnifrons-app::harness_adapter`, which this module has no
/// visibility into) -- but the shell already maps every `stderr` frame
/// straight to `AdapterEvent::Diagnostic` for an adapter launch
/// (`docs/spike-log.md` § Slice 3), which is exactly the right shape for
/// "a plumbing failure happened, surface it plainly" with no new concept
/// needed anywhere in the pipeline. Never causes a hang: this is called
/// from the same background task that already gave up on writing to
/// stdin, and simply reports the fact.
pub(crate) fn emit_stdin_write_failed(outputs: &OutputTable, id: ProcessId) {
    let handles = {
        let guard = outputs
            .lock()
            .expect("outputs mutex poisoned by a prior panic");
        let Some(entry) = guard.get(&id) else {
            return;
        };
        if entry.sender.is_none() {
            return; // already finalized; nothing left to deliver to
        }
        entry.handles()
    };
    // Through the readers' slot, like any reader frame: it takes the next
    // `seq` and can never follow the `State` frame.
    let _ = try_send(
        &handles,
        FramePayload::Text {
            stream: OutputStream::Stderr,
            text: "stdin write failed".to_string(),
            continued: false,
        },
    );
}

/// Record that `id`'s process reached `state`, and finalize its output
/// channel now if both reader tasks are already done.
///
/// Called from `observe`/`stop` the moment they confirm a reap -- safe to
/// call repeatedly (e.g. once per later `observe` on an already-terminal
/// id): [`try_finalize`] is itself idempotent.
pub(crate) fn record_confirmed_state(
    outputs: &OutputTable,
    id: ProcessId,
    state: ProcessTerminalState,
) {
    record_final_state(outputs, id, state, Vec::new(), false);
}

/// [`record_confirmed_state`], plus what the self-exit reaper settled with
/// it: `notes`, diagnostic `stderr` texts delivered in order just before the
/// `State` frame (an empty list keeps any already recorded), and, when
/// `abandon` is set, that the final frames no longer wait for a reader that
/// outlasted the exit grace (see this module's own doc comment).
pub(crate) fn record_final_state(
    outputs: &OutputTable,
    id: ProcessId,
    state: ProcessTerminalState,
    notes: Vec<String>,
    abandon: bool,
) {
    {
        let mut guard = outputs
            .lock()
            .expect("outputs mutex poisoned by a prior panic");
        let Some(entry) = guard.get_mut(&id) else {
            return;
        };
        entry.confirmed_state = Some(state);
        if !notes.is_empty() {
            entry.final_notes = notes;
        }
        entry.readers_abandoned |= abandon;
    }
    try_finalize(outputs, id);
}

/// Stop waiting for `id`'s readers, with `notes` to deliver before the
/// `State` frame: for a run whose child was reaped by someone else
/// (`stop`, or `observe` taking a settle over) while a reader still
/// outlasted the exit grace. The frame carries whatever state that reap
/// recorded, now or once it is recorded.
pub(crate) fn abandon_readers(outputs: &OutputTable, id: ProcessId, notes: Vec<String>) {
    {
        let mut guard = outputs
            .lock()
            .expect("outputs mutex poisoned by a prior panic");
        let Some(entry) = guard.get_mut(&id) else {
            return;
        };
        if !notes.is_empty() {
            entry.final_notes = notes;
        }
        entry.readers_abandoned = true;
    }
    try_finalize(outputs, id);
}

/// Whether any of `id`'s reader tasks is still running. `false` for an
/// entry already removed (finalized and subscribed-out).
pub(crate) fn readers_active(outputs: &OutputTable, id: ProcessId) -> bool {
    let guard = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    guard
        .get(&id)
        .is_some_and(|entry| entry.active_readers.load(Ordering::Acquire) != 0)
}

/// Claim the self-exit reap of `id`, at most once per child.
///
/// The reaper claims its child when it starts, at spawn, and keeps the
/// claim while it polls: `observe` defers to it meanwhile
/// ([`reaper_claim`]). Succeeds only when the final `State` frame is still
/// owed: not finalized, no terminal state confirmed, and no earlier claim.
pub(crate) fn claim_reap(outputs: &OutputTable, id: ProcessId) -> bool {
    let mut guard = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    let Some(entry) = guard.get_mut(&id) else {
        return false;
    };
    if entry.reaper_claimed || entry.sender.is_none() || entry.confirmed_state.is_some() {
        return false;
    }
    entry.reaper_claimed = true;
    entry.claimed_at = Some(std::time::Instant::now());
    entry.reap_polls = 0;
    entry.last_reap_poll_at = None;
    entry.deferral_expired_logged = false;
    true
}

/// What `observe` learns about a self-exit reaper's claim on a child.
pub(crate) struct ReaperClaim {
    /// When the reaper took the claim.
    pub(crate) claimed_at: std::time::Instant,
    /// How many times the reaper has polled since.
    pub(crate) polls: u32,
    /// When it last polled, if it has.
    pub(crate) last_poll_at: Option<std::time::Instant>,
}

/// The claim a self-exit reaper holds on `id` while the final frame is still
/// owed: from then on the reaper is the one that confirms the reap, so
/// `observe` leaves the child to it for a bounded time ([`release_reap`]
/// hands it back).
pub(crate) fn reaper_claim(outputs: &OutputTable, id: ProcessId) -> Option<ReaperClaim> {
    let guard = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    let entry = guard.get(&id)?;
    if !(entry.reaper_claimed && entry.sender.is_some()) {
        return None;
    }
    Some(ReaperClaim {
        claimed_at: entry.claimed_at?,
        polls: entry.reap_polls,
        last_poll_at: entry.last_reap_poll_at,
    })
}

/// Record one poll by the claiming reaper, for the stall warning.
pub(crate) fn note_reap_poll(outputs: &OutputTable, id: ProcessId) {
    let mut guard = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    if let Some(entry) = guard.get_mut(&id) {
        entry.reap_polls = entry.reap_polls.saturating_add(1);
        entry.last_reap_poll_at = Some(std::time::Instant::now());
    }
}

/// Mark that `observe` outlasted the reaper's claim on `id`; `true` only the
/// first time, so the caller logs the stall once per child.
pub(crate) fn note_deferral_expired(outputs: &OutputTable, id: ProcessId) -> bool {
    let mut guard = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    match guard.get_mut(&id) {
        Some(entry) if !entry.deferral_expired_logged => {
            entry.deferral_expired_logged = true;
            true
        }
        _ => false,
    }
}

/// Whether [`note_deferral_expired`] fired for `id` (test read accessor; its
/// only reader is the unix-only self-exit test module).
#[cfg(all(test, unix))]
pub(crate) fn deferral_expired(outputs: &OutputTable, id: ProcessId) -> bool {
    let guard = outputs.lock().expect("outputs mutex poisoned");
    guard.get(&id).is_some_and(|e| e.deferral_expired_logged)
}

/// Give up a claim taken by [`claim_reap`], for a reaper that ends without
/// having reaped the child, so `observe` confirms the reap again.
pub(crate) fn release_reap(outputs: &OutputTable, id: ProcessId) {
    let mut guard = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    if let Some(entry) = guard.get_mut(&id) {
        entry.reaper_claimed = false;
        entry.claimed_at = None;
    }
}

/// Whether `id`'s final `State` frame is still owed (the entry exists and
/// was not finalized): the reaper's signal to keep polling or to stop.
pub(crate) fn final_frame_owed(outputs: &OutputTable, id: ProcessId) -> bool {
    let guard = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    guard.get(&id).is_some_and(|entry| entry.sender.is_some())
}

/// Send the final frames -- the recorded diagnostics, then the `State`
/// frame -- and drop this channel's senders, but only once the process's
/// terminal state is confirmed *and* every reader task has finished (or
/// was abandoned, [`record_final_state`]) -- whichever happens second
/// performs the send. A no-op if either condition is not yet met, or if
/// this channel was already finalized.
///
/// The readers' slot is closed in the same step, under its own lock, so
/// the `State` frame is the last frame this channel ever carries (see
/// [`ReaderHandles`]).
///
/// Unlike a text frame (`try_send`, best-effort, `emit_text`/`try_send`
/// above), the final frames are guaranteed-delivered: the `State` frame is
/// the one signal a subscriber needs to know the process is truly done, so
/// it must reach them even against a delivery channel that is completely
/// full and undrained at this exact moment. Delivery is handed to a
/// dedicated `std::thread` doing blocking [`SyncSender::send`]s, which ends
/// once the frames are delivered (room eventually frees up, since no
/// further text frames are produced once this point is reached) or the
/// subscriber drops its receiver (the `send` then fails harmlessly) --
/// never on the async runtime, which must never block on a slow or absent
/// consumer.
///
/// Also removes `id`'s whole entry from `outputs` if it is already
/// subscribed-out (`receiver` already taken) at this point: finalized and
/// subscribed-out are exactly the two conditions [`OutputChannelState`]'s
/// own doc comment names for reclaiming it. If nobody has subscribed yet,
/// the entry (and its buffered channel, including the frame this call is
/// about to hand to the delivery thread) is left in place, so a later
/// `subscribe` still finds it.
fn try_finalize(outputs: &OutputTable, id: ProcessId) {
    let (sender, frames) = {
        let mut guard = outputs
            .lock()
            .expect("outputs mutex poisoned by a prior panic");
        let Some(entry) = guard.get_mut(&id) else {
            return;
        };
        if entry.sender.is_none() {
            return; // already finalized
        }
        if entry.active_readers.load(Ordering::Acquire) != 0 && !entry.readers_abandoned {
            return; // a reader task is still running
        }
        let Some(state) = entry.confirmed_state else {
            return; // reap not confirmed yet
        };

        let notes = std::mem::take(&mut entry.final_notes);
        let gate = Arc::clone(&entry.gate);
        let mut slot = gate
            .lock()
            .expect("output gate mutex poisoned by a prior panic");
        // Closed before any final `seq` is assigned, under the lock every
        // reader frame is sent under: nothing can follow the `State` frame.
        *slot = None;
        let mut frames: Vec<OutputFrame> = notes
            .into_iter()
            .map(|text| {
                entry.next_frame(FramePayload::Text {
                    stream: OutputStream::Stderr,
                    text,
                    continued: false,
                })
            })
            .collect();
        frames.push(entry.next_frame(FramePayload::State(state)));
        drop(slot);
        // Taken (not merely read) and cleared here, under the lock, so a
        // second concurrent call (from the other reader task finishing at
        // nearly the same instant as a `stop`/`observe` confirming reap)
        // sees `sender` already `None` and returns above -- exactly one
        // caller ever spawns the delivery thread below.
        let sender = entry
            .sender
            .take()
            .expect("checked Some via the is_none() guard above");
        let subscribed_out = entry.receiver.is_none();
        if subscribed_out {
            guard.remove(&id);
        }
        (sender, frames)
    };

    if let Err(error) = std::thread::Builder::new()
        .name("omnifrons-output-state-frame".to_string())
        .spawn(move || {
            for frame in frames {
                if sender.send(frame).is_err() {
                    break; // the subscriber is gone
                }
            }
        })
    {
        tracing::error!(
            %error,
            "failed to spawn the output state-frame delivery thread; the final state frame \
             was not delivered"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::pin::Pin;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use omnifrons_app::{FramePayload, OutputStream, ProcessId, ProcessTerminalState};
    use tokio::io::{AsyncRead, ReadBuf};

    use super::{
        CHANNEL_CAPACITY, MAX_LINE_BYTES, OutputTable, drain_stream, emit_stdin_write_failed,
        emit_text, new_channel, record_confirmed_state, record_final_state, take_receiver,
        try_send,
    };

    /// A run whose reader the reaper abandoned still ends while that reader
    /// runs: the notes, then the `State` frame, close the channel, and what
    /// the reader produces afterwards is discarded and counted, never
    /// delivered after the `State` frame.
    #[test]
    fn an_abandoned_reader_never_delivers_after_the_state_frame() {
        let id = ProcessId(13);
        let (state, handles) = new_channel(1);
        let outputs: OutputTable = Arc::new(Mutex::new(HashMap::from([(id, state)])));

        assert!(emit_text(&handles, OutputStream::Stdout, b"before", false));
        record_final_state(
            &outputs,
            id,
            ProcessTerminalState::OrphanRiskUncertain,
            vec!["first note".to_string(), "second note".to_string()],
            true,
        );
        assert!(!emit_text(&handles, OutputStream::Stdout, b"after", false));
        assert_eq!(handles.discarded.get(), 5);

        let receiver = take_receiver(&outputs, id).expect("subscribing must succeed");
        // Blocking `iter()`: it ends although `handles` is still alive.
        let frames: Vec<_> = receiver.iter().collect();
        let shape: Vec<(u64, String)> = frames
            .iter()
            .map(|frame| {
                let label = match &frame.payload {
                    FramePayload::Text { stream, text, .. } => format!("{stream:?}:{text}"),
                    FramePayload::State(state) => format!("state:{state:?}"),
                };
                (frame.seq, label)
            })
            .collect();
        assert_eq!(
            shape,
            [
                (0, "Stdout:before".to_string()),
                (1, "Stderr:first note".to_string()),
                (2, "Stderr:second note".to_string()),
                (3, "state:OrphanRiskUncertain".to_string()),
            ]
        );
    }

    /// An `AsyncRead` handing out exactly one predefined chunk per read
    /// call, then EOF -- so a test controls precisely where a line's bytes
    /// fall relative to read boundaries, which a real pipe never lets it.
    struct ChunkedReader {
        chunks: VecDeque<Vec<u8>>,
        // `Some` once every chunk is exhausted, to fail the next read
        // instead of returning EOF -- unused (stays `None`) by every
        // ordinary, EOF-ending `ChunkedReader`.
        trailing_error: Option<std::io::Error>,
    }

    impl ChunkedReader {
        fn new(chunks: &[&[u8]]) -> Self {
            Self {
                chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
                trailing_error: None,
            }
        }

        fn new_ending_in_error(chunks: &[&[u8]], error: std::io::Error) -> Self {
            Self {
                chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
                trailing_error: Some(error),
            }
        }
    }

    impl AsyncRead for ChunkedReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let this = self.get_mut();
            if let Some(chunk) = this.chunks.pop_front() {
                assert!(
                    chunk.len() <= buf.remaining(),
                    "a test chunk must fit drain_stream's own read buffer"
                );
                buf.put_slice(&chunk);
                return Poll::Ready(Ok(()));
            }
            match this.trailing_error.take() {
                Some(error) => Poll::Ready(Err(error)),
                None => Poll::Ready(Ok(())),
            }
        }
    }

    /// Run `drain_stream` over `chunks` as stdout and return every text
    /// frame's `(text, continued)`, in order.
    fn stdout_frames_for_chunks(chunks: &[&[u8]]) -> Vec<(String, bool)> {
        let id = ProcessId(7);
        let (state, handles) = new_channel(2);
        let outputs: OutputTable = Arc::new(Mutex::new(HashMap::from([(id, state)])));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime must build");
        runtime.block_on(drain_stream(
            ChunkedReader::new(chunks),
            OutputStream::Stdout,
            handles,
            Arc::clone(&outputs),
            id,
        ));
        let receiver = take_receiver(&outputs, id).expect("subscribing must succeed");
        receiver
            .try_iter()
            .filter_map(|frame| match frame.payload {
                FramePayload::Text {
                    text, continued, ..
                } => Some((text, continued)),
                FramePayload::State(_) => None,
            })
            .collect()
    }

    /// Like [`stdout_frames_for_chunks`], but keeps each frame's own
    /// `stream` visible: a read-error diagnostic is always `Stderr`,
    /// regardless of which stream is being drained.
    fn frames_with_stream_for<R>(
        reader: R,
        stream: OutputStream,
    ) -> Vec<(OutputStream, String, bool)>
    where
        R: AsyncRead + Unpin,
    {
        let id = ProcessId(7);
        let (state, handles) = new_channel(2);
        let outputs: OutputTable = Arc::new(Mutex::new(HashMap::from([(id, state)])));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime must build");
        runtime.block_on(drain_stream(
            reader,
            stream,
            handles,
            Arc::clone(&outputs),
            id,
        ));
        let receiver = take_receiver(&outputs, id).expect("subscribing must succeed");
        receiver
            .try_iter()
            .filter_map(|frame| match frame.payload {
                FramePayload::Text {
                    stream,
                    text,
                    continued,
                } => Some((stream, text, continued)),
                FramePayload::State(_) => None,
            })
            .collect()
    }

    /// A fixed, portable OS error for every read-error test here; its
    /// platform meaning is irrelevant, only that it is stable.
    fn injected_error() -> std::io::Error {
        std::io::Error::from_raw_os_error(9)
    }

    /// A read error flushes a pending partial line first (as EOF would),
    /// then delivers exactly one diagnostic frame on `Stderr`.
    #[test]
    fn a_read_error_delivers_the_pending_partial_line_then_one_diagnostic_stderr_frame() {
        let expected_kind = injected_error().kind();
        let frames = frames_with_stream_for(
            ChunkedReader::new_ending_in_error(&[b"partial"], injected_error()),
            OutputStream::Stdout,
        );

        assert_eq!(
            frames,
            vec![
                (OutputStream::Stdout, "partial".to_string(), false),
                (
                    OutputStream::Stderr,
                    format!(
                        "output capture ended by a read error on stdout: {expected_kind:?} \
                         (os error 9)"
                    ),
                    false,
                ),
            ]
        );
    }

    /// A clean EOF must never emit the diagnostic, only a real read error.
    #[test]
    fn a_clean_eof_emits_no_diagnostic_stderr_frame() {
        let frames = frames_with_stream_for(ChunkedReader::new(&[b"line\n"]), OutputStream::Stdout);
        assert_eq!(
            frames,
            vec![(OutputStream::Stdout, "line".to_string(), false)]
        );
    }

    /// A read error with nothing buffered delivers only the diagnostic.
    #[test]
    fn a_read_error_with_no_pending_partial_line_emits_only_the_diagnostic() {
        let expected_kind = injected_error().kind();
        let frames = frames_with_stream_for(
            ChunkedReader::new_ending_in_error(&[], injected_error()),
            OutputStream::Stderr,
        );

        assert_eq!(
            frames,
            vec![(
                OutputStream::Stderr,
                format!(
                    "output capture ended by a read error on stderr: {expected_kind:?} \
                     (os error 9)"
                ),
                false,
            )]
        );
    }

    /// A read error still decrements `active_readers` like a clean EOF, and
    /// once the terminal state is confirmed, the finalize tail still sends
    /// the final `State` frame -- after the diagnostic, never before it.
    #[test]
    fn a_read_error_still_lets_the_stream_finish_and_finalize_proceed() {
        let id = ProcessId(11);
        let (state, handles) = new_channel(1);
        let active_readers = Arc::clone(&handles.active_readers);
        let outputs: OutputTable = Arc::new(Mutex::new(HashMap::from([(id, state)])));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime must build");

        runtime.block_on(drain_stream(
            ChunkedReader::new_ending_in_error(&[], injected_error()),
            OutputStream::Stdout,
            handles,
            Arc::clone(&outputs),
            id,
        ));

        assert_eq!(
            active_readers.load(Ordering::Acquire),
            0,
            "a read error must still decrement active_readers, exactly like a clean EOF"
        );

        record_confirmed_state(&outputs, id, ProcessTerminalState::Exited { code: Some(0) });
        let receiver = take_receiver(&outputs, id).expect("subscribing must succeed");
        // Blocking `iter()`: the state frame is sent from a dedicated
        // thread (`try_finalize`'s doc comment), so this must wait for it.
        let frames: Vec<_> = receiver.iter().collect();
        // Exactly the diagnostic, then the state frame, in that order: the
        // diagnostic is this stream's last word, never something that
        // trails the frame every consumer treats as final.
        let shape: Vec<&str> = frames
            .iter()
            .map(|frame| match &frame.payload {
                FramePayload::Text {
                    stream: OutputStream::Stderr,
                    ..
                } => "diagnostic",
                FramePayload::Text { .. } => "text",
                FramePayload::State(ProcessTerminalState::Exited { code: Some(0) }) => "state",
                FramePayload::State(_) => "unexpected-state",
            })
            .collect();
        assert_eq!(
            shape,
            ["diagnostic", "state"],
            "the diagnostic must precede the final state frame, got {frames:#?}"
        );
    }

    fn x_times(count: usize) -> Vec<u8> {
        vec![b'x'; count]
    }

    fn stdout_text(text: impl Into<String>) -> FramePayload {
        FramePayload::Text {
            stream: OutputStream::Stdout,
            text: text.into(),
            continued: false,
        }
    }

    #[test]
    fn paused_consumer_drops_newest_frame_at_fixed_channel_capacity() {
        let (mut state, handles) = new_channel(1);

        for line in 0..CHANNEL_CAPACITY {
            try_send(&handles, stdout_text(format!("buffered-{line}")));
        }
        try_send(&handles, stdout_text("dropped-while-paused"));

        assert_eq!(handles.seq.load(Ordering::Acquire), CHANNEL_CAPACITY as u64);
        assert_eq!(
            handles.pending_drops.load(Ordering::Acquire),
            1,
            "the frame sent after a paused consumer filled the channel must be dropped"
        );

        let receiver = state
            .receiver
            .take()
            .expect("a new output channel must retain its receiver");
        let delivered: Vec<_> = receiver.try_iter().collect();

        assert_eq!(delivered.len(), CHANNEL_CAPACITY);
        assert_eq!(delivered.first().map(|frame| frame.seq), Some(0));
        assert_eq!(
            delivered.last().map(|frame| frame.seq),
            Some((CHANNEL_CAPACITY - 1) as u64)
        );
        assert!(
            delivered.iter().all(|frame| frame.dropped_before == 0),
            "a pending drop is attached to the next successful delivery, not retroactively"
        );
    }

    #[test]
    fn draining_then_refilling_can_exceed_capacity_over_the_receiver_lifetime() {
        let (mut state, handles) = new_channel(1);

        for line in 0..CHANNEL_CAPACITY {
            try_send(&handles, stdout_text(format!("buffered-{line}")));
        }
        try_send(&handles, stdout_text("dropped-while-paused"));

        let receiver = state
            .receiver
            .take()
            .expect("a new output channel must retain its receiver");
        let first = receiver
            .try_recv()
            .expect("the paused consumer must have a buffered first frame");
        assert_eq!(first.seq, 0);

        try_send(&handles, stdout_text("refilled-after-drain"));
        let delivered_after_refill: Vec<_> = receiver.try_iter().collect();

        assert_eq!(delivered_after_refill.len(), CHANNEL_CAPACITY);
        let refill = delivered_after_refill
            .last()
            .expect("the consumer must receive the refilled frame");
        assert_eq!(refill.seq, CHANNEL_CAPACITY as u64);
        assert_eq!(refill.dropped_before, 1);
        assert_eq!(refill.payload, stdout_text("refilled-after-drain"));
    }

    /// R3-012: an exactly-cap line terminated by CRLF, the CRLF arriving in
    /// the read after the buffer filled, is one complete frame with the CR
    /// stripped -- never a continuation plus an empty terminator.
    #[test]
    fn exact_cap_crlf_line_is_one_frame_without_the_cr() {
        let content = x_times(MAX_LINE_BYTES);
        let frames = stdout_frames_for_chunks(&[content.as_slice(), b"\r\n"]);
        assert_eq!(frames, vec![("x".repeat(MAX_LINE_BYTES), false)]);
    }

    /// R3-012: the same line with the CR and the LF arriving in separate
    /// reads -- the CR alone cannot decide anything until the LF is seen.
    #[test]
    fn exact_cap_crlf_line_with_cr_and_lf_in_separate_reads_is_one_frame_without_the_cr() {
        let content = x_times(MAX_LINE_BYTES);
        let frames = stdout_frames_for_chunks(&[content.as_slice(), b"\r", b"\n"]);
        assert_eq!(frames, vec![("x".repeat(MAX_LINE_BYTES), false)]);
    }

    /// The bare-LF path: the CR is the cap's own last byte and the LF is
    /// the next read's first byte. Guarded so the two CRLF paths never
    /// drift apart.
    #[test]
    fn exact_cap_content_ending_in_cr_then_lf_in_the_next_read_is_one_frame_without_the_cr() {
        let mut content = x_times(MAX_LINE_BYTES - 1);
        content.push(b'\r');
        let frames = stdout_frames_for_chunks(&[content.as_slice(), b"\n"]);
        assert_eq!(frames, vec![("x".repeat(MAX_LINE_BYTES - 1), false)]);
    }

    /// R3-012: cap+1 CRLF -> a continued frame of exactly the cap, then a
    /// one-byte terminator with no CR.
    #[test]
    fn cap_plus_one_crlf_line_splits_into_a_continued_frame_then_a_terminator_without_the_cr() {
        let content = x_times(MAX_LINE_BYTES);
        let frames = stdout_frames_for_chunks(&[content.as_slice(), b"x\r\n"]);
        assert_eq!(
            frames,
            vec![("x".repeat(MAX_LINE_BYTES), true), ("x".to_string(), false)]
        );
    }

    /// `count` euro signs (U+20AC, 3 bytes each) as bytes. 3 does not
    /// divide `MAX_LINE_BYTES` (8192 = 3 * 2730 + 2), so filling the buffer
    /// with these always lands two bytes into a character -- a naive cut at
    /// the cap would split one.
    fn euros(count: usize) -> Vec<u8> {
        "€".repeat(count).into_bytes()
    }

    /// Multi-byte content crossing the cap that then refills the buffer to
    /// exactly the cap at a character boundary: 5460 `€` (16 380 bytes)
    /// plus `"ab"`. The first 8192 bytes end two bytes into the 2731st
    /// `€`; once the split carries those two bytes over, the buffer refills
    /// to exactly 8192 bytes ending `...€ab`. Returned as the two reads
    /// that deliver it, the second ending on a CR.
    fn multibyte_content_ending_in_a_held_cr() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut content = euros(5460);
        content.extend_from_slice(b"ab");
        assert_eq!(content.len(), 16_382);
        let (first_read, rest) = content.split_at(MAX_LINE_BYTES);
        let first_read = first_read.to_vec();
        let mut second_read = rest.to_vec();
        second_read.push(b'\r');
        assert_eq!(
            second_read.len(),
            MAX_LINE_BYTES - 1,
            "the second read must end exactly on the CR"
        );
        (content, first_read, second_read)
    }

    fn rejoined(frames: &[(String, bool)]) -> String {
        frames.iter().map(|(text, _)| text.as_str()).collect()
    }

    fn assert_whole_characters(frames: &[(String, bool)]) {
        for (text, _) in frames {
            assert!(
                !text.contains('\u{FFFD}'),
                "no frame may start or end with a partial character (lossy decoding would \
                 have produced U+FFFD), got {text:?}"
            );
        }
    }

    /// R3-016: multi-byte content across the cap (the split backs off to
    /// the character boundary and carries the partial character over),
    /// then a CR as the last byte of one read and the LF as the first byte
    /// of the next, resolved through the pending-CR path. Every frame is
    /// whole characters, they re-join to the original, the CR is not
    /// content, and only the last frame is un-continued.
    #[test]
    fn multibyte_content_across_the_cap_with_crlf_split_across_reads_keeps_characters_whole() {
        let (content, first_read, second_read) = multibyte_content_ending_in_a_held_cr();

        let frames =
            stdout_frames_for_chunks(&[first_read.as_slice(), second_read.as_slice(), b"\n"]);

        assert_whole_characters(&frames);
        assert_eq!(
            frames,
            vec![
                ("€".repeat(2730), true),
                (format!("{}ab", "€".repeat(2730)), false),
            ]
        );
        assert_eq!(
            rejoined(&frames),
            String::from_utf8(content).expect("the content is valid UTF-8"),
            "the frames must re-join to the original content, with the CRLF not content"
        );
    }

    /// R3-016 mirror: the held CR is followed by a non-LF byte (here the
    /// first byte of another `€`), so it is content: the full buffer splits
    /// at a character boundary as `continued`, and the CR travels on with
    /// the rest of the line.
    #[test]
    fn multibyte_content_with_a_held_cr_followed_by_a_non_lf_byte_keeps_the_cr_as_content() {
        let (content, first_read, second_read) = multibyte_content_ending_in_a_held_cr();

        let frames = stdout_frames_for_chunks(&[
            first_read.as_slice(),
            second_read.as_slice(),
            "€\n".as_bytes(),
        ]);

        assert_whole_characters(&frames);
        assert_eq!(
            frames,
            vec![
                ("€".repeat(2730), true),
                (format!("{}ab", "€".repeat(2730)), true),
                ("\r€".to_string(), false),
            ]
        );
        let mut expected = String::from_utf8(content).expect("the content is valid UTF-8");
        expected.push_str("\r€");
        assert_eq!(rejoined(&frames), expected);
    }

    /// R3-015: a CR held after a full buffer, then another bare CR at the
    /// start of the next read: the first CR is content (the line continues
    /// past the cap) and only the CR immediately before the eventual LF is
    /// stripped -- whether that second CR arrives together with its LF or
    /// alone in its own read.
    #[test]
    fn a_held_cr_followed_by_another_cr_keeps_the_first_as_content_and_strips_only_the_last() {
        let content = x_times(MAX_LINE_BYTES);
        let expected = vec![
            ("x".repeat(MAX_LINE_BYTES), true),
            ("\r".to_string(), false),
        ];

        let together = stdout_frames_for_chunks(&[content.as_slice(), b"\r", b"\r\n"]);
        assert_eq!(together, expected);

        let apart = stdout_frames_for_chunks(&[content.as_slice(), b"\r", b"\r", b"\n"]);
        assert_eq!(apart, expected);
    }

    /// A CR after a full buffer that is *not* followed by an LF is
    /// content, so the line continues past the cap -- whether more bytes
    /// follow, or EOF does.
    #[test]
    fn a_bare_cr_after_a_full_buffer_is_content_and_the_line_continues() {
        let content = x_times(MAX_LINE_BYTES);
        let followed = stdout_frames_for_chunks(&[content.as_slice(), b"\r", b"y\n"]);
        assert_eq!(
            followed,
            vec![
                ("x".repeat(MAX_LINE_BYTES), true),
                ("\ry".to_string(), false)
            ]
        );
        let at_eof = stdout_frames_for_chunks(&[content.as_slice(), b"\r"]);
        assert_eq!(
            at_eof,
            vec![
                ("x".repeat(MAX_LINE_BYTES), true),
                ("\r".to_string(), false)
            ]
        );
    }

    /// R3-005: the stdin-write-failure path delivers exactly one synthetic
    /// `stderr` text frame carrying the fixed text, the next `seq`, and
    /// the pending drop count -- and moves the bookkeeping on (next `seq`
    /// advanced, pending drops reset) exactly like a delivered text frame.
    #[test]
    fn emit_stdin_write_failed_delivers_one_stderr_frame_with_seq_and_dropped_before() {
        let id = ProcessId(4242);
        let (state, handles) = new_channel(2);
        // Two frames already delivered (`seq` 2 is next), three dropped
        // since the last delivery.
        state.seq.store(2, Ordering::Release);
        state.pending_drops.store(3, Ordering::Release);
        let outputs: OutputTable = Arc::new(Mutex::new(HashMap::from([(id, state)])));

        emit_stdin_write_failed(&outputs, id);

        let receiver = take_receiver(&outputs, id).expect("subscribing must succeed");
        let frame = receiver
            .try_recv()
            .expect("exactly one frame must have been delivered");
        assert_eq!(frame.seq, 2, "the frame takes the next seq");
        assert_eq!(
            frame.dropped_before, 3,
            "the frame carries the drops pending before it"
        );
        assert_eq!(
            frame.payload,
            FramePayload::Text {
                stream: OutputStream::Stderr,
                text: "stdin write failed".to_string(),
                continued: false,
            }
        );
        assert!(
            receiver.try_recv().is_err(),
            "exactly one frame, never more"
        );
        assert_eq!(handles.seq.load(Ordering::Acquire), 3);
        assert_eq!(handles.pending_drops.load(Ordering::Acquire), 0);
    }

    /// An unknown id, or a channel already finalized (its sender taken by
    /// `try_finalize`), is a silent no-op: nothing to deliver to, no panic.
    #[test]
    fn emit_stdin_write_failed_is_a_no_op_for_an_unknown_or_finalized_id() {
        let outputs: OutputTable = Arc::new(Mutex::new(HashMap::new()));
        emit_stdin_write_failed(&outputs, ProcessId(1));

        let id = ProcessId(2);
        let (mut state, handles) = new_channel(2);
        state.sender = None; // already finalized
        outputs
            .lock()
            .expect("outputs mutex must not be poisoned")
            .insert(id, state);

        emit_stdin_write_failed(&outputs, id);

        assert_eq!(
            handles.seq.load(Ordering::Acquire),
            0,
            "a finalized channel's bookkeeping must be left untouched"
        );
    }
}
