//! Pseudo-terminal plumbing for the `TransportClass::Pty` launch path
//! (spike slice 4, `docs/spike-log.md` § Slice 4): open a master/slave
//! pair, wire the slave as the child's stdin, stdout, and stderr with the
//! child as the session leader of that controlling terminal, read the
//! master asynchronously as the launch's one `stdout` stream, and type the
//! prompt into it.
//!
//! Unix only in this slice (D2 of the brief): `openpty`, `setsid`, and
//! `TIOCSCTTY` are POSIX pseudo-terminal APIs `nix` exposes on both Linux
//! and macOS; Windows has no equivalent here (`ConPTY` is recorded as debt
//! next to the Job Object debt), so `spawn_approved` refuses a `Pty` plan
//! there with `SupervisorError::PtyUnsupported` before touching anything.
//!
//! A terminal is never a sandbox and never authorization
//! (`docs/threat-model.md` HAR-5, PRC-2): everything this module does is
//! process wiring, and every byte read from the master is untrusted
//! active content the core normalizes before a renderer sees it.
//!
//! This module holds this crate's second (and only other) explicit
//! `#[allow(unsafe_code)]`, isolated in [`install_controlling_terminal`]
//! with a `SAFETY` comment: `tokio::process::Command::pre_exec` is
//! `unsafe fn`, and the closure it runs between `fork` and `exec` may only
//! make async-signal-safe calls.

use std::io;
use std::os::fd::{AsFd as _, OwnedFd};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Weak};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
use nix::libc;
use nix::pty::{OpenptyResult, Winsize, openpty};
use nix::sys::termios::Termios;
use omnifrons_app::{ProcessId, SupervisorError};
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use crate::SendInputError;
use crate::output_capture::{self, OutputTable};

/// The fixed window size every PTY child starts with (spike default:
/// resizing is a control write path, deferred).
const WINSIZE: Winsize = Winsize {
    ws_row: 24,
    ws_col: 80,
    ws_xpixel: 0,
    ws_ypixel: 0,
};

/// The environment the supervisor sets explicitly on every PTY child --
/// never read from its own environment. `TERM=dumb` asks a well-behaved
/// program for the fewest escape sequences it can manage, which is what a
/// plain-text rendering with layout controls dropped wants; `COLUMNS` and
/// `LINES` restate [`WINSIZE`] for programs that read them instead of
/// `TIOCGWINSZ`.
pub(crate) const CHILD_ENV: [(&str, &str); 3] =
    [("TERM", "dumb"), ("COLUMNS", "80"), ("LINES", "24")];

/// The master side, registered with the supervisor's runtime and shared
/// by the one reader task and the prompt-typing task: `AsyncFd` polls
/// readiness through `&self`, so one registration serves both.
pub(crate) type Master = Arc<AsyncFd<OwnedFd>>;

/// How long one forwarded input may take, from the call to its last byte:
/// waiting for its turn behind the typed prompt or another input, then
/// writing. A terminal whose child stops reading fills its input queue and
/// never becomes writable again; past this wait the write is abandoned
/// rather than left pending.
pub(crate) const INPUT_WRITE_WAIT: Duration = Duration::from_secs(2);

/// One run's terminal as the supervisor's registry holds it: a `Weak`
/// reference to the master, never a strong one, so the master still
/// closes when its reader task drops it at end of input
/// (`docs/spike-log.md`: "the master is closed once the terminal `state`
/// frame arrives"), plus the run's write lock. Every write to the
/// terminal after spawn -- the typed prompt and each forwarded input --
/// holds that lock for its whole duration, so no two of them interleave.
pub(crate) struct TerminalEntry {
    master: Weak<AsyncFd<OwnedFd>>,
    turn: Arc<AsyncMutex<()>>,
}

impl TerminalEntry {
    /// A fresh entry for `master`, with the first turn of its write lock
    /// already taken and returned: the prompt typist holds it from before
    /// the entry is ever reachable, so no forwarded input can be written
    /// ahead of the prompt. Dropped at once when there is no prompt.
    pub(crate) fn new(master: &Master) -> (Self, OwnedMutexGuard<()>) {
        let turn = Arc::new(AsyncMutex::new(()));
        let first_turn = Arc::clone(&turn)
            .try_lock_owned()
            .expect("a write lock nobody else has seen yet is free");
        let entry = Self {
            master: Arc::downgrade(master),
            turn,
        };
        (entry, first_turn)
    }

