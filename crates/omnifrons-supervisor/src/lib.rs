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

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
mod output_capture;
#[cfg(unix)]
mod pty;

use output_capture::{OutputTable, ReaderHandles};

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
fn apply_env(command: &mut Command, env: &EnvPlan) {
    match env {
        EnvPlan::Inherit => {}
        EnvPlan::Allowlist(keys) => {
            command.env_clear();
            for key in keys {
                if is_secret_shaped(key) {
                    tracing::warn!(
                        key,
                        "refusing to set a secret-shaped environment variable on a spawned \
                         child, even though it was allowlisted"
                    );
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
    children: Mutex<HashMap<ProcessId, Tracked>>,
    /// The order, oldest first, in which entries in `children` transitioned
    /// to `Tracked::Terminal`. Every id pushed here exactly once, at the
    /// moment of that transition (`record_terminal_order`); consulted only
    /// to decide which entry to evict once [`MAX_RETAINED_TERMINAL_CHILDREN`]
    /// is exceeded.
    terminal_order: Mutex<VecDeque<ProcessId>>,
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
                children: Mutex::new(HashMap::new()),
                terminal_order: Mutex::new(VecDeque::new()),
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
    fn finish_spawn(
        &mut self,
        mut command: Command,
        program_label: &str,
        env: &EnvPlan,
        stdio: StdioPlan,
        prompt: Option<Vec<u8>>,
    ) -> Result<ProcessId, SupervisorError> {
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

        let mut child = command.spawn().map_err(|error| {
            tracing::warn!(program = %program_label, %error, "failed to spawn process");
            SupervisorError::Spawn(error.to_string())
        })?;
        // Released now, deliberately: on the PTY path `command` still
        // holds this process's copies of the slave end, and the master
        // only reports EOF once every slave descriptor outside the child
        // is closed. On the pipe path this is a no-op.
        drop(command);
        let pid = child
            .id()
            .ok_or_else(|| SupervisorError::Spawn("spawned child reported no pid".to_string()))?;
        let id = ProcessId(pid);

        let (output_state, handles) = output_capture::new_channel(wiring.reader_count());
        self.inner
            .outputs
            .lock()
            .expect("outputs mutex poisoned by a prior panic")
            .insert(id, output_state);

        // Every reader and writer task is attached before this child is
        // ever inserted into `children`, so no other code path can observe
        // a `Tracked::Running` whose streams have already been taken out
        // from under it.
        self.attach_output(&mut child, wiring, handles, id, prompt);

        self.inner
            .children
            .lock()
            .expect("children mutex poisoned by a prior panic")
            .insert(id, Tracked::Running(Box::new(child)));

        tracing::debug!(program = %program_label, pid, "spawned process");
        Ok(id)
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
                self.spawn_on_runtime(output_capture::drain_stream(
                    stdout,
                    omnifrons_app::OutputStream::Stdout,
                    handles,
                    Arc::clone(&self.inner.outputs),
                    id,
                ));
                self.spawn_on_runtime(output_capture::drain_stream(
                    stderr,
                    omnifrons_app::OutputStream::Stderr,
                    stderr_handles,
                    Arc::clone(&self.inner.outputs),
                    id,
                ));

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
                self.spawn_on_runtime(output_capture::drain_stream(
                    pty::MasterReader::new(Arc::clone(&master)),
                    omnifrons_app::OutputStream::Stdout,
                    handles,
                    Arc::clone(&self.inner.outputs),
                    id,
                ));
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
    /// `fs_prober` doc comment) so it is never inherited by some
    /// unrelated `exec` before this deliberate moment; immediately before
    /// spawning, this method clears that flag (`fcntl(F_SETFD,
    /// FdFlag::empty())`) so it *is* inherited across *this* `exec` --
    /// with no `unsafe` code, since `nix`'s `fcntl` wrapper is itself
    /// safe. A direct-binary target only ever needs the kernel's own
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
                use std::os::fd::AsFd as _;

                nix::fcntl::fcntl(
                    file.as_fd(),
                    nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                )
                .map_err(|error| {
                    SupervisorError::Spawn(format!(
                        "failed to clear close-on-exec on the approved executable's sealed \
                         memfd: {error}"
                    ))
                })?;

                let fd = {
                    use std::os::fd::AsRawFd as _;
                    file.as_raw_fd()
                };
                let mut command = Command::new(format!("/proc/self/fd/{fd}"));
                command.args(&plan.argv);
                command.current_dir(plan.cwd.path());
                // `file` must stay alive (and therefore its fd open) until
                // `finish_spawn` has actually called `spawn`.
                let result = self.finish_spawn(command, &label, &plan.env, stdio, prompt);
                drop(file);
                result
            }
            omnifrons_app::ExecHandle::File(file) => {
                drop(file);
                let mut command = Command::new(display_path);
                command.args(&plan.argv);
                command.current_dir(plan.cwd.path());
                self.finish_spawn(command, &label, &plan.env, stdio, prompt)
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
        )
    }

    fn stop(
        &mut self,
        id: ProcessId,
        deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        let _guard = self.inner.runtime.enter();

        #[cfg(unix)]
        {
            unix::stop(
                &self.inner.children,
                &self.inner.terminal_order,
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
        let mut children = self
            .inner
            .children
            .lock()
            .expect("children mutex poisoned by a prior panic");
        let tracked = children.get_mut(&id)?;
        let mut just_became_terminal = false;
        let result = match tracked {
            Tracked::Terminal(state) => Some(ProcessStatus::Terminal(*state)),
            Tracked::Running(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    let state = ProcessTerminalState::Exited {
                        code: status.code(),
                    };
                    // Confirmed reaped: evict now, so nothing ever touches
                    // this pid/pgid again.
                    *tracked = Tracked::Terminal(state);
                    just_became_terminal = true;
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
            },
        };
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
    /// kills: `stop`'s normal reap relies on this supervisor's own Tokio
    /// runtime, but that runtime is torn down around the same time as this
    /// cleanup runs (it is a sibling field, dropped in declaration order),
    /// so it cannot be relied on here. Without this, a killed child would
    /// become a zombie nobody ever collects. If the bounded wait gives up,
    /// this leaves a zombie rather than blocking drop indefinitely --
    /// acceptable, since this is best effort, not containment.
    ///
    /// This does not attempt Windows containment (the Job Object policy
    /// VP-001 VP-S5 needs is not yet implemented there, matching `stop`'s
    /// own honesty about that gap).
    ///
    /// Before any of that, this signals the driver thread to stop and joins
    /// it, so its `Arc<Runtime>` clone is guaranteed dropped -- and the
    /// runtime therefore guaranteed to actually shut down, not merely lose
    /// one of two owners -- before this function returns.
    fn drop(&mut self) {
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

        #[cfg(unix)]
        {
            let children = match self.children.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            for (id, tracked) in children.iter() {
                if matches!(tracked, Tracked::Running(_)) {
                    let Ok(raw_pid) = i32::try_from(id.0) else {
                        continue;
                    };
                    let pid = nix::unistd::Pid::from_raw(raw_pid);
                    if let Err(error) =
                        nix::sys::signal::killpg(pid, nix::sys::signal::Signal::SIGKILL)
                    {
                        tracing::warn!(
                            pid = id.0,
                            %error,
                            "best-effort SIGKILL on supervisor drop failed"
                        );
                    }
                    // Bounded best-effort reap: a killed child normally
                    // dies within milliseconds, so give it a short window
                    // rather than blocking drop indefinitely.
                    let deadline =
                        std::time::Instant::now() + std::time::Duration::from_millis(200);
                    loop {
                        match nix::sys::wait::waitpid(
                            pid,
                            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
                        ) {
                            Ok(nix::sys::wait::WaitStatus::StillAlive)
                                if std::time::Instant::now() < deadline =>
                            {
                                std::thread::sleep(std::time::Duration::from_millis(5));
                            }
                            _ => break,
                        }
                    }
                }
            }
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

    use crate::Tracked;
    use crate::output_capture::{self, OutputTable};
    use crate::record_terminal_order;

    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    const KILL_GRACE: Duration = Duration::from_millis(500);

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
        outputs: &OutputTable,
        id: ProcessId,
        deadline: Duration,
    ) -> Result<ProcessTerminalState, SupervisorError> {
        {
            let guard = children
                .lock()
                .expect("children mutex poisoned by a prior panic");
            match guard.get(&id) {
                None => return Err(SupervisorError::UnknownProcess),
                // Already confirmed terminal: report the same recorded
                // state again, without ever touching this pid/pgid --
                // which the OS may since have recycled for something
                // else.
                Some(Tracked::Terminal(state)) => return Ok(*state),
                Some(Tracked::Running(_)) => {}
            }
        }

        // `process_group(0)` at spawn time made the child its own group
        // leader, so its pid doubles as the pgid: killpg targets the
        // whole group, not only the direct child.
        let Ok(raw_pid) = i32::try_from(id.0) else {
            return Err(SupervisorError::UnknownProcess);
        };
        let pgid = Pid::from_raw(raw_pid);

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
            outputs,
            id,
            deadline,
            ProcessTerminalState::Exited { code: None },
        ) {
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
            outputs,
            id,
            KILL_GRACE,
            ProcessTerminalState::Killed,
        ) {
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

    /// Poll `try_wait` until the child is reaped or `budget` elapses.
    ///
    /// On reap, evicts the entry to `Tracked::Terminal` (dropping the
    /// `Child` handle) and returns the exit-derived terminal state
    /// (`Exited` carries the real exit code; the caller-supplied `on_reap`
    /// template is used only to pick `Exited`/`Killed` framing when no
    /// exit code is meaningful, e.g. after a signal). Until reap is
    /// confirmed, the entry is left `Running`: its pid cannot yet have
    /// been recycled by the OS.
    fn poll_until_reaped(
        children: &Mutex<HashMap<ProcessId, Tracked>>,
        terminal_order: &Mutex<VecDeque<ProcessId>>,
        outputs: &OutputTable,
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
                    unreachable!("this entry is only ever reached while still Running")
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
                    output_capture::record_confirmed_state(outputs, id, state);
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
