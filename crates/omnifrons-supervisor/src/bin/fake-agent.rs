//! Test-only helper binary: a synthetic `stream-json` agent CLI, standing
//! in for a real built-in-adapter target
//! (`crates/omnifrons-adapters/src/line_agent.rs`) in this crate's own
//! adapter-launch tests (`tests/adapter_launch.rs`, `tests/adapter_events.rs`).
//!
//! Deliberately always built (no cargo feature gate), exactly like
//! `demo-harness`: tests reach it via `env!("CARGO_BIN_EXE_fake-agent")`.
//! This crate has no dependency on `serde_json` (`docs/spike-log.md` §
//! Slice 3: `omnifrons-supervisor`'s own dependency table stays unchanged),
//! so every JSON line below is hand-built with a small, deliberately
//! minimal escaper -- sufficient for the fixed, self-controlled content
//! this binary ever emits, never a general-purpose JSON writer.
//!
//! Default behavior (no flags, or `--exit-code`/`--no-eof` only): reads
//! stdin to EOF (unless `--no-eof`), then emits, one per stdout line:
//! 1. `{"type":"system","subtype":"init","cwd":...,"model":"fake-model","session_id":"fake-session"}`,
//!    with `cwd` this process's own observed working directory;
//! 2. one assistant `text` block per prompt line read from stdin, each
//!    echoing that line verbatim;
//! 3. one assistant `tool_use` block (`name: "write_file"`, `input:
//!    {"path": "notes.md"}`);
//! 4. one deliberately truncated line, `{"type":"assistant"` (no closing
//!    brace);
//! 5. one stderr line, `diagnostic: hello`;
//! 6. one assistant `text` block whose text is this process's own full
//!    environment, as `KEY=VALUE` lines joined by `\n` (embedded, JSON-
//!    escaped, inside that one text field) -- so a test can read exactly
//!    what environment this child actually observed;
//! 7. `{"type":"result","subtype":"success"}`.
//!
//! Then exits with the code `--exit-code N` requested (default `0`).
//!
//! `--flood N`: emits `N` assistant `text` lines (`"flood <n>"`) with no
//! inter-line sleeping at all, then exits -- for deterministically
//! overflowing an undrained output-delivery channel
//! (`tests/adapter_events.rs`).
//!
//! `--huge-line N`: emits exactly one assistant `text` line whose `text`
//! field is `N` bytes of padding, then exits -- for exercising
//! `LineAssembler`'s truncation cap (`tests/adapter_events.rs`).
//!
//! `--stdin-byte-count`: reads stdin to EOF and prints exactly one plain
//! line, `stdin-byte-count <n>`, with the number of bytes read, then exits
//! -- for proving a `StdinPlan::Null` launch really hands the child an
//! empty stdin on every platform (`tests/approved_launch.rs`).
//!
//! Spike slice 4's pseudo-terminal modes (`tests/pty_launch.rs`):
//!
//! `--pty-report`: prints `isatty stdin=<bool> stdout=<bool>
//! stderr=<bool>`, then `TERM=<value>`, `COLUMNS=<value>`, and
//! `LINES=<value>` (the three values the supervisor itself sets on a PTY
//! child), then one `env-keys <K1> <K2> ...` line naming every
//! environment key this process observes -- names only, never values --
//! and exits 0.
//!
//! `--pty-echo`: reads one line from stdin and prints `typed: <line>`
//! (line terminator stripped), then exits 0.
//!
//! `--pty-corpus`: writes the shared terminal corpus
//! (`omnifrons_app::terminal_normalizer::corpus::bytes`) to stdout
//! verbatim, then a newline and a final `corpus-done` line, and exits 0.
//!
//! `--pty-ignore-sigterm` (unix): installs the demo harness's own
//! `SIGTERM`-ignore disposition, prints `ready` once it is in place (the
//! synchronization point a test waits on before calling `stop`), then
//! sleeps until killed.
//!
//! Spike slice 5's outbox modes (`tests/outbox_launch.rs`; the shell's
//! own tests cannot spawn this fixture), each reading the declared run
//! subdirectory from `OMNIFRONS_OUTPUT_DIR` and never printing its value:
//!
//! `--report-output-dir`: prints `output-dir set=<bool> writable=<bool>`
//! (whether the key is set, and whether it names a directory this process
//! can create a file in), then one `env-keys <K1> <K2> ...` line -- key
//! names only, never values.
//!
//! `--write-outbox <n>`: writes `n` heavy files into the declared
//! directory, cycling a PDF, a PNG, and a ZIP by magic bytes
//! (`artifact-<i>.<ext>`), plus one Markdown note (`notes.md`) and one
//! executable-shaped file (`tool.bin`, an ELF header), then emits exactly
//! one `artifact.publish` `tool_use` line naming every even-indexed heavy
//! file by its SHA-256 -- computed by this binary's own minimal SHA-256
//! ([`sha256`]), so this crate's dependency set stays unchanged, exactly
//! as its hand-built JSON already does. `--propose-missing` additionally
//! names `ghost.pdf`, a file it never writes, so a proposal naming a
//! digest not found in the subdirectory can be exercised.
//!
//! `--plant-link`: writes `outside.bin` beside the declared directory (at
//! the outbox root), then plants `escape-link` (a symbolic link to it) and
//! `linked.bin` (a hard link to it) inside the declared directory and
//! prints `planted`; where a symbolic link cannot be created (Windows
//! without the privilege) it prints `symlink-skipped` first.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The one environment key HAP-001 adds to a launch
/// (`omnifrons_app::OUTPUT_DIR_ENV_KEY`); named here verbatim so this
/// fixture reads exactly the key the product declares.
const OUTPUT_DIR_ENV_KEY: &str = "OMNIFRONS_OUTPUT_DIR";

