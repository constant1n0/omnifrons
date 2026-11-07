//! Proving that a stopped child left no descendant process behind (Linux).
//!
//! `stop` used to send `killpg(SIGTERM)` to the child's process group and
//! report `Exited` the moment the *direct child* was reaped, without ever
//! asking whether anything else was still running. VP-001 evidence row
//! `VP-001-VP-S6-02` (CI run 35450847942, pinned `ubuntu-24.04` baseline)
//! recorded what that costs: an agent spawned a descendant that called
//! `setsid`, so `killpg` never reached it, and the product reported
//! `exited (code unreported)` while that descendant was still alive with
//! its original `starttime`. `docs/target-architecture.md` § Required
//! failure states requires the opposite -- "Process descendants unproven
//! stopped -> Orphan-risk/uncertain".
//!
//! This module supplies the proof `stop` was missing:
//!
//! 1. **Census** ([`Census::take`]), taken while the child is still alive:
//!    walk `/proc` by parent chain and record every descendant as
//!    `(pid, starttime)`.
//! 2. **Verify and sweep** ([`Census::settle`]), after the child is
//!    reaped: re-read every recorded `(pid, starttime)`, terminate the
//!    ones still running (`SIGTERM`, bounded wait, `SIGKILL`), and check
//!    the process group itself for anything the census never saw.
//! 3. **Report honestly**: the caller's reap-derived state survives only
//!    when *every* recorded process is proven gone and the group is empty.
//!    Anything unproven -- including a recorded descendant still running --
//!    becomes [`ProcessTerminalState::OrphanRiskUncertain`].
//!
//! The sweep only reaches what the census recorded. Anything outside it --
//! a descendant born after the snapshot -- is reported by the group check
//! and never swept, because this module will not signal a pid it cannot
//! identify.
//!
//! # The census of a child that exits on its own
//!
//! A child that exits by itself is not being stopped, so nothing is swept
//! for it: [`Census::leftovers`] reports what is still running and leaves it
//! running (Stop stays the operator's way to end it). Its census cannot be
//! taken at the exit -- the child's descendants are reparented the moment
//! it exits, before it is even reaped, so a walk then finds none -- so the
//! self-exit reaper (`crate::SelfExitReaper`) walks while the child lives
//! and merges every walk ([`Census::merge`]: a union of sightings, never a
//! replacement):
//!
//! - **Interval.** One walk at spawn, then one every 250 ms for the first
//!   minute, then one every 30 s for as long as the child lives; the
//!   250 ms cadence restarts for a minute when the child's output closes.
//!   A full `/proc` walk reads every process's `stat`, which is what bounds
//!   how often it can run.
//! - **Blind spot.** A walk links a descendant to the child only through
//!   its parent chain, and that chain breaks the moment the child -- or any
//!   process between them -- exits. A descendant that calls `setsid` (or
//!   otherwise leaves the process group) and loses that link between two
//!   walks, before any walk saw it, is never seen: neither by the census
//!   nor by the group check. That covers one forked within the last
//!   interval before the child exits (at most 250 ms early on, 30 s later)
//!   and one double-forked through an intermediate that exits at once.
//!   `sh -c 'setsid sleep 3600 & exit 0'` is the shortest form: the child
//!   is gone within a millisecond of the fork, before any walk.
//!
//! # What this still cannot prove
//!
//! This is **not** cgroup containment, and nothing here should be read as
//! a containment guarantee (`src-tauri/src/health.rs` still reports
//! containment as `unproven`, deliberately):
//!
//! - **The census is a snapshot.** Reading `/proc` is racy by nature: a
//!   descendant spawned *after* the snapshot and *before* the signal is not
//!   in the census. If it stays in the process group the group check below
//!   still catches it; if it also calls `setsid` it is invisible to both,
//!   and this stop reports a clean state it did not actually prove for that
//!   one process. Closing that window needs a kernel-side boundary (a
//!   cgroup, a PID namespace), not a better `/proc` walk.
//! - **`(pid, starttime)` is checked, then signalled.** The pid could in
//!   principle be recycled in the window between reading `/proc/<pid>/stat`
//!   and the `kill` that follows it. The check makes that window as small
//!   as a userspace implementation can; only a pidfd would remove it.
//! - **An entry we cannot read is not evidence.** A `/proc` that hides or
//!   refuses entries (`hidepid`, a restricted container) makes every stop
//!   report `orphan-risk/uncertain`, because the supervisor genuinely
//!   cannot see the process tree it is claiming things about.
//!
//! Linux only: this walks `/proc`, which the other unixes this workspace
//! builds for do not have. There, `stop` keeps its pre-existing behaviour
//! (`crate::unix::Census`) -- the same documented gap as before this
//! module existed, not a new one.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use omnifrons_app::ProcessTerminalState;

/// Where the census reads from. Parameterized only so this module's own
/// tests can point the same code at a synthetic tree.
const PROC_ROOT: &str = "/proc";

/// `proc(5)`'s process state for a terminated-but-unreaped process. A
/// zombie runs no code and holds no resources: it is already terminated,
/// so it is never counted as a survivor and never signalled (which would
/// be a no-op regardless).
const ZOMBIE_STATE: char = 'Z';

