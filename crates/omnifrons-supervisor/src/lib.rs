//! Rust process supervisor: implements `omnifrons_app::ProcessSupervisor`
//! over `tokio::process`, with Unix process-group termination and a
//! Windows Job Object stub.
//!
//! Containment is platform-specific and "must be proven in the planned
//! desktop verification plan" (docs/adr/0002-desktop-technology-stack.md §
//! Process supervision). On Unix this crate spawns each child in its own
//! process group and terminates the whole group (SIGTERM, then SIGKILL
//! after the deadline). On Windows, group termination is not yet proven --
//! that requires a Job Object policy (VP-001 VP-S5) -- so `stop` reports
//! [`ProcessTerminalState::OrphanRiskUncertain`] there rather than a false
//! "cleanly stopped" (docs/target-architecture.md § Required failure
//! states).

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use omnifrons_app::harness_adapter::{EnvPlan, LaunchPlan, TransportClass};
use omnifrons_app::{
    HarnessRequest, ProcessId, ProcessOutput, ProcessSpec, ProcessStatus, ProcessSupervisor,
    ProcessTerminalState, StdinPlan, SupervisorError, is_secret_shaped,
};
use std::time::Duration;
use tokio::io::AsyncWriteExt as _;
use tokio::process::{Child, Command};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub mod demo;
/// The `/proc`-backed descendant census `stop` uses to prove, on Linux,
/// that a stopped child left nothing running behind it.
#[cfg(target_os = "linux")]
mod descendants;
mod output_capture;
#[cfg(unix)]
mod pty;

use output_capture::{OutputTable, ReaderHandles};

/// Keeps every `fork` this crate makes (`finish_spawn`) apart from every
/// moment it holds a descriptor that is not yet close-on-exec
/// ([`pty::open_pair`], the sealed memfd). A `static`, not an `Inner` field:
/// two supervisors in one process race like two handles of one. Without
/// it `openpty`'s non-atomic close-on-exec step leaked a pty, mostly the
/// master, into 62 of 15600 concurrent spawns (13 of 13 runs); 0 of 3600
/// single-threaded. On macOS std's `pipe` has the same step (no `pipe2`).
///
/// ASSUMPTION: nothing else that runs in this process forks (the other
/// `Command::new` sites are test code or standalone binaries) -- a future
/// spawn site that does must take this lock too. COST: a `spawn` that never
/// returns now blocks every other spawn in the process, not only itself.
///
/// Guards no data, so a panic while held must never poison later spawns.
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

/// Acquires [`SPAWN_LOCK`]: for one fork, or one not-yet-close-on-exec step.
pub(crate) fn spawn_lock() -> MutexGuard<'static, ()> {
    SPAWN_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a child's standard streams are wired -- the one choice
/// [`TokioProcessSupervisor::finish_spawn`] branches on, after the shared
/// launch policy (environment allowlist, running cap, `kill_on_drop`,
/// bookkeeping) has been applied identically to both.
enum StdioPlan {
    /// Piped stdout/stderr (one reader task each) and stdin per the plan:
    /// the demo harness, the plain `ProcessSpec` path, and every
    /// `TransportClass::StructuredStreamingCli` launch.
    Pipes(StdinPlan),
    /// One pseudo-terminal as stdin, stdout, and stderr, read by one task
    /// on its master side (`docs/spike-log.md` § Slice 4). Unix only in
    /// this slice.
    #[cfg(unix)]
    Pty(pty::PtyPair),
}

/// What [`TokioProcessSupervisor::finish_spawn`] prepared on `command`
/// before spawning, so the reader and prompt tasks can be attached after.
enum Wiring {
    Pipes,
    #[cfg(unix)]
    Pty(pty::Master),
}

impl Wiring {
    /// How many reader tasks this wiring spawns -- what gates the final
    /// `State` frame (`output_capture::new_channel`).
    const fn reader_count(&self) -> u8 {
        match self {
            Self::Pipes => 2,
            #[cfg(unix)]
            Self::Pty(_) => 1,
        }
    }
}

/// Turn a `LaunchPlan`'s transport into the stdio wiring plus the prompt
/// bytes to deliver: over stdin (only under `StdinPlan::PipePromptThenClose`)
/// for the structured transport, or typed into the terminal for the PTY
/// transport. This is where a pseudo-terminal is actually opened, so
/// [`TokioProcessSupervisor::spawn_approved`] runs its running-cap check
/// before calling it. On a platform without the PTY path a `Pty` plan is
/// refused here too (keeping this total), though `spawn_approved` has
/// already refused it before consulting the cap.
fn plan_stdio(plan: &LaunchPlan) -> Result<(StdioPlan, Option<Vec<u8>>), SupervisorError> {
    let prompt_bytes = plan
        .prompt
        .as_ref()
        .map(|prompt| prompt.as_str().as_bytes().to_vec());
    match plan.transport {
        TransportClass::StructuredStreamingCli => {
            let prompt = match plan.stdin {
                StdinPlan::PipePromptThenClose => prompt_bytes,
                StdinPlan::Null => None,
            };
            Ok((StdioPlan::Pipes(plan.stdin), prompt))
        }
        TransportClass::Pty => {
            #[cfg(unix)]
            {
                Ok((StdioPlan::Pty(pty::open_pair()?), prompt_bytes))
            }
            #[cfg(not(unix))]
            {
                Err(SupervisorError::PtyUnsupported)
            }
        }
    }
}

/// Apply `env` to `command`: inherit everything, or clear and set only the
/// allowlisted keys -- re-validating every one of them against
/// [`is_secret_shaped`] immediately before it would be set on a real
/// child, regardless of what the caller's own `EnvPlan` claims. The one
/// place in this crate that ever sets an environment variable on a child.
///
/// A key the plan itself assigned (`EnvPlan::assign`; spike slice 5's
/// `OMNIFRONS_OUTPUT_DIR`, HAP-001 D4) is set from that assignment and
/// never resolved from this process's environment, so a value of the same
/// name in the parent can never reach the child; every other allowlisted
/// key is resolved from the parent at spawn time, as before.
fn apply_env(command: &mut Command, env: &EnvPlan) {
    match env {
        EnvPlan::Inherit => {}
        EnvPlan::Allowlist(_) => {
            command.env_clear();
            for key in env.allowed_keys() {
                if is_secret_shaped(key) {
                    tracing::warn!(
                        key,
                        "refusing to set a secret-shaped environment variable on a spawned \
                         child, even though it was allowlisted"
                    );
                    continue;
                }
                if let Some(assignment) = env.assignment_for(key) {
                    command.env(key, &assignment.value);
                    continue;
                }
                if let Ok(value) = std::env::var(key) {
                    command.env(key, value);
                }
            }
        }
    }
}

/// Wire `command`'s standard streams and process grouping per `stdio`,
/// returning what [`TokioProcessSupervisor::attach_output`] needs after
/// the spawn. The one place in this crate that ever configures a child's
/// standard streams.
///
/// On a platform without the PTY path, `StdioPlan` has one `Copy`-able
/// variant and this function has no error case, so the by-value parameter
/// and the `Result` are only needed on unix -- the allows are scoped to
/// exactly that, rather than restructuring the shared signature per
/// platform.
#[cfg_attr(
    not(unix),
    allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)
)]
fn configure_stdio(command: &mut Command, stdio: StdioPlan) -> Result<Wiring, SupervisorError> {
    match stdio {
        StdioPlan::Pipes(stdin_plan) => {
            command.stdin(match stdin_plan {
                StdinPlan::Null => std::process::Stdio::null(),
                StdinPlan::PipePromptThenClose => std::process::Stdio::piped(),
            });
            // Piped, not inherited: output capture is what drains these,
            // one reader task per stream, so the child never blocks
            // writing into a full, undrained OS pipe buffer.
            command.stdout(std::process::Stdio::piped());
            command.stderr(std::process::Stdio::piped());
            #[cfg(unix)]
            {
                // Group the child under its own process group so a later
                // stop can signal the whole group, not just the direct
                // child (docs/adr/0002 § Process supervision). Stable since
                // Rust 1.64 (`std::os::unix::process::CommandExt::process_group`,
                // re-exposed by `tokio::process::Command`). Not on the PTY
                // path: `setsid` there makes the child a session and group
                // leader by itself (`pty::install_controlling_terminal`).
                command.process_group(0);
            }
            Ok(Wiring::Pipes)
        }
        #[cfg(unix)]
        StdioPlan::Pty(pair) => Ok(Wiring::Pty(pty::prepare_child(command, pair)?)),
    }
}

/// The maximum number of concurrently `Running` children one supervisor
/// tracks at once. `spawn`/`spawn_harness` refuse a request beyond this cap
/// with [`SupervisorError::TooManyProcesses`], rather than growing
/// `Inner::children` (and the OS resources a real spawn would consume)
/// without bound.
const MAX_RUNNING_CHILDREN: usize = 16;

/// The maximum number of `Terminal` entries retained once evicted from the
/// running cap above. Past this, the oldest entry by termination order
/// (`Inner::terminal_order`) is evicted from `Inner::children` outright: a
/// later `stop`/`observe` on an evicted id then reports
/// `SupervisorError::UnknownProcess`/`None`, exactly as it would for an id
/// this supervisor never spawned. This never re-signals a pid the OS may
/// have since recycled -- an evicted entry was, by construction, already
/// confirmed reaped (`Tracked::Terminal`) before it was ever eligible for
/// eviction.
const MAX_RETAINED_TERMINAL_CHILDREN: usize = 256;

/// Record that `id` just transitioned to `Tracked::Terminal` (the caller
/// has already made that assignment in `children`), and evict the oldest
/// retained terminal entry if [`MAX_RETAINED_TERMINAL_CHILDREN`] is now
/// exceeded.
///
/// Called with `children`'s own lock already released by the caller: this
/// briefly re-acquires it only to remove the evicted entry, never while
/// still holding it from the transition itself, so it composes safely with
/// call sites that already had `children` locked a moment before (both
/// `TokioProcessSupervisor::observe` and `unix::poll_until_reaped` drop
/// their guard first).
fn record_terminal_order(
    children: &Mutex<HashMap<ProcessId, Tracked>>,
    terminal_order: &Mutex<VecDeque<ProcessId>>,
    id: ProcessId,
) {
    let mut order = terminal_order
        .lock()
        .expect("terminal_order mutex poisoned by a prior panic");
    order.push_back(id);
    if order.len() > MAX_RETAINED_TERMINAL_CHILDREN
        && let Some(evicted) = order.pop_front()
    {
        children
            .lock()
            .expect("children mutex poisoned by a prior panic")
            .remove(&evicted);
    }
}

/// How often the idle-phase reaper retakes its descendant census: a procfs
/// walk is far costlier than a `try_wait`, and a child that has been
/// running for over a minute changes slowly.
const CENSUS_IDLE_REFRESH: Duration = Duration::from_secs(30);

/// The most `(pid, starttime)` sightings one self-exit census keeps
/// (`descendants::Census::refresh`). Every refresh forgets the sightings
/// that are no longer alive, so this bounds the descendants alive at the
/// same time -- far above what any real agent keeps at once -- not how many
/// the child ever ran.
const CENSUS_SIGHTINGS_BOUND: usize = 4096;

/// The cadence at which the self-exit reaper asks `try_wait`, from spawn.
///
/// While the child's output is open it asks every `slow_poll` until
/// `slow_window` has passed since spawn, then every `idle_poll`; the last
/// reader's end wakes it at once. A child that closes its output is
/// normally about to exit, so from then on the run ends promptly:
/// `fast_poll` for the first `fast_window`, then `slow_poll` until
/// `slow_window` has passed. Past that the reaper logs once that the child
/// is still running and keeps asking every `idle_poll`, with no end: a
/// child that exits later still gets its `State` frame, at the price of one
/// cheap `try_wait` per `idle_poll` per such child.
#[derive(Clone, Copy)]
struct ReapPacing {
    fast_poll: Duration,
    fast_window: Duration,
    slow_poll: Duration,
    slow_window: Duration,
    idle_poll: Duration,
    /// How often the descendant census is retaken while less than
    /// `slow_window` has passed since spawn, or since the output closed.
    census_refresh: Duration,
    /// How often it is retaken after that.
    census_idle_refresh: Duration,
    /// The most sightings the census keeps ([`CENSUS_SIGHTINGS_BOUND`]).
    census_bound: usize,
    /// How long, from the moment the reaper notices the child's exit, its
    /// readers may keep the run open: a descendant that inherited the
    /// output holds it past the exit, and past this grace the run ends
    /// without it (`output_capture`'s own doc comment). The exit is noticed
    /// at the next poll, which while the output is open comes within
    /// `slow_poll` (250 ms) in the child's first minute and within
    /// `idle_poll` (2 s) after it, so with the defaults the run ends at most
    /// about 4 s after the exit (measured: 2.07 s for a young child, 2.81 s
    /// for one older than a minute). Shorter than `settle_wait`, which
    /// bounds the whole settle this grace is part of.
    exit_grace: Duration,
    /// How long `observe` keeps deferring to a claimed reaper that has not
    /// polled: three times the slowest poll (2 s idle), so a healthy reaper
    /// delayed by runtime load is not mistaken for a stalled one.
    stall_grace: Duration,
    /// How long `stop` waits for a settling child's verdict, and how long
    /// `observe` defers to it before taking the settle over. The sweep
    /// itself is bounded at two signal rounds of about 300 ms each, so this
    /// is only a backstop against a stuck blocking pool.
    settle_wait: Duration,
}

impl ReapPacing {
    const DEFAULT: Self = Self {
        fast_poll: Duration::from_millis(10),
        fast_window: Duration::from_secs(3),
        slow_poll: Duration::from_millis(250),
        slow_window: Duration::from_secs(60),
        idle_poll: Duration::from_secs(2),
        census_refresh: Duration::from_millis(250),
        census_idle_refresh: CENSUS_IDLE_REFRESH,
        census_bound: CENSUS_SIGHTINGS_BOUND,
        exit_grace: Duration::from_secs(2),
        stall_grace: Duration::from_secs(6),
        settle_wait: Duration::from_secs(5),
    };

    /// The poll interval `elapsed` after the output closed.
    fn interval(&self, elapsed: Duration) -> Duration {
        if elapsed < self.fast_window {
            self.fast_poll
        } else if elapsed < self.slow_window {
            self.slow_poll
        } else {
            self.idle_poll
        }
    }

    /// The poll interval `elapsed` after spawn, while the output is open.
    fn open_interval(&self, elapsed: Duration) -> Duration {
        if elapsed < self.slow_window {
            self.slow_poll
        } else {
            self.idle_poll
        }
    }

    /// The census interval `elapsed` into the current phase.
    fn census_interval(&self, elapsed: Duration) -> Duration {
        if elapsed < self.slow_window {
            self.census_refresh
        } else {
            self.census_idle_refresh
        }
    }
}

