//! A deterministic OS-level *measurement probe*, not a correctness test.
//!
//! `tests/pty_launch.rs`'s
//! `observed_env_is_exactly_the_base_allowlist_plus_term_columns_lines`
//! ("the report must carry an env-keys line") fails intermittently on
//! macOS CI, never on Linux. This measures, without asserting, one
//! unproven hypothesis for why: on macOS, bytes a child wrote to a pty
//! slave just before exiting may be lost if the master is read only after
//! the child is gone, because last-close of the slave can discard
//! whatever the line discipline had not yet delivered. It re-creates
//! `src/pty.rs`'s setup (`openpty`, three slave dups as stdin/stdout/
//! stderr, `setsid` + `TIOCSCTTY` in `pre_exec`, the parent releasing its
//! own slave right after `spawn`) directly, since those pieces are
//! `pub(crate)` there.
//!
//! Four arms: reading promptly while the child lingers (harness sanity,
//! the only arm asserting a byte count); reading late after the child is
//! reaped (the product's own shape); the same but the parent also holds
//! one extra slave open across the read (a candidate mitigation); and the
//! same with no controlling terminal, separating "last close of the
//! slave" from "session-leader exit" as the cause. Arms 2-4 assert
//! nothing about the byte count: that count *is* the measurement.
//!
//! Each arm writes one line with a raw `write(2)` to fd 2 (never
//! `print!`/`eprintln!`, which `cargo test` without `--nocapture` would
//! swallow for a passing test), shaped `pty-exit-drain-probe arm=<name>
//! os=<os> written=64 received=<n> end=<eof-zero|eof-eio|idle-timeout|
//! errno-*> wait=<reaped|timeout>`. Read a run with
//! `grep 'pty-exit-drain-probe' <log>`. On Linux `received=64` is
//! expected for every arm; whether macOS differs is the question. A line
//! with `wait=timeout` measured nothing and must be disregarded.
//!
//! The arms run one at a time (see [`ARM_SERIAL`]): run concurrently they
//! can leak a slave descriptor into each other's children and falsify the
//! very thing being measured.

#![cfg(unix)]

use std::io;
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
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

/// Matches `src/pty.rs`'s `WINSIZE`; the dimensions are not load bearing.
const WINSIZE: Winsize = Winsize {
    ws_row: 24,
    ws_col: 80,
    ws_xpixel: 0,
    ws_ypixel: 0,
};

/// Serializes the arms. `openpty` cannot return close-on-exec descriptors
/// atomically, and `libtest` runs the four arms on parallel threads of one
/// process: an arm forking in the window before another arm's
/// `set_cloexec` would hand its child that arm's slave, silently turning a
/// hazard arm into the mitigation arm. Measured on Linux before this lock
/// existed: the hazard arm ended without `EIO` in 2 of 30 parallel runs
/// and 0 of 30 serial ones.
static ARM_SERIAL: Mutex<()> = Mutex::new(());

/// How long the master may yield nothing before the drain loop calls it
/// idle (`end=idle-timeout`, the normal ending while a slave is held
/// open). Longer than the control child's 2 s linger, so that arm always
/// ends on a real end of input; generous, so a slow runner cannot turn a
/// late byte into an apparent loss.
const IDLE_LIMIT: Duration = Duration::from_secs(3);
/// Pause between non-blocking reads that found nothing.
const READ_RETRY: Duration = Duration::from_millis(10);
/// How long a late-read arm waits after reaping the child before reading.
const LATE_READ_DELAY: Duration = Duration::from_millis(300);
/// How long a late-read arm waits for reap before killing the child and
/// recording `wait=timeout` instead of `wait=reaped`.
const REAP_DEADLINE: Duration = Duration::from_secs(10);

/// One arm: whether the child gets a controlling terminal (`setsid` +
/// `TIOCSCTTY`), whether it exits the instant it has written the payload
/// or lingers so the parent can read promptly, and whether the parent
/// keeps one extra slave descriptor open across the child's exit.
struct ArmConfig {
    name: &'static str,
    controlling_terminal: bool,
    exit_immediately: bool,
    hold_extra_slave: bool,
}

/// What one arm observed: the spawned pid (proof a child ran), the bytes
/// the master yielded (never asserted `== 64` outside the sanity arm --
/// the count is the measurement), how the drain loop ended, and whether
/// the child was reaped before `REAP_DEADLINE`.
struct Observation {
    pid: u32,
    received: Vec<u8>,
    end: String,
    wait: &'static str,
}

/// `TIOCSCTTY`'s request argument in the exact type this platform's
/// `ioctl` declares. Mirrors `src/pty.rs`'s `pub(crate) tiocsctty_request`,
/// duplicated because that function is unreachable here.
#[cfg(target_os = "linux")]
fn tiocsctty_request() -> libc::Ioctl {
    libc::TIOCSCTTY
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn tiocsctty_request() -> libc::c_ulong {
    libc::c_ulong::from(libc::TIOCSCTTY)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "ios")))]