/// How long recorded survivors are given to die after `SIGTERM`, and again
/// after `SIGKILL`. Bounded and fixed: `stop` already has the caller's own
/// deadline for the direct child, and this sweep must not turn into a
/// second, open-ended wait.
const SWEEP_GRACE: Duration = Duration::from_millis(300);

/// How often the sweep re-reads `/proc` while waiting out a grace period.
const SWEEP_POLL: Duration = Duration::from_millis(20);

/// One recorded process: its pid paired with the `starttime`
/// (`proc(5)` field 22) observed for it at census time.
///
/// The pair, never the pid alone: pids are recycled, and a recorded pid
/// whose `starttime` no longer matches is a *different* process: never a
/// survivor of the one that was recorded, and never something this module
/// will signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pid: Pid,
    starttime: u64,
}

/// The four `/proc/<pid>/stat` fields this module reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stat {
    /// `proc(5)` field 3.
    state: char,
    /// `proc(5)` field 4.
    ppid: Pid,
    /// `proc(5)` field 5.
    pgrp: Pid,
    /// `proc(5)` field 22.
    starttime: u64,
}

/// Why something about a stopped child's descendants could not be proven.
///
/// Every variant means the same thing to a caller -- report
/// [`ProcessTerminalState::OrphanRiskUncertain`], never a clean stop --
/// and differs only in what gets logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unproven {
    /// `/proc` itself could not be listed, so no census exists at all.
    ProcUnreadable,
    /// A `/proc/<pid>` entry existed but could not be read or parsed: one
    /// live process could not be classified, and it may be a descendant.
    EntryUnreadable,
    /// The child had no `/proc` entry to walk down from, so its
    /// descendants (if any) could never be enumerated. Not an ordinary
    /// outcome: `stop` takes the census while still holding an unreaped
    /// `Child`, and even an exited-but-unreaped child is a zombie with a
    /// `/proc` entry of its own.
    ChildEntryMissing,
    /// A recorded descendant was still running after both sweep signals
    /// and both grace periods.
    DescendantStillRunning,
    /// Signalling a recorded survivor failed for a reason other than "that
    /// process is already gone", so whether it was terminated is unknown.
    SignalFailed(Errno),
    /// Something was still in the stopped child's process group once the
    /// census had been accounted for -- a descendant spawned after the
    /// snapshot, which is exactly the residual race this module cannot
    /// close.
    GroupNotEmpty,
    /// A recorded descendant was still running when the child that exited
    /// on its own was reaped ([`Census::leftovers`], which never sweeps).
    DescendantOutlivedChild,
    /// The census reached [`MAX_SIGHTINGS`] and stopped recording.
    TooManySightings,
}

impl std::fmt::Display for Unproven {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProcUnreadable => f.write_str("/proc could not be listed"),
            Self::EntryUnreadable => f.write_str("a /proc entry could not be read or parsed"),
            Self::ChildEntryMissing => f.write_str("the child had no /proc entry to census"),
            Self::DescendantStillRunning => f.write_str("a recorded descendant outlived the sweep"),
            Self::SignalFailed(errno) => {
                write!(f, "signalling a recorded descendant failed: {errno}")
            }
            Self::GroupNotEmpty => {
                f.write_str("the child's process group still held a process the census never saw")
            }
            Self::DescendantOutlivedChild => {
                f.write_str("a recorded descendant outlived the child that exited on its own")
            }
            Self::TooManySightings => f.write_str("the census reached its sighting bound"),
        }
    }
}

/// The most distinct `(pid, starttime)` sightings one census keeps
/// ([`Census::merge`]). Far above what any real agent leaves alive at once;
/// a bound only so a pathological child cannot grow it without limit.
const MAX_SIGHTINGS: usize = 4096;

/// Shown for a descendant whose command name could no longer be read.
const UNKNOWN_COMM: &str = "?";

/// What a child that exited on its own left behind ([`Census::leftovers`]).
///
/// Nothing here was signalled: a self-exit is not a stop, so its
/// descendants are reported and left running, and Stop stays the operator's
/// way to end them. [`Census::settle`] is the stop-time counterpart, which
/// sweeps.
pub(crate) struct Leftovers {
    /// Why the reap-derived state cannot stand, if it cannot: a recorded
    /// descendant still running, a process still in the child's group, or a
    /// census that proves nothing.
    pub(crate) unproven: Option<Unproven>,
    /// Recorded descendants still running *outside* the child's process
    /// group, as `(pid, command name)`: the ones a group check cannot see.
    pub(crate) escaped: Vec<(Pid, String)>,
    /// Every process still running in the child's process group, with its
    /// start time: what lets a later Stop prove the group is still the
    /// original one ([`member_still_in_group`]).
    pub(crate) members: Vec<Identity>,
}

/// What was recorded about a child's descendants before it was signalled.
pub(crate) enum Census {
    /// A `(pid, starttime)` snapshot of every descendant observed while
    /// the child was still alive. Possibly empty -- which is a real
    /// observation ("it had none at that instant"), not a missing one.
    Snapshot(Vec<Identity>),
    /// No usable snapshot exists, for the recorded reason. Nothing about
    /// descendants can be proven, so no clean stop can be reported.
    Unavailable(Unproven),
}

