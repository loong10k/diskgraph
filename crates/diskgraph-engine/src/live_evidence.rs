//! Live-evidence collectors (P7 tasks 8.6–8.8, EV-06, EC-02, EV-03). Unlike
//! the deterministic manifest collectors, these observe the *live* system:
//! which process holds a file open, what a local Git repository says about
//! itself, and what changed in a watched tree between two samples.
//!
//! The one rule that shapes everything here: an observation that could not
//! be made says so. A collector that could not look NEVER reports "nobody
//! is using this" — it reports unobservable (EV-06), and a repository
//! without an upstream never reports "pushed" (EV-02).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

// ------------------------------------------------------------------ 8.6 ---

/// How much of the system the sample could actually see.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UsageCoverage {
    /// The probe ran and answered for every named path.
    Full,
    /// The probe ran but could not see everything (partial permissions).
    Partial { reason: String },
    /// The probe could not run or could not answer at all.
    Unobservable { reason: String },
}

/// A process that holds one of the sampled paths open.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ProcessHolder {
    pub pid: u32,
    pub command: String,
}

/// One sample of who is using the named paths, with the visibility caveat
/// attached. Only `Full` coverage with an empty holder list means "nobody
/// is using it"; any other coverage means "unknown", whatever the list
/// holds (EV-06).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UsageSample {
    pub sampled_at_unix_ms: u64,
    pub coverage: UsageCoverage,
    pub holders: Vec<ProcessHolder>,
}

impl UsageSample {
    /// The honest review phrasing: never "safe to remove".
    pub fn verdict(&self) -> String {
        match &self.coverage {
            UsageCoverage::Full if self.holders.is_empty() => {
                "no open handles were visible to the probe".into()
            }
            UsageCoverage::Full => format!("{} process(es) hold open handles", self.holders.len()),
            UsageCoverage::Partial { reason } => {
                format!("the probe saw only part of the system: {reason}")
            }
            UsageCoverage::Unobservable { reason } => {
                format!("usage is unobservable: {reason}")
            }
        }
    }
}

/// Samples open handles for exact paths with `lsof`. The program is fixed,
/// the arguments are structured, there is no shell, and the probe is
/// bounded in time and output. A missing or failing lsof is an
/// `Unobservable` sample, never an empty one.
pub fn sample_process_usage(lsof: &Path, paths: &[&Path]) -> UsageSample {
    let sampled_at_unix_ms = now_ms();
    if paths.is_empty() {
        return UsageSample {
            sampled_at_unix_ms,
            coverage: UsageCoverage::Full,
            holders: Vec::new(),
        };
    }
    let mut command = Command::new(lsof);
    // -w: suppress warnings; -F pcn0: machine-readable pid/command/name
    // records, NUL-terminated; `--`: end of options, so a path that begins
    // with `-` is still a path.
    command.args(["-w", "-F", "pcn0", "--"]);
    command.args(paths);
    command.env_clear();
    if let Some(path_env) = std::env::var_os("PATH") {
        command.env("PATH", path_env);
    }
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdin(Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return UsageSample {
                sampled_at_unix_ms,
                coverage: UsageCoverage::Unobservable {
                    reason: format!("the handle probe could not run: {error}"),
                },
                holders: Vec::new(),
            };
        }
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                return UsageSample {
                    sampled_at_unix_ms,
                    coverage: UsageCoverage::Unobservable {
                        reason: format!("the handle probe failed: {error}"),
                    },
                    holders: Vec::new(),
                };
            }
        }
    }
    let mut output = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut output);
    }
    let exit = child
        .try_wait()
        .ok()
        .flatten()
        .and_then(|status| status.code());
    if timed_out {
        return UsageSample {
            sampled_at_unix_ms,
            coverage: UsageCoverage::Unobservable {
                reason: "the handle probe timed out".into(),
            },
            holders: Vec::new(),
        };
    }
    if exit != Some(0) {
        // lsof exits 1 when it found nothing at all — which is a real
        // answer, not a failure; any other code is a probe problem.
        if exit == Some(1) && output.is_empty() {
            return UsageSample {
                sampled_at_unix_ms,
                coverage: UsageCoverage::Full,
                holders: Vec::new(),
            };
        }
        return UsageSample {
            sampled_at_unix_ms,
            coverage: UsageCoverage::Unobservable {
                reason: format!("the handle probe exited with {exit:?}"),
            },
            holders: Vec::new(),
        };
    }
    // Parse `p<pid>\0c<comm>\0n<path>\0` records into holders.
    let text = String::from_utf8_lossy(&output);
    let mut holders: Vec<ProcessHolder> = Vec::new();
    let mut current_pid: Option<u32> = None;
    let mut current_command: Option<String> = None;
    // lsof prints the kernel's view of a path (on macOS `/var` is
    // `/private/var`) and marks deleted-but-open files with a suffix, so
    // matching canonicalizes what was asked for and strips the marker.
    let canonical_asked: Vec<PathBuf> = paths
        .iter()
        .map(|asked| asked.canonicalize().unwrap_or_else(|_| asked.to_path_buf()))
        .collect();
    for record in text.split('\0') {
        let record = record.trim_start_matches('\n');
        if record.is_empty() {
            continue;
        }
        let (tag, value) = record.split_at(1);
        match tag {
            "p" => {
                current_pid = value.parse().ok();
                current_command = None;
            }
            "c" => current_command = Some(value.to_owned()),
            "n" => {
                let reported = value.strip_suffix(" (deleted)").unwrap_or(value);
                // Only paths that were asked for count as holders.
                if let (Some(pid), Some(command_name)) = (current_pid, current_command.clone())
                    && canonical_asked
                        .iter()
                        .any(|asked| asked.to_string_lossy() == reported)
                {
                    holders.push(ProcessHolder {
                        pid,
                        command: command_name,
                    });
                }
            }
            _ => {}
        }
    }
    holders.sort();
    holders.dedup();
    UsageSample {
        sampled_at_unix_ms,
        coverage: UsageCoverage::Full,
        holders,
    }
}

