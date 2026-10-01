//! Specialist cleanup adapters (P6 tasks 7.5–7.7). A specialist is an
//! external tool (cargo, docker) whose exact vocabulary DiskGraph cannot
//! reinvent; the adapter layer exists so an agent can use it without ever
//! obtaining a general "delete things" capability.
//!
//! The three hard lines, from the spec:
//! - **EC-03** an adapter only exists on an explicit allow-list, reports
//!   `unavailable` cleanly when its tool is missing, and plans *exact
//!   objects*. There is no fallback that force-deletes a directory.
//! - **EC-04** external programs run through a sandboxed runner: a fixed
//!   program path, structured argv (never a shell string), a minimal
//!   environment, and bounded time/output/retries.
//! - Result interpretation is conservative: an unprovable outcome parks as
//!   [`SpecialistVerdict::NeedsAttention`] instead of claiming success.

#[cfg(unix)]
use std::io::Read;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::{Duration, Instant};

use crate::OpsError;

// ------------------------------------------------------------------ 7.5 ---

/// One specialist capability the host may allow: what it is, which program
/// provides it, and what that program must at least be.
#[derive(Clone, Copy, Debug)]
pub struct AdapterCapability {
    /// Stable id used in plans and logs, e.g. `cargo-clean`.
    pub id: &'static str,
    pub display: &'static str,
    /// The program's well-known name. The deployment resolves it to a full
    /// path once, at registration; the runner never searches a PATH.
    pub program: &'static str,
    /// Minimum version accepted by `probe`.
    pub min_version: &'static str,
}

/// The capabilities this build knows about. Knowing is not allowing.
pub const CARGO_CLEAN: AdapterCapability = AdapterCapability {
    id: "cargo-clean",
    display: "Cargo build directory cleanup",
    program: "cargo",
    min_version: "1.70.0",
};

pub const DOCKER_INVENTORY: AdapterCapability = AdapterCapability {
    id: "docker-inventory",
    display: "Docker disk usage inventory",
    program: "docker",
    min_version: "24.0.0",
};

pub const DOCKER_CLEAN: AdapterCapability = AdapterCapability {
    id: "docker-clean",
    display: "Docker exact-object cleanup",
    program: "docker",
    min_version: "24.0.0",
};

/// All capabilities, for listing surfaces.
pub const ALL_CAPABILITIES: &[AdapterCapability] = &[CARGO_CLEAN, DOCKER_INVENTORY, DOCKER_CLEAN];

