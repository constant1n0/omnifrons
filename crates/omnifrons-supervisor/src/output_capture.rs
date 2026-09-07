//! Output capture: pipes a spawned child's stdout/stderr, frames the bytes
//! into [`OutputFrame`]s bounded by [`MAX_LINE_BYTES`], and delivers them
//! through a bounded per-child channel with best-effort backpressure for
//! text (`try_send` only -- a full channel drops the newest frame and
//! counts it, never blocks the reader). The one exception is the final
//! `State` frame ([`try_finalize`]): it is guaranteed-delivered via a
//! dedicated blocking-send thread rather than dropped alongside text when
//! the channel is full, since it is the only signal a subscriber has that
//! the process is truly done.
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
//! Deliberately independent of this crate's own `Tracked` bookkeeping
//! (`docs/spike-log.md` § IPC contract): `Tracked`'s Running/Terminal
//! transition is the `ProcessSupervisor` port's own contract and must keep
//! reporting a confirmed reap immediately, regardless of whether stdout/
//! stderr have finished draining. The final `State` frame this module
//! sends therefore waits for *both* the confirmed terminal state
//! ([`record_confirmed_state`], called from `observe`/`stop`) and both
//! reader tasks finishing ([`drain_stream`]'s own bookkeeping) -- whichever
//! happens second performs the send, so the state frame is never emitted
//! ahead of output that was already read but not yet delivered.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use omnifrons_app::{FramePayload, OutputFrame, OutputStream, ProcessId, ProcessTerminalState};
use tokio::io::AsyncReadExt;

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
    /// `None` once the final `State` frame has been sent (or, on a
    /// platform where reap is never confirmed, forever `Some` -- see this
    /// module's own doc comment): dropping every sender clone is what lets
    /// a subscriber's `Receiver` iterator end.
    sender: Option<SyncSender<OutputFrame>>,
    /// Shared with both of this process's reader tasks via their own
    /// [`ReaderHandles`] clone. See [`ReaderHandles`]'s own doc comment for
    /// why plain, unlocked atomics are sound here despite `seq`'s
    /// read-then-write sequence (`try_send`) not itself being one atomic
    /// operation.
    seq: Arc<AtomicU64>,
    pending_drops: Arc<AtomicU64>,
    /// Starts at 2 (stdout, stderr); each reader task decrements it once,
    /// on EOF or a read error.
    active_readers: Arc<AtomicU8>,
    /// Set once `observe`/`stop` confirms the process's terminal state,
    /// independent of whether the readers are done yet.
    confirmed_state: Option<ProcessTerminalState>,
}

/// What each reader task holds its own clone of.
///
/// ## Why plain atomics, with no lock around `try_send`'s own critical
/// section, are sound
///
/// [`try_send`] reads `seq`/`pending_drops`, builds a frame from what it
/// read, attempts delivery, and only *then* writes `seq`/`pending_drops`
/// back -- three separate atomic operations, not one, with no `.await`
/// between them. A genuinely concurrent second caller (this process's
/// *other* stream's reader task, the only other code that ever calls
/// `try_send` for the same [`OutputChannelState`]) interleaving between
/// that read and that write would let both readers capture the same `seq`
/// value and emit two frames claiming it -- silently breaking the
/// contiguous-per-process-`seq` guarantee `docs/spike-log.md` § IPC
/// contract makes.
///
/// This is safe *only* because `TokioProcessSupervisor` always builds a
/// `new_current_thread()` runtime, driven by exactly one dedicated OS
/// thread (`TokioProcessSupervisor::build`'s own doc comment). A process's
/// stdout and stderr reader tasks are both spawned onto that one runtime,
/// so at most one of them is ever actually *executing* at a given instant
/// -- Tokio only switches between them at an `.await` point, and
/// `try_send`'s own critical section contains none. If this runtime were
/// ever built as a genuine multi-thread runtime instead, the two reader
/// tasks could run on separate OS threads in true parallel, and this
/// reasoning would no longer hold.
/// `tests/runtime_drives_tasks.rs`'s `the_supervisors_runtime_is_current_thread_flavor`
/// test stands guard on that assumption directly.
#[derive(Clone)]
pub(crate) struct ReaderHandles {
    sender: SyncSender<OutputFrame>,
    seq: Arc<AtomicU64>,
    pending_drops: Arc<AtomicU64>,
    active_readers: Arc<AtomicU8>,
}

