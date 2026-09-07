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
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
use nix::libc;
use nix::pty::{OpenptyResult, Winsize, openpty};
use nix::sys::termios::Termios;
use omnifrons_app::{ProcessId, SupervisorError};
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::process::Command;

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

/// An opened master/slave pair, not yet wired to a child.
pub(crate) struct PtyPair {
    master: OwnedFd,
    slave: OwnedFd,
}

fn spawn_error(context: &str, error: impl std::fmt::Display) -> SupervisorError {
    SupervisorError::Spawn(format!("{context}: {error}"))
}

/// Open a fresh pair at [`WINSIZE`] with default terminal settings. Both
/// ends are marked close-on-exec: the master must never leak into the
/// child (or any other exec), and the slave is only ever handed to the
/// child by `dup2` onto 0/1/2, which clears the flag on those copies. The
/// master is also switched to non-blocking, which `AsyncFd` requires.
///
/// # Errors
///
/// Returns [`SupervisorError::Spawn`] if the pair could not be opened or
/// configured.
pub(crate) fn open_pair() -> Result<PtyPair, SupervisorError> {
    let OpenptyResult { master, slave } =
        openpty(&WINSIZE, None::<&Termios>).map_err(|error| spawn_error("openpty", error))?;
    for fd in [&master, &slave] {
        fcntl(fd.as_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
            .map_err(|error| spawn_error("fcntl(F_SETFD, FD_CLOEXEC) on the pty", error))?;
    }
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
/// line discipline's end of line), on the supervisor's own runtime. A
/// write failure -- the child exited before reading, the terminal went
/// away -- surfaces exactly like a stdin-pipe write failure
/// (`output_capture::emit_stdin_write_failed`, the same synthetic frame),
/// never as a hang.
pub(crate) async fn type_prompt(
    master: Master,
    prompt: Vec<u8>,
    outputs: OutputTable,
    id: ProcessId,
) {
    let mut bytes = prompt;
    bytes.push(b'\r');
    let mut written = 0;
    while written < bytes.len() {
        let Ok(mut guard) = master.writable().await else {
            output_capture::emit_stdin_write_failed(&outputs, id);
            return;
        };
        match guard.try_io(|inner| {
            nix::unistd::write(inner.get_ref(), &bytes[written..]).map_err(io::Error::from)
        }) {
            Ok(Ok(count)) if count > 0 => written += count,
            Ok(_) => {
                output_capture::emit_stdin_write_failed(&outputs, id);
                return;
            }
            Err(_would_block) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::fd::OwnedFd;
    use std::sync::{Arc, Mutex};

    use omnifrons_app::{FramePayload, OutputStream, ProcessId};
    use tokio::io::AsyncReadExt as _;
    use tokio::io::unix::AsyncFd;

    use super::{MasterReader, open_pair, type_prompt};
    use crate::output_capture::{OutputTable, new_channel, take_receiver};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime must build")
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
            type_prompt(master, b"hello".to_vec(), outputs, id).await;
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
                type_prompt(master, b"hello".to_vec(), outputs, id).await;
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
}