/// The allow-list. A capability absent from it cannot be probed, planned, or
/// run, whatever an agent asks for (EC-03).
pub struct AdapterRegistry {
    allowed: Vec<(&'static str, PathBuf)>,
}

impl AdapterRegistry {
    /// Opens a registry that allows exactly the named capabilities, resolved
    /// to the program paths the host configuration recorded.
    pub fn allowing(allowed: &[(&'static str, PathBuf)]) -> Self {
        Self {
            allowed: allowed.to_vec(),
        }
    }

    /// A registry that allows nothing; the safe default.
    pub fn empty() -> Self {
        Self {
            allowed: Vec::new(),
        }
    }

    /// The capability record and its resolved program, or a refusal that
    /// names the allow-list as the reason.
    pub fn capability(&self, id: &str) -> Result<(&'static AdapterCapability, &Path), OpsError> {
        let known = ALL_CAPABILITIES
            .iter()
            .find(|capability| capability.id == id)
            .ok_or_else(|| {
                OpsError::NotAuthorized(format!("no such specialist capability: {id}"))
            })?;
        let (_, program) = self
            .allowed
            .iter()
            .find(|(allowed_id, _)| allowed_id == &known.id)
            .ok_or_else(|| {
                OpsError::NotAuthorized(format!(
                    "specialist capability {id} is not on this deployment's allow-list"
                ))
            })?;
        Ok((known, program.as_path()))
    }

    /// True when the capability may be used at all.
    pub fn is_allowed(&self, id: &str) -> bool {
        self.capability(id).is_ok()
    }
}

/// Whether a specialist tool is usable, and if not, exactly why. An
/// unavailable adapter offers nothing: there is no degraded mode that
/// force-deletes directories instead (EC-03).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterStatus {
    Available { version: String },
    Unavailable { reason: String },
}

/// Probes a resolved program through the sandboxed runner: it must exist,
/// answer `--version` with exit 0, and report a version at or above the
/// capability's minimum.
pub fn probe(
    capability: &AdapterCapability,
    program: &Path,
    runner: &dyn CommandRunner,
) -> AdapterStatus {
    let spec = CommandSpec {
        program: program.to_path_buf(),
        args: vec!["--version".into()],
        cwd: None,
        env: default_env(),
        timeout_ms: 10_000,
        max_output_bytes: 4_096,
        retries: 0,
    };
    let outcome = match runner.run(&spec) {
        Ok(outcome) => outcome,
        Err(error) => {
            return AdapterStatus::Unavailable {
                reason: format!("{} could not be run: {error}", capability.program),
            };
        }
    };
    if outcome.timed_out {
        return AdapterStatus::Unavailable {
            reason: format!("{} --version timed out", capability.program),
        };
    }
    if outcome.exit_code != 0 {
        return AdapterStatus::Unavailable {
            reason: format!(
                "{} --version exited with {}",
                capability.program, outcome.exit_code
            ),
        };
    }
    let version = String::from_utf8_lossy(&outcome.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    if version.is_empty() {
        return AdapterStatus::Unavailable {
            reason: format!("{} printed no version line", capability.program),
        };
    }
    let found = version
        .split_whitespace()
        .find(|token| token.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or("");
    if version_at_least(found, capability.min_version) {
        AdapterStatus::Available { version }
    } else {
        AdapterStatus::Unavailable {
            reason: format!(
                "{} reports {found}, below the required {}",
                capability.program, capability.min_version
            ),
        }
    }
}

/// Dotted numeric comparison, tolerant of suffixes: "1.75" >= "1.70.0".
fn version_at_least(found: &str, minimum: &str) -> bool {
    fn numbers(version: &str) -> Vec<u64> {
        version
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse().ok())
            .collect()
    }
    let found = numbers(found);
    let minimum = numbers(minimum);
    for index in 0..minimum.len().max(found.len()) {
        let left = found.get(index).copied().unwrap_or(0);
        let right = minimum.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    true
}

// ------------------------------------------------------------------ 7.6 ---

/// A fully specified external run. Every field is structural: there is no
/// shell string anywhere in this type, so command injection has no surface.
#[derive(Clone, Debug)]
pub struct CommandSpec {
    /// The exact program to execute. It comes from the registry's resolved
    /// path, never from user text or a project file.
    pub program: PathBuf,
    /// One argument per element, passed verbatim as one argv entry each.
    pub args: Vec<String>,
    /// The working directory, when the tool needs one.
    pub cwd: Option<PathBuf>,
    /// The whole environment the child receives: cleared first, then these.
    pub env: Vec<(String, String)>,
    pub timeout_ms: u64,
    /// Combined cap per stream; output past it is discarded and flagged.
    pub max_output_bytes: usize,
    /// How many times a failed run may be retried.
    pub retries: u32,
}

/// What a run produced. `truncated` and `timed_out` exist so a caller can
/// refuse to interpret output it did not fully see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutcome {
    /// The child's exit code, or -1 when it was killed or never started.
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub truncated: bool,
}

/// Runs [`CommandSpec`]s. The ops layer never calls `std::process::Command`
/// anywhere else, so the sandbox has exactly one definition.
pub trait CommandRunner {
    fn run(&self, spec: &CommandSpec) -> Result<RunOutcome, OpsError>;
}

/// The production runner: no shell, minimal env, bounded time and output.
pub struct SandboxedRunner;

impl SandboxedRunner {
    /// The environment children get when a caller has no stronger opinion:
    /// enough to find libraries and render output, nothing that leaks a
    /// caller's whole environment into a project-configured context.
    pub fn default_env() -> Vec<(String, String)> {
        default_env()
    }
}

fn default_env() -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        env.push(("PATH".into(), path.to_string_lossy().into_owned()));
    }
    if let Ok(lang) = std::env::var("LANG") {
        env.push(("LANG".into(), lang));
    }
    env
}

impl CommandRunner for SandboxedRunner {
    fn run(&self, spec: &CommandSpec) -> Result<RunOutcome, OpsError> {
        let mut attempt = 0_u32;
        loop {
            attempt += 1;
            let outcome = run_once(spec)?;
            // A retry is for transient failures only; a timeout or a clean
            // non-zero exit is the tool's answer and is reported as such.
            let transient = !outcome.timed_out && outcome.exit_code != 0;
            if !transient || attempt > spec.retries {
                return Ok(outcome);
            }
        }
    }
}

#[cfg(unix)]
fn run_once(spec: &CommandSpec) -> Result<RunOutcome, OpsError> {
    use std::process::{Command, Stdio};

    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    // The child sees only what the spec names. A project file cannot conjure
    // credentials into its own cleanup tool (EC-04).
    command.env_clear();
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Put the command and ordinary descendants in their own process
        // group so timeout can close pipes inherited by background children.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().map_err(|error| {
        OpsError::Stale(format!(
            "specialist program {} is unavailable: {error}",
            spec.program.display()
        ))
    })?;

    // Readers own their pipes so the poll loop never blocks on a quiet child.
    let cap = spec.max_output_bytes.max(1);
    let deadline = Instant::now() + Duration::from_millis(spec.timeout_ms.max(1));
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || drain(stdout_pipe, cap, deadline));
    let stderr_reader = std::thread::spawn(move || drain(stderr_pipe, cap, deadline));

    let mut timed_out = false;
    loop {
        match child.try_wait()? {
            Some(_status) => break,
            None if Instant::now() >= deadline => {
                timed_out = true;
                #[cfg(unix)]
                unsafe {
                    let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| OpsError::Stale("stdout reader failed".into()))?;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| OpsError::Stale("stderr reader failed".into()))?;
    if Instant::now() >= deadline && (stdout_truncated || stderr_truncated) {
        timed_out = true;
        // The direct child may have exited while a background descendant
        // retained stdout/stderr; its process group can still be alive.
        unsafe {
            let _ = libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
    }
    let exit_code = match child.try_wait()? {
        Some(status) => status.code().unwrap_or(-1),
        None if timed_out => -1,
        None => -1,
    };
    Ok(RunOutcome {
        exit_code,
        stdout,
        stderr,
        timed_out,
        truncated: stdout_truncated || stderr_truncated,
    })
}

#[cfg(not(unix))]
fn run_once(_spec: &CommandSpec) -> Result<RunOutcome, OpsError> {
    Err(OpsError::Stale(
        "unsupported: bounded specialist subprocesses are not implemented on this platform".into(),
    ))
}

/// Reads a pipe to the end, keeping at most `cap` bytes and discarding the
/// rest (so a chatty child can still finish), and reporting the discard.
#[cfg(unix)]
fn drain(
    pipe: Option<impl Read + std::os::fd::AsRawFd>,
    cap: usize,
    deadline: Instant,
) -> (Vec<u8>, bool) {
    let mut buffer = Vec::new();
    let Some(mut pipe) = pipe else {
        return (buffer, false);
    };
    let fd = std::os::fd::AsRawFd::as_raw_fd(&pipe);
    // SAFETY: fd stays owned by this reader for its whole lifetime.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return (buffer, true);
        }
    }
    let mut chunk = [0_u8; 8_192];
    loop {
        if Instant::now() >= deadline {
            return (buffer, true);
        }
        match pipe.read(&mut chunk) {
            Ok(0) => return (buffer, false),
            Ok(read) => {
                if buffer.len() < cap {
                    let keep = read.min(cap - buffer.len());
                    buffer.extend_from_slice(&chunk[..keep]);
                    if keep < read {
                        // The cap is hit; keep draining so the child is not
                        // blocked on a full pipe, but the tail is lost.
                        drain_rest(&mut pipe, deadline);
                        return (buffer, true);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return (buffer, true),
        }
    }
}

#[cfg(unix)]
fn drain_rest(pipe: &mut impl Read, deadline: Instant) {
    let mut chunk = [0_u8; 8_192];
    while Instant::now() < deadline {
        match pipe.read(&mut chunk) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return,
        }
    }
}

/// What an external run proved. Interpretation is conservative: only a clean
/// exit with the expected marker in fully-seen output counts as done.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpecialistVerdict {
    /// The expected effect is visible in the tool's own report.
    Confirmed,
    /// The outcome cannot be proven: the operation parks for reconciliation
    /// instead of being reported as a success nobody verified (OP-08).
    NeedsAttention { reason: String },
}

/// Judges a specialist run against the marker its own output must contain to
/// prove the effect happened. Exit codes alone prove nothing: a tool may exit
/// 0 after doing nothing, and output past the cap may hold the only evidence.
pub fn verify_specialist_result(outcome: &RunOutcome, expected_marker: &str) -> SpecialistVerdict {
    if outcome.timed_out {
        return SpecialistVerdict::NeedsAttention {
            reason: "the specialist command timed out; its effect is unknown".into(),
        };
    }
    if outcome.truncated {
        return SpecialistVerdict::NeedsAttention {
            reason: "the specialist command's output was truncated; its effect is unproven".into(),
        };
    }
    if outcome.exit_code != 0 {
        return SpecialistVerdict::NeedsAttention {
            reason: format!(
                "the specialist command exited with {}; its effect is unproven",
                outcome.exit_code
            ),
        };
    }
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    if stdout.contains(expected_marker) || stderr.contains(expected_marker) {
        SpecialistVerdict::Confirmed
    } else {
        SpecialistVerdict::NeedsAttention {
            reason: format!(
                "the specialist command's output does not mention {expected_marker:?}; its effect is unproven"
            ),
        }
    }
}

// ------------------------------------------------------------------ 7.7 ---

/// One exact object a specialist adapter offers for review. `kind` says what
/// it is; nothing here is a general directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryObject {
    pub path: PathBuf,
    pub bytes: u64,
    /// e.g. `cargo-target`; the kind bounds what a plan may do with it.
    pub kind: &'static str,
}

