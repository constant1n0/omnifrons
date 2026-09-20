//! A deterministic OS-level *measurement probe* that also pins one result.
//!
//! `tests/pty_launch.rs`'s
//! `observed_env_is_exactly_the_base_allowlist_plus_term_columns_lines`
//! ("the report must carry an env-keys line") fails intermittently on
//! macOS CI, never on Linux. This measures what each platform does with
//! bytes a child wrote to a pty slave just before exiting when the master
//! is read only afterwards. Measured on macOS CI: with a controlling
//! terminal nothing is lost -- the child's exit waits until the master has
//! drained the output, and the child cannot be reaped before that (a
//! blocking `wait` there hung this probe's first run until the job timed
//! out); without one, all 64 bytes were lost. Linux loses nothing either
//! way. So that failure is not kernel-side loss. The setup re-creates
//! `src/pty.rs`'s own, whose pieces are `pub(crate)`.
//!
//! Four arms: a control reading promptly while the child lingers; the
//! product's own shape, read late; the same with the parent holding one
//! slave open; and the same with no controlling terminal. The first two
//! assert all 64 bytes -- the product relies on the second -- and the
//! other two only record: what they show differs by platform.
//!
//! Each arm writes raw to fd 2 (`print!`/`eprintln!` are swallowed for a
//! passing test without `--nocapture`): a `phase=begin` marker, then
//! `pty-exit-drain-probe arm=<name> os=<os> written=64 received=<n>
//! end=<eof-zero|eof-eio|idle-timeout|errno-*> reaped=<stage>
//! released=<stage>`. `reaped` is when the child became reapable,
//! `released` when the parent's close of its own slave copies returned:
//! `prompt` (before the master was read), `after-drain`,
//! `after-master-close`, or `never`; neither is asserted. Linux shows
//! `reaped=prompt` for the late arms, macOS `reaped=after-drain` with a
//! controlling terminal. `grep 'pty-exit-drain-probe' <log>`.
//!
//! The arms run one at a time (see [`ARM_SERIAL`]), nothing here blocks
//! without a bound, and a watchdog names the arm and phase if one does.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::io;
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{FcntlArg, FdFlag, OFlag, fcntl};
use nix::libc;
use nix::pty::{OpenptyResult, Winsize, openpty};
use nix::sys::termios::Termios;
use nix::unistd::{read, write};

/// Exactly 64 ASCII bytes, no newline (the pty would turn it into `\r\n`).
/// The array-length annotation makes a wrong count a compile error.
const PAYLOAD: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz01";

/// Serializes the arms. `openpty` cannot return close-on-exec descriptors
/// atomically, and `libtest` runs the four arms on parallel threads of one
/// process: an arm forking in the window before another arm's
/// `set_cloexec` would hand its child that arm's slave, silently turning a
/// hazard arm into the mitigation arm (seen on Linux, 2 of 30 runs).
static ARM_SERIAL: Mutex<()> = Mutex::new(());
/// Where the running arm is, for the watchdog's report.
static PHASE: Mutex<&'static str> = Mutex::new("idle");

/// How long the master may yield nothing before the drain loop calls it
/// idle (`end=idle-timeout`, normal while a slave is held open). Above the
/// control child's 2 s linger; a slow runner must not fake a loss.
const IDLE_LIMIT: Duration = Duration::from_secs(3);
/// Pause between polls that found nothing.
const RETRY: Duration = Duration::from_millis(10);
/// How long every arm waits after the first stage before reading.
const LATE_READ_DELAY: Duration = Duration::from_millis(300);
/// How long each stage gives the child to become reapable and the release
/// to return. Always polled, never a blocking `wait` or `join`, not even
/// after `kill`: on macOS that can wait forever.
const STAGE_WAIT: Duration = Duration::from_secs(5);
/// An arm still running after this long is named and the process exits.
/// Above the worst case of every stage timing out (about 30 s).
const WATCHDOG: Duration = Duration::from_secs(40);

/// One arm: controlling terminal or not (`setsid` + `TIOCSCTTY`), exit
/// right after writing or linger, and whether the parent holds a slave.
struct ArmConfig {
    name: &'static str,
    controlling_terminal: bool,
    exit_immediately: bool,
    hold_extra_slave: bool,
}

/// What one arm observed: the spawned pid (proof a child ran), the bytes
/// the master yielded, and how the drain loop ended.
struct Observation {
    pid: u32,
    received: Vec<u8>,
    end: String,
}

/// `TIOCSCTTY`'s request argument in the exact type this platform's
/// `ioctl` declares. Mirrors `src/pty.rs`'s `pub(crate) tiocsctty_request`,
/// duplicated because that function is unreachable here.
#[cfg(target_os = "linux")]
fn tiocsctty_request() -> libc::Ioctl {
    libc::TIOCSCTTY
}
#[cfg(target_os = "macos")]
fn tiocsctty_request() -> libc::c_ulong {
    libc::c_ulong::from(libc::TIOCSCTTY)
}