// ------------------------------------------------------------------ 8.7 ---

/// What a local Git repository says about itself, sampled without any
/// network access (EC-02, EV-02): dirty state, stashes, and the ahead/behind
/// counts against the configured upstream — which stay `None` (unknown)
/// when there is no upstream, because "no upstream" is not "pushed".
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitSample {
    pub head: Option<String>,
    pub dirty_count: u64,
    pub stash_count: u64,
    pub ahead_of_upstream: Option<u64>,
    pub behind_upstream: Option<u64>,
    pub notes: Vec<String>,
    pub sampled_at_unix_ms: u64,
}

/// Samples one repository with the local `git` binary. Everything runs with
/// the project as cwd and a minimal environment; no command here talks to
/// the network.
pub fn sample_git(git: &Path, project: &Path) -> Result<GitSample, String> {
    let run = |args: &[&str]| -> Result<String, String> {
        let output = Command::new(git)
            .args(args)
            .current_dir(project)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|error| format!("git could not run: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("git failed")
                .to_owned());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };

    let head = run(&["rev-parse", "HEAD"]).ok();
    if head.is_none() {
        // Distinguish "not a repository" from other failures.
        run(&["rev-parse", "--is-inside-work-tree"])
            .map_err(|reason| format!("not a usable git repository: {reason}"))?;
        return Ok(GitSample {
            head: None,
            dirty_count: 0,
            stash_count: 0,
            ahead_of_upstream: None,
            behind_upstream: None,
            notes: vec!["the repository has no commits yet".into()],
            sampled_at_unix_ms: now_ms(),
        });
    }
    let head = head.map(|text| text.trim().to_owned());
    let dirty_count = run(&["status", "--porcelain"])?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u64;
    let stash_count = run(&["stash", "list"])?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u64;
    let mut notes = Vec::new();
    let mut ahead = None;
    let mut behind = None;
    match run(&[
        "rev-parse",
        "--abbrev-ref",
        "--symbolic-full-name",
        "@{upstream}",
    ]) {
        Ok(upstream) => {
            let upstream = upstream.trim().to_owned();
            ahead = run(&["rev-list", "--count", &format!("{upstream}..HEAD")])?
                .trim()
                .parse()
                .ok();
            behind = run(&["rev-list", "--count", &format!("HEAD..{upstream}")])?
                .trim()
                .parse()
                .ok();
        }
        Err(_) => {
            notes.push(
                "no upstream is configured: whether local commits are pushed is unknown, \
                 and is never reported as pushed"
                    .into(),
            );
        }
    }
    Ok(GitSample {
        head,
        dirty_count,
        stash_count,
        ahead_of_upstream: ahead,
        behind_upstream: behind,
        notes,
        sampled_at_unix_ms: now_ms(),
    })
}

