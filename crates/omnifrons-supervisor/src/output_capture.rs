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
/// split into consecutive frames instead of growing one frame unbounded.
pub(crate) const MAX_LINE_BYTES: usize = 8192;

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

    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let mut start = 0;
        while start < read {
            let space = MAX_LINE_BYTES - buf.len();
            let window_end = start + space.min(read - start);
            if let Some(relative_newline) =
                chunk[start..window_end].iter().position(|&b| b == b'\n')
            {
                buf.extend_from_slice(&chunk[start..start + relative_newline]);
                strip_trailing_cr(&mut buf);
                emit_text(&handles, stream, &buf);
                buf.clear();
                start += relative_newline + 1;
            } else if window_end - start == space {
                // The MAX_LINE_BYTES cap was reached with no newline in
                // sight: split here, but never mid-character. Extend buf to
                // the cap, then back off to the last complete UTF-8
                // character boundary within it -- any trailing partial-
                // character bytes are carried into the next buf instead of
                // being emitted (and thereby corrupted into a replacement
                // character) now.
                buf.extend_from_slice(&chunk[start..window_end]);
                let boundary = utf8_safe_boundary(&buf);
                let carry_over = buf.split_off(boundary);
                emit_text(&handles, stream, &buf);
                buf = carry_over;
                start = window_end;
            } else {
                buf.extend_from_slice(&chunk[start..read]);
                start = read;
            }
        }
    }
    if !buf.is_empty() {
        emit_text(&handles, stream, &buf);
    }

    if handles.active_readers.fetch_sub(1, Ordering::AcqRel) == 1 {
        try_finalize(&outputs, id);
    }
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

/// Frame and best-effort-deliver one decoded text chunk.
fn emit_text(handles: &ReaderHandles, stream: OutputStream, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes).into_owned();
    try_send(handles, FramePayload::Text { stream, text });
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