    /// The live master and the write lock, or `None` once the master was
    /// closed.
    pub(crate) fn upgrade(&self) -> Option<(Master, Arc<AsyncMutex<()>>)> {
        let master = self.master.upgrade()?;
        Some((master, Arc::clone(&self.turn)))
    }

    /// Whether the master is still open somewhere (a reader, the prompt
    /// typist, or an input being written).
    pub(crate) fn is_live(&self) -> bool {
        self.master.strong_count() > 0
    }
}

/// An opened master/slave pair, not yet wired to a child.
pub(crate) struct PtyPair {
    master: OwnedFd,
    slave: OwnedFd,
}

fn spawn_error(context: &str, error: impl std::fmt::Display) -> SupervisorError {
    SupervisorError::Spawn(format!("{context}: {error}"))
}

/// Open a fresh pair at [`WINSIZE`] with default terminal settings. Both
/// ends are marked close-on-exec before this returns: `openpty` hands them
/// back without it, so that step through both `fcntl(F_SETFD, FD_CLOEXEC)`
/// calls runs under [`crate::spawn_lock`] (see its doc for why). The
/// master is switched to non-blocking afterward, needing no lock.
///
/// # Errors
///
/// Returns [`SupervisorError::Spawn`] if the pair could not be opened or
/// configured.
pub(crate) fn open_pair() -> Result<PtyPair, SupervisorError> {
    let guard = crate::spawn_lock();
    let OpenptyResult { master, slave } =
        openpty(&WINSIZE, None::<&Termios>).map_err(|error| spawn_error("openpty", error))?;
    for fd in [&master, &slave] {
        fcntl(fd.as_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
            .map_err(|error| spawn_error("fcntl(F_SETFD, FD_CLOEXEC) on the pty", error))?;
    }
    drop(guard);
    let status = fcntl(master.as_fd(), FcntlArg::F_GETFL)
        .map_err(|error| spawn_error("fcntl(F_GETFL) on the pty master", error))?;
    fcntl(
        master.as_fd(),
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(status) | OFlag::O_NONBLOCK),
    )
    .map_err(|error| spawn_error("fcntl(F_SETFL, O_NONBLOCK) on the pty master", error))?;
    Ok(PtyPair { master, slave })
}

/// Wire `pair` into `command`: the slave becomes the child's stdin, stdout,
/// and stderr, the child's environment gains [`CHILD_ENV`], the child is
/// made the session leader of that controlling terminal, and the master is
/// registered with the current runtime. The parent's own slave descriptor
/// is released here; `command` keeps its three copies until it is dropped
/// right after `spawn`, at which point the child holds the only remaining
/// ones and the master reports EOF once the child exits.
///
/// Must be called inside the supervisor's runtime context (`AsyncFd::new`
/// registers with the current reactor).
///
/// # Errors
///
/// Returns [`SupervisorError::Spawn`] if the slave could not be duplicated
/// or the master could not be registered.
pub(crate) fn prepare_child(
    command: &mut Command,
    pair: PtyPair,
) -> Result<Master, SupervisorError> {
    let PtyPair { master, slave } = pair;
    let stdin = slave
        .try_clone()
        .map_err(|error| spawn_error("duplicating the pty slave", error))?;
    let stdout = slave
        .try_clone()
        .map_err(|error| spawn_error("duplicating the pty slave", error))?;
    command.stdin(Stdio::from(stdin));
    command.stdout(Stdio::from(stdout));
    command.stderr(Stdio::from(slave));
    for (key, value) in CHILD_ENV {
        command.env(key, value);
    }
    install_controlling_terminal(command);
    let master = AsyncFd::new(master)
        .map_err(|error| spawn_error("registering the pty master with the runtime", error))?;
    Ok(Arc::new(master))
}

/// `TIOCSCTTY`'s request argument in the exact type this platform's
/// `ioctl` declares: `libc::Ioctl` (`c_ulong` on glibc, `c_int` on musl)
/// on Linux, where the constant already has that type.
#[cfg(target_os = "linux")]
fn tiocsctty_request() -> libc::Ioctl {
    libc::TIOCSCTTY
}