/// The flags this fixture accepts, parsed once from argv. Independent
/// switches rather than a mode enum, because several combine
/// (`--report-output-dir` with `--write-outbox`, `--exit-code` with
/// anything), which is what the tests rely on.
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
struct Flags {
    exit_code: u8,
    flood: Option<u32>,
    huge_line: Option<usize>,
    stdin_byte_count: bool,
    no_eof: bool,
    pty_report: bool,
    pty_echo: bool,
    pty_corpus: bool,
    pty_ignore_sigterm: bool,
    report_output_dir: bool,
    write_outbox: Option<u32>,
    propose_missing: bool,
    plant_link: bool,
}

/// Parse the fixture's argv. A value-taking flag consumes the next
/// argument; an unknown flag is a test-setup bug and panics.
fn parse_flags(args: &[String]) -> Flags {
    let mut flags = Flags::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--pty-report" => flags.pty_report = true,
            "--pty-echo" => flags.pty_echo = true,
            "--pty-corpus" => flags.pty_corpus = true,
            "--pty-ignore-sigterm" => flags.pty_ignore_sigterm = true,
            "--report-output-dir" => flags.report_output_dir = true,
            "--write-outbox" => {
                i += 1;
                flags.write_outbox = Some(
                    args.get(i)
                        .and_then(|value| value.parse().ok())
                        .expect("--write-outbox requires a u32 value"),
                );
            }
            "--propose-missing" => flags.propose_missing = true,
            "--plant-link" => flags.plant_link = true,
            "--exit-code" => {
                i += 1;
                flags.exit_code = args
                    .get(i)
                    .and_then(|value| value.parse().ok())
                    .expect("--exit-code requires a u8 value");
            }
            "--flood" => {
                i += 1;
                flags.flood = Some(
                    args.get(i)
                        .and_then(|value| value.parse().ok())
                        .expect("--flood requires a u32 value"),
                );
            }
            "--huge-line" => {
                i += 1;
                flags.huge_line = Some(
                    args.get(i)
                        .and_then(|value| value.parse().ok())
                        .expect("--huge-line requires a usize value"),
                );
            }
            "--stdin-byte-count" => flags.stdin_byte_count = true,
            "--no-eof" => flags.no_eof = true,
            other => panic!("unknown fake-agent flag: {other}"),
        }
        i += 1;
    }
    flags
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flags = parse_flags(&args);
    let exit_code = flags.exit_code;
    let mut stdout = std::io::stdout();

    if flags.pty_report {
        run_pty_report(&mut stdout);
        return ExitCode::from(exit_code);
    }

    if flags.report_output_dir || flags.write_outbox.is_some() || flags.plant_link {
        if flags.report_output_dir {
            run_report_output_dir(&mut stdout);
        }
        if let Some(count) = flags.write_outbox {
            run_write_outbox(&mut stdout, count, flags.propose_missing);
        }
        if flags.plant_link {
            run_plant_link(&mut stdout);
        }
        return ExitCode::from(exit_code);
    }

    if flags.pty_echo {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .expect("reading one line from stdin must succeed");
        let line = line.trim_end_matches(['\r', '\n']);
        emit(&mut stdout, &format!("typed: {line}"));
        return ExitCode::from(exit_code);
    }

    if flags.pty_corpus {
        stdout
            .write_all(&omnifrons_app::terminal_normalizer::corpus::bytes())
            .expect("stdout must accept the corpus");
        stdout
            .write_all(b"\ncorpus-done\n")
            .expect("stdout must accept the corpus-done line");
        stdout.flush().expect("stdout must flush");
        return ExitCode::from(exit_code);
    }

    if flags.pty_ignore_sigterm {
        #[cfg(unix)]
        omnifrons_supervisor::demo::ignore_sigterm();
        emit(&mut stdout, "ready");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    if flags.stdin_byte_count {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .expect("reading stdin to EOF must succeed");
        emit(&mut stdout, &format!("stdin-byte-count {}", bytes.len()));
        return ExitCode::from(exit_code);
    }

    if let Some(n) = flags.flood {
        for line in 1..=n {
            emit(&mut stdout, &assistant_text_line(&format!("flood {line}")));
        }
        return ExitCode::from(exit_code);
    }

    if let Some(bytes) = flags.huge_line {
        let padding = "x".repeat(bytes);
        emit(&mut stdout, &assistant_text_line(&padding));
        return ExitCode::from(exit_code);
    }

    run_default_sequence(&mut stdout, flags.no_eof);
    ExitCode::from(exit_code)
}