/// Ends a run whose child exited on its own, with no operator action.
///
/// `output_capture::try_finalize` sends the final `State` frame only once
/// the reap is confirmed. One reaper per child starts at spawn
/// ([`Self::run`], spawned by `finish_spawn`): it polls for the child's exit
/// through the same children table and evicts the entry exactly as
/// `observe` does, independently of whether the child's output has closed,
/// and takes the descendant census while the child lives
/// (`crate::descendants` states its interval and blind spot). It reads the
/// exit without reaping (`waitid` with `WNOWAIT`), reads the child's group
/// while the zombie still holds its id, and only then reaps
/// ([`reap_after_exit_census`], shared with `observe`). Once the child is
/// reaped its readers get
/// [`ReapPacing::exit_grace`] to finish; then the verdict is settled
/// without signalling anything ([`self_exit_verdict`]): leftover
/// descendants report `OrphanRiskUncertain` rather than a clean exit, a
/// recorded one that left the process group is named in a diagnostic, and a
/// reader that outlasted the grace -- a descendant holding the output open
/// -- is abandoned so the run still ends. Until the verdict is recorded the
/// id is in `Inner::settling`: `observe` reports the child as running and
/// `stop` waits, so the output is finalized only with the settled verdict.
///
/// It polls without an end (see [`ReapPacing`]) and stops once the child
/// is reaped by anyone or the channel is finalized; when `stop` (or
/// `observe` taking over) reaped it, the reaper still gives the readers the
/// same grace before ending the run with the state that reap recorded. It
/// holds the tables, never `Inner`, and never holds a lock across an await.
struct SelfExitReaper {
    children: Arc<Mutex<HashMap<ProcessId, Tracked>>>,
    terminal_order: Arc<Mutex<VecDeque<ProcessId>>>,
    stopping: Arc<Mutex<HashSet<ProcessId>>>,
    settling: Arc<Mutex<HashMap<ProcessId, std::time::Instant>>>,
    lingering: LingeringGroups,
    exit_members: LingeringGroups,
    pacing: ReapPacing,
    outputs: OutputTable,
    /// Notified by the child's last reader to finish.
    readers_done: Arc<tokio::sync::Notify>,
    id: ProcessId,
    /// The claim `finish_spawn` took for this reaper, if it got one.
    claim: Option<output_capture::ClaimToken>,
}

/// A recorded member of a self-exited child's process group -- always a
/// descendant the census sighted while the child lived, or a member read
/// while the exited child was still an unreaped zombie (which holds the
/// pid that is also the group's id, so that id cannot have been reused);
/// never a process first seen after the reap: what lets a later Stop prove
/// the group is still the original one before signalling it
/// (`descendants::member_still_in_group`). Nothing can be proven without
/// `/proc`, so elsewhere none is ever recorded.
#[cfg(target_os = "linux")]
type GroupMember = descendants::Identity;
#[cfg(not(target_os = "linux"))]
type GroupMember = ();

/// The live members of each self-exited child's process group whose
/// descendant held its output past the exit grace, recorded when its
/// verdict was; see [`Inner::lingering`].
type LingeringGroups = Arc<Mutex<HashMap<ProcessId, Vec<GroupMember>>>>;

/// The diagnostic delivered just before the `State` frame of a run whose
/// readers outlasted the exit grace.
const LINGERING_NOTE: &str = "the child exited, but a descendant still holds its output open: \
                              the run ends here and later output is discarded";

/// At most this many escaped descendants are named one by one; the rest
/// are counted in one more line.
#[cfg(target_os = "linux")]
const MAX_NAMED_ESCAPES: usize = 8;

/// The diagnostic lines naming recorded descendants that left the child's
/// process group and outlived it, by pid and command name (never a path).
#[cfg(target_os = "linux")]
fn escape_notes(escaped: &[(nix::unistd::Pid, String)]) -> Vec<String> {
    let mut notes: Vec<String> = escaped
        .iter()
        .take(MAX_NAMED_ESCAPES)
        .map(|(pid, comm)| {
            format!(
                "descendant {} ({comm}) left the run's process group and is still running",
                pid.as_raw()
            )
        })
        .collect();
    if escaped.len() > MAX_NAMED_ESCAPES {
        notes.push(format!(
            "and {} more descendants left the run's process group and are still running",
            escaped.len() - MAX_NAMED_ESCAPES
        ));
    }
    notes
}

/// What the self-exit reaper settled for a child it reaped.
struct SelfExitVerdict {
    state: ProcessTerminalState,
    /// Diagnostic texts delivered just before the `State` frame.
    notes: Vec<String>,
    /// The group's live members that the census sighted while the child
    /// lived or that were read while it was an unreaped zombie, recorded
    /// only when a reader outlasted the grace: what lets Stop still signal
    /// that group. Never a process first seen after the reap
    /// (`descendants::Leftovers::members`).
    members: Vec<GroupMember>,
    /// Whether a reader outlasted the grace and the run ends without it.
    abandon_readers: bool,
}

impl SelfExitVerdict {
    /// The verdict when the settle itself could not run.
    fn unproven(lingering: bool) -> Self {
        Self {
            state: ProcessTerminalState::OrphanRiskUncertain,
            notes: lingering
                .then(|| LINGERING_NOTE.to_string())
                .into_iter()
                .collect(),
            members: Vec::new(),
            abandon_readers: lingering,
        }
    }
}

/// Settle a reaped self-exited child's verdict from its census, signalling
/// nothing: the reap-derived state stands only when no recorded descendant
/// is still running and the process group is empty, and never when a
/// reader outlasted the exit grace (`lingering`), since something then
/// still holds the output open. `trusted` are the group members read while
/// the exited child was still an unreaped zombie ([`reap_after_exit_census`]).
#[cfg(target_os = "linux")]
fn self_exit_verdict(
    census: &Census,
    id: ProcessId,
    reaped: ProcessTerminalState,
    lingering: bool,
    trusted: &[GroupMember],
) -> SelfExitVerdict {
    let found = census.leftovers(pid_of(id), trusted);
    if let Some(reason) = found.unproven {
        tracing::warn!(
            pid = id.0,
            %reason,
            "a self-exited child's descendants are not proven gone; orphan-risk/uncertain"
        );
    }
    let state = if lingering || found.unproven.is_some() {
        ProcessTerminalState::OrphanRiskUncertain
    } else {
        reaped
    };
    let mut notes = Vec::new();
    if lingering {
        notes.push(LINGERING_NOTE.to_string());
    }
    notes.extend(escape_notes(&found.escaped));
    SelfExitVerdict {
        state,
        notes,
        members: if lingering { found.members } else { Vec::new() },
        abandon_readers: lingering,
    }
}

/// Without `/proc` there is no census to settle from (see the `Census`
/// stub): the reap-derived state stands, unless a reader outlasted the
/// exit grace.
#[cfg(not(target_os = "linux"))]
fn self_exit_verdict(
    census: &Census,
    id: ProcessId,
    reaped: ProcessTerminalState,
    lingering: bool,
    trusted: &[GroupMember],
) -> SelfExitVerdict {
    let _ = (census, id, trusted);
    if lingering {
        SelfExitVerdict::unproven(true)
    } else {
        SelfExitVerdict {
            state: reaped,
            notes: Vec::new(),
            members: Vec::new(),
            abandon_readers: false,
        }
    }
}

/// Marks `id` as being stopped for as long as it lives (see
/// [`Inner::stopping`]).
struct StoppingGuard<'a> {
    stopping: &'a Mutex<HashSet<ProcessId>>,
    id: ProcessId,
}

impl<'a> StoppingGuard<'a> {
    fn new(stopping: &'a Mutex<HashSet<ProcessId>>, id: ProcessId) -> Self {
        stopping
            .lock()
            .expect("stopping mutex poisoned by a prior panic")
            .insert(id);
        Self { stopping, id }
    }
}

/// Gives the self-exit reaper's claim on `id` back when its task ends, on
/// every path: a return, a panic, or the runtime dropping the task at
/// shutdown. A reaper that ends without having reaped the child thus never
/// leaves `observe` deferring to it until `stall_grace`; once the run is
/// finalized the release changes nothing, because a claim is honoured only
/// while the final frame is still owed ([`output_capture::reaper_claim`]).
/// The release names its own claim's token, so once the child is reaped
/// and its pid recycled, it never clears the claim of the new child's
/// reaper under the same id.
struct ClaimRelease {
    outputs: OutputTable,
    id: ProcessId,
    token: Option<output_capture::ClaimToken>,
}

impl Drop for ClaimRelease {
    fn drop(&mut self) {
        if let Some(token) = self.token {
            output_capture::release_reap(&self.outputs, self.id, token);
        }
    }
}

/// Marks `id` as settling (see [`Inner::settling`]) until dropped. Owns its
/// set, so it travels with the reaper task across the blocking sweep.
struct SettlingGuard {
    settling: Arc<Mutex<HashMap<ProcessId, std::time::Instant>>>,
    id: ProcessId,
}

impl Drop for SettlingGuard {
    fn drop(&mut self) {
        // Poison-tolerant: this runs during unwinding as readily as not.
        let mut map = match self.settling.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        map.remove(&self.id);
    }
}

/// How often `stop` re-checks whether a settle is still in flight.
const SETTLE_POLL: Duration = Duration::from_millis(5);

impl Drop for StoppingGuard<'_> {
    fn drop(&mut self) {
        // Poison-tolerant: this runs during unwinding as readily as not.
        let mut set = match self.stopping.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        set.remove(&self.id);
    }
}

impl SelfExitReaper {
    /// Polls for the child's exit from spawn until its run's final frame is
    /// sent. `finish_spawn` takes this reaper's claim before the child is
    /// visible to anyone, so `observe` defers to it from the first instant;
    /// [`ClaimRelease`] gives the claim back when the task ends, however it
    /// ends.
    async fn run(self) {
        let _claim = ClaimRelease {
            outputs: Arc::clone(&self.outputs),
            id: self.id,
            token: self.claim,
        };
        let mut census = take_census_blocking(self.id).await;
        let mut census_at = std::time::Instant::now();
        let mut refresh: Option<JoinHandle<Census>> = None;
        // The current cadence phase began at spawn, and begins again when
        // the child's output closes.
        let mut phase_began = std::time::Instant::now();
        let mut output_closed = false;
        let mut logged_long_running = false;
        loop {
            // Adopt a finished refresh (an unfinished one is left running).
            // While one runs, `census` is unchanged: the refresh started
            // from a copy of it.
            if refresh.as_ref().is_some_and(JoinHandle::is_finished)
                && let Some(handle) = refresh.take()
            {
                census = adopt_refresh(census, handle).await;
            }
            output_capture::note_reap_poll(&self.outputs, self.id);
            #[cfg(test)]
            panic_if_reaper_registered(self.id);
            // The exit is read without reaping; once it is there, the group
            // is read on the blocking pool while the zombie still holds its
            // id, and only then is the child reaped (`try_reap`).
            let exit = match peek_exit(self.id) {
                ExitCensus::Exited(_) => {
                    let id = self.id;
                    tokio::task::spawn_blocking(move || {
                        take_exit_group(id, ExitCensus::Exited(Vec::new()))
                    })
                    .await
                    .unwrap_or(ExitCensus::Exited(Vec::new()))
                }
                other => other,
            };
            match self.try_reap(exit) {
                ReapAttempt::Reaped(state, settling) => {
                    // A walk still in flight is dropped (detached), never
                    // adopted: it may read `/proc` after this reap, once the
                    // child's pid is free for an unrelated process, and only
                    // sightings taken while the child held its pid may later
                    // authorize a group signal (`descendants::Leftovers`).
                    // Every adopted walk finished before this reap.
                    drop(refresh.take());
                    self.end_after_reap(census, state).await;
                    #[cfg(test)]
                    hold_reaper_end(self.id).await;
                    drop(settling);
                    return;
                }
                ReapAttempt::ReapedElsewhere => {
                    self.end_after_reap_elsewhere().await;
                    return;
                }
                ReapAttempt::Finished => return,
                ReapAttempt::Pending => {}
            }
            // Someone else finalized the channel (`observe`/`stop`): done.
            if !output_capture::final_frame_owed(&self.outputs, self.id) {
                return;
            }
            if !output_closed && !output_capture::readers_active(&self.outputs, self.id) {
                output_closed = true;
                phase_began = std::time::Instant::now();
            }
            let elapsed = phase_began.elapsed();
            if output_closed && elapsed >= self.pacing.slow_window && !logged_long_running {
                logged_long_running = true;
                tracing::warn!(
                    pid = self.id.0,
                    after = ?self.pacing.slow_window,
                    "child closed its output but is still running; \
                     polling for its exit at a slow rate"
                );
            }
            // Keep the census fresh while the child lives: a procfs walk
            // costs far more than a `try_wait`, so it runs on the blocking
            // pool, every `census_refresh` early in a phase and every
            // `census_idle_refresh` after.
            if refresh.is_none() && census_at.elapsed() >= self.pacing.census_interval(elapsed) {
                let (id, bound, recorded) = (self.id, self.pacing.census_bound, census.clone());
                refresh = Some(tokio::task::spawn_blocking(move || {
                    refresh_census(recorded, id, bound)
                }));
                census_at = std::time::Instant::now();
            }
            // Stamp again so a long sleep is preceded by a fresh poll time.
            output_capture::note_reap_poll(&self.outputs, self.id);
            if output_closed {
                tokio::time::sleep(self.pacing.interval(elapsed)).await;
            } else {
                // The last reader's end wakes the reaper at once: a child
                // whose output closes is normally exiting.
                let _ = tokio::time::timeout(
                    self.pacing.open_interval(elapsed),
                    self.readers_done.notified(),
                )
                .await;
            }
        }
    }

    /// Give the readers the exit grace, counted from now: `true` when one
    /// is still running after it.
    async fn readers_outlast_grace(&self) -> bool {
        let until = std::time::Instant::now() + self.pacing.exit_grace;
        loop {
            if !output_capture::readers_active(&self.outputs, self.id) {
                return false;
            }
            let left = until.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return true;
            }
            let _ = tokio::time::timeout(left, self.readers_done.notified()).await;
        }
    }

    /// This reaper reaped the child: give its readers the exit grace, then
    /// settle the verdict and record it. The census is read on the blocking
    /// pool, never on the runtime thread that drives every reader task.
    ///
    /// The caller holds this id's [`SettlingGuard`] across the call, so
    /// until the verdict is recorded `observe` does not report (or finalize
    /// with) the reap-derived state and `stop` waits for it. The grace is
    /// shorter than `settle_wait`, which bounds that wait.
    async fn end_after_reap(&self, census: Census, reaped: ProcessTerminalState) {
        let trusted = take_group_record(&self.exit_members, self.id);
        let lingering = self.readers_outlast_grace().await;
        let id = self.id;
        let settle_began = std::time::Instant::now();
        let verdict = tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            hold_settle(id);
            self_exit_verdict(&census, id, reaped, lingering, &trusted)
        })
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "descendant settle task failed; orphan-risk/uncertain");
            SelfExitVerdict::unproven(lingering)
        });
        if settle_began.elapsed() > self.pacing.settle_wait {
            tracing::warn!(
                pid = self.id.0,
                took = ?settle_began.elapsed(),
                "descendant settle took longer than the settle wait"
            );
        }
        record_self_exit_verdict(
            &self.children,
            &self.settling,
            &self.lingering,
            &self.outputs,
            self.id,
            verdict,
        );
    }

    /// `stop` (or `observe` taking over) reaped the child and records its
    /// state. If the readers still hold the final frame back, give them the
    /// same exit grace, then end the run without them, with that recorded
    /// state. When `observe` reaped it, the group members it read while the
    /// child was a zombie are kept for a later Stop, as on the reaper's own
    /// path; `stop` reads none (it signalled the group while the child
    /// lived).
    async fn end_after_reap_elsewhere(&self) {
        let trusted = take_group_record(&self.exit_members, self.id);
        if !output_capture::final_frame_owed(&self.outputs, self.id) {
            return;
        }
        if self.readers_outlast_grace().await {
            if !trusted.is_empty() {
                let children = self
                    .children
                    .lock()
                    .expect("children mutex poisoned by a prior panic");
                keep_group_record(&self.lingering, &children, self.id, trusted);
            }
            output_capture::abandon_readers(
                &self.outputs,
                self.id,
                vec![LINGERING_NOTE.to_string()],
            );
        }
    }

    /// One non-blocking reap attempt, through [`reap_after_exit_census`]
    /// with the exit-time census `exit` taken just before. `Reaped` when
    /// this call reaped the child (it is already `Terminal` and ordered, and
    /// the census's group members are kept for its verdict);
    /// `ReapedElsewhere` when someone else did (then `observe`/`stop` own
    /// the state); `Finished` when nothing is left to poll for -- the entry
    /// is gone, or `try_wait` failed (logged).
    fn try_reap(&self, exit: ExitCensus) -> ReapAttempt {
        let (state, settling) = {
            // Never waits for the table: this runs on the runtime thread
            // that also drives every reader task, and `Inner`'s `Drop` holds
            // `children` while it kills and reaps, relying on those readers
            // to drain a killed PTY child (see its doc comment). The reaper
            // polls every child from spawn, flooding ones included, so a
            // busy table is simply polled again later.
            let mut children = match self.children.try_lock() {
                Ok(guard) => guard,
                Err(std::sync::TryLockError::WouldBlock) => return ReapAttempt::Pending,
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    panic!("children mutex poisoned by a prior panic")
                }
            };
            let Some(tracked) = children.get_mut(&self.id) else {
                return ReapAttempt::Finished;
            };
            let Tracked::Running(child) = tracked else {
                return ReapAttempt::ReapedElsewhere;
            };
            // Checked with `children` held (the only nesting of the two
            // locks): a `stop` that registered before this point owns the
            // state it is about to prove.
            if self
                .stopping
                .lock()
                .expect("stopping mutex poisoned by a prior panic")
                .contains(&self.id)
            {
                return ReapAttempt::Pending;
            }
            match reap_after_exit_census(child, exit) {
                Ok(Some((status, members))) => {
                    let state = ProcessTerminalState::Exited {
                        code: status.code(),
                    };
                    // Registered in the same critical section as the flip,
                    // so no `observe`/`stop` can see the `Terminal` entry
                    // without also seeing that its verdict is still owed.
                    self.settling
                        .lock()
                        .expect("settling mutex poisoned by a prior panic")
                        .insert(self.id, std::time::Instant::now());
                    *tracked = Tracked::Terminal(state);
                    keep_group_record(&self.exit_members, &children, self.id, members);
                    (
                        state,
                        SettlingGuard {
                            settling: Arc::clone(&self.settling),
                            id: self.id,
                        },
                    )
                }
                Ok(None) => return ReapAttempt::Pending,
                // Not a confirmed reap; retrying an error learns nothing.
                Err(error) => {
                    // The reaper ends: its `ClaimRelease` gives the claim back.
                    tracing::warn!(
                        pid = self.id.0,
                        %error,
                        "try_wait failed for a child the self-exit reaper polls; \
                         leaving the reap to observe/stop"
                    );
                    return ReapAttempt::Finished;
                }
            }
        };
        // With `children` unlocked, as `observe` does.
        record_terminal_order(&self.children, &self.terminal_order, self.id);
        ReapAttempt::Reaped(state, settling)
    }
}