/// Mirrors `src/pty.rs`'s `pub(crate) install_controlling_terminal`
/// one-for-one, duplicated for the same reason.
#[allow(unsafe_code)]
fn install_controlling_terminal(command: &mut Command) {
    // SAFETY: `pre_exec` runs this closure in the forked child before
    // `exec`, where only async-signal-safe calls are permitted. It makes
    // exactly two: `setsid(2)` through `nix`'s thin wrapper (reads only
    // `errno`), and `ioctl(2)` with `TIOCSCTTY` on fd 0, already
    // redirected to the pty slave by this function's caller. Neither
    // allocates, locks, or touches Rust runtime state.
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

/// Marks `fd` close-on-exec, as `src/pty.rs`'s `open_pair` does: a
/// descriptor this probe keeps for itself must never leak into a child.
fn set_cloexec(fd: &OwnedFd) {
    fcntl(fd.as_fd(), FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))
        .expect("marking a probe pty descriptor close-on-exec must succeed");
}

/// Puts the master in non-blocking mode, as `src/pty.rs`'s `open_pair`
/// does, so no read in the drain loop can ever block.
fn set_nonblocking(fd: &OwnedFd) {
    let status = fcntl(fd.as_fd(), FcntlArg::F_GETFL)
        .expect("reading the probe pty master's status flags must succeed");
    fcntl(
        fd.as_fd(),
        FcntlArg::F_SETFL(OFlag::from_bits_truncate(status) | OFlag::O_NONBLOCK),
    )
    .expect("putting the probe pty master in non-blocking mode must succeed");
}

/// Drains the non-blocking master until end of input, [`IDLE_LIMIT`]
/// without a byte, or an unexpected errno -- never panicking on the
/// latter. A would-block or an interrupted read is retried, never taken
/// for an ending: either would under-report `received` and fake a loss.
/// Bounded by construction: the writer is finite, and every other path
/// ends within [`IDLE_LIMIT`] of the last byte.
fn drain_master(master: &OwnedFd) -> (Vec<u8>, String) {
    let mut received = Vec::new();
    let mut buf = [0u8; 128];
    let mut last_byte = Instant::now();
    loop {
        match read(master, &mut buf) {
            Ok(0) => return (received, "eof-zero".to_string()),
            Ok(n) => {
                received.extend_from_slice(&buf[..n]);
                last_byte = Instant::now();
            }
            Err(Errno::EIO) => return (received, "eof-eio".to_string()),
            Err(Errno::EAGAIN | Errno::EINTR) => {
                if last_byte.elapsed() >= IDLE_LIMIT {
                    return (received, "idle-timeout".to_string());
                }
                std::thread::sleep(RETRY);
            }
            Err(other) => return (received, format!("errno-{other:?}")),
        }
    }
}

/// Polls `done` for up to [`STAGE_WAIT`]. The only way this probe waits
/// for anything that macOS might never let happen.
fn happens_within_stage(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + STAGE_WAIT;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(RETRY);
    }
}

/// The stage at which something first happened.
fn stage(before_drain: bool, after_drain: bool, after_master_close: bool) -> &'static str {
    match (before_drain, after_drain, after_master_close) {
        (true, ..) => "prompt",
        (_, true, _) => "after-drain",
        (.., true) => "after-master-close",
        _ => "never",
    }
}

/// Raw `write(2)` on fd 2, bypassing `libtest`'s capture of `print!`/
/// `eprintln!`'s thread-local sink. The leading newline keeps the line at
/// column 0 whatever `libtest` had already printed on the current one.
fn emit(name: &str, fields: &str) {
    let line = format!(
        "\npty-exit-drain-probe arm={name} os={os} {fields}\n",
        os = std::env::consts::OS
    );
    // One call: a line this short reaches a pipe whole, never interleaved.
    let _ = write(io::stderr(), line.as_bytes());
}

fn set_phase(phase: &'static str) {
    *PHASE.lock().unwrap_or_else(PoisonError::into_inner) = phase;
}

/// Names the arm and its phase, then ends the process, if the arm is still
/// running after [`WATCHDOG`]. Exiting closes the master, which releases
/// whatever the kernel was holding back for it.
fn start_watchdog(name: &'static str) -> Arc<AtomicBool> {
    let finished = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&finished);
    std::thread::spawn(move || {
        std::thread::sleep(WATCHDOG);
        if !seen.load(Ordering::SeqCst) {
            let phase = *PHASE.lock().unwrap_or_else(PoisonError::into_inner);
            emit(name, &format!("watchdog=fired phase={phase}"));
            std::process::exit(101);
        }
    });
    finished
}

/// `/bin/sh -c` writing exactly [`PAYLOAD`] to a slave wired as its stdio,
/// plus the extra slave copy the mitigation arm holds outside `Command`.
fn build_command(config: &ArmConfig, slave: OwnedFd) -> (Command, Option<OwnedFd>) {
    let payload = std::str::from_utf8(PAYLOAD).expect("the payload is plain ASCII");
    let linger = if config.exit_immediately {
        ""
    } else {
        "; sleep 2"
    };
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(format!("printf '%s' '{payload}'{linger}"));

    let stdin = slave.try_clone().expect("cloning the slave for stdin");
    let stdout = slave.try_clone().expect("cloning the slave for stdout");
    let extra_slave = config.hold_extra_slave.then(|| {
        let extra = slave
            .try_clone()
            .expect("cloning the slave for the held-open mitigation copy");
        set_cloexec(&extra);
        extra
    });
    command.stdin(Stdio::from(stdin));
    command.stdout(Stdio::from(stdout));
    command.stderr(Stdio::from(slave));
    if config.controlling_terminal {
        install_controlling_terminal(&mut command);
    }
    (command, extra_slave)
}

