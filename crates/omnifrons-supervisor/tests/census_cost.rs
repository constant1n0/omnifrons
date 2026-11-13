//! Measurements for #197, run by hand:
//! `cargo test -p omnifrons-supervisor --test census_cost -- --ignored --nocapture --test-threads=1`.
//!
//! - **Census cost.** The CPU time this process spends per second while
//!   children live through their first minute, for 1 child and for 16 (the
//!   running cap), with the host's process count. Every census walk reads
//!   every process's `/proc/<pid>/stat`, on this process's blocking pool.
//! - **Exit latency.** The time from spawn to the `State` frame of
//!   `sh -c 'echo x'`, and the time of one full `/proc` walk: the exit census
//!   adds one such walk to every run's end.
//!
//! Ignored by default: the numbers depend on the host and its load, so they
//! are printed, never asserted.

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use nix::sys::resource::{UsageWho, getrusage};
use omnifrons_app::{FramePayload, ProcessOutput, ProcessSpec, ProcessSupervisor};
use omnifrons_supervisor::TokioProcessSupervisor;

/// How long each cost window lasts.
const WINDOW: Duration = Duration::from_secs(3);

/// How many runs or walks each latency figure is taken over.
const SAMPLES: usize = 20;

/// User plus system CPU time of this whole process, every thread included.
fn cpu_time() -> Duration {
    let usage = getrusage(UsageWho::RUSAGE_SELF).expect("getrusage must succeed");
    let micros = |time: nix::sys::time::TimeVal| {
        Duration::from_secs(u64::try_from(time.tv_sec()).unwrap_or(0))
            + Duration::from_micros(u64::try_from(time.tv_usec()).unwrap_or(0))
    };
    micros(usage.user_time()) + micros(usage.system_time())
}

/// How many processes the host has right now.
fn host_processes() -> usize {
    std::fs::read_dir("/proc")
        .expect("/proc must be readable")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.parse::<u32>().is_ok())
        })
        .count()
}

/// One walk shaped like the supervisor's: every `/proc/<pid>/stat`, read.
fn walk_proc() {
    for entry in std::fs::read_dir("/proc")
        .expect("/proc must be readable")
        .filter_map(Result::ok)
    {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.parse::<u32>().is_ok())
        {
            let _ = std::fs::read_to_string(entry.path().join("stat"));
        }
    }
}

fn median_and_p90(mut samples: Vec<Duration>) -> (Duration, Duration) {
    samples.sort();
    (samples[samples.len() / 2], samples[samples.len() * 9 / 10])
}

/// CPU milliseconds per second this process spends while `children` idle
/// children live through the first seconds of their first minute.
fn census_cpu_per_second(children: usize) -> f64 {
    let mut supervisor = TokioProcessSupervisor::new();
    let ids: Vec<_> = (0..children)
        .map(|_| {
            supervisor
                .spawn(ProcessSpec::new("/bin/sh").with_args(["-c", "sleep 6"]))
                .expect("spawning sh must succeed")
        })
        .collect();
    // Past every spawn census.
    std::thread::sleep(Duration::from_millis(300));
    let (cpu, wall) = (cpu_time(), Instant::now());
    std::thread::sleep(WINDOW);
    let spent = cpu_time().saturating_sub(cpu);
    let elapsed = wall.elapsed();
    for id in ids {
        let _ = supervisor.stop(id, Duration::from_secs(5));
    }
    spent.as_secs_f64() * 1000.0 / elapsed.as_secs_f64()
}

#[test]
#[ignore = "a measurement, run by hand with --ignored --nocapture"]
fn census_cost_per_second_for_one_and_for_sixteen_children() {
    let processes = host_processes();
    let one = census_cpu_per_second(1);
    let sixteen = census_cpu_per_second(16);
    eprintln!(
        "census cost, {processes} host processes: 1 child {one:.1} ms CPU/s, \
         16 children {sixteen:.1} ms CPU/s"
    );
}

#[test]
#[ignore = "a measurement, run by hand with --ignored --nocapture"]
fn exit_latency_and_one_proc_walk() {
    let mut supervisor = TokioProcessSupervisor::new();
    let mut runs = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let id = supervisor
            .spawn(ProcessSpec::new("/bin/sh").with_args(["-c", "echo x"]))
            .expect("spawning sh must succeed");
        let frames = supervisor.subscribe(id).expect("subscribe must succeed");
        loop {
            let frame = frames
                .recv_timeout(Duration::from_secs(10))
                .expect("the run must end");
            if matches!(frame.payload, FramePayload::State(_)) {
                break;
            }
        }
        runs.push(started.elapsed());
    }
    let walks = (0..SAMPLES)
        .map(|_| {
            let started = Instant::now();
            walk_proc();
            started.elapsed()
        })
        .collect();
    let (run_median, run_p90) = median_and_p90(runs);
    let (walk_median, walk_p90) = median_and_p90(walks);
    eprintln!(
        "exit latency, {} host processes: spawn to State of `sh -c 'echo x'` median {run_median:?} \
         p90 {run_p90:?}; one /proc walk median {walk_median:?} p90 {walk_p90:?}",
        host_processes()
    );
}