impl Census {
    /// Record every descendant of `child` alive right now.
    ///
    /// Must be called while `child` is still alive (or at least unreaped):
    /// once a process is reaped its children have already been reparented
    /// away, and the chain that identifies them as *its* descendants is
    /// gone for good.
    pub(crate) fn take(child: Pid) -> Self {
        Self::take_in(Path::new(PROC_ROOT), child)
    }

    fn take_in(proc_root: &Path, child: Pid) -> Self {
        match read_table(proc_root) {
            Err(reason) => Self::Unavailable(reason),
            Ok(table) if !table.contains_key(&child) => {
                Self::Unavailable(Unproven::ChildEntryMissing)
            }
            Ok(table) => Self::Snapshot(descendants_of(&table, child)),
        }
    }

    /// Merge a later walk into this census: the union of every
    /// `(pid, starttime)` either one sighted, never a replacement.
    ///
    /// A walk only adds what it saw. One taken once the child has exited
    /// (an unreaped zombie, whose descendants were reparented at its exit,
    /// so the walk finds none) adds nothing and erases nothing; one that
    /// found no entry for the child at all ran after the reap and is no
    /// evidence either way. Any other failed walk may have missed a
    /// descendant, so from then on the census is unproven and keeps that
    /// reason, whatever later walks see.
    ///
    /// Bounded: past [`MAX_SIGHTINGS`] distinct sightings the census
    /// becomes unproven ([`Unproven::TooManySightings`]) instead of growing
    /// or forgetting one. Walks only run while the child lives, at the
    /// self-exit reaper's census interval, so a census holds one entry per
    /// distinct descendant alive at some walk.
    pub(crate) fn merge(self, fresh: Self) -> Self {
        match (self, fresh) {
            (Self::Snapshot(mut seen), Self::Snapshot(new)) => {
                for identity in new {
                    if !seen.contains(&identity) {
                        seen.push(identity);
                    }
                }
                if seen.len() > MAX_SIGHTINGS {
                    Self::Unavailable(Unproven::TooManySightings)
                } else {
                    Self::Snapshot(seen)
                }
            }
            (kept, Self::Unavailable(Unproven::ChildEntryMissing))
            | (Self::Unavailable(Unproven::ChildEntryMissing), kept) => kept,
            (Self::Unavailable(reason), _) | (_, Self::Unavailable(reason)) => {
                Self::Unavailable(reason)
            }
        }
    }

    /// What a child that exited on its own left behind, read without
    /// signalling anything (see [`Leftovers`]). Called after the child's
    /// reap, like [`Self::settle`].
    pub(crate) fn leftovers(&self, pgid: Pid) -> Leftovers {
        self.leftovers_in(Path::new(PROC_ROOT), pgid)
    }

    fn leftovers_in(&self, proc_root: &Path, pgid: Pid) -> Leftovers {
        let mut found = Leftovers {
            unproven: None,
            escaped: Vec::new(),
            members: Vec::new(),
        };
        let table = match read_table(proc_root) {
            Ok(table) => table,
            Err(reason) => {
                found.unproven = Some(reason);
                return found;
            }
        };
        found.members = table
            .iter()
            .filter(|(_, stat)| stat.pgrp == pgid && stat.state != ZOMBIE_STATE)
            .map(|(pid, stat)| Identity {
                pid: *pid,
                starttime: stat.starttime,
            })
            .collect();
        found.members.sort_by_key(|identity| identity.pid.as_raw());
        match self {
            Self::Unavailable(reason) => found.unproven = Some(*reason),
            Self::Snapshot(recorded) => {
                for identity in recorded {
                    let Some(stat) = table.get(&identity.pid) else {
                        continue;
                    };
                    if stat.starttime != identity.starttime || stat.state == ZOMBIE_STATE {
                        continue;
                    }
                    found
                        .unproven
                        .get_or_insert(Unproven::DescendantOutlivedChild);
                    if stat.pgrp != pgid {
                        let comm = read_comm(proc_root, identity.pid)
                            .unwrap_or_else(|| UNKNOWN_COMM.to_string());
                        found.escaped.push((identity.pid, comm));
                    }
                }
            }
        }
        if !found.members.is_empty() {
            found.unproven.get_or_insert(Unproven::GroupNotEmpty);
        }
        found
    }

    /// Sweep whatever survived, then answer what the caller may honestly
    /// report: `reaped` (the state the direct child's own reap produced)
    /// only when every recorded process is proven gone and `pgid`'s group
    /// is empty, and [`ProcessTerminalState::OrphanRiskUncertain`]
    /// otherwise.
    ///
    /// Must be called *after* the direct child has been reaped: until then
    /// the child is in its own process group and the group check can only
    /// report it as a survivor of itself.
    pub(crate) fn settle(&self, pgid: Pid, reaped: ProcessTerminalState) -> ProcessTerminalState {
        match self.prove(Path::new(PROC_ROOT), pgid) {
            Ok(()) => reaped,
            Err(reason) => {
                tracing::warn!(
                    pgid = pgid.as_raw(),
                    %reason,
                    "a stopped child's descendants are not proven gone; orphan-risk/uncertain"
                );
                ProcessTerminalState::OrphanRiskUncertain
            }
        }
    }