/// Record the self-exit reaper's verdict as this child's settled terminal
/// state -- only while `id` is still in `settling`. Once `observe` took the
/// settle over it has reported the reap-derived state, so a late verdict is
/// discarded; a run whose readers outlasted the grace is still ended then,
/// with the state `observe` recorded, rather than left open.
///
/// When the verdict carries the group's live members (a reader outlasted
/// the grace), they are kept for a later `stop` ([`Inner::lingering`]),
/// pruned of ids no longer in `children` so the table stays bounded by it.
///
/// The verdict leaves `settling` before its `State` frame can be sent, so a
/// consumer that calls `observe` on receiving the frame sees the settled
/// state. Its notes are staged first, without a state, so nothing that
/// finalizes the channel (a reader's end, or an `observe` that finds the
/// child terminal) can send the frame yet; then `id` leaves `settling`;
/// only then is the state recorded, and whichever finalize runs first
/// delivers the notes with it. (`stop` on a child already terminal returns
/// the recorded state without finalizing.)
///
/// Locks `children`, then `settling`, then `lingering`; `outputs` and then
/// `settling` are each locked alone after.
fn record_self_exit_verdict(
    children: &Mutex<HashMap<ProcessId, Tracked>>,
    settling: &Mutex<HashMap<ProcessId, std::time::Instant>>,
    lingering: &Mutex<HashMap<ProcessId, Vec<GroupMember>>>,
    outputs: &OutputTable,
    id: ProcessId,
    verdict: SelfExitVerdict,
) {
    {
        let mut guard = children
            .lock()
            .expect("children mutex poisoned by a prior panic");
        let settling = settling
            .lock()
            .expect("settling mutex poisoned by a prior panic");
        if !settling.contains_key(&id) {
            if let Some(Tracked::Terminal(recorded)) = guard.get(&id) {
                tracing::warn!(
                    pid = id.0,
                    settled = ?verdict.state,
                    reported = ?recorded,
                    same_verdict = (*recorded == verdict.state),
                    "late settle verdict discarded; observe already reported \
                     the reap-derived state"
                );
            }
            drop(settling);
            drop(guard);
            if verdict.abandon_readers {
                output_capture::abandon_readers(outputs, id, verdict.notes);
            }
            return;
        }
        if let Some(tracked) = guard.get_mut(&id) {
            *tracked = Tracked::Terminal(verdict.state);
        }
        keep_group_record(lingering, &guard, id, verdict.members);
    }
    output_capture::stage_final_notes(outputs, id, verdict.notes, verdict.abandon_readers);
    settling
        .lock()
        .expect("settling mutex poisoned by a prior panic")
        .remove(&id);
    output_capture::record_confirmed_state(outputs, id, verdict.state);
}

/// Adopt a finished census refresh ([`refresh_census`]): it started from a
/// copy of `census`, which has not changed since, so its result replaces
/// it. A refresh task that failed (it panicked, or the runtime is shutting
/// down) changes nothing.
async fn adopt_refresh(census: Census, refresh: JoinHandle<Census>) -> Census {
    match refresh.await {
        Ok(refreshed) => refreshed,
        Err(error) => {
            tracing::error!(%error, "census refresh task failed; keeping the earlier census");
            census
        }
    }
}

/// The descendant census `stop` takes before it signals anything and
/// consults again before it reports a clean stop.
///
/// On Linux this is the real thing (`crate::descendants`): a
/// `(pid, starttime)` snapshot of the child's descendants, a verified sweep
/// of whatever survived, and a process-group check -- the machinery that
/// keeps `stop` from claiming a clean stop it cannot prove (VP-001
/// `VP-S6-02`).
#[cfg(target_os = "linux")]
use descendants::Census;

/// The census everywhere else (a unix without `/proc` to walk, and
/// Windows): a deliberate no-op, not an oversight. There is no portable way
/// to enumerate a process's descendants there, so `stop` keeps the
/// behaviour it had before the Linux census existed -- the direct child's
/// own reap, with descendants neither proven stopped nor proven running.
/// That is the same pre-existing gap `src-tauri/src/health.rs` already
/// reports as containment `"unproven"`; reporting every such stop as
/// `orphan-risk/uncertain` would be a regression in what this crate can
/// say, not an improvement in what it can prove.
///
/// Generic over the process-group argument (`nix`'s `Pid` on unix, nothing
/// on Windows) so one placeholder serves every call site.
#[cfg(not(target_os = "linux"))]
#[derive(Clone)]
struct Census;

#[cfg(not(target_os = "linux"))]
impl Census {
    fn take<P>(_child: P) -> Self {
        Self
    }

    // Takes `&self` it does not read, so this stub keeps the exact shape
    // the Linux census has and `stop` stays one platform-independent call
    // site (`clippy::unused_self` would have it an associated function).
    // The call sites are unix-only: `windows::stop` never settles, so
    // on Windows the stub is compiled but never used.
    #[allow(clippy::unused_self)]
    #[cfg_attr(not(unix), allow(dead_code))]
    fn settle<P>(&self, _pgid: P, reaped: ProcessTerminalState) -> ProcessTerminalState {
        reaped
    }
}

/// The pid as the kernel's `pid_t`. A pid beyond `i32::MAX` cannot be a real
/// one (`pid_max` is far lower), so the clamp is never a live id.
#[cfg(unix)]
fn pid_of(id: ProcessId) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(i32::try_from(id.0).unwrap_or(i32::MAX))
}

/// Whether `pgid` is still the process group `members` were recorded in
/// (`descendants::member_still_in_group`): only then may a self-exited
/// child's group be signalled by its id.
#[cfg(target_os = "linux")]
fn group_still_original(pgid: nix::unistd::Pid, members: &[GroupMember]) -> bool {
    descendants::member_still_in_group(pgid, members)
}

/// Without `/proc` no member is ever recorded, so nothing is provable.
#[cfg(all(unix, not(target_os = "linux")))]
fn group_still_original(_pgid: nix::unistd::Pid, _members: &[GroupMember]) -> bool {
    false
}

/// What is known about a child's exit before it is reaped
/// ([`reap_after_exit_census`]).
///
/// Only Linux can tell an exit without reaping: `nix` offers `waitid` there
/// and not on Apple targets (calling libc's would need `unsafe`, which this
/// crate denies), and without `/proc` a group walk would see nothing anyway.
/// Elsewhere every census is `Unknown`, which reaps as before.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum ExitCensus {
    /// The child has not exited: it is not reaped now.
    Running,
    /// The child has exited and is still an unreaped zombie; these are the
    /// members of its process group read meanwhile ([`take_exit_group`]).
    Exited(Vec<GroupMember>),
    /// Whether it exited could not be read without reaping it: the reap
    /// goes ahead, as it did before this census existed, and nothing it
    /// finds may authorize a signal.
    Unknown,
}