/// What a specialist adapter observed, ready for a human to review. It is a
/// review queue, never a plan and never an authorization.
#[derive(Clone, Debug)]
pub struct CleanupInventory {
    pub adapter: &'static str,
    pub objects: Vec<InventoryObject>,
    /// Measurements the caller must see: shared directories, active builds,
    /// anything that bounds what a cleanup may honestly claim.
    pub notes: Vec<String>,
    /// True when the tool's own markers say a build is in flight; cleanup
    /// must wait.
    pub active_build: bool,
}

/// Sums the bytes of a directory tree; measurement, not a promise.
fn dir_size(path: &Path) -> u64 {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return 0,
    };
    if !metadata.is_dir() {
        return metadata.len();
    }
    let mut total = 0_u64;
    let Ok(entries) = std::fs::read_dir(path) else {
        return total;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            total = total.saturating_add(dir_size(&path));
        } else if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            total = total.saturating_add(metadata.len());
        }
    }
    total
}

/// Inventories the Cargo build output of one project directory, reading the
/// real environment for the shared-target setting.
pub fn cargo_inventory(project_dir: &Path) -> Result<CleanupInventory, OpsError> {
    let shared = std::env::var("CARGO_TARGET_DIR").ok();
    cargo_inventory_with_env(project_dir, shared.as_deref())
}