    fn prove(&self, proc_root: &Path, pgid: Pid) -> Result<(), Unproven> {
        let recorded = match self {
            Self::Snapshot(recorded) => recorded,
            Self::Unavailable(reason) => return Err(*reason),
        };
        sweep(proc_root, recorded)?;
        if group_survivors_in(proc_root, pgid)?.is_empty() {
            Ok(())
        } else {
            Err(Unproven::GroupNotEmpty)
        }
    }
}

/// Terminate every recorded process still running, `SIGTERM` first and
/// `SIGKILL` second, each round re-verifying `(pid, starttime)` immediately
/// before it signals anything.
///
/// Bounded by construction: exactly two signal rounds, each followed by at
/// most one [`SWEEP_GRACE`], and one final classification. There is no
/// "until clean" loop.
fn sweep(proc_root: &Path, recorded: &[Identity]) -> Result<(), Unproven> {
    for signal in [Signal::SIGTERM, Signal::SIGKILL] {
        // Re-read immediately before signalling, every round: this is what
        // keeps a recycled pid from being signalled, and it is also the
        // module's irreducible residual -- the pid could in principle be
        // recycled between this read and the `kill` below.
        let survivors = survivors_in(proc_root, recorded)?;
        if survivors.is_empty() {
            return Ok(());
        }
        for identity in &survivors {
            match kill(identity.pid, signal) {
                // ESRCH: it died between the read above and this signal.
                Ok(()) | Err(Errno::ESRCH) => {}
                Err(errno) => return Err(Unproven::SignalFailed(errno)),
            }
        }
        wait_out_grace(proc_root, recorded);
    }
    if survivors_in(proc_root, recorded)?.is_empty() {
        Ok(())
    } else {
        Err(Unproven::DescendantStillRunning)
    }
}

/// Poll until nothing recorded is running any more, or [`SWEEP_GRACE`]
/// elapses -- whichever comes first. A read failure is not resolved here:
/// the caller's next [`survivors_in`] reports it.
fn wait_out_grace(proc_root: &Path, recorded: &[Identity]) {
    let start = Instant::now();
    loop {
        if matches!(survivors_in(proc_root, recorded), Ok(ref survivors) if survivors.is_empty()) {
            return;
        }
        let elapsed = start.elapsed();
        if elapsed >= SWEEP_GRACE {
            return;
        }
        std::thread::sleep(SWEEP_POLL.min(SWEEP_GRACE.saturating_sub(elapsed)));
    }
}

/// Which of `recorded` are still running: still present, still the same
/// `(pid, starttime)`, and not already a zombie.
///
/// A vanished entry is a conclusive answer (that pid is not running), but
/// an entry that exists and cannot be read is not evidence of anything --
/// that is an error, never an empty result.
fn survivors_in(proc_root: &Path, recorded: &[Identity]) -> Result<Vec<Identity>, Unproven> {
    let mut survivors = Vec::new();
    for identity in recorded {
        match read_stat(proc_root, identity.pid) {
            Ok(Some(stat))
                if stat.starttime == identity.starttime && stat.state != ZOMBIE_STATE =>
            {
                survivors.push(*identity);
            }
            // Gone, recycled into a different process, or already a
            // zombie: in every one of those the recorded process itself is
            // no longer running, and none of them may be signalled.
            Ok(_) => {}
            Err(reason) => return Err(reason),
        }
    }
    Ok(survivors)
}

/// Every process still in `pgid`'s process group, excluding zombies.
///
/// The census cannot see a descendant spawned after its snapshot, but one
/// that stayed in the group is still visible here -- which is why this
/// check exists on top of the census rather than instead of it.
fn group_survivors_in(proc_root: &Path, pgid: Pid) -> Result<Vec<Pid>, Unproven> {
    Ok(read_table(proc_root)?
        .into_iter()
        .filter(|(_, stat)| stat.pgrp == pgid && stat.state != ZOMBIE_STATE)
        .map(|(pid, _)| pid)
        .collect())
}

/// Whether `pgid` is provably still the process group `members` were
/// recorded in: at least one of them is still running, with its recorded
/// start time, inside that group.
///
/// Only then may the group be signalled by its id after the child's reap.
/// A process-group id is not reused while any process is still in that
/// group, but once the group is empty its id (the reaped child's pid) can
/// be handed to an unrelated process that starts a group of its own. A
/// member that cannot be read is not evidence: it does not count. As with
/// [`sweep`], the member could in principle leave the group between this
/// read and the signal that follows it.
pub(crate) fn member_still_in_group(pgid: Pid, members: &[Identity]) -> bool {
    member_still_in_group_in(Path::new(PROC_ROOT), pgid, members)
}

fn member_still_in_group_in(proc_root: &Path, pgid: Pid, members: &[Identity]) -> bool {
    members.iter().any(|identity| {
        matches!(
            read_stat(proc_root, identity.pid),
            Ok(Some(stat)) if stat.starttime == identity.starttime
                && stat.state != ZOMBIE_STATE
                && stat.pgrp == pgid
        )
    })
}