/// The default full `stream-json` sequence: init, one text block per
/// prompt line, one `tool_use` block, a truncated line, a stderr
/// diagnostic, an environment dump, then a result.
fn run_default_sequence(stdout: &mut impl Write, no_eof: bool) {
    let prompt_lines: Vec<String> = if no_eof {
        Vec::new()
    } else {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .expect("reading stdin to EOF must succeed");
        input.lines().map(str::to_string).collect()
    };

    let cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    emit(
        stdout,
        &format!(
            r#"{{"type":"system","subtype":"init","cwd":{cwd},"model":"fake-model","session_id":"fake-session"}}"#,
            cwd = json_string(&cwd)
        ),
    );

    for line in &prompt_lines {
        emit(stdout, &assistant_text_line(line));
    }

    emit(
        stdout,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"tool-1","name":"write_file","input":{"path":"notes.md"}}]}}"#,
    );

    // Deliberately truncated: no closing brace at all.
    writeln!(stdout, r#"{{"type":"assistant""#).expect("stdout must accept the truncated line");
    stdout.flush().expect("stdout must flush");

    let mut stderr = std::io::stderr();
    writeln!(stderr, "diagnostic: hello").expect("stderr must accept the diagnostic line");
    stderr.flush().expect("stderr must flush");

    let mut env_pairs: Vec<(String, String)> = std::env::vars().collect();
    env_pairs.sort();
    let env_dump = env_pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    emit(stdout, &assistant_text_line(&env_dump));

    emit(stdout, r#"{"type":"result","subtype":"success"}"#);
}

/// `--pty-report`: whether each standard stream is a terminal, the three
/// variables the supervisor sets on a PTY child, and every environment key
/// name (never a value).
fn run_pty_report(stdout: &mut impl Write) {
    use std::io::IsTerminal as _;

    emit(
        stdout,
        &format!(
            "isatty stdin={} stdout={} stderr={}",
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
            std::io::stderr().is_terminal()
        ),
    );
    for key in ["TERM", "COLUMNS", "LINES"] {
        emit(
            stdout,
            &format!("{key}={}", std::env::var(key).unwrap_or_default()),
        );
    }
    let mut keys: Vec<String> = std::env::vars_os()
        .map(|(key, _value)| key.to_string_lossy().into_owned())
        .collect();
    keys.sort();
    emit(stdout, &format!("env-keys {}", keys.join(" ")));
}

/// The declared run subdirectory, or `None` when the key is unset. The
/// value is never printed by any mode: a failure names the key only.
fn declared_output_dir() -> Option<PathBuf> {
    std::env::var_os(OUTPUT_DIR_ENV_KEY).map(PathBuf::from)
}

/// The declared run subdirectory, required by the modes that write into
/// it.
fn required_output_dir() -> PathBuf {
    declared_output_dir()
        .unwrap_or_else(|| panic!("{OUTPUT_DIR_ENV_KEY} must be set for this fake-agent mode"))
}

/// `--report-output-dir`: whether the key is set and names a directory
/// this process can create a file in, then every environment key name
/// (never a value).
fn run_report_output_dir(stdout: &mut impl Write) {
    let declared = declared_output_dir();
    let writable = declared.as_deref().is_some_and(is_writable_directory);
    emit(
        stdout,
        &format!("output-dir set={} writable={writable}", declared.is_some()),
    );
    let mut keys: Vec<String> = std::env::vars_os()
        .map(|(key, _value)| key.to_string_lossy().into_owned())
        .collect();
    keys.sort();
    emit(stdout, &format!("env-keys {}", keys.join(" ")));
}

/// Whether `dir` is a directory a probe file can be created in (and
/// removed from again).
fn is_writable_directory(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(".omnifrons-fake-agent-write-probe");
    let created = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    created
}

/// The heavy kinds `--write-outbox` cycles through: extension and the
/// leading magic bytes of each.
const HEAVY_KINDS: [(&str, &[u8]); 3] = [
    ("pdf", b"%PDF-1.7\n"),
    ("png", &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
    ("zip", b"PK\x03\x04\x14\x00"),
];

/// `--write-outbox <count>`: the heavy files, the note, the
/// executable-shaped file, then one `artifact.publish` proposal naming
/// every even-indexed heavy file by digest (plus `ghost.pdf` under
/// `--propose-missing`).
fn run_write_outbox(stdout: &mut impl Write, count: u32, propose_missing: bool) {
    let dir = required_output_dir();
    let mut proposed: Vec<(String, [u8; 32])> = Vec::new();
    for index in 0..count {
        let (extension, magic) = HEAVY_KINDS[(index % 3) as usize];
        let name = format!("artifact-{index}.{extension}");
        let mut content = magic.to_vec();
        content.extend_from_slice(format!("omnifrons fake artifact {index}\n").as_bytes());
        std::fs::write(dir.join(&name), &content)
            .unwrap_or_else(|error| panic!("writing {name} into the output dir failed: {error}"));
        if index % 2 == 0 {
            proposed.push((name, sha256::digest(&content)));
        }
    }
    std::fs::write(
        dir.join("notes.md"),
        format!("# Fake agent notes\n\nwrote {count} heavy files\n"),
    )
    .expect("writing notes.md into the output dir must succeed");
    let mut elf = vec![0x7F, b'E', b'L', b'F', 2, 1, 1, 0];
    elf.resize(64, 0);
    std::fs::write(dir.join("tool.bin"), &elf)
        .expect("writing tool.bin into the output dir must succeed");
    if propose_missing {
        proposed.push(("ghost.pdf".to_string(), sha256::digest(b"ghost")));
    }

    let entries = proposed
        .iter()
        .map(|(name, digest)| {
            format!(
                r#"{{"name":{},"sha256":"{}"}}"#,
                json_string(name),
                hex(digest)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    emit(
        stdout,
        &format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"tool-publish","name":"artifact.publish","input":{{"entries":[{entries}]}}}}]}}}}"#
        ),
    );
}