/// Whether `id` has exited, read without reaping it:
/// `waitid(P_PID, pid, WEXITED | WNOHANG | WNOWAIT)` leaves the zombie in
/// place, so its pid -- the id of its process group -- stays allocated.
#[cfg(target_os = "linux")]
fn peek_exit(id: ProcessId) -> ExitCensus {
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
    let flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT;
    match waitid(Id::Pid(pid_of(id)), flags) {
        Ok(WaitStatus::StillAlive) => ExitCensus::Running,
        Ok(_) => ExitCensus::Exited(Vec::new()),
        Err(errno) => {
            tracing::debug!(
                pid = id.0,
                %errno,
                "waitid(WNOWAIT) failed; reaping without an exit-time group census"
            );
            ExitCensus::Unknown
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn peek_exit(id: ProcessId) -> ExitCensus {
    let _ = id;
    ExitCensus::Unknown
}

/// Fill an `Exited` census with the members of the child's process group,
/// read while the zombie still holds the group's id
/// (`descendants::exit_group_census`). Blocking: a procfs walk.
fn take_exit_group(id: ProcessId, census: ExitCensus) -> ExitCensus {
    #[cfg(target_os = "linux")]
    if matches!(census, ExitCensus::Exited(_)) {
        return ExitCensus::Exited(descendants::exit_group_census(pid_of(id)));
    }
    let _ = id;
    census
}

/// [`peek_exit`] then [`take_exit_group`], in one blocking call.
fn take_exit_census(id: ProcessId) -> ExitCensus {
    take_exit_group(id, peek_exit(id))
}

/// The one way the self-exit reaper and `observe` reap a child: never
/// ahead of its exit-time census. A child known to be running is not
/// reaped at all; an exited one is reaped (`try_wait`, which keeps Tokio's
/// `Child` aware of it) together with the group members read while it was a
/// zombie, which may later authorize Stop to signal that group exactly like
/// sightings made while it lived. `None` when there is nothing to reap.
fn reap_after_exit_census(
    child: &mut Child,
    census: ExitCensus,
) -> std::io::Result<Option<(std::process::ExitStatus, Vec<GroupMember>)>> {
    let members = match census {
        ExitCensus::Running => return Ok(None),
        ExitCensus::Exited(members) => members,
        ExitCensus::Unknown => Vec::new(),
    };
    Ok(child.try_wait()?.map(|status| (status, members)))
}

/// Keep `members` for `id` in `records`, pruned of ids no longer in
/// `children` so the table stays bounded by it. Called with `children`
/// locked; locks `records` after it.
fn keep_group_record(
    records: &Mutex<HashMap<ProcessId, Vec<GroupMember>>>,
    children: &HashMap<ProcessId, Tracked>,
    id: ProcessId,
    members: Vec<GroupMember>,
) {
    if members.is_empty() {
        return;
    }
    let mut records = records
        .lock()
        .expect("group records mutex poisoned by a prior panic");
    records.retain(|kept, _| children.contains_key(kept));
    records.insert(id, members);
}

/// Take what [`keep_group_record`] kept for `id`, if anything.
fn take_group_record(
    records: &Mutex<HashMap<ProcessId, Vec<GroupMember>>>,
    id: ProcessId,
) -> Vec<GroupMember> {
    records
        .lock()
        .expect("group records mutex poisoned by a prior panic")
        .remove(&id)
        .unwrap_or_default()
}

/// Take a descendant census for `id` now.
///
/// What it can see depends on when it runs. A census names the processes
/// that are still the child's descendants at that instant; they are
/// reparented to init the moment the child *exits*, not when it is reaped,
/// so a walk after the exit sees none of them. The self-exit reaper
/// therefore walks from spawn, while the child lives, and merges every walk
/// (`crate::descendants` states the interval and its blind spot); a helper
/// that stayed in the child's group is also caught afterwards by the
/// process-group check, but one that called `setsid` only by a walk that
/// saw it while the child lived.
fn take_census(id: ProcessId) -> Census {
    #[cfg(test)]
    CENSUS_TAKES
        .lock()
        .expect("census counter poisoned")
        .push(id.0);
    #[cfg(test)]
    assert!(
        !FAILING_CENSUSES
            .lock()
            .expect("failing censuses poisoned")
            .contains(&id.0),
        "census walk panicked on purpose (test)"
    );
    #[cfg(unix)]
    {
        Census::take(pid_of(id))
    }
    #[cfg(not(unix))]
    {
        let _ = id;
        Census::take(())
    }
}

/// [`take_census`] on the blocking pool (a procfs walk must not run on the
/// runtime thread that drives every reader task). A task that fails (it
/// panicked, or the runtime is shutting down) is not retried here, on that
/// thread: the census is recorded as unavailable ([`census_task_failed`]).
async fn take_census_blocking(id: ProcessId) -> Census {
    match tokio::task::spawn_blocking(move || take_census(id)).await {
        Ok(census) => census,
        Err(error) => {
            tracing::error!(%error, "census task failed; recording the census as unavailable");
            census_task_failed()
        }
    }
}

/// The census of a failed census task: nothing was seen, so on Linux no
/// clean exit can be claimed from it (the verdict is `OrphanRiskUncertain`).
#[cfg(target_os = "linux")]
fn census_task_failed() -> Census {
    Census::Unavailable(descendants::Unproven::CensusTaskFailed)
}

/// Without `/proc` the census records nothing either way.
#[cfg(not(target_os = "linux"))]
fn census_task_failed() -> Census {
    Census
}

/// Walk again for `id` and fold the walk into `census`, keeping at most
/// `bound` sightings (`descendants::Census::refresh`: sightings no longer
/// alive are forgotten, everything alive is kept). Blocking: a procfs walk.
fn refresh_census(census: Census, id: ProcessId, bound: usize) -> Census {
    #[cfg(test)]
    CENSUS_TAKES
        .lock()
        .expect("census counter poisoned")
        .push(id.0);
    #[cfg(target_os = "linux")]
    {
        census.refresh(pid_of(id), bound)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Nothing to walk: the stub census records nothing.
        let _ = (id, bound);
        census
    }
}

/// Every pid a census was taken for, in order; unit tests count their own.
#[cfg(test)]
static CENSUS_TAKES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Pids whose self-exit settle a unit test holds for the given time, so a
/// settle can be made to outlast the settle wait without a sweep.
#[cfg(test)]
static HELD_SETTLES: Mutex<Vec<(u32, Duration)>> = Mutex::new(Vec::new());

/// Sleep for the time a unit test registered for `id` in [`HELD_SETTLES`].
#[cfg(test)]
fn hold_settle(id: ProcessId) {
    let held = HELD_SETTLES
        .lock()
        .expect("held settles poisoned")
        .iter()
        .find(|(pid, _)| *pid == id.0)
        .map(|(_, hold)| *hold);
    if let Some(hold) = held {
        std::thread::sleep(hold);
    }
}

/// Pids whose self-exit reaper a unit test makes panic at its next poll,
/// so a reaper task that ends without reaping can be exercised.
#[cfg(test)]
static PANICKING_REAPERS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Panic when a unit test registered `id` in [`PANICKING_REAPERS`].
#[cfg(test)]
fn panic_if_reaper_registered(id: ProcessId) {
    let registered = PANICKING_REAPERS
        .lock()
        .expect("panicking reapers poisoned")
        .contains(&id.0);
    assert!(!registered, "self-exit reaper panicked on purpose (test)");
}

/// Pids whose reaper a unit test holds for the given time after the
/// verdict is recorded and before the reaper task ends.
#[cfg(test)]
static HELD_REAPER_ENDS: Mutex<Vec<(u32, Duration)>> = Mutex::new(Vec::new());

/// Wait for the time a unit test registered for `id` in [`HELD_REAPER_ENDS`].
#[cfg(test)]
async fn hold_reaper_end(id: ProcessId) {
    let held = HELD_REAPER_ENDS
        .lock()
        .expect("held reaper ends poisoned")
        .iter()
        .find(|(pid, _)| *pid == id.0)
        .map(|(_, hold)| *hold);
    if let Some(hold) = held {
        tokio::time::sleep(hold).await;
    }
}

/// Pids whose census walk a unit test makes panic, so a failed census task
/// can be exercised.
#[cfg(test)]
static FAILING_CENSUSES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// What one [`SelfExitReaper::try_reap`] concluded.
enum ReapAttempt {
    /// Still running (or a `stop` owns it): ask again later.
    Pending,
    /// This call reaped the child with this reap-derived state; the guard
    /// keeps the id settling until the verdict is recorded.
    Reaped(ProcessTerminalState, SettlingGuard),
    /// `stop` or `observe` reaped it; they record its state.
    ReapedElsewhere,
    /// Nothing left to poll for.
    Finished,
}

/// The bookkeeping this supervisor holds for one spawned process.
///
/// A child is evicted to `Terminal` the moment it is confirmed reaped (in
/// `stop`, or when `observe` notices exit via `try_wait`), and its `Child`
/// handle is dropped at that point. This is what makes a second `stop` (or
/// `observe`) call on the same id safe: once `Terminal`, neither ever
/// touches the OS with `id`'s pid/pgid again, so there is no way to
/// re-signal a process id the OS has since recycled for something else.
/// Every other outcome (a signal error, or a deadline elapsing without a
/// confirmed reap) reports `OrphanRiskUncertain` to the caller but leaves
/// the entry `Running`: the `Child` handle is still live and unreaped, so
/// the OS cannot yet have recycled its pid.
enum Tracked {
    /// A live child, not yet confirmed reaped. Boxed: `Child` is
    /// significantly larger on Windows than the `Terminal` variant, and
    /// boxing keeps every `Tracked` entry (including the far more common
    /// eventual `Terminal` one) at the smaller size instead of every entry
    /// paying for the largest variant.
    Running(Box<Child>),
    /// Confirmed terminal: the `Child` handle has been dropped, and this
    /// state is returned directly to any later `stop`/`observe` call.
    Terminal(ProcessTerminalState),
}

/// A `ProcessSupervisor` backed by `tokio::process`, driven from behind a
/// synchronous interface.
///
/// A cheap, `Clone`-able handle over shared state (`Arc<Inner>`), not a
/// value that itself needs an external lock to be shared across threads.
/// The port's `spawn`/`stop`/`observe` methods take `&mut self` (see
/// `omnifrons_app::ProcessSupervisor`), but that requirement is satisfied
/// per-clone: two clones are two independent local values, so two threads
/// each holding their own clone can call `&mut self` methods concurrently
/// without contending on any single call's duration -- the actual shared
/// mutable state (`Inner::children`, `Inner::outputs`) is separately, and
/// far more finely, synchronized inside. This is what lets a caller (the
/// Tauri shell) store the handle in managed state with no outer `Mutex`:
/// wrapping the whole supervisor in one `Mutex` would otherwise serialize
/// every command behind whichever one happened to be running a
/// multi-second `stop` (`tests/concurrent_handles.rs`).
#[derive(Clone)]
pub struct TokioProcessSupervisor {
    inner: Arc<Inner>,
}

/// The state every [`TokioProcessSupervisor`] handle shares a clone of.
///
/// The actual process I/O this crate needs -- Tokio's SIGCHLD-driven
/// child-exit notification, and the output-capture reader tasks
/// (`docs/spike-log.md` § IPC contract) -- is async. This struct owns a
/// private current-thread [`Runtime`], wrapped in its own [`Arc`] so a
/// dedicated background driver thread (spawned in [`TokioProcessSupervisor::build`])
/// can hold its own clone and actually drive it, independent of `Inner`'s
/// own reference count.
///
/// ## Why a driver thread, not just `enter()`
///
/// Merely `enter()`ing a current-thread runtime around each synchronous
/// call (an earlier implementation) makes the runtime's handle available so
/// `tokio::process::Command` and friends can be constructed, but it does
/// not poll anything: a current-thread runtime's I/O and timer drivers, and
/// every task ever `spawn`ed onto it, only make progress while some thread
/// is inside [`Runtime::block_on`] for that specific runtime (a `Handle`'s
/// own `block_on` does not count -- it "cannot drive IO or timer drivers"
/// on a current-thread runtime "unless another thread is actively calling
/// `Runtime::block_on` on the same runtime", per the Tokio documentation).
/// Without a driver, a reader task spawned to drain a child's stdout would
/// simply never run. The driver thread's sole job is to call
/// `Runtime::block_on` on a future that only resolves when [`Drop`] signals
/// it to (`driver_shutdown`), so the runtime is driven for this
/// supervisor's entire lifetime; `spawn`/`stop`/`observe` still use
/// `enter()` from whichever thread calls them, exactly as before, purely to
/// construct runtime-dependent resources on that thread.
struct Inner {
    runtime: Arc<Runtime>,
    /// `Arc`-shared so the self-exit reaper task ([`SelfExitReaper::run`])
    /// can hold the table without holding `Inner` itself: `Inner`'s `Drop`
    /// joins the driver thread, which a task running on that thread must
    /// never be the one to trigger.
    children: Arc<Mutex<HashMap<ProcessId, Tracked>>>,
    /// The order, oldest first, in which entries in `children` transitioned
    /// to `Tracked::Terminal`. Every id pushed here exactly once, at the
    /// moment of that transition (`record_terminal_order`); consulted only
    /// to decide which entry to evict once [`MAX_RETAINED_TERMINAL_CHILDREN`]
    /// is exceeded.
    terminal_order: Arc<Mutex<VecDeque<ProcessId>>>,
    /// Ids a `stop` call is working on right now. The self-exit reaper
    /// leaves these alone, so the state `stop` proves (`Killed`, or the
    /// settled descendant verdict) is never pre-empted by a bare reap of the
    /// child that `stop` itself just signalled.
    stopping: Arc<Mutex<HashSet<ProcessId>>>,
    /// Ids whose self-exit reap is done but whose descendant sweep is not.
    /// Their `Tracked::Terminal` carries only the reap-derived state, so
    /// until the settle records the verdict `observe` reports the child as
    /// still running (and finalizes nothing) and `stop` waits.
    settling: Arc<Mutex<HashMap<ProcessId, std::time::Instant>>>,
    /// The live members of a self-exited child's process group, recorded
    /// with its verdict when a descendant still held its output after the
    /// exit grace -- only descendants the census sighted while the child
    /// lived and members read while it was an unreaped zombie, never a
    /// process first seen after the reap. A later `stop`
    /// signals that group only while one of them is provably still in it,
    /// so the group id cannot have been recycled
    /// (`unix::signal_lingering_group`). Never recorded without `/proc`.
    /// Kept no longer than the id's own `children` entry.
    lingering: LingeringGroups,
    /// The group members read while a child that exited on its own was
    /// still an unreaped zombie ([`reap_after_exit_census`]), kept from its
    /// reap until the self-exit reaper settles the run and moves the
    /// relevant ones into `lingering`. Bounded like it.
    exit_members: LingeringGroups,
    /// The self-exit reaper's cadence; only tests use anything but the default.
    reap_pacing: ReapPacing,
    /// Per-child output-capture bookkeeping (`output_capture` module),
    /// deliberately a separate table from `children`: it must outlive a
    /// `Tracked` entry's own Running/Terminal transition (`observe`/`stop`
    /// must keep reporting a confirmed reap immediately, not wait on
    /// output capture), and a `subscribe` call must still find a process
    /// that has already gone terminal.
    outputs: OutputTable,
    /// Set only by [`TokioProcessSupervisor::with_demo_launcher`]; the path
    /// `spawn_harness` execs. No program path or argument vector for a
    /// harness request ever crosses IPC -- this is the one place a real
    /// path lives, fixed at construction (`docs/spike-log.md` § IPC
    /// contract).
    demo_launcher: Option<PathBuf>,
    /// Sending on this tells the driver thread's `block_on` to return.
    /// `None` after `Drop` has already taken it. Never touched outside
    /// construction and `Drop`, so this needs no lock of its own: by the
    /// time `Drop::drop` runs, this `Inner` -- the last surviving
    /// `Arc<Inner>` having just been dropped -- is uniquely owned again.
    driver_shutdown: Option<oneshot::Sender<()>>,
    /// Joined in `Drop`, after signalling shutdown, so the driver thread's
    /// own `Arc<Runtime>` clone is guaranteed dropped before this `Inner`'s
    /// other fields (including its own `runtime` clone) are.
    driver_thread: Option<std::thread::JoinHandle<()>>,
}

impl Default for TokioProcessSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl TokioProcessSupervisor {
    /// Build a new supervisor with its own private Tokio runtime, driven by
    /// a dedicated background thread for this supervisor's entire lifetime.
    ///
    /// # Panics
    ///
    /// Panics if a current-thread Tokio runtime could not be built (e.g.
    /// the OS refuses to create the runtime's I/O/timer driver), or if the
    /// driver thread could not be spawned.
    #[must_use]
    pub fn new() -> Self {
        Self::build(None)
    }

    /// Build a supervisor exactly like [`Self::new`], additionally
    /// configured to run demo harness requests against the binary at
    /// `path` (the shell passes `tauri::process::current_binary(&env)`;
    /// this crate's own tests pass `env!("CARGO_BIN_EXE_demo-harness")`).
    ///
    /// # Panics
    ///
    /// Panics under the same conditions as [`Self::new`].
    #[must_use]
    pub fn with_demo_launcher(path: PathBuf) -> Self {
        Self::build(Some(path))
    }

    /// Shared construction for [`Self::new`] and [`Self::with_demo_launcher`].
    /// `demo_launcher` must be known up front, before `Inner` is wrapped in
    /// its `Arc`: unlike the pre-handle design, there is no later point at
    /// which a lone, uniquely-owned value could still be mutated in place.
    fn build(demo_launcher: Option<PathBuf>) -> Self {
        Self::build_with(demo_launcher, ReapPacing::DEFAULT)
    }

    fn build_with(demo_launcher: Option<PathBuf>, reap_pacing: ReapPacing) -> Self {
        let runtime = Arc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build the process supervisor's Tokio runtime"),
        );

        let (driver_shutdown, shutdown_rx) = oneshot::channel();
        let driver_runtime = Arc::clone(&runtime);
        let driver_thread = std::thread::Builder::new()
            .name("omnifrons-supervisor-driver".to_string())
            .spawn(move || {
                // `Runtime::block_on` (not `Handle::block_on`) is required
                // here: only it actually drives a current-thread runtime's
                // I/O and timer reactors and every task spawned onto it --
                // see `Inner`'s own doc comment.
                driver_runtime.block_on(async move {
                    let _ = shutdown_rx.await;
                });
            })
            .expect("failed to spawn the process supervisor's runtime driver thread");

        Self {
            inner: Arc::new(Inner {
                runtime,
                children: Arc::new(Mutex::new(HashMap::new())),
                terminal_order: Arc::new(Mutex::new(VecDeque::new())),
                stopping: Arc::new(Mutex::new(HashSet::new())),
                settling: Arc::new(Mutex::new(HashMap::new())),
                lingering: Arc::new(Mutex::new(HashMap::new())),
                exit_members: Arc::new(Mutex::new(HashMap::new())),
                reap_pacing,
                outputs: Arc::new(Mutex::new(HashMap::new())),
                demo_launcher,
                driver_shutdown: Some(driver_shutdown),
                driver_thread: Some(driver_thread),
            }),
        }
    }

    /// Turn a validated [`HarnessRequest`] into a real process, without
    /// ever letting a program path or argument vector cross whatever
    /// boundary produced the request (`docs/spike-log.md` § IPC contract):
    /// the launcher path was already fixed at construction
    /// ([`Self::with_demo_launcher`]), and `kind`/`rate_hz`/`lines` are the
    /// only values this method turns into argv.
    ///
    /// # Errors
    ///
    /// Returns [`SupervisorError::Spawn`] if the underlying process could
    /// not be started.
    ///
    /// # Panics
    ///
    /// Panics if this supervisor was not built via
    /// [`Self::with_demo_launcher`].
    pub fn spawn_harness(
        &mut self,
        request: &HarnessRequest,
    ) -> Result<ProcessId, SupervisorError> {
        let launcher = self.inner.demo_launcher.clone().expect(
            "spawn_harness requires a supervisor built via TokioProcessSupervisor::with_demo_launcher",
        );
        let spec = ProcessSpec::new(launcher.to_string_lossy().into_owned()).with_args([
            demo::kind_arg(&request.kind())
                .expect(
                    "omnifrons_app::HarnessRequest::new rejects HarnessKind::Approved with \
                     InvalidRequest::KindNotDemo, so a HarnessRequest's own kind() is never \
                     Approved here",
                )
                .to_string(),
            request.rate_hz().to_string(),
            request.lines().to_string(),
        ]);
        self.spawn(spec)
    }

    /// Spawn `future` onto this supervisor's runtime, to actually run in
    /// the background on its driver thread.
    ///
    /// Exposed beyond this crate's own use (the output-capture reader
    /// tasks) so the runtime-is-actually-driven property is independently
    /// testable (`tests/runtime_drives_tasks.rs`) without reaching into
    /// this struct's private fields.
    pub fn spawn_on_runtime<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.inner.runtime.spawn(future)
    }
}

impl TokioProcessSupervisor {
    /// The ids of every child currently `Running`, each refreshed through
    /// the same reap check [`ProcessSupervisor::observe`] performs, so a
    /// child that exited but was never observed is not reported as running.
    /// Spike slice 5b: the one source the shell's publication surface
    /// consults to stay frozen while a run is live, exactly as
    /// `harness_stop` and `harness_observe` consult this supervisor.
    ///
    /// # Panics
    ///
    /// Panics if the children mutex was poisoned by a prior panic while
    /// held, as every other accessor of it does.
    #[must_use]
    pub fn running_ids(&self) -> Vec<ProcessId> {
        let mut ids: Vec<ProcessId> = self
            .inner
            .children
            .lock()
            .expect("children mutex poisoned by a prior panic")
            .keys()
            .copied()
            .collect();
        ids.sort_by_key(|id| id.0);
        ids.into_iter()
            .filter(|id| matches!(self.observe(*id), Some(ProcessStatus::Running)))
            .collect()
    }

    /// Refuse now, with [`SupervisorError::TooManyProcesses`], if
    /// [`MAX_RUNNING_CHILDREN`] children are already `Running`.
    ///
    /// Called twice on an approved launch: by [`Self::spawn_approved`]
    /// before any transport-specific resource is allocated (so a caller at
    /// the cap is refused before a pseudo-terminal is ever opened, exactly
    /// as the pipe path -- which allocates nothing before
    /// [`Self::finish_spawn`] -- always was), and by `finish_spawn` itself
    /// immediately before the spawn. Neither call holds the `children` lock
    /// through to the insertion, so two concurrent launches can both pass
    /// the check and briefly exceed the cap by one: a pre-existing window,
    /// shared by both transports, and accepted for a bound that exists to
    /// keep bookkeeping finite rather than to be exact.
    fn ensure_running_capacity(&self) -> Result<(), SupervisorError> {
        let children = self
            .inner
            .children
            .lock()
            .expect("children mutex poisoned by a prior panic");
        let running_count = children
            .values()
            .filter(|tracked| matches!(tracked, Tracked::Running(_)))
            .count();
        if running_count >= MAX_RUNNING_CHILDREN {
            return Err(SupervisorError::TooManyProcesses);
        }
        Ok(())
    }

    /// Shared tail of [`ProcessSupervisor::spawn`] and [`Self::spawn_approved`]:
    /// given an already-configured `Command` (program, args, and any
    /// platform-specific process-group setup already applied by the
    /// caller), enforce the running-process cap, pipe stdout/stderr for
    /// capture, spawn, and wire this supervisor's own bookkeeping.
    ///
    /// `program_label` is used only for `tracing`/error text -- never the
    /// literal argv the child actually receives, which the caller has
    /// already baked into `command`.
    ///
    /// `env` and the stdio wiring are applied here, centrally, so every
    /// caller ([`ProcessSupervisor::spawn`] and [`Self::spawn_approved`],
    /// on either transport) shares exactly one place that ever actually
    /// sets an environment variable, configures a standard stream, or
    /// groups a child on a real process -- including the secret-shaped
    /// re-check ([`is_secret_shaped`]) on every allowlisted key, a single
    /// choke point regardless of what the caller's own `env` claims
    /// (`docs/spike-log.md` § Slice 3), and, as of spike slice 4, the
    /// branch between piped stdio and a pseudo-terminal ([`StdioPlan`]).
    ///
    /// `prompt`, when `Some`, is delivered on this supervisor's own runtime
    /// so it never blocks the calling thread: written to the child's stdin
    /// then the write end dropped (signalling EOF) under
    /// [`StdioPlan::Pipes`], or typed into the terminal followed by a
    /// carriage return under [`StdioPlan::Pty`]. Either way a write failure
    /// is surfaced as a `stderr` frame
    /// (`output_capture::emit_stdin_write_failed`) rather than silently
    /// swallowed or left to hang. Under `Pipes`, `prompt` must be `Some`
    /// only when its stdin is [`StdinPlan::PipePromptThenClose`] --
    /// [`plan_stdio`] upholds that pairing for every caller.
    ///
    /// `inheritable_fd`, when `Some` (Linux only), is cleared, spawned, then
    /// restored -- even on a failed spawn -- all inside [`SPAWN_LOCK`]. Only
    /// a failing restore, logged as an error, could leave it inheritable.
    fn finish_spawn(
        &mut self,
        mut command: Command,
        program_label: &str,
        env: &EnvPlan,
        stdio: StdioPlan,
        prompt: Option<Vec<u8>>,
        inheritable_fd: Option<&std::fs::File>,
    ) -> Result<ProcessId, SupervisorError> {
        #[cfg(target_os = "linux")]
        use std::os::fd::AsFd as _;
        let _ = inheritable_fd; // read below on Linux only, where one exists

        apply_env(&mut command, env);
        let wiring = configure_stdio(&mut command, stdio)?;

        self.ensure_running_capacity()?;

        // Best-effort safety net: if this supervisor is dropped without an
        // explicit `stop` for this child (a panicking test, an early
        // return), `Drop` below still tries to signal it, but `kill_on_drop`
        // also asks Tokio itself to try killing the direct child should the
        // `Child` handle be dropped some other way. Neither is containment
        // -- see `Drop`'s own doc comment.
        command.kill_on_drop(true);

        // `SPAWN_LOCK` covers the fork and `inheritable_fd`'s flag.
        let guard = spawn_lock();

        #[cfg(target_os = "linux")]
        if let Some(file) = inheritable_fd {
            nix::fcntl::fcntl(
                file.as_fd(),
                nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
            )
            .map_err(|error| {
                SupervisorError::Spawn(format!(
                    "close-on-exec clear on sealed memfd failed: {error}"
                ))
            })?;
        }

        let spawned = command.spawn();

        #[cfg(target_os = "linux")]
        if let Some(file) = inheritable_fd
            && let Err(error) = nix::fcntl::fcntl(
                file.as_fd(),
                nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
            )
        {
            tracing::error!(%error, "close-on-exec restore on sealed memfd failed");
        }
        drop(guard);

        let mut child = spawned.map_err(|error| {
            tracing::warn!(program = %program_label, %error, "failed to spawn process");
            SupervisorError::Spawn(error.to_string())
        })?;
        let pid = child
            .id()
            .ok_or_else(|| SupervisorError::Spawn("spawned child reported no pid".to_string()))?;
        let id = ProcessId(pid);

        let (output_state, handles) = output_capture::new_channel(wiring.reader_count());
        let readers_done = handles.readers_done();
        self.inner
            .outputs
            .lock()
            .expect("outputs mutex poisoned by a prior panic")
            .insert(id, output_state);
        // The self-exit reaper's claim, taken here rather than by its task:
        // the child is not in `children` yet, so no `observe` can reap it
        // ahead of the reaper that owns the verdict.
        let claim = output_capture::claim_reap(&self.inner.outputs, id);
        debug_assert!(claim.is_some(), "a fresh output entry is always claimable");

        // Every reader and writer task is attached before this child is
        // ever inserted into `children`, so no other code path can observe
        // a `Tracked::Running` whose streams have already been taken out
        // from under it.
        self.attach_output(&mut child, wiring, handles, id, prompt);

        // On the PTY path `command` still holds this process's copies of
        // the slave end, and the master only reports EOF once every slave
        // descriptor outside the child is closed, so they are released
        // here -- but only now, with the reader attached. Released before
        // it, on macOS this close blocked for about 600 ms whenever the
        // child had already written and exited, and the child's whole
        // output was gone when it returned
        // (`tests/pty_fast_exit_keeps_output.rs`). On the pipe path this is
        // a no-op.
        drop(command);

        self.inner
            .children
            .lock()
            .expect("children mutex poisoned by a prior panic")
            .insert(id, Tracked::Running(Box::new(child)));

        // From spawn, not from the readers' end: a descendant that inherited
        // the output can keep the readers running long after the child has
        // exited. Spawned only now, with the entry in `children` to poll.
        self.spawn_on_runtime(self.reaper(id, readers_done, claim).run());

        tracing::debug!(program = %program_label, pid, "spawned process");
        Ok(id)
    }

    /// The handles the self-exit reaper needs for `id`, cloned out of
    /// `Inner` so the reaper task never keeps `Inner` itself alive.
    fn reaper(
        &self,
        id: ProcessId,
        readers_done: Arc<tokio::sync::Notify>,
        claim: Option<output_capture::ClaimToken>,
    ) -> SelfExitReaper {
        SelfExitReaper {
            children: Arc::clone(&self.inner.children),
            terminal_order: Arc::clone(&self.inner.terminal_order),
            stopping: Arc::clone(&self.inner.stopping),
            settling: Arc::clone(&self.inner.settling),
            lingering: Arc::clone(&self.inner.lingering),
            exit_members: Arc::clone(&self.inner.exit_members),
            pacing: self.inner.reap_pacing,
            outputs: Arc::clone(&self.inner.outputs),
            readers_done,
            id,
            claim,
        }
    }

    /// Attach the reader task(s) and, when `prompt` is `Some`, the prompt
    /// writer for a freshly spawned `child`, per its [`Wiring`]: two
    /// drains (stdout, stderr) plus a stdin writer under `Pipes`; one drain
    /// on the master plus the prompt typist under `Pty`. Every task runs on
    /// this supervisor's own runtime, never the calling thread, so a slow
    /// or absent reader on the child's side can never block
    /// `spawn`/`spawn_approved` itself.
    ///
    /// `wiring` is consumed on unix (the `Pty` variant moves the master
    /// into its tasks); on a platform without the PTY path it is a
    /// single `Copy`-able variant, hence the scoped allow.
    #[cfg_attr(not(unix), allow(clippy::needless_pass_by_value))]
    fn attach_output(
        &self,
        child: &mut Child,
        wiring: Wiring,
        handles: ReaderHandles,
        id: ProcessId,
        prompt: Option<Vec<u8>>,
    ) {
        match wiring {
            Wiring::Pipes => {
                let stdout = child
                    .stdout
                    .take()
                    .expect("stdout was configured as piped by configure_stdio");
                let stderr = child
                    .stderr
                    .take()
                    .expect("stderr was configured as piped by configure_stdio");
                let stderr_handles = handles.clone();
                let stdout_drain = output_capture::drain_stream(
                    stdout,
                    omnifrons_app::OutputStream::Stdout,
                    handles,
                    Arc::clone(&self.inner.outputs),
                    id,
                );
                let stderr_drain = output_capture::drain_stream(
                    stderr,
                    omnifrons_app::OutputStream::Stderr,
                    stderr_handles,
                    Arc::clone(&self.inner.outputs),
                    id,
                );
                self.spawn_on_runtime(stdout_drain);
                self.spawn_on_runtime(stderr_drain);

                if let Some(prompt_bytes) = prompt {
                    let mut stdin_handle = child
                        .stdin
                        .take()
                        .expect("stdin was configured as piped for a Some prompt");
                    let outputs_for_stdin = Arc::clone(&self.inner.outputs);
                    self.spawn_on_runtime(async move {
                        if stdin_handle.write_all(&prompt_bytes).await.is_err() {
                            output_capture::emit_stdin_write_failed(&outputs_for_stdin, id);
                        }
                        // Dropping the write end signals EOF to the child,
                        // regardless of whether the write itself succeeded.
                        drop(stdin_handle);
                    });
                }
            }
            #[cfg(unix)]
            Wiring::Pty(master) => {
                // One reader on the master, labelled `stdout`: a
                // pseudo-terminal has no separate stderr, and the frames
                // keep slice 1's framing (cap, `continued`, CRLF handling)
                // by reusing the same drain.
                let drain = output_capture::drain_stream(
                    pty::MasterReader::new(Arc::clone(&master)),
                    omnifrons_app::OutputStream::Stdout,
                    handles,
                    Arc::clone(&self.inner.outputs),
                    id,
                );
                self.spawn_on_runtime(drain);
                if let Some(prompt_bytes) = prompt {
                    self.spawn_on_runtime(pty::type_prompt(
                        master,
                        prompt_bytes,
                        Arc::clone(&self.inner.outputs),
                        id,
                    ));
                }
            }
        }
    }

    /// Launch `handle` with no arguments and `stdin` always `/dev/null`
    /// (never the executable's own bytes), after a `LaunchGate` decision
    /// has confirmed it may run (`docs/spike-log.md` § Slice 2).
    /// `display_path` is used only for `tracing`/error text.
    ///
    /// ## `ExecHandle::SealedMemory` (Linux)
    ///
    /// Execs the sealed `memfd` itself via the classic `/proc/self/fd/<n>`
    /// technique (glibc's own `fexecve` falls back to exactly this on
    /// Linux when the direct `execveat` syscall path is unavailable): the
    /// *path string* given to `execve`, `/proc/self/fd/<n>`, is resolved
    /// through `/proc`'s special handling to the exact open file
    /// description fd `<n>` names, not looked up by name in the ordinary
    /// filesystem -- so the content actually read for the exec is exactly
    /// the sealed bytes `omnifrons_app::ExecutableProber::probe` hashed,
    /// regardless of what `display_path` resolves to by the time this
    /// call runs, or ever again afterward (the seal makes further
    /// modification impossible in the first place).
    ///
    /// The memfd was created `MFD_CLOEXEC` (`omnifrons-adapters`'
    /// `fs_prober` doc comment) so it is never inherited by some unrelated
    /// `exec` before this deliberate moment; [`Self::finish_spawn`] clears
    /// that flag under [`SPAWN_LOCK`] for this `spawn` only and restores it
    /// right after, so it *is* inherited across *this* `exec` and, unless
    /// that restore fails, no other -- with no `unsafe`: `nix`'s `fcntl` is
    /// itself safe. A direct-binary target only ever needs the kernel's own
    /// single internal open of `/proc/self/fd/<n>` (performed before the
    /// calling process's old image is replaced), but a *script* target (a
    /// `#!`-interpreted file) does not: the kernel's shebang handling
    /// re-execs the named interpreter, whose own userspace start-up code
    /// re-opens that same `/proc/self/fd/<n>` string a *second* time, as
    /// an ordinary syscall from its own, by then fully-committed process
    /// image -- which only succeeds because the fd survived the first
    /// `execve` inheritably. The fd therefore remains open in the child by
    /// design, for as long as the child process runs -- not a leak.
    ///
    /// `stdin` is always `Stdio::null()`: unlike an earlier version of
    /// this method (which passed the executable itself as `stdin` to
    /// exploit `dup2`'s always-inheritable duplicate, sidestepping the
    /// close-on-exec question a different way), the executable's own
    /// bytes must never be reachable through the child's standard input
    /// (`crates/omnifrons-supervisor/tests/approved_launch.rs`'s
    /// `spawn_approved_never_feeds_the_executable_as_stdin` proves a
    /// fixture reading its own `stdin` observes immediate `EOF`, never the
    /// script's own source).
    ///
    /// ## `ExecHandle::File` (macOS, Windows, or a Linux fallback)
    ///
    /// Spawns by `display_path` directly, dropping `handle` first: no
    /// sealing is available for a plain, unsealed file, so only the
    /// *inode identity* observed while hashing is guaranteed -- nothing
    /// prevents that same inode's content from being modified in place,
    /// or the path from being retargeted, between this call and the
    /// actual `execve`/`CreateProcess` (`docs/spike-log.md` § Slice 2).
    ///
    /// As of the spike slice-3 spike, `plan` additionally supplies the
    /// argument vector (`plan.argv` -- still never anything but a
    /// `HarnessAdapter`'s own fixed template; see
    /// `docs/spike-log.md` § Slice 3), the environment (`plan.env`), the
    /// working directory (`plan.cwd`), and, when `plan.stdin` is
    /// [`StdinPlan::PipePromptThenClose`], the prompt delivered over
    /// stdin then the write end closed. `stdin` is otherwise always
    /// `Stdio::null()` (`plan.stdin` is [`StdinPlan::Null`]) -- never the
    /// executable's own bytes, matching this method's pre-slice-3
    /// behavior exactly for a plan carrying no argv/prompt (an empty argv,
    /// an allowlist environment plan, and `StdinPlan::Null` reproduces the
    /// slice-2 no-argument, `/dev/null`-stdin launch precisely).
    ///
    /// ## `TransportClass::Pty` (spike slice 4, unix only)
    ///
    /// When `plan.transport` is [`TransportClass::Pty`], the child gets one
    /// pseudo-terminal (fixed 80x24) as stdin, stdout, and stderr, runs as
    /// the session leader of that controlling terminal (`setsid` plus
    /// `TIOCSCTTY` in a `pre_exec`, in place of `process_group(0)` -- see
    /// `pty::install_controlling_terminal`), and receives `TERM=dumb`,
    /// `COLUMNS=80`, `LINES=24` set explicitly by this supervisor on top
    /// of `plan.env`; `plan.prompt`, when `Some`, is typed into the
    /// terminal followed by a carriage return, and `plan.stdin` is unused.
    /// Output is captured by one reader on the master, labelled `stdout`
    /// (a terminal has no stderr). Both handle kinds work on this path:
    /// the sealed-memfd exec survives the fork-then-exec `pre_exec`
    /// forces, since `std` never closes inherited descriptors in the
    /// child. On Windows this returns [`SupervisorError::PtyUnsupported`]
    /// before touching the handle or any bookkeeping
    /// (`docs/spike-log.md` § Slice 4).
    ///
    /// # Errors
    ///
    /// Returns [`SupervisorError::Spawn`] if the underlying process could
    /// not be started (including if clearing close-on-exec on a sealed
    /// memfd failed, or a pseudo-terminal could not be opened),
    /// [`SupervisorError::TooManyProcesses`] under the same running-child
    /// cap as [`ProcessSupervisor::spawn`], or
    /// [`SupervisorError::PtyUnsupported`] for a `Pty` plan on a platform
    /// without the PTY path.
    pub fn spawn_approved(
        &mut self,
        handle: omnifrons_app::ExecHandle,
        display_path: PathBuf,
        plan: &LaunchPlan,
    ) -> Result<ProcessId, SupervisorError> {
        let _guard = self.inner.runtime.enter();
        let label = display_path.to_string_lossy().into_owned();
        // Decided before `handle` is touched, in this order: an unsupported
        // transport is refused first, with nothing else consulted; then the
        // running cap, before any transport-specific resource exists (a
        // pseudo-terminal is opened by `plan_stdio` below, and a caller at
        // the cap must never cause that allocation); only then the wiring.
        #[cfg(not(unix))]
        if plan.transport == TransportClass::Pty {
            return Err(SupervisorError::PtyUnsupported);
        }
        self.ensure_running_capacity()?;
        let (stdio, prompt) = plan_stdio(plan)?;

        match handle {
            #[cfg(target_os = "linux")]
            omnifrons_app::ExecHandle::SealedMemory(file) => {
                let fd = {
                    use std::os::fd::AsRawFd as _;
                    file.as_raw_fd()
                };
                let mut command = Command::new(format!("/proc/self/fd/{fd}"));
                command.args(&plan.argv);
                command.current_dir(plan.cwd.path());
                // `Some(&file)` is what lets it become briefly inheritable.
                let result =
                    self.finish_spawn(command, &label, &plan.env, stdio, prompt, Some(&file));
                drop(file);
                result
            }
            omnifrons_app::ExecHandle::File(file) => {
                drop(file);
                let mut command = Command::new(display_path);
                command.args(&plan.argv);
                command.current_dir(plan.cwd.path());
                self.finish_spawn(command, &label, &plan.env, stdio, prompt, None)
            }
        }
    }
}

impl ProcessSupervisor for TokioProcessSupervisor {
    fn spawn(&mut self, spec: ProcessSpec) -> Result<ProcessId, SupervisorError> {
        let _guard = self.inner.runtime.enter();

        let mut command = Command::new(&spec.program);
        command.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }

        self.finish_spawn(
            command,
            &spec.program,
            &spec.env,
            StdioPlan::Pipes(spec.stdin),
            None,
            None,
        )
    }

    fn stop(
        &mut self,
        id: ProcessId,
        deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        let _guard = self.inner.runtime.enter();
        let _stopping = StoppingGuard::new(&self.inner.stopping, id);
        // With the guard registered no new self-exit reap can start, so a
        // settle in flight is the only one there will be: wait for its
        // verdict instead of reporting (and recording) the bare reap.
        let waited = std::time::Instant::now();
        while self
            .inner
            .settling
            .lock()
            .expect("settling mutex poisoned by a prior panic")
            .contains_key(&id)
        {
            if waited.elapsed() >= self.inner.reap_pacing.settle_wait {
                tracing::warn!(
                    pid = id.0,
                    "settle verdict not recorded in time; stopping anyway"
                );
                break;
            }
            std::thread::sleep(SETTLE_POLL);
        }

        #[cfg(unix)]
        {
            unix::stop(
                &self.inner.children,
                &self.inner.terminal_order,
                &self.inner.lingering,
                &self.inner.outputs,
                id,
                deadline,
            )
        }
        #[cfg(windows)]
        {
            windows::stop(&self.inner.children, &self.inner.outputs, id, deadline)
        }
    }

    fn observe(&self, id: ProcessId) -> Option<ProcessStatus> {
        let _guard = self.inner.runtime.enter();
        // Read before `children` is locked. While the self-exit reaper
        // holds this child it alone confirms the reap, so that its
        // descendant sweep decides the terminal state; `observe` reports
        // `Running` meanwhile (at most the reaper's next poll plus the
        // sweep). The deferral lasts while the reaper keeps polling: it ends
        // once its last poll (its claim, before the first) is older than
        // `stall_grace`, and `observe` then confirms the reap itself.
        let claim = output_capture::reaper_claim(&self.inner.outputs, id);
        let reaper_active = match &claim {
            Some(claim)
                if claim.last_poll_at.unwrap_or(claim.claimed_at).elapsed()
                    < self.inner.reap_pacing.stall_grace =>
            {
                true
            }
            Some(claim) => {
                if output_capture::note_deferral_expired(&self.inner.outputs, id) {
                    tracing::warn!(
                        pid = id.0,
                        claimed_ago = ?claim.claimed_at.elapsed(),
                        polls = claim.polls,
                        last_poll_ago = ?claim.last_poll_at.map(|at| at.elapsed()),
                        "self-exit reaper stopped polling; observing the child directly"
                    );
                }
                false
            }
            None => false,
        };
        let claimed = claim.is_some();
        // A child `observe` may reap is reaped like the reaper's: only after
        // its exit-time census, taken before `children` is locked (it may
        // walk `/proc`) and only for a child still running here.
        // Its own statement, so the lock is released before the census.
        let running = matches!(
            self.inner
                .children
                .lock()
                .expect("children mutex poisoned by a prior panic")
                .get(&id),
            Some(Tracked::Running(_))
        );
        let mut exit = (!reaper_active && running).then(|| take_exit_census(id));
        let mut children = self
            .inner
            .children
            .lock()
            .expect("children mutex poisoned by a prior panic");
        let tracked = children.get_mut(&id)?;
        let mut just_became_terminal = false;
        let mut trusted = Vec::new();
        let result = match tracked {
            // The self-exit reaper reaped it but has not recorded its
            // settled verdict: report the child as still running and
            // finalize nothing with the reap-derived state.
            // Past `settle_wait` `observe` takes the settle over: it drops the
            // id from `settling` inside this `children` critical section, so
            // the reaper's late verdict is discarded and one child never shows
            // two terminal states. `children` may nest `settling`, not back.
            Tracked::Terminal(state) => {
                let mut settling = self
                    .inner
                    .settling
                    .lock()
                    .expect("settling mutex poisoned by a prior panic");
                match settling.get(&id).map(std::time::Instant::elapsed) {
                    Some(waited) if waited < self.inner.reap_pacing.settle_wait => {
                        Some(ProcessStatus::Running)
                    }
                    Some(waited) => {
                        settling.remove(&id);
                        tracing::warn!(
                            pid = id.0,
                            waited = ?waited,
                            "settle verdict not recorded within the settle wait; \
                             reporting the reap-derived state"
                        );
                        Some(ProcessStatus::Terminal(*state))
                    }
                    None => Some(ProcessStatus::Terminal(*state)),
                }
            }
            Tracked::Running(_) if reaper_active => Some(ProcessStatus::Running),
            // Without a census taken above, nothing is reaped.
            Tracked::Running(child) => {
                match reap_after_exit_census(child, exit.take().unwrap_or(ExitCensus::Running)) {
                    Ok(Some((status, members))) => {
                        let state = ProcessTerminalState::Exited {
                            code: status.code(),
                        };
                        // Confirmed reaped: evict now, so nothing ever touches
                        // this pid/pgid again.
                        *tracked = Tracked::Terminal(state);
                        just_became_terminal = true;
                        trusted = members;
                        if claimed {
                            tracing::warn!(
                                pid = id.0,
                                "observe reaped a child the self-exit reaper had claimed"
                            );
                        }
                        Some(ProcessStatus::Terminal(state))
                    }
                    Ok(None) => Some(ProcessStatus::Running),
                    // The OS could not answer whether the child is still
                    // running -- not a confirmed reap, so the entry stays
                    // `Running` (its pid cannot yet have been recycled).
                    // Descendants (and the child itself) are then unproven
                    // stopped rather than provably running or exited.
                    Err(_) => Some(ProcessStatus::Terminal(
                        ProcessTerminalState::OrphanRiskUncertain,
                    )),
                }
            }
        };
        keep_group_record(&self.inner.exit_members, &children, id, trusted);
        drop(children);

        // Recorded only on the transition itself (not on a later `observe`
        // of an already-`Terminal` entry), so `Inner::terminal_order` holds
        // each id at most once -- see `record_terminal_order`'s own doc
        // comment on why this must run with `children` already unlocked.
        if just_became_terminal {
            record_terminal_order(&self.inner.children, &self.inner.terminal_order, id);
        }

        // Output capture's own finalize is independent of, and must never
        // gate, this port's own reap reporting above -- see
        // `output_capture`'s doc comment.
        if let Some(ProcessStatus::Terminal(state)) = result {
            output_capture::record_confirmed_state(&self.inner.outputs, id, state);
        }

        result
    }
}