/// Runs one arm under the serial lock and the watchdog, and emits its line.
fn run_arm(config: &ArmConfig) -> Observation {
    // Held from before `openpty` to after the last descriptor closes. A
    // poisoned lock only means another arm panicked; this arm is unharmed.
    let _serial = ARM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let finished = start_watchdog(config.name);
    set_phase("spawning");
    emit(config.name, "phase=begin");

    let OpenptyResult { master, slave } =
        openpty(None::<&Winsize>, None::<&Termios>).expect("openpty must succeed for the probe");
    set_cloexec(&master);
    set_cloexec(&slave);
    set_nonblocking(&master);
    let (mut command, extra_slave) = build_command(config, slave);
    let mut child = command.spawn().expect("spawning the probe child");
    let pid = child.id();
    // Release the parent's own slave copies right after `spawn`, as the
    // product does -- on a thread of its own, because this may be the last
    // close of a slave with unread output, and whether that blocks is part
    // of what is being measured.
    let releaser = std::thread::spawn(move || drop(command));
    let mut is_reaped = || matches!(child.try_wait(), Ok(Some(_)));

    set_phase("before-drain");
    let reaped_unread = config.exit_immediately && happens_within_stage(&mut is_reaped);
    std::thread::sleep(LATE_READ_DELAY);
    let released_unread = releaser.is_finished();

    set_phase("draining");
    let (received, end) = drain_master(&master);
    set_phase("after-drain");
    let reaped_drained = reaped_unread || happens_within_stage(&mut is_reaped);
    let released_drained = released_unread || happens_within_stage(|| releaser.is_finished());

    // The master goes before the held slave: with no master left, closing
    // the last slave has nothing to wait for.
    set_phase("after-master-close");
    drop(master);
    let reaped_closed = reaped_drained || happens_within_stage(&mut is_reaped);
    let released_closed = released_drained || happens_within_stage(|| releaser.is_finished());
    set_phase("closing-held-slave");
    drop(extra_slave);
    if !reaped_closed {
        let _ = child.kill();
    }
    finished.store(true, Ordering::SeqCst);

    let reap = stage(reaped_unread, reaped_drained, reaped_closed);
    let release = stage(released_unread, released_drained, released_closed);
    let (written, got) = (PAYLOAD.len(), received.len());
    emit(
        config.name,
        &format!("written={written} received={got} end={end} reaped={reap} released={release}"),
    );
    Observation { pid, received, end }
}

/// What every late-read arm holds on any platform: a child ran, and
/// whatever arrived is a prefix of the payload, never more than it.
fn assert_platform_independent_invariants(observation: &Observation) {
    assert!(observation.pid > 0, "a child must have been spawned");
    assert!(
        observation.received.len() <= PAYLOAD.len(),
        "received more than was written (end={})",
        observation.end
    );
    assert!(
        PAYLOAD.starts_with(observation.received.as_slice()),
        "received bytes must prefix the payload, got {:?}",
        observation.received
    );
}

/// Harness sanity: controlling terminal, slave released, prompt read
/// while the child lingers.
#[test]
fn control_lingering_child() {
    let observation = run_arm(&ArmConfig {
        name: "control_lingering_child",
        controlling_terminal: true,
        exit_immediately: false,
        hold_extra_slave: false,
    });
    assert_eq!(
        observation.received.len(),
        PAYLOAD.len(),
        "a prompt read of a live child must see every byte (end={})",
        observation.end
    );
}

/// A late-read arm: the child exits the instant it has written.
fn measure_late_read(name: &'static str, ctty: bool, hold_slave: bool) -> Observation {
    let observation = run_arm(&ArmConfig {
        name,
        controlling_terminal: ctty,
        exit_immediately: true,
        hold_extra_slave: hold_slave,
    });
    assert_platform_independent_invariants(&observation);
    observation
}

/// The product's own shape: controlling terminal, slave released. Pinned,
/// not just recorded: the product relies on losing nothing here.
#[test]
fn hazard_slave_released_late_read() {
    let observation = measure_late_read("hazard_slave_released_late_read", true, false);
    assert_eq!(
        observation.received.len(),
        PAYLOAD.len(),
        "with a controlling terminal, output written just before exit must survive a late \
         read (end={})",
        observation.end
    );
}

/// The hazard arm, with the parent holding one extra slave open.
#[test]
fn mitigation_slave_held_late_read() {
    measure_late_read("mitigation_slave_held_late_read", true, true);
}

/// The hazard arm, with no `setsid` and no `TIOCSCTTY`.
#[test]
fn hazard_without_controlling_terminal() {
    measure_late_read("hazard_without_controlling_terminal", false, false);
}