fn tiocsctty_request() -> libc::c_ulong {
    libc::TIOCSCTTY
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
                std::thread::sleep(READ_RETRY);
            }
            Err(other) => return (received, format!("errno-{other:?}")),
        }
    }
}

/// Waits for `child` to be reaped inside `timeout`, else kills it.
fn wait_for_reap(child: &mut Child, timeout: Duration) -> &'static str {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return "reaped",
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return "timeout";
            }
        }
    }
}

/// Raw `write(2)` on fd 2, bypassing `libtest`'s capture of `print!`/
/// `eprintln!`'s thread-local sink.
fn write_stderr_raw(line: &str) {
    let bytes = line.as_bytes();
    let mut written = 0;
    while written < bytes.len() {
        match write(io::stderr(), &bytes[written..]) {
            Ok(0) => break,
            Ok(n) => written += n,
            Err(Errno::EINTR) => {}
            Err(_) => break,
        }
    }
}

/// The leading newline keeps the line at column 0 whatever `libtest` had
/// already printed on the current one.
fn emit_line(name: &str, observation: &Observation) {
    let line = format!(
        "\npty-exit-drain-probe arm={name} os={os} written={written} received={received} \
         end={end} wait={wait}\n",
        os = std::env::consts::OS,
        written = PAYLOAD.len(),
        received = observation.received.len(),
        end = observation.end,
        wait = observation.wait,
    );
    write_stderr_raw(&line);
}

/// Opens a pty pair, spawns `/bin/sh -c` writing exactly [`PAYLOAD`],
/// releases the parent's slave copies right after `spawn` (except
/// `hold_extra_slave`), reads on the arm's schedule, emits the result.
fn run_arm(config: &ArmConfig) -> Observation {
    // Held from before `openpty` to after the last descriptor closes. A
    // poisoned lock only means another arm panicked; this arm is unharmed.
    let _serial = ARM_SERIAL.lock().unwrap_or_else(PoisonError::into_inner);

    let OpenptyResult { master, slave } =
        openpty(&WINSIZE, None::<&Termios>).expect("openpty must succeed for the probe");
    set_cloexec(&master);
    set_cloexec(&slave);
    set_nonblocking(&master);

    let payload = std::str::from_utf8(PAYLOAD).expect("the payload is plain ASCII");
    let script = if config.exit_immediately {
        format!("printf '%s' '{payload}'")
    } else {
        format!("printf '%s' '{payload}'; sleep 2")
    };

    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(script);

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

    let mut child = command.spawn().expect("spawning the probe child");
    let pid = child.id();
    // Release the parent's own slave copies right after `spawn`, same as
    // the product: dropping `command` closes its three dup'd/original
    // copies; `extra_slave` lives outside `command` and survives.
    drop(command);

    let (received, end, wait) = if config.exit_immediately {
        let wait = wait_for_reap(&mut child, REAP_DEADLINE);
        std::thread::sleep(LATE_READ_DELAY);
        let (received, end) = drain_master(&master);
        (received, end, wait)
    } else {
        let (received, end) = drain_master(&master);
        let wait = wait_for_reap(&mut child, REAP_DEADLINE);
        (received, end, wait)
    };
    drop(extra_slave);

    let observation = Observation {
        pid,
        received,
        end,
        wait,
    };
    emit_line(config.name, &observation);
    observation
}

/// Invariants every hazard/mitigation arm holds regardless of platform: a
/// child ran, it never handed back more than it was given, and whatever
/// arrived is a prefix of the payload.
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
/// while the child lingers. The only arm asserting a byte count.
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

/// The product's own shape: controlling terminal, slave released, child
/// exits immediately, parent reads only after reap plus a short delay.
#[test]
fn hazard_slave_released_late_read() {
    let observation = run_arm(&ArmConfig {
        name: "hazard_slave_released_late_read",
        controlling_terminal: true,
        exit_immediately: true,
        hold_extra_slave: false,
    });
    assert_platform_independent_invariants(&observation);
}

/// Same as the hazard arm, except the parent holds one extra slave
/// descriptor open across the exit and the late read.
#[test]
fn mitigation_slave_held_late_read() {
    let observation = run_arm(&ArmConfig {
        name: "mitigation_slave_held_late_read",
        controlling_terminal: true,
        exit_immediately: true,
        hold_extra_slave: true,
    });
    assert_platform_independent_invariants(&observation);
}

/// Same as the hazard arm, except the child never becomes a session
/// leader and never runs `TIOCSCTTY`.
#[test]
fn hazard_without_controlling_terminal() {
    let observation = run_arm(&ArmConfig {
        name: "hazard_without_controlling_terminal",
        controlling_terminal: false,
        exit_immediately: true,
        hold_extra_slave: false,
    });
    assert_platform_independent_invariants(&observation);
}