// ------------------------------------------------------------------ 8.8 ---

/// One observed change in a watched tree. A rename under polling arrives as
/// a `Removed` plus an `Appeared` pair, because that is what the sampling
/// can honestly distinguish (EV-03).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FsEvent {
    pub path: PathBuf,
    pub kind: FsEventKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsEventKind {
    Appeared,
    Modified,
    Removed,
}

/// The diff between two samples of one tree. Polling never drops events
/// (each poll diffs whole-tree snapshots), but a change burst larger than
/// the event budget is an overflow: the report says `rescan_needed` so the
/// caller schedules a controlled rescan instead of trusting a partial view
/// (FS-06).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WatchReport {
    pub events: Vec<FsEvent>,
    pub overflow: bool,
    pub rescan_needed: bool,
}

/// The caller-held snapshot of one watched tree: path → (length, mtime ns).
pub type WatchSnapshot = BTreeMap<PathBuf, (u64, i64)>;

/// Takes a fresh snapshot of `root` and diffs it against `previous`, which
/// is updated in place. Metadata unavailable to the sampler is recorded as
/// zeros, so a stat failure surfaces as a change rather than silence.
pub fn poll_changes(root: &Path, previous: &mut WatchSnapshot, max_events: usize) -> WatchReport {
    let mut fresh: WatchSnapshot = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                stack.push(path);
                continue;
            }
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos() as i64)
                .unwrap_or(0);
            fresh.insert(path, (metadata.len(), mtime));
        }
    }
    let mut events = Vec::new();
    for (path, state) in &fresh {
        match previous.get(path) {
            None => events.push(FsEvent {
                path: path.clone(),
                kind: FsEventKind::Appeared,
            }),
            Some(old) if old != state => events.push(FsEvent {
                path: path.clone(),
                kind: FsEventKind::Modified,
            }),
            Some(_) => {}
        }
    }
    for path in previous.keys() {
        if !fresh.contains_key(path) {
            events.push(FsEvent {
                path: path.clone(),
                kind: FsEventKind::Removed,
            });
        }
    }
    *previous = fresh;
    let overflow = events.len() > max_events;
    WatchReport {
        events,
        overflow,
        rescan_needed: overflow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[cfg(unix)]
    #[test]
    fn the_usage_sample_sees_the_test_process_own_open_file() {
        let workspace = tempfile::TempDir::with_prefix("dg-live-usage-").unwrap();
        let path = workspace.path().join("held.bin");
        std::fs::write(&path, b"data").unwrap();
        // Keep the handle open for the duration of the sample.
        let _held = File::open(&path).unwrap();
        let sample = sample_process_usage(Path::new("/usr/sbin/lsof"), &[&path]);
        assert_eq!(sample.coverage, UsageCoverage::Full, "{sample:?}");
        assert!(
            !sample.holders.is_empty(),
            "this process holds the file open; the probe must see it"
        );
        assert!(sample.verdict().contains("hold open handles"));
    }

    #[test]
    fn a_missing_probe_is_unobservable_never_nobody() {
        let workspace = tempfile::TempDir::with_prefix("dg-live-unobs-").unwrap();
        let path = workspace.path().join("f.bin");
        std::fs::write(&path, b"data").unwrap();
        let sample = sample_process_usage(Path::new("/nonexistent/lsof"), &[&path]);
        assert!(matches!(
            sample.coverage,
            UsageCoverage::Unobservable { .. }
        ));
        assert!(
            !sample.verdict().contains("no open handles"),
            "an unobservable probe must never read as nobody"
        );
    }

    #[test]
    fn git_samples_dirty_stash_and_upstream_honestly() {
        let git = PathBuf::from("/usr/bin/git");
        if !git.is_file() {
            // The drill needs a git binary; say so rather than fake it.
            panic!("no /usr/bin/git on this host");
        }
        let workspace = tempfile::TempDir::with_prefix("dg-live-git-").unwrap();
        let project = workspace.path().join("repo");
        std::fs::create_dir_all(&project).unwrap();
        let config = ["-c", "user.email=d@example", "-c", "user.name=d"];
        let run = |args: &[&str]| {
            let mut all: Vec<&str> = config.to_vec();
            all.extend_from_slice(args);
            let output = Command::new(&git)
                .args(&all)
                .current_dir(&project)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "-q"]);
        // Pin the branch name: newer gits default to whatever the host
        // config says, and the upstream wiring below names this branch.
        run(&["symbolic-ref", "HEAD", "refs/heads/main"]);
        // A pristine repository: no commits, so the sample reports exactly
        // that instead of pretending a HEAD exists.
        let pristine = sample_git(&git, &project).unwrap();
        assert!(pristine.head.is_none());

        std::fs::write(project.join("a.txt"), b"one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "one"]);
        // With no upstream configured, ahead/behind are unknown — and the
        // note says so, never "pushed" (EV-02).
        let sample = sample_git(&git, &project).unwrap();
        assert!(sample.head.is_some());
        assert_eq!(sample.dirty_count, 0);
        assert_eq!(sample.ahead_of_upstream, None);
        assert!(
            sample
                .notes
                .iter()
                .any(|note| note.contains("never reported as pushed"))
        );

        // A dirty working tree counts.
        std::fs::write(project.join("a.txt"), b"two\n").unwrap();
        let sample = sample_git(&git, &project).unwrap();
        assert_eq!(sample.dirty_count, 1);
        run(&["stash", "push", "-q"]);
        let sample = sample_git(&git, &project).unwrap();
        assert_eq!(sample.dirty_count, 0);
        assert_eq!(sample.stash_count, 1);
        run(&["stash", "pop", "-q"]);

        // Point the branch's upstream at a local ref: fully offline.
        run(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        run(&["config", "remote.origin.url", "."]);
        run(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ]);
        run(&["config", "branch.main.remote", "origin"]);
        run(&["config", "branch.main.merge", "refs/heads/main"]);
        std::fs::write(project.join("a.txt"), b"three\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "two"]);
        let sample = sample_git(&git, &project).unwrap();
        assert_eq!(sample.ahead_of_upstream, Some(1));
        assert_eq!(sample.behind_upstream, Some(0));
    }

    #[test]
    fn the_polling_watcher_reports_added_modified_removed_and_overflow() {
        let workspace = tempfile::TempDir::with_prefix("dg-live-watch-").unwrap();
        let root = workspace.path().join("tree");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("base.txt"), b"0").unwrap();

        let mut snapshot: WatchSnapshot = BTreeMap::new();
        // The baseline poll establishes the snapshot; its events describe
        // the initial population.
        let baseline = poll_changes(&root, &mut snapshot, 100);
        assert!(
            baseline
                .events
                .iter()
                .any(|event| event.kind == FsEventKind::Appeared)
        );
        assert!(!baseline.rescan_needed);

        // A modification, visible on its own poll.
        std::fs::write(root.join("base.txt"), b"changed").unwrap();
        let report = poll_changes(&root, &mut snapshot, 100);
        assert!(
            report
                .events
                .iter()
                .any(|event| event.kind == FsEventKind::Modified)
        );
        assert!(!report.rescan_needed);

        // An appearance and a removal share the next poll. A rename arrives
        // as exactly this pair: the sampling can honestly tell no better.
        std::fs::write(root.join("new.txt"), b"n").unwrap();
        std::fs::remove_file(root.join("base.txt")).unwrap();
        let report = poll_changes(&root, &mut snapshot, 100);
        assert!(
            report
                .events
                .iter()
                .any(|event| event.kind == FsEventKind::Appeared)
        );
        assert!(
            report
                .events
                .iter()
                .any(|event| event.kind == FsEventKind::Removed)
        );
        assert!(!report.rescan_needed);

        // A burst past the event budget is an overflow that asks for a
        // controlled rescan rather than a trusted partial view.
        for index in 0..5 {
            std::fs::write(root.join(format!("burst-{index}.txt")), b"x").unwrap();
        }
        let report = poll_changes(&root, &mut snapshot, 2);
        assert!(report.overflow);
        assert!(report.rescan_needed);
    }
}