impl ProcessOutput for TokioProcessSupervisor {
    fn subscribe(
        &mut self,
        id: ProcessId,
    ) -> Result<std::sync::mpsc::Receiver<omnifrons_app::OutputFrame>, SupervisorError> {
        output_capture::take_receiver(&self.inner.outputs, id)
    }
}

/// How long [`Inner`]'s `Drop` gives each killed child to become reapable.
#[cfg(unix)]
const DROP_REAP_BUDGET: std::time::Duration = std::time::Duration::from_millis(200);
/// How often it asks in the meantime.
#[cfg(unix)]
const DROP_REAP_POLL: std::time::Duration = std::time::Duration::from_millis(5);

impl Drop for Inner {
    /// Best-effort cleanup, not containment: runs only once the *last*
    /// [`TokioProcessSupervisor`] handle sharing this `Inner` is dropped
    /// (`Arc`'s own contract). If that happens without a prior `stop` for
    /// every child spawned through any handle (e.g. a panicking test, or an
    /// early return), send SIGKILL to the process group of every entry
    /// still `Running` on unix, so a bug elsewhere in this process does not
    /// silently orphan a live descendant. Errors are swallowed except for a
    /// `warn`: there is no caller left to report them to, and this runs
    /// during unwinding as readily as during a normal drop, where panicking
    /// would abort the process.
    ///
    /// Also makes a short, bounded best-effort attempt to reap each child it
    /// kills, so a killed child does not become a zombie nobody ever
    /// collects. The reap goes through the `Child` itself (`try_wait`), never
    /// a raw `waitpid` behind its back: Tokio's `Child` disarms its own
    /// `kill_on_drop` only when it is the one that learns of the exit, and a
    /// `Child` left believing its process is alive sends `SIGKILL` to that
    /// pid when it is dropped moments later -- a pid the OS may by then have
    /// given to something else. If the bounded wait gives up, this logs it
    /// and leaves a zombie rather than blocking drop indefinitely --
    /// acceptable, since this is best effort, not containment. That `Child`
    /// still knows its process is un-reaped, so its own drop signals a pid
    /// that is still its own.
    ///
    /// This does not attempt Windows containment (the Job Object policy
    /// VP-001 VP-S5 needs is not yet implemented there, matching `stop`'s
    /// own honesty about that gap).
    ///
    /// ## Order: kill and reap *before* stopping the driver thread
    ///
    /// The kill-and-reap loop below runs first, while the driver thread (and
    /// therefore every output-capture reader task) is still alive and still
    /// draining every child's master/pipes; only afterward does this signal
    /// the driver thread to stop and join it. This order is required, not
    /// incidental: on macOS, a child that is a session leader with a
    /// controlling terminal -- which is how this crate spawns every PTY
    /// child -- cannot finish exiting while its pty output is unread, and is
    /// therefore not reapable, not even after SIGKILL, until its master is
    /// drained or closed (measured in `tests/pty_exit_drain_probe.rs`; on
    /// Linux the same child reaps in milliseconds regardless, so this is
    /// invisible there). Stopping the reader tasks before killing -- the
    /// previous order -- stops that draining exactly when the kill needs it
    /// most, so the bounded reap below gave up on macOS for any child that
    /// still had unread output at the moment of the kill
    /// (`tests/drop_reaps_flooding_pty_child.rs` pins this). With the
    /// readers still running, a killed PTY child's pending output keeps
    /// draining, its exit completes, and the bounded reap below collects it
    /// exactly as it already does on Linux.
    ///
    /// Joining the driver thread still happens before this function
    /// returns, matching [`Inner`]'s own doc comment that the runtime is
    /// guaranteed shut down by then -- it is simply the last thing this does
    /// rather than the first.
    ///
    /// One case stays unfixed even with this order: if a child's reader has
    /// already ended while something else still holds its master open,
    /// nothing drains it, and the bounded reap below can still give up on
    /// macOS. Best effort, not containment.
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let mut children = match self.children.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            for (id, tracked) in children.iter_mut() {
                let Tracked::Running(child) = tracked else {
                    continue;
                };
                let Ok(raw_pid) = i32::try_from(id.0) else {
                    continue;
                };
                let pid = nix::unistd::Pid::from_raw(raw_pid);
                if let Err(error) = nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL)
                {
                    tracing::warn!(
                        pid = id.0,
                        %error,
                        "best-effort SIGKILL on supervisor drop failed"
                    );
                }
                // Bounded best-effort reap: a killed child normally dies
                // within milliseconds, so give it a short window rather than
                // blocking drop indefinitely. Through the `Child` itself,
                // never a raw `waitpid` behind its back: a `Child` that does
                // not know it was reaped still sends `SIGKILL` to that pid
                // when it is dropped (`kill_on_drop`), and by then the OS may
                // have given the pid to something else.
                match unix::reap_within(DROP_REAP_BUDGET, DROP_REAP_POLL, || child.try_wait()) {
                    unix::BoundedReap::Reaped => {}
                    unix::BoundedReap::GaveUp => tracing::warn!(
                        pid = id.0,
                        budget = ?DROP_REAP_BUDGET,
                        "child not reapable within the drop window; left un-reaped"
                    ),
                    unix::BoundedReap::Failed(kind) => tracing::warn!(
                        pid = id.0,
                        ?kind,
                        "reaping a child on supervisor drop failed; left un-reaped"
                    ),
                }
            }
        }

        if let Some(shutdown) = self.driver_shutdown.take() {
            // The driver thread's receiver can only already be gone if the
            // driver thread itself already exited (e.g. it panicked); a
            // send error here is not actionable beyond skipping the join.
            let _ = shutdown.send(());
        }
        if let Some(driver_thread) = self.driver_thread.take()
            && let Err(panic_payload) = driver_thread.join()
        {
            // `Box<dyn Any + Send>` has no `Debug` impl; a panic payload is
            // almost always the `&str`/`String` message `panic!` itself
            // constructs, so recover that where possible rather than
            // logging an opaque payload description.
            let message = panic_payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic_payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("<non-string panic payload>");
            tracing::error!(
                message,
                "the process supervisor's runtime driver thread panicked"
            );
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use nix::errno::Errno;
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;
    use omnifrons_app::{ProcessId, ProcessTerminalState, SupervisorError};

    use crate::output_capture::{self, OutputTable};
    use crate::record_terminal_order;
    use crate::{GroupMember, Tracked};

    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const KILL_GRACE: Duration = Duration::from_millis(500);

    use crate::Census;

    /// The outcome of a `killpg` call, classified for what the caller can
    /// safely conclude from it.
    #[derive(Debug, PartialEq, Eq)]
    enum SignalOutcome {
        /// The signal was delivered, or the group was already gone
        /// (`ESRCH`) -- not itself a failure, since the child may have
        /// exited in a race with this call; the reap poll confirms the
        /// actual terminal state.
        Proceed,
        /// `killpg` failed with something other than "group already
        /// gone" (e.g. `EPERM`): the caller cannot tell whether the
        /// signal landed, so the outcome is unproven rather than assumed
        /// delivered.
        Uncertain(Errno),
    }

    /// Classify a `killpg` result into what the caller may safely assume.
    fn classify_killpg_result(result: nix::Result<()>) -> SignalOutcome {
        match result {
            Ok(()) | Err(Errno::ESRCH) => SignalOutcome::Proceed,
            Err(other) => SignalOutcome::Uncertain(other),
        }
    }

    pub(crate) fn stop(
        children: &Mutex<HashMap<ProcessId, Tracked>>,
        terminal_order: &Mutex<VecDeque<ProcessId>>,
        lingering: &Mutex<HashMap<ProcessId, Vec<GroupMember>>>,
        outputs: &OutputTable,
        id: ProcessId,
        deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        let (recorded, members) = {
            let guard = children
                .lock()
                .expect("children mutex poisoned by a prior panic");
            match guard.get(&id) {
                None => return Err(SupervisorError::UnknownProcess),
                Some(Tracked::Terminal(state)) => (
                    Some(*state),
                    lingering
                        .lock()
                        .expect("lingering mutex poisoned by a prior panic")
                        .get(&id)
                        .cloned(),
                ),
                Some(Tracked::Running(_)) => (None, None),
            }
        };
        // Already confirmed terminal: report the same recorded state again.
        // The pid/pgid is touched again only for a self-exit whose
        // descendant held the output past the exit grace, and then only
        // while that group is provably the original one -- the OS may
        // otherwise have recycled the id for something else.
        if let Some(state) = recorded {
            if let Some(members) = members {
                signal_lingering_group(id, &members, deadline);
            }
            return Ok(state);
        }

        // `process_group(0)` at spawn time made the child its own group
        // leader, so its pid doubles as the pgid: killpg targets the
        // whole group, not only the direct child.
        let Ok(raw_pid) = i32::try_from(id.0) else {
            return Err(SupervisorError::UnknownProcess);
        };
        let pgid = Pid::from_raw(raw_pid);

        // Taken *now*, while the child is still unreaped and its
        // descendants still point back at it: once it is reaped they have
        // been reparented away and the chain that identifies them as its
        // descendants is gone for good. A descendant that leaves the
        // process group (`setsid`) is invisible to every `killpg` below but
        // is in this census, which is the whole point of taking one.
        let census = Census::take(pgid);

        // Best-effort: a group that has already exited yields ESRCH here,
        // which is not itself a failure -- the loop below confirms the
        // actual terminal state via `try_wait`. Any other error (e.g.
        // EPERM) means we cannot tell whether the signal landed, so that
        // outcome must not be treated as if it had.
        tracing::debug!(pid = id.0, "sending SIGTERM to process group");
        if let SignalOutcome::Uncertain(errno) =
            classify_killpg_result(killpg(pgid, Signal::SIGTERM))
        {
            tracing::warn!(
                pid = id.0,
                %errno,
                "killpg(SIGTERM) failed unexpectedly; orphan-risk/uncertain"
            );
            return Ok(ProcessTerminalState::OrphanRiskUncertain);
        }

        if let Some(state) = poll_until_reaped(
            children,
            terminal_order,
            id,
            deadline,
            ProcessTerminalState::Exited { code: None },
        ) {
            // The direct child is reaped; that alone says nothing about
            // what it spawned. `settle` sweeps every recorded descendant
            // that is still running and downgrades this state to
            // `OrphanRiskUncertain` unless all of them -- and the process
            // group itself -- are proven gone.
            let state = census.settle(pgid, state);
            record_settled_state(children, outputs, id, state);
            tracing::debug!(pid = id.0, ?state, "process group terminated gracefully");
            return Ok(state);
        }

        tracing::warn!(
            pid = id.0,
            ?deadline,
            "deadline elapsed; escalating to SIGKILL"
        );
        if let SignalOutcome::Uncertain(errno) =
            classify_killpg_result(killpg(pgid, Signal::SIGKILL))
        {
            tracing::warn!(
                pid = id.0,
                %errno,
                "killpg(SIGKILL) failed unexpectedly; orphan-risk/uncertain"
            );
            return Ok(ProcessTerminalState::OrphanRiskUncertain);
        }

        if let Some(state) = poll_until_reaped(
            children,
            terminal_order,
            id,
            KILL_GRACE,
            ProcessTerminalState::Killed,
        ) {
            // Same proof obligation as the graceful path above: a reaped
            // direct child is not a contained process tree.
            let state = census.settle(pgid, state);
            record_settled_state(children, outputs, id, state);
            return Ok(state);
        }

        // Even SIGKILL could not be confirmed reaped within the grace
        // period: descendants are not provably stopped
        // (docs/target-architecture.md § Required failure states).
        tracing::error!(
            pid = id.0,
            "SIGKILL sent but reap unconfirmed; orphan-risk/uncertain"
        );
        Ok(ProcessTerminalState::OrphanRiskUncertain)
    }

    /// Record `state` as this child's settled terminal state: the one
    /// [`stop`] is about to return, after the descendant census has had its
    /// say, not the reap-derived one that preceded it.
    ///
    /// The entry is already `Tracked::Terminal` by the time this runs
    /// ([`poll_until_reaped`] put it there, with the reap-derived state, so
    /// the `Child` handle is dropped the moment the reap is confirmed).
    /// Overwriting it here is what keeps a later `observe`, a later `stop`,
    /// and the final `State` frame telling the same story as the `stop`
    /// call that proved it -- rather than the strictly weaker claim the
    /// reap alone supported. During the sweep itself (bounded, and only
    /// when something was actually found to sweep) a concurrent `observe`
    /// can still see that earlier reap-derived state: a transient, not a
    /// final answer, and never one this supervisor reports after `stop`
    /// has returned.
    pub(crate) fn record_settled_state(
        children: &Mutex<HashMap<ProcessId, Tracked>>,
        outputs: &OutputTable,
        id: ProcessId,
        state: ProcessTerminalState,
    ) {
        {
            let mut guard = children
                .lock()
                .expect("children mutex poisoned by a prior panic");
            if let Some(tracked) = guard.get_mut(&id) {
                *tracked = Tracked::Terminal(state);
            }
        }
        // Output capture learns the terminal state only here, once it is
        // settled: its final `State` frame is sent at most once, so sending
        // it with the reap-derived state first would publish a claim this
        // stop had not yet proven.
        output_capture::record_confirmed_state(outputs, id, state);
    }

    /// Stop's reach into the process group of a child that already exited
    /// on its own, while a descendant still held its output (S5d): the
    /// recorded state does not change -- the run already ended with it --
    /// but its lingering group is signalled like a running child's,
    /// `SIGTERM` then, after `deadline`, `SIGKILL`.
    ///
    /// Only while the group is provably the original one: a member
    /// recorded with the verdict is still in it with its recorded start
    /// time, so the group id (the reaped child's pid) cannot have been
    /// recycled ([`crate::group_still_original`]). Those members are only
    /// descendants the census sighted while the child lived, and members
    /// read in its group while the exited child was still an unreaped
    /// zombie: the zombie holds the pid that is also the group's id, so
    /// that id cannot have been reused yet ([`crate::reap_after_exit_census`]
    /// reaps only after that read). A process first seen in the group after
    /// the reap may belong to an unrelated group that took over the id, and
    /// never authorizes a signal. Once no recorded member is left, nothing
    /// more is signalled, even if a process that joined the group later is
    /// still in it; Stop then reports the recorded state alone.
    fn signal_lingering_group(id: ProcessId, members: &[GroupMember], deadline: Duration) {
        let Ok(raw_pid) = i32::try_from(id.0) else {
            return;
        };
        let pgid = Pid::from_raw(raw_pid);
        for (signal, budget) in [(Signal::SIGTERM, deadline), (Signal::SIGKILL, KILL_GRACE)] {
            if !crate::group_still_original(pgid, members) {
                return;
            }
            tracing::debug!(pid = id.0, ?signal, "signalling a lingering process group");
            if let SignalOutcome::Uncertain(errno) = classify_killpg_result(killpg(pgid, signal)) {
                tracing::warn!(
                    pid = id.0,
                    %errno,
                    "killpg on a lingering process group failed unexpectedly"
                );
                return;
            }
            let start = Instant::now();
            while crate::group_still_original(pgid, members) && start.elapsed() < budget {
                std::thread::sleep(POLL_INTERVAL);
            }
        }
        if crate::group_still_original(pgid, members) {
            tracing::warn!(
                pid = id.0,
                "a recorded member of a lingering process group outlived SIGKILL"
            );
        }
    }

    /// Poll `try_wait` until the child is reaped or `budget` elapses.
    ///
    /// On reap, evicts the entry to `Tracked::Terminal` (dropping the
    /// `Child` handle) and returns the exit-derived terminal state
    /// (`Exited` carries the real exit code; the caller-supplied `on_reap`
    /// template is used only to pick `Exited`/`Killed` framing when no
    /// exit code is meaningful, e.g. after a signal). Until reap is
    /// confirmed, the entry is left `Running`: its pid cannot yet have
    /// been recycled by the OS.
    ///
    /// The state recorded here is about the *direct child* only. What the
    /// caller may finally report also depends on its descendants, so
    /// publishing it to output capture is deliberately left to
    /// [`record_settled_state`], after that proof exists.
    fn poll_until_reaped(
        children: &Mutex<HashMap<ProcessId, Tracked>>,
        terminal_order: &Mutex<VecDeque<ProcessId>>,
        id: ProcessId,
        budget: Duration,
        on_reap: ProcessTerminalState,
    ) -> Option<ProcessTerminalState> {
        let start = Instant::now();
        loop {
            {
                let mut guard = children
                    .lock()
                    .expect("children mutex poisoned by a prior panic");
                let tracked = guard.get_mut(&id).expect("checked present by the caller");
                let Tracked::Running(child) = tracked else {
                    // Reaped and recorded by the self-exit reaper (or an
                    // `observe`) after this call's own `Running` check:
                    // report that same recorded state.
                    let Tracked::Terminal(state) = tracked else {
                        unreachable!("the entry is not Running, so it is Terminal")
                    };
                    return Some(*state);
                };
                if let Ok(Some(status)) = child.try_wait() {
                    let state = match on_reap {
                        ProcessTerminalState::Exited { .. } => ProcessTerminalState::Exited {
                            code: status.code(),
                        },
                        other => other,
                    };
                    *tracked = Tracked::Terminal(state);
                    drop(guard);
                    record_terminal_order(children, terminal_order, id);
                    return Some(state);
                }
            }
            let elapsed = start.elapsed();
            if elapsed >= budget {
                return None;
            }
            std::thread::sleep(POLL_INTERVAL.min(budget.saturating_sub(elapsed)));
        }
    }

    /// What a bounded reap attempt concluded.
    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum BoundedReap {
        /// `try_wait` reported an exit status: the child is reaped, and
        /// whatever handle `try_wait` belongs to knows it.
        Reaped,
        /// Still not reapable when the budget ran out.
        GaveUp,
        /// `try_wait` itself failed. Retrying an error learns nothing, so
        /// the attempt ends there.
        Failed(std::io::ErrorKind),
    }

    /// Poll `try_wait` every `poll` until it reports an exit status, fails,
    /// or `budget` is spent. The deadline is checked after each poll, so
    /// this can outlast `budget` by one `poll` and one `try_wait`, never by
    /// more.
    ///
    /// Takes the poll as a closure so the supervisor's `Drop` can hand it
    /// its own `Child::try_wait` -- reaping *through* that handle rather
    /// than behind its back -- and so the bound itself is testable without
    /// a process that refuses to be reaped.
    pub(crate) fn reap_within(
        budget: Duration,
        poll: Duration,
        mut try_wait: impl FnMut() -> std::io::Result<Option<std::process::ExitStatus>>,
    ) -> BoundedReap {
        let deadline = Instant::now() + budget;
        loop {
            match try_wait() {
                Ok(Some(_status)) => return BoundedReap::Reaped,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(poll),
                Ok(None) => return BoundedReap::GaveUp,
                Err(error) => return BoundedReap::Failed(error.kind()),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::SignalOutcome;
        use super::classify_killpg_result;
        use nix::errno::Errno;

        #[test]
        fn ok_proceeds() {
            assert!(matches!(
                classify_killpg_result(Ok(())),
                SignalOutcome::Proceed
            ));
        }

        #[test]
        fn esrch_proceeds_as_group_already_gone() {
            assert!(matches!(
                classify_killpg_result(Err(Errno::ESRCH)),
                SignalOutcome::Proceed
            ));
        }

        #[test]
        fn eperm_is_uncertain() {
            assert!(matches!(
                classify_killpg_result(Err(Errno::EPERM)),
                SignalOutcome::Uncertain(Errno::EPERM)
            ));
        }

        #[test]
        fn other_errno_is_uncertain() {
            assert!(matches!(
                classify_killpg_result(Err(Errno::EINVAL)),
                SignalOutcome::Uncertain(Errno::EINVAL)
            ));
        }

        use std::io;
        use std::os::unix::process::ExitStatusExt as _;
        use std::process::ExitStatus;
        use std::time::{Duration, Instant};

        use super::{BoundedReap, reap_within};

        const POLL: Duration = Duration::from_millis(1);

        /// A child that is reaped on a later poll is reported reaped, and
        /// polling stops there.
        #[test]
        fn a_child_reaped_on_a_later_poll_is_reaped() {
            let mut polls = 0;
            let outcome = reap_within(Duration::from_secs(5), POLL, || {
                polls += 1;
                Ok((polls == 3).then(|| ExitStatus::from_raw(0)))
            });
            assert_eq!(outcome, BoundedReap::Reaped);
            assert_eq!(polls, 3, "polling must stop at the reap");
        }

        /// A child that never becomes reapable is given up on once the
        /// budget is spent -- bounded, never a hang.
        #[test]
        fn a_child_never_reapable_is_given_up_on_within_the_budget() {
            let budget = Duration::from_millis(40);
            let started = Instant::now();
            let outcome = reap_within(budget, POLL, || Ok(None));
            assert_eq!(outcome, BoundedReap::GaveUp);
            assert!(started.elapsed() >= budget, "it must use its budget");
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "and must not outlive it by much"
            );
        }

        /// A failing `try_wait` ends the attempt at once: retrying an error
        /// learns nothing.
        #[test]
        fn a_failing_try_wait_ends_the_attempt_at_once() {
            // Derived, not named: the kind an errno maps to is the standard
            // library's business and differs across versions.
            let failure = || io::Error::from_raw_os_error(nix::libc::ECHILD);
            let mut polls = 0;
            let outcome = reap_within(Duration::from_secs(5), POLL, || {
                polls += 1;
                Err(failure())
            });
            assert_eq!(outcome, BoundedReap::Failed(failure().kind()));
            assert_eq!(polls, 1);
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    use omnifrons_app::{ProcessId, ProcessTerminalState, SupervisorError};

    use crate::Tracked;
    use crate::output_capture::OutputTable;

    /// Stub pending the Job Object implementation (VP-001 VP-S5).
    ///
    /// Windows process-group termination is not yet proven: without a Job
    /// Object policy, a breakaway-attempting descendant may survive
    /// unnoticed. Until that is implemented and verified, this always
    /// reports `OrphanRiskUncertain` rather than a containment claim this
    /// crate cannot back (docs/target-architecture.md § Required failure
    /// states; docs/adr/0002-desktop-technology-stack.md § Process
    /// supervision). Because containment is unproven, a confirmed reap
    /// never happens here either, so the entry is deliberately never
    /// evicted to `Terminal`: every call, including a repeat one, takes the
    /// same honest, unproven path. Output capture's final `State` frame is
    /// consequently never sent on Windows either -- the same, already-
    /// documented gap, not a new one (`output_capture`'s own doc comment).
    pub(crate) fn stop(
        children: &Mutex<HashMap<ProcessId, Tracked>>,
        _outputs: &OutputTable,
        id: ProcessId,
        _deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        let mut guard = children
            .lock()
            .expect("children mutex poisoned by a prior panic");
        match guard.get_mut(&id).ok_or(SupervisorError::UnknownProcess)? {
            Tracked::Terminal(state) => Ok(*state),
            Tracked::Running(child) => {
                // Best-effort direct-child termination; containment of any
                // descendant it spawned is unproven without a Job Object.
                let _ = child.start_kill();
                Ok(ProcessTerminalState::OrphanRiskUncertain)
            }
        }
    }
}

#[cfg(all(test, unix))]
mod self_exit_tests {
    use std::time::Duration;

    use omnifrons_app::{
        FramePayload, ProcessOutput, ProcessSpec, ProcessStatus, ProcessSupervisor,
        ProcessTerminalState,
    };

    use super::{ReapPacing, TokioProcessSupervisor};

    /// A cadence whose slow window ends after 100 ms, so "the child outlives
    /// the window" takes a fraction of a second instead of a minute.
    const SHORT: ReapPacing = ReapPacing {
        fast_poll: Duration::from_millis(5),
        fast_window: Duration::from_millis(50),
        slow_poll: Duration::from_millis(10),
        slow_window: Duration::from_millis(100),
        idle_poll: Duration::from_millis(20),
        census_refresh: Duration::from_secs(1),
        census_idle_refresh: Duration::from_secs(10),
        census_bound: super::CENSUS_SIGHTINGS_BOUND,
        exit_grace: Duration::from_secs(2),
        stall_grace: Duration::from_secs(3),
        settle_wait: Duration::from_secs(5),
    };

    /// Every `State` frame the receiver yields until its channel closes.
    fn drain_states(
        frames: &std::sync::mpsc::Receiver<omnifrons_app::OutputFrame>,
    ) -> Vec<ProcessTerminalState> {
        let mut states = Vec::new();
        while let Ok(frame) = frames.recv_timeout(Duration::from_secs(2)) {
            if let FramePayload::State(state) = frame.payload {
                states.push(state);
            }
        }
        states
    }

    #[test]
    fn a_child_that_exits_after_the_slow_window_still_gets_its_state_frame() {
        let mut supervisor = TokioProcessSupervisor::build_with(None, SHORT);
        // Output closes at once; the child then outlives `SHORT.slow_window`
        // (100 ms) by an order of magnitude before exiting by itself.
        let spec = ProcessSpec::new("/bin/sh").with_args(["-c", "exec 1>&- 2>&-; exec sleep 1"]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");

        let end = std::time::Instant::now() + Duration::from_secs(10);
        let state = loop {
            let left = end.saturating_duration_since(std::time::Instant::now());
            match frames.recv_timeout(left) {
                Ok(frame) => {
                    if let FramePayload::State(state) = frame.payload {
                        break Some(state);
                    }
                }
                Err(_) => break None,
            }
        };
        assert_eq!(state, Some(ProcessTerminalState::Exited { code: Some(0) }));
    }

    /// A pacing whose reaper never polls again after its first attempt.
    const NEVER_POLLS: ReapPacing = ReapPacing {
        fast_poll: Duration::from_secs(3600),
        fast_window: Duration::from_secs(3600),
        slow_poll: Duration::from_secs(3600),
        slow_window: Duration::from_secs(7200),
        idle_poll: Duration::from_secs(3600),
        census_refresh: Duration::from_secs(3600),
        census_idle_refresh: Duration::from_secs(3600),
        census_bound: super::CENSUS_SIGHTINGS_BOUND,
        exit_grace: Duration::from_secs(2),
        stall_grace: Duration::from_millis(300),
        settle_wait: Duration::from_secs(5),
    };

    #[test]
    fn observe_stops_deferring_to_a_reaper_that_never_polls() {
        let mut supervisor = TokioProcessSupervisor::build_with(None, NEVER_POLLS);
        // Output closes at once and the reaper's first attempt finds the
        // child alive; the child exits by itself a moment later, after which
        // the stalled reaper never looks again.
        let spec = ProcessSpec::new("/bin/sh").with_args(["-c", "exec 1>&- 2>&-; exec sleep 2"]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");

        let claim_end = std::time::Instant::now() + Duration::from_secs(5);
        while super::output_capture::reaper_claim(&supervisor.inner.outputs, id).is_none() {
            assert!(
                std::time::Instant::now() < claim_end,
                "the reaper never claimed the child"
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        let end = std::time::Instant::now() + NEVER_POLLS.stall_grace + Duration::from_secs(5);
        let mut expired = false;
        let status = loop {
            let status = supervisor.observe(id);
            // Read while the entry exists: the final frame removes it.
            expired |= super::output_capture::deferral_expired(&supervisor.inner.outputs, id);
            match status {
                Some(ProcessStatus::Running) if std::time::Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                other => break other,
            }
        };
        assert_eq!(
            status,
            Some(ProcessStatus::Terminal(ProcessTerminalState::Exited {
                code: Some(0)
            }))
        );
        assert!(expired, "observe never logged the stalled reaper");
        let exited = ProcessTerminalState::Exited { code: Some(0) };
        assert_eq!(drain_states(&frames), vec![exited]);
    }

    #[test]
    fn observe_takes_over_a_settle_that_outlasts_the_settle_wait() {
        let pacing = ReapPacing {
            settle_wait: Duration::from_millis(50),
            ..SHORT
        };
        let mut supervisor = TokioProcessSupervisor::build_with(None, pacing);
        // A self-exit settle sweeps nothing, so this one is held for 300 ms
        // (`HELD_SETTLES`), well past the 50 ms settle wait. Both helpers
        // stay in the group, so the settled verdict is
        // `OrphanRiskUncertain`, unlike the reap-derived `Exited` that
        // `observe` reports on taking over.
        let script = "(trap '' TERM; exec sleep 3) >/dev/null 2>&1 </dev/null & \
                      exec 1>&- 2>&-; sleep 0.2; \
                      (exec sleep 2) >/dev/null 2>&1 </dev/null & exit 0";
        let spec = ProcessSpec::new("/bin/sh").with_args(["-c", script]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        // Registered while the child still sleeps its 0.2 s: before its settle.
        super::HELD_SETTLES
            .lock()
            .expect("held settles poisoned")
            .push((id.0, Duration::from_millis(300)));
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");

        let exited = ProcessTerminalState::Exited { code: Some(0) };
        let end = std::time::Instant::now() + Duration::from_secs(2);
        while supervisor.observe(id) != Some(ProcessStatus::Terminal(exited)) {
            assert!(std::time::Instant::now() < end, "observe never took over");
            std::thread::sleep(Duration::from_millis(5));
        }
        // The reaper's late verdict is discarded: the state never changes.
        let end = std::time::Instant::now() + Duration::from_millis(600);
        while std::time::Instant::now() < end {
            assert_eq!(
                supervisor.observe(id),
                Some(ProcessStatus::Terminal(exited))
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(drain_states(&frames), vec![exited]);
    }

    #[test]
    fn the_census_is_not_retaken_forever_for_a_child_that_lives_on() {
        let mut supervisor = TokioProcessSupervisor::build_with(None, SHORT);
        let spec = ProcessSpec::new("/bin/sh").with_args(["-c", "exec 1>&- 2>&-; exec sleep 1"]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");
        // The child outlives `SHORT`'s slow window many times over, and the
        // census refresh interval (1 s) exceeds that window: one census at
        // the start, none in the idle phase.
        let _ = frames.recv_timeout(Duration::from_secs(10));
        let takes = super::CENSUS_TAKES
            .lock()
            .expect("census counter poisoned")
            .iter()
            .filter(|pid| **pid == id.0)
            .count();
        assert!(
            takes <= 2,
            "census taken {takes} times for one long-lived child"
        );
    }

    /// The verifier's probe c6, scaled down: a child that ran many
    /// short-lived processes, every one finished before it exits cleanly,
    /// must still end `Exited`. Only descendants alive at once count against
    /// the census bound; here a walk every 10 ms over 300 short `sleep`s,
    /// ten at a time, sights far more of them than the lowered bound of 32.
    #[cfg(target_os = "linux")]
    #[test]
    fn many_finished_descendants_never_exhaust_the_census_bound() {
        let pacing = ReapPacing {
            census_refresh: Duration::from_millis(10),
            slow_window: Duration::from_secs(60),
            census_bound: 32,
            ..SHORT
        };
        let mut supervisor = TokioProcessSupervisor::build_with(None, pacing);
        let script = "i=0; while [ \"$i\" -lt 300 ]; do sleep 0.05 & i=$((i+1)); \
                      if [ $((i % 10)) -eq 0 ]; then wait; fi; done; wait; exit 0";
        let spec = ProcessSpec::new("/bin/sh").with_args(["-c", script]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");

        let end = std::time::Instant::now() + Duration::from_secs(20);
        let state = loop {
            let left = end.saturating_duration_since(std::time::Instant::now());
            match frames.recv_timeout(left) {
                Ok(frame) => {
                    if let FramePayload::State(state) = frame.payload {
                        break Some(state);
                    }
                }
                Err(_) => break None,
            }
        };
        let walks = super::CENSUS_TAKES
            .lock()
            .expect("census counter poisoned")
            .iter()
            .filter(|pid| **pid == id.0)
            .count();
        assert!(
            walks > 20,
            "only {walks} census walks: the probe saw too little"
        );
        assert_eq!(state, Some(ProcessTerminalState::Exited { code: Some(0) }));
    }

    #[test]
    fn observe_keeps_deferring_to_a_healthy_slow_reaper() {
        // A reaper polling every 200 ms outlives `stall_grace` (500 ms).
        let pacing = ReapPacing {
            idle_poll: Duration::from_millis(200),
            stall_grace: Duration::from_millis(500),
            ..SHORT
        };
        let mut supervisor = TokioProcessSupervisor::build_with(None, pacing);
        let spec = ProcessSpec::new("/bin/sh").with_args(["-c", "exec 1>&- 2>&-; exec sleep 1.5"]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        let _frames = supervisor.subscribe(id).expect("subscribe must succeed");
        let outputs = supervisor.inner.outputs.clone();
        while super::output_capture::reaper_claim(&outputs, id).is_none() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let (watch, end) = (Duration::from_millis(1200), Duration::from_secs(4));
        let began = std::time::Instant::now();
        let status = loop {
            let status = supervisor.observe(id);
            if status != Some(ProcessStatus::Running) || began.elapsed() > end {
                break status;
            }
            if began.elapsed() < watch {
                assert!(super::output_capture::reaper_claim(&outputs, id).is_some());
                assert!(!super::output_capture::deferral_expired(&outputs, id));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let exited = ProcessTerminalState::Exited { code: Some(0) };
        assert_eq!(status, Some(ProcessStatus::Terminal(exited)));
    }

    /// The first `State` frame the receiver yields within `within`.
    fn first_state(
        frames: &std::sync::mpsc::Receiver<omnifrons_app::OutputFrame>,
        within: Duration,
    ) -> Option<ProcessTerminalState> {
        let end = std::time::Instant::now() + within;
        loop {
            let left = end.saturating_duration_since(std::time::Instant::now());
            match frames.recv_timeout(left) {
                Ok(frame) => {
                    if let FramePayload::State(state) = frame.payload {
                        return Some(state);
                    }
                }
                Err(_) => return None,
            }
        }
    }

    /// #192: a reaper task that ends without reaping (here it panics) gives
    /// its claim back, so `observe` confirms the exit itself as soon as the
    /// child exits, instead of deferring until `stall_grace` (30 s here) and
    /// then logging a stalled reaper.
    #[test]
    fn a_reaper_that_panics_hands_the_reap_back_at_once() {
        let pacing = ReapPacing {
            stall_grace: Duration::from_secs(30),
            ..SHORT
        };
        let mut supervisor = TokioProcessSupervisor::build_with(None, pacing);
        let spec = ProcessSpec::new("/bin/sh").with_args(["-c", "exec 1>&- 2>&-; exec sleep 1"]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        // The reaper panics at its next poll, while the child still sleeps.
        super::PANICKING_REAPERS
            .lock()
            .expect("panicking reapers poisoned")
            .push(id.0);
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");

        // The child exits after 1 s; 5 s is far short of the stall grace.
        let end = std::time::Instant::now() + Duration::from_secs(5);
        let mut expired = false;
        let status = loop {
            let status = supervisor.observe(id);
            // Read while the entry exists: the final frame removes it.
            expired |= super::output_capture::deferral_expired(&supervisor.inner.outputs, id);
            match status {
                Some(ProcessStatus::Running) if std::time::Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                other => break other,
            }
        };
        let exited = ProcessTerminalState::Exited { code: Some(0) };
        assert_eq!(status, Some(ProcessStatus::Terminal(exited)));
        assert!(!expired, "observe waited out a reaper that no longer ran");
        assert_eq!(drain_states(&frames), vec![exited]);
    }

    /// #192: the settled verdict leaves `settling` before its `State` frame
    /// is sent, so a consumer that calls `observe` on receiving the frame
    /// sees that same state, even while the reaper task is slow to end
    /// (held 2 s here, after the verdict is recorded).
    #[test]
    fn observe_reports_the_settled_state_once_its_frame_arrives() {
        let mut supervisor = TokioProcessSupervisor::build_with(None, SHORT);
        let spec =
            ProcessSpec::new("/bin/sh").with_args(["-c", "exec 1>&- 2>&-; sleep 0.2; exit 0"]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        // Registered while the child still sleeps its 0.2 s: before its reap.
        super::HELD_REAPER_ENDS
            .lock()
            .expect("held reaper ends poisoned")
            .push((id.0, Duration::from_secs(2)));
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");

        let exited = ProcessTerminalState::Exited { code: Some(0) };
        assert_eq!(first_state(&frames, Duration::from_secs(10)), Some(exited));
        assert_eq!(
            supervisor.observe(id),
            Some(ProcessStatus::Terminal(exited))
        );
    }

    /// #192: a census task that fails (here it panics) leaves the census
    /// unavailable instead of walking `/proc` again inline, on the runtime
    /// thread its own doc keeps procfs walks off.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_failed_census_task_is_unavailable_and_never_retaken_inline() {
        // Never a live pid (`pid_max` is far lower), so no other test uses it.
        let id = omnifrons_app::ProcessId(u32::MAX - 192);
        super::FAILING_CENSUSES
            .lock()
            .expect("failing censuses poisoned")
            .push(id.0);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime must build");
        let census = runtime.block_on(super::take_census_blocking(id));
        assert!(matches!(
            census,
            super::Census::Unavailable(super::descendants::Unproven::CensusTaskFailed)
        ));
        let walks = super::CENSUS_TAKES
            .lock()
            .expect("census counter poisoned")
            .iter()
            .filter(|pid| **pid == id.0)
            .count();
        assert_eq!(
            walks, 1,
            "the census was walked again after its task failed"
        );
    }

    /// PR #198 (Codex P2), end to end: a run's reaper ends (its
    /// `ClaimRelease` drops) only after the run's pid came back for a new
    /// run, whose channel and claim sit under the same id. The old
    /// reaper's release must leave the new claim alone. The reaper's end is
    /// held (`HELD_REAPER_ENDS`) so the new claim lands first.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_old_reapers_end_leaves_the_claim_of_a_run_that_reused_its_pid() {
        let mut supervisor = TokioProcessSupervisor::new();
        let spec =
            ProcessSpec::new("/bin/sh").with_args(["-c", "exec 1>&- 2>&-; sleep 0.2; exit 0"]);
        let id = supervisor.spawn(spec).expect("spawning sh must succeed");
        // Registered while the child still sleeps its 0.2 s: before its reap.
        super::HELD_REAPER_ENDS
            .lock()
            .expect("held reaper ends poisoned")
            .push((id.0, Duration::from_millis(500)));
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");
        assert!(first_state(&frames, Duration::from_secs(10)).is_some());

        // The old reaper now waits out its held end, its guard still alive.
        // The pid comes back: a new run's channel and claim, same id.
        let outputs = std::sync::Arc::clone(&supervisor.inner.outputs);
        let (state, _new_handles) = super::output_capture::new_channel(1);
        outputs.lock().expect("outputs poisoned").insert(id, state);
        assert!(super::output_capture::claim_reap(&outputs, id).is_some());

        // Past the held end: the old guard has dropped.
        std::thread::sleep(Duration::from_millis(1500));
        let survived = super::output_capture::reaper_claim(&outputs, id).is_some();
        super::HELD_REAPER_ENDS
            .lock()
            .expect("held reaper ends poisoned")
            .retain(|(pid, _)| *pid != id.0);
        outputs.lock().expect("outputs poisoned").remove(&id);
        assert!(
            survived,
            "the old reaper's release cleared the new run's claim"
        );
    }
}