/// The inventory with the environment injected, so tests never mutate process
/// state to exercise a setting.
///
/// Without running cargo (EC-03), the target directory's existence and cargo's
/// own lock markers answer everything the inventory needs. The real target
/// scope is the project-local `target/`. A directory named by
/// `CARGO_TARGET_DIR` lives outside the project, is potentially shared with
/// other projects, and is therefore reported as a note but never offered as
/// an object. A `.cargo-lock` inside the target tree marks an active build,
/// and the inventory says cleanup must wait (EC-03, OP-04).
pub fn cargo_inventory_with_env(
    project_dir: &Path,
    cargo_target_dir: Option<&str>,
) -> Result<CleanupInventory, OpsError> {
    if !project_dir.join("Cargo.toml").is_file() {
        return Err(OpsError::Stale(
            "the project directory has no Cargo.toml; it is not a cargo project".into(),
        ));
    }
    let mut notes = Vec::new();
    let mut objects = Vec::new();
    let mut active_build = false;
    let local = project_dir.join("target");
    if std::fs::symlink_metadata(&local).is_ok_and(|metadata| metadata.is_dir()) {
        // The lock FILE persists after a build finishes (verified on a real
        // host), so existence proves nothing. Cargo guards a build with an
        // advisory lock on that file, so an uncontended lock attempt is the
        // honest active-build signal.
        let lock = local.join("debug").join(".cargo-lock");
        if lock.is_file() && build_lock_is_contended(&lock) {
            active_build = true;
            notes.push(
                "cargo's build lock is held: a build is in progress; cleanup must wait".into(),
            );
        }
        objects.push(InventoryObject {
            path: local.clone(),
            bytes: dir_size(&local),
            kind: "cargo-target",
        });
    } else {
        notes.push("the project has no target directory; there is nothing to clean".into());
    }
    if let Some(shared) = cargo_target_dir.filter(|shared| !shared.is_empty()) {
        notes.push(format!(
            "CARGO_TARGET_DIR={shared} names a build directory outside this project; \
             it may be shared with other projects and is never planned"
        ));
    }
    Ok(CleanupInventory {
        adapter: CARGO_CLEAN.id,
        objects,
        notes,
        active_build,
    })
}