/// `--plant-link`: `outside.bin` at the outbox root, then a symbolic link
/// and a hard link to it inside the declared directory.
fn run_plant_link(stdout: &mut impl Write) {
    let dir = required_output_dir();
    let parent = dir
        .parent()
        .expect("the declared output dir must have a parent (the outbox)");
    let outside = parent.join("outside.bin");
    std::fs::write(&outside, b"outside the run subdirectory\n")
        .expect("writing outside.bin at the outbox root must succeed");
    #[cfg(unix)]
    let symlinked = std::os::unix::fs::symlink(&outside, dir.join("escape-link")).is_ok();
    #[cfg(windows)]
    let symlinked = std::os::windows::fs::symlink_file(&outside, dir.join("escape-link")).is_ok();
    #[cfg(not(any(unix, windows)))]
    let symlinked = false;
    if !symlinked {
        emit(stdout, "symlink-skipped");
    }
    std::fs::hard_link(&outside, dir.join("linked.bin"))
        .expect("planting the hard link inside the output dir must succeed");
    emit(stdout, "planted");
}

/// Lowercase hex of a digest.
fn hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// A minimal SHA-256 (FIPS 180-4) for this fixture's own small, fixed
/// content, so that `--write-outbox` can name its files by real digest
/// without adding a hashing dependency to this crate (`docs/spike-log.md`
/// § Slice 3's hand-built JSON is the precedent). Never production code:
/// `omnifrons-adapters` hashes with `sha2`, and `tests/outbox_launch.rs`
/// checks this implementation against it; the unit tests below pin the
/// standard's own test vectors.
mod sha256 {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    /// The SHA-256 digest of `data`.
    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut state = INITIAL;
        let bit_length = u64::try_from(data.len())
            .expect("a fixture payload fits in u64")
            .wrapping_mul(8);
        let mut message = data.to_vec();
        message.push(0x80);
        while message.len() % 64 != 56 {
            message.push(0);
        }
        message.extend_from_slice(&bit_length.to_be_bytes());
        for block in message.as_chunks::<64>().0 {
            compress(&mut state, block);
        }
        let mut out = [0u8; 32];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
            *chunk = word.to_be_bytes();
        }
        out
    }

    /// One compression round over a 64-byte block. The single-letter
    /// names are FIPS 180-4's own working variables, kept so the loop
    /// reads against the standard.
    #[allow(clippy::many_single_char_names)]
    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for t in 16..64 {
            let s0 = w[t - 15].rotate_right(7) ^ w[t - 15].rotate_right(18) ^ (w[t - 15] >> 3);
            let s1 = w[t - 2].rotate_right(17) ^ w[t - 2].rotate_right(19) ^ (w[t - 2] >> 10);
            w[t] = w[t - 16]
                .wrapping_add(s0)
                .wrapping_add(w[t - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for (k, word) in K.iter().zip(w) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(word);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::digest;

        fn hex(digest: [u8; 32]) -> String {
            crate::hex(&digest)
        }

        /// FIPS 180-4's published SHA-256 test vectors.
        #[test]
        fn matches_the_fips_180_4_vectors() {
            assert_eq!(
                hex(digest(b"")),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            );
            assert_eq!(
                hex(digest(b"abc")),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
            assert_eq!(
                hex(digest(
                    b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
                )),
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
            );
        }

        /// A payload spanning several blocks, including one whose padding
        /// needs an extra block (a length of exactly 56 bytes modulo 64).
        #[test]
        fn handles_padding_at_the_block_boundary() {
            assert_eq!(
                hex(digest(&[b'a'; 56])),
                "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a"
            );
            assert_eq!(
                hex(digest(&vec![b'a'; 1_000_000])),
                "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
            );
        }
    }
}

/// Write `line` followed by a newline, then flush -- every emitted line is
/// visible to a reader immediately, never buffered indefinitely.
fn emit(writer: &mut impl Write, line: &str) {
    writeln!(writer, "{line}").expect("stdout must accept fake-agent output");
    writer.flush().expect("stdout must flush");
}

/// Build one assistant `text`-block `stream-json` line.
fn assistant_text_line(text: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":{}}}]}}}}"#,
        json_string(text)
    )
}

/// A minimal JSON string literal (quotes plus escaping), sufficient for
/// this binary's own fixed, self-controlled content -- never a
/// general-purpose JSON writer (this crate has no `serde_json`
/// dependency, deliberately, per `docs/spike-log.md` § Slice 3).
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