/// The command name (`proc(5)` field 2, `comm`) of `pid`, if it can still
/// be read: the executable's file name, never its path, cut at 15 bytes by
/// the kernel.
fn read_comm(proc_root: &Path, pid: Pid) -> Option<String> {
    let contents =
        std::fs::read_to_string(proc_root.join(pid.as_raw().to_string()).join("stat")).ok()?;
    parse_comm(&contents)
}

/// `comm` sits between the line's first `(` and its *last* `)`: it can
/// itself contain both (see [`parse_stat`]).
fn parse_comm(contents: &str) -> Option<String> {
    let open = contents.find('(')?;
    let close = contents.rfind(')')?;
    contents.get(open + 1..close).map(str::to_string)
}

/// Read and parse one `/proc/<pid>/stat`, or `Ok(None)` if that process
/// has vanished.
fn read_stat(proc_root: &Path, pid: Pid) -> Result<Option<Stat>, Unproven> {
    match std::fs::read_to_string(proc_root.join(pid.as_raw().to_string()).join("stat")) {
        Ok(contents) => parse_stat(&contents)
            .map(Some)
            .ok_or(Unproven::EntryUnreadable),
        Err(error) if is_vanished(&error) => Ok(None),
        Err(_) => Err(Unproven::EntryUnreadable),
    }
}

/// Whether this read failed because the process is simply not there any
/// more. `/proc` reports that two ways: the entry is already gone
/// (`ENOENT`), or it was opened just as the task exited and the read
/// itself reports `ESRCH`.
fn is_vanished(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
        || error.raw_os_error() == Some(Errno::ESRCH as i32)
}

/// Snapshot every numeric `/proc/<pid>` entry.
///
/// An entry that vanishes mid-walk is skipped rather than fatal: processes
/// exit constantly, and treating every one of them as a failure would make
/// every stop on a busy machine report `orphan-risk/uncertain` for no real
/// reason. The residual that skipping accepts is named in this module's own
/// header: a descendant that exits mid-walk may leave a *reparented* child
/// this walk can no longer link to the census root.
fn read_table(proc_root: &Path) -> Result<HashMap<Pid, Stat>, Unproven> {
    let mut table = HashMap::new();
    for entry in std::fs::read_dir(proc_root).map_err(|_| Unproven::ProcUnreadable)? {
        let entry = entry.map_err(|_| Unproven::ProcUnreadable)?;
        // `/proc` also holds non-numeric entries (`self`, `meminfo`, ...);
        // only the numeric ones are processes.
        let Some(raw) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let pid = Pid::from_raw(raw);
        if let Some(stat) = read_stat(proc_root, pid)? {
            table.insert(pid, stat);
        }
    }
    Ok(table)
}

/// Every transitive descendant of `root` in `table`, `root` itself
/// excluded.
///
/// Breadth-first over a `visited` set, so an internally inconsistent
/// snapshot (each `/proc` entry is read separately, and the process tree
/// changes underneath) can never produce an unbounded walk.
fn descendants_of(table: &HashMap<Pid, Stat>, root: Pid) -> Vec<Identity> {
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, stat) in table {
        children.entry(stat.ppid).or_default().push(*pid);
    }

    let mut found = Vec::new();
    let mut visited: HashSet<Pid> = HashSet::from([root]);
    let mut queue: VecDeque<Pid> = VecDeque::from([root]);
    while let Some(current) = queue.pop_front() {
        for child in children.get(&current).into_iter().flatten() {
            if !visited.insert(*child) {
                continue;
            }
            if let Some(stat) = table.get(child) {
                found.push(Identity {
                    pid: *child,
                    starttime: stat.starttime,
                });
            }
            queue.push_back(*child);
        }
    }
    found
}