/// True when some process already holds an advisory lock on `lock`. The
/// attempt never blocks: acquiring and immediately releasing the lock is the
/// harmless probe; contention is the evidence.
fn build_lock_is_contended(lock: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock)
        {
            Ok(file) => file,
            Err(_) => return false,
        };
        // SAFETY: flock on a file we just opened is the documented advisory
        // protocol; the descriptors are valid for the duration of the calls.
        let acquired = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if acquired == 0 {
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
            return false;
        }
        true
    }
    #[cfg(not(unix))]
    {
        let _ = lock;
        false
    }
}

#[cfg(all(test, windows))]
mod unsupported_windows_tests {
    use super::*;

    #[test]
    fn bounded_external_tool_execution_refuses_without_a_windows_sandbox() {
        let spec = CommandSpec {
            program: PathBuf::from("missing-tool.exe"),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            timeout_ms: 1000,
            max_output_bytes: 1024,
            retries: 0,
        };
        assert!(matches!(
            SandboxedRunner.run(&spec),
            Err(OpsError::Stale(message)) if message.contains("unsupported")
        ));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Write;

    /// Writes an executable shell script and returns its path, so the tests
    /// can play the specialist tool without needing the real one installed.
    fn fake_tool(directory: &Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(file, "#!/bin/sh").unwrap();
        write!(file, "{body}").unwrap();
        drop(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn spec_for(program: PathBuf, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program,
            args: args.iter().map(|argument| argument.to_string()).collect(),
            cwd: None,
            env: SandboxedRunner::default_env(),
            timeout_ms: 10_000,
            max_output_bytes: 1 << 20,
            retries: 0,
        }
    }

    #[test]
    fn an_unlisted_capability_is_refused_even_when_known() {
        let registry = AdapterRegistry::empty();
        assert!(matches!(
            registry.capability("cargo-clean"),
            Err(OpsError::NotAuthorized(message)) if message.contains("allow-list")
        ));
        // An id this build has never heard of is refused for a different
        // reason, and equally offers nothing.
        assert!(registry.capability("rm-everything").is_err());
    }

    #[test]
    fn a_missing_tool_reports_unavailable_and_offers_nothing() {
        let program = PathBuf::from("/nonexistent/cargo");
        let registry = AdapterRegistry::allowing(&[("cargo-clean", program.clone())]);
        let (capability, resolved) = registry.capability("cargo-clean").unwrap();
        assert_eq!(resolved, program.as_path());
        match probe(capability, resolved, &SandboxedRunner) {
            AdapterStatus::Unavailable { reason } => {
                assert!(reason.contains("unavailable"), "{reason}");
            }
            other => panic!("expected unavailable, got {other:?}"),
        }
    }

    #[test]
    fn a_probe_accepts_a_new_enough_version_and_refuses_an_old_one() {
        let workspace = tempfile::TempDir::with_prefix("dg-specialist-probe-").unwrap();
        let new_tool = fake_tool(workspace.path(), "cargo-new", "echo \"fake-tool 1.99.0\"\n");
        let old_tool = fake_tool(workspace.path(), "cargo-old", "echo \"fake-tool 0.9.1\"\n");
        let (capability, _) = AdapterRegistry::allowing(&[("cargo-clean", new_tool.clone())])
            .capability("cargo-clean")
            .unwrap();
        assert!(matches!(
            probe(capability, &new_tool, &SandboxedRunner),
            AdapterStatus::Available { version } if version.contains("1.99.0")
        ));
        assert!(matches!(
            probe(capability, &old_tool, &SandboxedRunner),
            AdapterStatus::Unavailable { reason } if reason.contains("below the required")
        ));
    }

    #[test]
    fn structured_arguments_pass_through_without_a_shell() {
        // The payload would execute under a shell; argv passthrough must
        // deliver it as one literal argument instead (EC-04).
        let payload = "x;$(whoami)`rm -rf /`|deadly";
        let outcome = SandboxedRunner
            .run(&spec_for(
                PathBuf::from("/usr/bin/printf"),
                &["%s\\n", payload],
            ))
            .unwrap();
        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.stdout, format!("{payload}\n").into_bytes());
    }

    #[test]
    fn a_child_sees_only_the_named_environment() {
        let mut spec = spec_for(PathBuf::from("/usr/bin/env"), &[]);
        spec.env = vec![("PATH".into(), "/usr/bin:/bin".into())];
        let outcome = SandboxedRunner.run(&spec).unwrap();
        let printed = String::from_utf8_lossy(&outcome.stdout);
        assert!(printed.contains("PATH="));
        assert!(
            !printed.contains("HOME="),
            "the caller's whole environment leaked into the child"
        );
    }

    #[test]
    fn a_timeout_kills_a_stuck_child() {
        let started = Instant::now();
        let mut spec = spec_for(PathBuf::from("/bin/sleep"), &["30"]);
        spec.timeout_ms = 200;
        let outcome = SandboxedRunner.run(&spec).unwrap();
        assert!(outcome.timed_out);
        assert_eq!(outcome.exit_code, -1);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_descendants_that_inherit_output_pipes() {
        let started = Instant::now();
        let mut spec = spec_for(PathBuf::from("/bin/sh"), &["-c", "sleep 2 & wait"]);
        spec.timeout_ms = 50;
        let outcome = SandboxedRunner.run(&spec).unwrap();
        assert!(outcome.timed_out);
        assert!(
            started.elapsed() < Duration::from_millis(800),
            "descendant kept the reader threads alive after timeout"
        );
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_a_descendant_after_the_direct_child_exits() {
        let workspace = tempfile::tempdir().unwrap();
        let marker = workspace.path().join("escaped");
        let script = format!(
            "(/bin/sleep 1; /usr/bin/touch {}) & exit 0",
            marker.display()
        );
        let mut spec = spec_for(PathBuf::from("/bin/sh"), &["-c", &script]);
        spec.timeout_ms = 50;
        let outcome = SandboxedRunner.run(&spec).unwrap();
        assert!(outcome.timed_out);
        std::thread::sleep(Duration::from_millis(1_200));
        assert!(!marker.exists(), "a descendant survived the timeout");
    }

    #[test]
    fn output_past_the_cap_is_discarded_and_flagged() {
        let workspace = tempfile::TempDir::with_prefix("dg-specialist-cap-").unwrap();
        let chatty = fake_tool(workspace.path(), "chatty", "head -c 100000 /dev/zero\n");
        let mut spec = spec_for(chatty, &[]);
        spec.max_output_bytes = 1_024;
        let outcome = SandboxedRunner.run(&spec).unwrap();
        assert!(outcome.truncated);
        assert!(outcome.stdout.len() <= 1_024);
    }

    #[test]
    fn retries_rerun_a_transient_failure() {
        let workspace = tempfile::TempDir::with_prefix("dg-specialist-retry-").unwrap();
        let counter = workspace.path().join("count");
        let flaky = fake_tool(
            workspace.path(),
            "flaky",
            &format!(
                "n=$(cat {} 2>/dev/null || echo 0); n=$((n+1)); echo $n > {}; [ $n -ge 3 ]\n",
                counter.display(),
                counter.display()
            ),
        );
        let mut spec = spec_for(flaky, &[]);
        spec.retries = 3;
        let outcome = SandboxedRunner.run(&spec).unwrap();
        assert_eq!(outcome.exit_code, 0);
        assert_eq!(std::fs::read_to_string(&counter).unwrap().trim(), "3");
    }

    #[test]
    fn only_a_provable_outcome_counts_as_confirmed() {
        let good = RunOutcome {
            exit_code: 0,
            stdout: b"Deleted volumes: vol-1".to_vec(),
            stderr: Vec::new(),
            timed_out: false,
            truncated: false,
        };
        assert_eq!(
            verify_specialist_result(&good, "Deleted volumes"),
            SpecialistVerdict::Confirmed
        );
        let mut failed = good.clone();
        failed.exit_code = 1;
        assert!(matches!(
            verify_specialist_result(&failed, "Deleted volumes"),
            SpecialistVerdict::NeedsAttention { .. }
        ));
        let mut cut = good.clone();
        cut.truncated = true;
        assert!(matches!(
            verify_specialist_result(&cut, "Deleted volumes"),
            SpecialistVerdict::NeedsAttention { reason } if reason.contains("truncated")
        ));
        let mut silent = good;
        silent.stdout.clear();
        assert!(matches!(
            verify_specialist_result(&silent, "Deleted volumes"),
            SpecialistVerdict::NeedsAttention { .. }
        ));
    }

    #[test]
    fn an_active_build_is_detected_by_the_held_lock_not_by_the_file() {
        let workspace = tempfile::TempDir::with_prefix("dg-specialist-cargo-").unwrap();
        let project = workspace.path().join("proj");
        std::fs::create_dir_all(project.join("target/debug")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(project.join("target/debug/blob"), vec![0_u8; 4096]).unwrap();
        std::fs::write(project.join("target/debug/.cargo-lock"), b"").unwrap();

        // The lock file persists after a build, so an idle target is idle
        // even with the file present.
        let idle = cargo_inventory(&project).unwrap();
        assert!(
            !idle.active_build,
            "an uncontended lock file is not a build"
        );

        // Holding the advisory lock is the real in-progress signal.
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let held = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(project.join("target/debug/.cargo-lock"))
                .unwrap();
            assert_eq!(
                unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                0
            );
            let busy = cargo_inventory(&project).unwrap();
            assert!(busy.active_build, "a held lock marks a live build");
            assert!(busy.notes.iter().any(|note| note.contains("wait")));
            unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_UN) };
        }

        let inventory = cargo_inventory(&project).unwrap();
        assert_eq!(inventory.objects.len(), 1);
        assert_eq!(inventory.objects[0].kind, "cargo-target");
        assert_eq!(inventory.objects[0].path, project.join("target"));
        assert!(inventory.objects[0].bytes >= 4096);
    }

    #[test]
    fn cargo_inventory_refuses_a_directory_that_is_not_a_cargo_project() {
        let workspace = tempfile::TempDir::with_prefix("dg-specialist-nocargo-").unwrap();
        assert!(matches!(
            cargo_inventory(workspace.path()),
            Err(OpsError::Stale(message)) if message.contains("Cargo.toml")
        ));
    }

    #[test]
    fn a_shared_cargo_target_dir_is_reported_but_never_planned() {
        let workspace = tempfile::TempDir::with_prefix("dg-specialist-shared-").unwrap();
        let project = workspace.path().join("proj");
        std::fs::create_dir_all(project.join("target")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "[package]\n").unwrap();
        let shared = workspace.path().join("shared-target");
        std::fs::create_dir_all(&shared).unwrap();
        let inventory =
            cargo_inventory_with_env(&project, Some(&shared.to_string_lossy())).unwrap();
        assert_eq!(inventory.objects.len(), 1);
        assert_eq!(inventory.objects[0].path, project.join("target"));
        assert!(
            inventory
                .notes
                .iter()
                .any(|note| note.contains("never planned")),
            "a shared build directory is a note, never an object"
        );
    }
}