/// Build a fresh channel and bookkeeping for a newly spawned child, and the
/// [`ReaderHandles`] template to clone once per stream.
pub(crate) fn new_channel() -> (OutputChannelState, ReaderHandles) {
    let (sender, receiver) = sync_channel(CHANNEL_CAPACITY);
    let seq = Arc::new(AtomicU64::new(0));
    let pending_drops = Arc::new(AtomicU64::new(0));
    let active_readers = Arc::new(AtomicU8::new(2));

    let handles = ReaderHandles {
        sender: sender.clone(),
        seq: Arc::clone(&seq),
        pending_drops: Arc::clone(&pending_drops),
        active_readers: Arc::clone(&active_readers),
    };
    let state = OutputChannelState {
        receiver: Some(receiver),
        sender: Some(sender),
        seq,
        pending_drops,
        active_readers,
        confirmed_state: None,
    };
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
/// EOF is reached, not dropped for lacking a terminator -- then marks this
/// stream done and, if it was the last of the two, asks [`try_finalize`] to
/// send the final state frame (a no-op if the terminal state is not
/// confirmed yet).
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

    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
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

    if handles.active_readers.fetch_sub(1, Ordering::AcqRel) == 1 {
        try_finalize(&outputs, id);
    }
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

/// Frame and best-effort-deliver one decoded text chunk. `continued` is
/// `true` only for a chunk that the per-frame cap forced to end early with
/// more of the same logical line still to come (see [`drain_stream`]).
fn emit_text(handles: &ReaderHandles, stream: OutputStream, bytes: &[u8], continued: bool) {
    let text = String::from_utf8_lossy(bytes).into_owned();
    try_send(
        handles,
        FramePayload::Text {
            stream,
            text,
            continued,
        },
    );
}

/// Best-effort delivery shared by text frames and (via [`try_finalize`])
/// the final state frame: `try_send` only. A full channel increments the
/// pending-drop count, surfaced as `dropped_before` on the next frame that
/// does get through, rather than blocking the caller.
fn try_send(handles: &ReaderHandles, payload: FramePayload) {
    let seq = handles.seq.load(Ordering::Acquire);
    let dropped_before = handles.pending_drops.load(Ordering::Acquire);
    let frame = OutputFrame::with_dropped_before(seq, dropped_before, payload);
    match handles.sender.try_send(frame) {
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
    let mut guard = outputs
        .lock()
        .expect("outputs mutex poisoned by a prior panic");
    let Some(entry) = guard.get_mut(&id) else {
        return;
    };
    let Some(sender) = entry.sender.clone() else {
        return; // already finalized; nothing left to deliver to
    };
    let seq = entry.seq.fetch_add(1, Ordering::AcqRel);
    let dropped_before = entry.pending_drops.swap(0, Ordering::AcqRel);
    drop(guard);

    let frame = OutputFrame::with_dropped_before(
        seq,
        dropped_before,
        FramePayload::Text {
            stream: OutputStream::Stderr,
            text: "stdin write failed".to_string(),
            continued: false,
        },
    );
    let _ = sender.try_send(frame);
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
    {
        let mut guard = outputs
            .lock()
            .expect("outputs mutex poisoned by a prior panic");
        let Some(entry) = guard.get_mut(&id) else {
            return;
        };
        entry.confirmed_state = Some(state);
    }
    try_finalize(outputs, id);
}

/// Send the final `State` frame and drop this channel's sender, but only
/// once the process's terminal state is confirmed *and* both reader tasks
/// have finished -- whichever of the two happens second performs the send.
/// A no-op if either condition is not yet met, or if this channel was
/// already finalized.
///
/// Unlike a text frame (`try_send`, best-effort, `emit_text`/`try_send`
/// above), the terminal frame is guaranteed-delivered: it is the one signal
/// a subscriber needs to know the process is truly done, so it must reach
/// them even against a delivery channel that is completely full and
/// undrained at this exact moment. Delivery is handed to a dedicated
/// `std::thread` doing a blocking [`SyncSender::send`], which ends once the
/// frame is delivered (room eventually frees up, since no further text
/// frames are produced once this point is reached) or the subscriber drops
/// its receiver (the `send` then fails harmlessly) -- never on the async
/// runtime, which must never block on a slow or absent consumer.
///
/// Also removes `id`'s whole entry from `outputs` if it is already
/// subscribed-out (`receiver` already taken) at this point: finalized and
/// subscribed-out are exactly the two conditions [`OutputChannelState`]'s
/// own doc comment names for reclaiming it. If nobody has subscribed yet,
/// the entry (and its buffered channel, including the frame this call is
/// about to hand to the delivery thread) is left in place, so a later
/// `subscribe` still finds it.
fn try_finalize(outputs: &OutputTable, id: ProcessId) {
    let (sender, frame) = {
        let mut guard = outputs
            .lock()
            .expect("outputs mutex poisoned by a prior panic");
        let Some(entry) = guard.get_mut(&id) else {
            return;
        };
        if entry.sender.is_none() {
            return; // already finalized
        }
        if entry.active_readers.load(Ordering::Acquire) != 0 {
            return; // one or both reader tasks still running
        }
        let Some(state) = entry.confirmed_state else {
            return; // reap not confirmed yet
        };

        let seq = entry.seq.fetch_add(1, Ordering::AcqRel);
        let dropped_before = entry.pending_drops.swap(0, Ordering::AcqRel);
        let frame =
            OutputFrame::with_dropped_before(seq, dropped_before, FramePayload::State(state));
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
        (sender, frame)
    };

    if let Err(error) = std::thread::Builder::new()
        .name("omnifrons-output-state-frame".to_string())
        .spawn(move || {
            let _ = sender.send(frame);
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

    use omnifrons_app::{FramePayload, OutputStream, ProcessId};
    use tokio::io::{AsyncRead, ReadBuf};

    use super::{
        MAX_LINE_BYTES, OutputTable, drain_stream, emit_stdin_write_failed, new_channel,
        take_receiver,
    };

    /// An `AsyncRead` handing out exactly one predefined chunk per read
    /// call, then EOF -- so a test controls precisely where a line's bytes
    /// fall relative to read boundaries, which a real pipe never lets it.
    struct ChunkedReader {
        chunks: VecDeque<Vec<u8>>,
    }

    impl ChunkedReader {
        fn new(chunks: &[&[u8]]) -> Self {
            Self {
                chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
            }
        }
    }

    impl AsyncRead for ChunkedReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if let Some(chunk) = self.get_mut().chunks.pop_front() {
                assert!(
                    chunk.len() <= buf.remaining(),
                    "a test chunk must fit drain_stream's own read buffer"
                );
                buf.put_slice(&chunk);
            }
            Poll::Ready(Ok(()))
        }
    }

    /// Run `drain_stream` over `chunks` as stdout and return every text
    /// frame's `(text, continued)`, in order.
    fn stdout_frames_for_chunks(chunks: &[&[u8]]) -> Vec<(String, bool)> {
        let id = ProcessId(7);
        let (state, handles) = new_channel();
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

    fn x_times(count: usize) -> Vec<u8> {
        vec![b'x'; count]
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
        let (state, handles) = new_channel();
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
        let (mut state, handles) = new_channel();
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