/// Parse the fields this module needs out of one `/proc/<pid>/stat`.
///
/// `comm` (field 2) can itself contain spaces and `)`, so per `proc(5)` the
/// only reliable way to skip it is the line's *last* `)`. After it, field
/// 3 (`state`) sits at index 0, so `proc(5)`'s field N is at index N - 3.
fn parse_stat(contents: &str) -> Option<Stat> {
    let fields: Vec<&str> = contents
        .get(contents.rfind(')')? + 1..)?
        .split_whitespace()
        .collect();
    Some(Stat {
        state: fields.first()?.chars().next()?,
        ppid: Pid::from_raw(fields.get(1)?.parse().ok()?),
        pgrp: Pid::from_raw(fields.get(2)?.parse().ok()?),
        starttime: fields.get(19)?.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use nix::unistd::Pid;

    use omnifrons_app::ProcessTerminalState;

    use super::{
        Census, Identity, Stat, Unproven, descendants_of, member_still_in_group_in, parse_comm,
        parse_stat, survivors_in,
    };

    /// A throwaway directory holding a synthetic `/proc` tree, so the
    /// census and the survivor probe can be exercised over entries that
    /// would be impossible to arrange reliably with real processes -- a
    /// recycled pid above all. Hand-rolled, matching this crate's existing
    /// test helpers (`tests/outbox_launch.rs`), rather than adding a
    /// dependency.
    struct ProcRoot(PathBuf);

    impl ProcRoot {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "omnifrons-proc-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("the synthetic proc root must be creatable");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        /// Write one synthetic `/proc/<pid>/stat`.
        fn entry(
            &self,
            process: i32,
            state: char,
            parent: i32,
            group: i32,
            starttime: u64,
        ) -> &Self {
            let dir = self.0.join(process.to_string());
            std::fs::create_dir_all(&dir).expect("a synthetic entry directory must be creatable");
            std::fs::write(
                dir.join("stat"),
                stat_line(process, state, parent, group, starttime),
            )
            .expect("a synthetic stat file must be writable");
            self
        }

        /// Write a `/proc/<pid>` entry whose `stat` cannot be read: a
        /// directory where the file should be, which fails with something
        /// other than "not found".
        fn unreadable_entry(&self, process: i32) -> &Self {
            std::fs::create_dir_all(self.0.join(process.to_string()).join("stat"))
                .expect("the unreadable entry must be creatable");
            self
        }
    }

    impl Drop for ProcRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A `/proc/<pid>/stat` line shaped like the real thing: pid, `(comm)`,
    /// then `state ppid pgrp session ...` through `starttime`, which
    /// `proc(5)` puts at field 22 -- index 19 of the remainder, hence the
    /// twelve filler fields.
    fn stat_line(process: i32, state: char, parent: i32, group: i32, starttime: u64) -> String {
        let filler = "0 ".repeat(12);
        format!(
            "{process} (fake-agent) {state} {parent} {group} {group} 0 -1 4194304 \
             {filler}{starttime} 0 0 0"
        )
    }

    fn pid(raw: i32) -> Pid {
        Pid::from_raw(raw)
    }

    fn identity(raw: i32, starttime: u64) -> Identity {
        Identity {
            pid: pid(raw),
            starttime,
        }
    }

    fn table(entries: &[(i32, char, i32, i32, u64)]) -> HashMap<Pid, Stat> {
        entries
            .iter()
            .map(|&(raw, state, ppid, pgrp, starttime)| {
                (
                    pid(raw),
                    Stat {
                        state,
                        ppid: pid(ppid),
                        pgrp: pid(pgrp),
                        starttime,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn parse_stat_reads_state_ppid_pgrp_and_starttime() {
        let parsed = parse_stat(&stat_line(4242, 'S', 17, 4200, 999_888))
            .expect("a well-formed stat line must parse");
        assert_eq!(
            parsed,
            Stat {
                state: 'S',
                ppid: pid(17),
                pgrp: pid(4200),
                starttime: 999_888,
            }
        );
    }

    /// `comm` can itself contain spaces and `)` (`proc(5)`); only the
    /// line's *last* `)` reliably marks its end.
    #[test]
    fn parse_stat_survives_a_comm_containing_spaces_and_parens() {
        let filler = "0 ".repeat(12);
        let line = format!("77 (cmd) with ) parens) R 1 55 55 0 -1 0 {filler}555000 0 0 0");
        let parsed = parse_stat(&line).expect("the last paren must delimit comm");
        assert_eq!(parsed.state, 'R');
        assert_eq!(parsed.ppid, pid(1));
        assert_eq!(parsed.pgrp, pid(55));
        assert_eq!(parsed.starttime, 555_000);
    }

    #[test]
    fn parse_stat_rejects_a_line_without_the_fields_it_needs() {
        assert_eq!(parse_stat("1 (init) S 0 1 1"), None);
        assert_eq!(parse_stat("no parens here at all"), None);
    }

    /// The census is the whole parent chain, not just direct children: a
    /// grandchild is exactly what a shell-wrapped agent leaves behind.
    #[test]
    fn descendants_of_collects_the_whole_parent_chain_and_excludes_the_root() {
        let table = table(&[
            (100, 'S', 1, 100, 10),
            (101, 'S', 100, 100, 11),
            (102, 'S', 101, 102, 12),
            (200, 'S', 1, 200, 20),
        ]);

        let mut found = descendants_of(&table, pid(100));
        found.sort_by_key(|entry| entry.pid.as_raw());

        assert_eq!(found, vec![identity(101, 11), identity(102, 12)]);
    }

    /// A `/proc` snapshot is read entry by entry and can be internally
    /// inconsistent; the walk must still terminate.
    #[test]
    fn descendants_of_terminates_on_a_cyclic_table() {
        let table = table(&[
            (100, 'S', 101, 100, 10),
            (101, 'S', 100, 100, 11),
            (102, 'S', 101, 100, 12),
        ]);

        let mut found = descendants_of(&table, pid(100));
        found.sort_by_key(|entry| entry.pid.as_raw());

        assert_eq!(found, vec![identity(101, 11), identity(102, 12)]);
    }

    #[test]
    fn take_in_snapshots_the_childs_descendants() {
        let root = ProcRoot::new("snapshot");
        root.entry(100, 'S', 1, 100, 10)
            .entry(101, 'S', 100, 101, 11)
            .entry(200, 'S', 1, 200, 20);

        let Census::Snapshot(recorded) = Census::take_in(root.path(), pid(100)) else {
            panic!("a readable proc root containing the child must yield a snapshot")
        };

        assert_eq!(recorded, vec![identity(101, 11)]);
    }

    /// The child has no `/proc` entry to walk down from, so nothing can be
    /// enumerated: its descendants (if any) are unaccounted for, which is
    /// the definition of unproven -- never a clean stop.
    #[test]
    fn take_in_refuses_to_census_a_child_with_no_proc_entry() {
        let root = ProcRoot::new("child-missing");
        root.entry(200, 'S', 1, 200, 20);

        assert!(matches!(
            Census::take_in(root.path(), pid(100)),
            Census::Unavailable(Unproven::ChildEntryMissing)
        ));
    }

    #[test]
    fn take_in_refuses_to_census_an_unreadable_proc() {
        let missing = Path::new("/nonexistent-omnifrons-proc-root");

        assert!(matches!(
            Census::take_in(missing, pid(100)),
            Census::Unavailable(Unproven::ProcUnreadable)
        ));
    }

    /// One `/proc` entry that exists but cannot be read means one live
    /// process that could not be classified -- possibly a descendant.
    #[test]
    fn take_in_refuses_to_census_past_an_unreadable_entry() {
        let root = ProcRoot::new("entry-unreadable");
        root.entry(100, 'S', 1, 100, 10).unreadable_entry(101);

        assert!(matches!(
            Census::take_in(root.path(), pid(100)),
            Census::Unavailable(Unproven::EntryUnreadable)
        ));
    }

    /// The pid-reuse guard: the recorded pid is present, but it is a
    /// different process now, so the recorded one is gone -- and the pid
    /// that replaced it is a stranger that must never be signalled.
    #[test]
    fn survivors_in_never_reports_a_recycled_pid_as_a_survivor() {
        let root = ProcRoot::new("recycled");
        root.entry(4242, 'S', 1, 4242, 999_999);

        let survivors = survivors_in(root.path(), &[identity(4242, 111)])
            .expect("a readable entry must be classifiable");

        assert!(
            survivors.is_empty(),
            "a pid whose starttime changed is a different process: it is neither a survivor \
             nor something this code may ever signal"
        );
    }

    #[test]
    fn survivors_in_reports_a_vanished_entry_as_gone() {
        let root = ProcRoot::new("vanished");

        let survivors = survivors_in(root.path(), &[identity(4242, 111)])
            .expect("an absent entry is a conclusive answer: that pid is not running");

        assert!(survivors.is_empty());
    }

    /// A zombie has already terminated; it is simply waiting to be reaped
    /// by a parent that this stop has usually just killed. Nothing is left
    /// running, so it is not a survivor -- and signalling it would be a
    /// no-op anyway.
    #[test]
    fn survivors_in_treats_a_zombie_as_terminated() {
        let root = ProcRoot::new("zombie");
        root.entry(4242, 'Z', 1, 4242, 111);

        let survivors =
            survivors_in(root.path(), &[identity(4242, 111)]).expect("a zombie is classifiable");

        assert!(survivors.is_empty());
    }

    #[test]
    fn survivors_in_still_reports_a_live_recorded_process() {
        let root = ProcRoot::new("alive");
        root.entry(4242, 'S', 1, 4242, 111);

        let survivors =
            survivors_in(root.path(), &[identity(4242, 111)]).expect("a live entry is readable");

        assert_eq!(survivors, vec![identity(4242, 111)]);
    }

    /// An entry that cannot be read is not evidence of anything: it must
    /// never be reported as "gone".
    #[test]
    fn survivors_in_refuses_to_conclude_from_an_unreadable_entry() {
        let root = ProcRoot::new("probe-unreadable");
        root.unreadable_entry(4242);

        assert_eq!(
            survivors_in(root.path(), &[identity(4242, 111)]),
            Err(Unproven::EntryUnreadable)
        );
    }

    /// Every unavailable census settles as `orphan-risk/uncertain`, never
    /// as the clean state the reap alone would have suggested.
    #[test]
    fn an_unavailable_census_never_settles_as_a_clean_stop() {
        for reason in [
            Unproven::ProcUnreadable,
            Unproven::EntryUnreadable,
            Unproven::ChildEntryMissing,
        ] {
            let census = Census::Unavailable(reason);
            assert_eq!(
                census.prove(Path::new("/nonexistent-omnifrons-proc-root"), pid(100)),
                Err(reason)
            );
            assert_eq!(
                census.settle(pid(100), ProcessTerminalState::Exited { code: Some(0) }),
                ProcessTerminalState::OrphanRiskUncertain,
                "an unprovable census must never be reported as a clean stop"
            );
        }
    }

    /// A census with nothing recorded still has to prove the process group
    /// itself is empty before a clean stop is honest.
    #[test]
    fn an_empty_census_still_checks_the_group() {
        let census = Census::Snapshot(Vec::new());
        let root = ProcRoot::new("group-survivor");
        root.entry(100, 'S', 1, 100, 10);

        assert_eq!(
            census.prove(root.path(), pid(100)),
            Err(Unproven::GroupNotEmpty),
            "a process still in the stopped child's group is an escapee the census missed"
        );
    }

    #[test]
    fn an_empty_group_and_an_empty_census_prove_a_clean_stop() {
        let census = Census::Snapshot(Vec::new());
        let root = ProcRoot::new("group-empty");
        root.entry(200, 'S', 1, 200, 20);

        assert_eq!(census.prove(root.path(), pid(100)), Ok(()));
    }

    /// A terminated-but-unreaped group member is not a running process:
    /// it must not block a clean stop.
    #[test]
    fn a_zombie_group_member_does_not_block_a_clean_stop() {
        let census = Census::Snapshot(Vec::new());
        let root = ProcRoot::new("group-zombie");
        root.entry(100, 'Z', 1, 100, 10);

        assert_eq!(census.prove(root.path(), pid(100)), Ok(()));
    }

    /// The census records sightings: a walk taken once the child is an
    /// unreaped zombie finds no descendant (they were reparented at its
    /// exit), and that must never erase the descendant an earlier walk saw.
    #[test]
    fn a_walk_of_the_zombie_child_never_erases_an_earlier_sighting() {
        let root = ProcRoot::new("zombie-walk");
        root.entry(100, 'S', 1, 100, 10)
            .entry(101, 'S', 100, 101, 11);
        let alive = Census::take_in(root.path(), pid(100));

        root.entry(100, 'Z', 1, 100, 10).entry(101, 'S', 1, 101, 11);
        let zombie = Census::take_in(root.path(), pid(100));

        let Census::Snapshot(recorded) = alive.merge(zombie) else {
            panic!("two readable walks must merge into a snapshot")
        };
        assert_eq!(recorded, vec![identity(101, 11)]);
    }

    /// Two walks merge as a union without duplicates; a walk that found no
    /// entry for the child changes nothing; any other failed walk leaves
    /// the census unproven for good.
    #[test]
    fn merge_is_a_union_and_only_a_failed_walk_other_than_a_missing_child_sticks() {
        let first = Census::Snapshot(vec![identity(101, 11)]);
        let second = Census::Snapshot(vec![identity(101, 11), identity(102, 12)]);
        let Census::Snapshot(union) = first.merge(second) else {
            panic!("two snapshots merge into a snapshot")
        };
        assert_eq!(union, vec![identity(101, 11), identity(102, 12)]);

        let late = Census::Unavailable(Unproven::ChildEntryMissing);
        let Census::Snapshot(kept) = Census::Snapshot(vec![identity(101, 11)]).merge(late) else {
            panic!("a walk after the reap must not erase the snapshot")
        };
        assert_eq!(kept, vec![identity(101, 11)]);

        let unreadable = Census::Unavailable(Unproven::EntryUnreadable);
        let after = Census::Snapshot(vec![identity(101, 11)])
            .merge(unreadable)
            .merge(Census::Snapshot(vec![identity(102, 12)]));
        assert!(matches!(
            after,
            Census::Unavailable(Unproven::EntryUnreadable)
        ));
    }

    /// A self-exit settle names a recorded descendant that left the group,
    /// counts one still in it, records the group's live members, and
    /// ignores what is gone, recycled, or a zombie.
    #[test]
    fn leftovers_name_an_escaped_descendant_and_record_the_groups_members() {
        let root = ProcRoot::new("leftovers");
        root.entry(101, 'S', 1, 101, 11) // escaped: its own group now
            .entry(102, 'S', 1, 100, 12) // still in the child's group
            .entry(103, 'S', 1, 103, 99) // recycled pid: another process
            .entry(104, 'Z', 1, 104, 14); // a zombie: already terminated
        let census = Census::Snapshot(vec![
            identity(101, 11),
            identity(102, 12),
            identity(103, 13),
            identity(104, 14),
            identity(105, 15), // gone
        ]);

        let found = census.leftovers_in(root.path(), pid(100));

        assert_eq!(found.escaped, vec![(pid(101), "fake-agent".to_string())]);
        assert_eq!(found.members, vec![identity(102, 12)]);
        assert_eq!(found.unproven, Some(Unproven::DescendantOutlivedChild));
    }

    #[test]
    fn leftovers_of_a_clean_exit_prove_nothing_is_left() {
        let root = ProcRoot::new("leftovers-clean");
        root.entry(200, 'S', 1, 200, 20);
        let found = Census::Snapshot(vec![identity(101, 11)]).leftovers_in(root.path(), pid(100));

        assert_eq!(found.unproven, None);
        assert!(found.escaped.is_empty() && found.members.is_empty());
    }

    /// A group may be signalled after the child's reap only while a
    /// recorded member is still in it with its recorded start time.
    #[test]
    fn a_group_is_still_the_original_only_while_a_recorded_member_is_in_it() {
        let root = ProcRoot::new("member");
        let members = [identity(102, 12)];

        root.entry(102, 'S', 1, 100, 12);
        assert!(member_still_in_group_in(root.path(), pid(100), &members));

        root.entry(102, 'S', 1, 102, 12); // left the group
        assert!(!member_still_in_group_in(root.path(), pid(100), &members));

        root.entry(102, 'S', 1, 100, 99); // recycled pid
        assert!(!member_still_in_group_in(root.path(), pid(100), &members));

        root.entry(102, 'Z', 1, 100, 12); // terminated
        assert!(!member_still_in_group_in(root.path(), pid(100), &members));

        let gone = ProcRoot::new("member-gone");
        assert!(!member_still_in_group_in(gone.path(), pid(100), &members));
    }

    #[test]
    fn parse_comm_keeps_spaces_and_parens_inside_the_name() {
        assert_eq!(
            parse_comm("77 (cmd) with ) parens) R 1 55 55"),
            Some("cmd) with ) parens".to_string())
        );
    }
}