/// On macOS `libc` declares `TIOCSCTTY` as `c_uint` while `ioctl` takes a
/// `c_ulong` request (verified against the vendored libc 0.2.189 source:
/// `src/new/apple/xnu/sys/ttycom.rs` and `src/unix/bsd/mod.rs`), so the
/// constant is widened losslessly here rather than cast.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn tiocsctty_request() -> libc::c_ulong {
    libc::c_ulong::from(libc::TIOCSCTTY)
}

/// The BSDs declare both the constant and the request as `c_ulong`. Not a
/// pinned baseline of this project; kept so the module stays buildable
/// there.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios")))]
fn tiocsctty_request() -> libc::c_ulong {
    libc::TIOCSCTTY
}

/// Make the child the session leader of its own new session and give it
/// the terminal on its stdin as controlling terminal, between `fork` and
/// `exec`.
///
/// `setsid` also makes the child its own process-group leader, so the
/// crate's existing stop path (`killpg` on the child's pid, then SIGKILL
/// after the deadline) keeps working without `process_group(0)` -- which
/// must not be called on this path: a process that is already a group
/// leader cannot `setsid`.
///
/// This is the one place this crate adds `unsafe` in spike slice 4,
/// matching the workspace's `unsafe_code = "deny"` default everywhere
/// else, with the demo harness's SIGTERM-ignore as the precedent.
#[allow(unsafe_code)]
fn install_controlling_terminal(command: &mut Command) {
    // SAFETY: `pre_exec` runs this closure in the forked child before
    // `exec`, where only async-signal-safe calls are permitted (a
    // multi-threaded parent's locks may be held by threads that no longer
    // exist in the child). The closure makes exactly two calls, both on
    // POSIX's async-signal-safe list: `setsid(2)` (through `nix`'s thin
    // wrapper, which only reads `errno`) and `ioctl(2)` with `TIOCSCTTY`
    // on fd 0, which `Command` has already redirected to the pty slave.
    // Neither allocates, takes a lock, or touches any Rust runtime state;
    // an error is reported through `io::Error::last_os_error`/
    // `from_raw_os_error`, which build the `Os` variant without
    // allocating. Descriptor 0 is the slave by construction
    // (`prepare_child` sets it before this is installed).
    unsafe {
        command.pre_exec(|| {
            nix::unistd::setsid()?;
            if libc::ioctl(0, tiocsctty_request(), 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// The master side as an `AsyncRead`, for `output_capture::drain_stream`:
/// a read that fails with `EIO` -- what Linux reports on the master once
/// every slave descriptor is closed -- is end of input, exactly like the
/// zero-byte read macOS reports in the same situation.
pub(crate) struct MasterReader {
    master: Master,
}

impl MasterReader {
    pub(crate) fn new(master: Master) -> Self {
        Self { master }
    }
}

impl AsyncRead for MasterReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            let mut guard = ready!(this.master.poll_read_ready(cx))?;
            let unfilled = buf.initialize_unfilled();
            match guard.try_io(|inner| {
                nix::unistd::read(inner.get_ref(), unfilled).map_err(io::Error::from)
            }) {
                Ok(Ok(read)) => {
                    buf.advance(read);
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(error)) if error.raw_os_error() == Some(libc::EIO) => {
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(error)) => return Poll::Ready(Err(error)),
                Err(_would_block) => {}
            }
        }
    }
}

/// Type `prompt` into the terminal, followed by a carriage return (the
/// line discipline's end of line), on the supervisor's own runtime,
/// holding `turn` -- the run's write lock, taken before this task was
/// spawned ([`TerminalEntry::new`]) -- for the whole write, so a forwarded
/// input never interleaves with or precedes the prompt. A write failure
/// -- the child exited before reading, the terminal went away -- surfaces
/// exactly like a stdin-pipe write failure
/// (`output_capture::emit_stdin_write_failed`, the same synthetic frame),
/// never as a hang.
pub(crate) async fn type_prompt(
    master: Master,
    prompt: Vec<u8>,
    outputs: OutputTable,
    id: ProcessId,
    turn: OwnedMutexGuard<()>,
) {
    let mut bytes = prompt;
    bytes.push(b'\r');
    if write_all(&master, &bytes).await.is_err() {
        output_capture::emit_stdin_write_failed(&outputs, id);
    }
    drop(turn);
}

/// Write `bytes` to `master` on the supervisor's own runtime, holding the
/// run's write lock `turn` for the whole write, all within
/// [`INPUT_WRITE_WAIT`]. Unlike [`type_prompt`] it appends nothing and
/// emits no synthetic frame: the caller gets the typed error instead. On
/// timeout the pending write is dropped before this returns, so nothing
/// more is written after the error.
pub(crate) async fn forward_input(
    master: Master,
    turn: Arc<AsyncMutex<()>>,
    bytes: Vec<u8>,
) -> Result<(), SendInputError> {
    let write = async {
        let _turn = turn.lock().await;
        write_all(&master, &bytes).await
    };
    match tokio::time::timeout(INPUT_WRITE_WAIT, write).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(SendInputError::WriteFailed),
    }
}

/// Write every byte of `bytes` to `master`, waiting for writability as
/// often as the terminal asks. A readiness error, a write error, or a
/// zero-byte write (no progress possible) ends it with an error.
async fn write_all(master: &AsyncFd<OwnedFd>, bytes: &[u8]) -> io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let mut guard = master.writable().await?;
        match guard.try_io(|inner| {
            nix::unistd::write(inner.get_ref(), &bytes[written..]).map_err(io::Error::from)
        }) {
            Ok(Ok(0)) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(Ok(count)) => written += count,
            Ok(Err(error)) => return Err(error),
            Err(_would_block) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::fd::OwnedFd;
    use std::sync::{Arc, Mutex};

    use omnifrons_app::{FramePayload, OutputStream, ProcessId};
    use tokio::io::AsyncReadExt as _;
    use tokio::io::unix::AsyncFd;

    use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

    use super::{INPUT_WRITE_WAIT, MasterReader, forward_input, open_pair, type_prompt};
    use crate::SendInputError;
    use crate::output_capture::{OutputTable, new_channel, take_receiver};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime must build")
    }

    /// A held turn of a write lock nobody else shares.
    fn own_turn() -> OwnedMutexGuard<()> {
        Arc::new(AsyncMutex::new(()))
            .try_lock_owned()
            .expect("a fresh write lock is free")
    }

    /// Reading the master after every slave descriptor is gone is end of
    /// input: `Ok(0)` -- Linux's `EIO` mapped, macOS's own zero-byte read
    /// passed through -- never an error that would make the reader task
    /// log a failure.
    #[test]
    fn master_read_after_the_slave_is_closed_is_eof() {
        let pair = open_pair().expect("openpty must succeed");
        drop(pair.slave);
        let runtime = runtime();
        let read = runtime.block_on(async move {
            let master = Arc::new(AsyncFd::new(pair.master).expect("register the master"));
            let mut reader = MasterReader::new(master);
            let mut buf = [0u8; 16];
            reader.read(&mut buf).await
        });
        assert!(matches!(read, Ok(0)), "expected EOF, got {read:?}");
    }

    /// The typed prompt reaches the write target followed by exactly one
    /// carriage return.
    #[test]
    fn type_prompt_writes_the_prompt_then_a_carriage_return() {
        let (reader, writer) = std::io::pipe().expect("a pipe must open");
        let id = ProcessId(11);
        let (state, _handles) = new_channel(1);
        let outputs: OutputTable = Arc::new(Mutex::new(HashMap::from([(id, state)])));
        let runtime = runtime();
        runtime.block_on(async move {
            let master =
                Arc::new(AsyncFd::new(OwnedFd::from(writer)).expect("register the writer"));
            type_prompt(master, b"hello".to_vec(), outputs, id, own_turn()).await;
        });
        let mut received = Vec::new();
        std::io::Read::read_to_end(&mut &reader, &mut received).expect("read the pipe");
        assert_eq!(received, b"hello\r");
    }

    /// A write failure surfaces as the same synthetic `stderr` frame the
    /// stdin-pipe path emits (`"stdin write failed"`), not a hang and not
    /// silence: here the "master" is a pipe's write end whose read end is
    /// already closed, so the reactor reports it ready and the write fails
    /// with `EPIPE` deterministically (a Rust test binary ignores
    /// `SIGPIPE`, so the failure is an error return, never a signal). A
    /// read end would be the wrong fixture: a read-only descriptor never
    /// becomes writable, and `type_prompt` would wait forever.
    #[test]
    fn type_prompt_write_failure_emits_the_stdin_write_failed_frame() {
        let (reader, writer) = std::io::pipe().expect("a pipe must open");
        drop(reader);
        let id = ProcessId(12);
        let (state, _handles) = new_channel(1);
        let outputs: OutputTable = Arc::new(Mutex::new(HashMap::from([(id, state)])));
        let runtime = runtime();
        runtime.block_on({
            let outputs = Arc::clone(&outputs);
            async move {
                let master =
                    Arc::new(AsyncFd::new(OwnedFd::from(writer)).expect("register the write end"));
                type_prompt(master, b"hello".to_vec(), outputs, id, own_turn()).await;
            }
        });
        let receiver = take_receiver(&outputs, id).expect("subscribing must succeed");
        let frame = receiver
            .try_recv()
            .expect("exactly one frame must have been delivered");
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
    }

    /// A forwarded input is written exactly as given: unlike the typed
    /// prompt, nothing is appended.
    #[test]
    fn forward_input_writes_exactly_its_bytes() {
        let (reader, writer) = std::io::pipe().expect("a pipe must open");
        let runtime = runtime();
        let result = runtime.block_on(async move {
            let master =
                Arc::new(AsyncFd::new(OwnedFd::from(writer)).expect("register the writer"));
            let turn = Arc::new(AsyncMutex::new(()));
            forward_input(master, turn, b"\x1b[A".to_vec()).await
        });
        assert_eq!(result, Ok(()));
        let mut received = Vec::new();
        std::io::Read::read_to_end(&mut &reader, &mut received).expect("read the pipe");
        assert_eq!(received, b"\x1b[A");
    }

    /// A write error is the caller's typed `WriteFailed` (the same `EPIPE`
    /// fixture as the prompt's failure test above).
    #[test]
    fn forward_input_write_failure_is_write_failed() {
        let (reader, writer) = std::io::pipe().expect("a pipe must open");
        drop(reader);
        let runtime = runtime();
        let result = runtime.block_on(async move {
            let master =
                Arc::new(AsyncFd::new(OwnedFd::from(writer)).expect("register the write end"));
            forward_input(master, Arc::new(AsyncMutex::new(())), b"x".to_vec()).await
        });
        assert_eq!(result, Err(SendInputError::WriteFailed));
    }

    /// An input that cannot get its turn -- here the run's write lock is
    /// held throughout, as a prompt stuck on a child that never reads would
    /// hold it -- gives up with `WriteFailed` after the bounded wait, and
    /// is never written once the lock is released afterwards.
    #[test]
    fn forward_input_gives_up_after_the_bounded_wait_and_never_writes_later() {
        let (reader, writer) = std::io::pipe().expect("a pipe must open");
        let runtime = runtime();
        let (result, elapsed) = runtime.block_on(async move {
            let master =
                Arc::new(AsyncFd::new(OwnedFd::from(writer)).expect("register the writer"));
            let turn = Arc::new(AsyncMutex::new(()));
            let held = Arc::clone(&turn).lock_owned().await;
            let started = std::time::Instant::now();
            let result = forward_input(Arc::clone(&master), turn, b"late".to_vec()).await;
            let elapsed = started.elapsed();
            drop(held);
            for _ in 0..8 {
                tokio::task::yield_now().await;
            }
            (result, elapsed)
        });
        assert_eq!(result, Err(SendInputError::WriteFailed));
        assert!(elapsed >= INPUT_WRITE_WAIT, "gave up after {elapsed:?}");
        // Every writer is gone with the runtime's future: end of input.
        let mut received = Vec::new();
        std::io::Read::read_to_end(&mut &reader, &mut received).expect("read the pipe");
        assert!(
            received.is_empty(),
            "nothing may be written, got {received:?}"
        );
    }
}
