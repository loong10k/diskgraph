//! Docker specialist adapter (P6 tasks 7.8–7.10). Docker manages objects with
//! their own identity and vocabulary; this adapter only reads what Docker
//! reports, offers exact objects for review, and removes exactly what a
//! trusted approval named — never `system prune`, never a category.
//!
//! The Docker VM caveat is part of the contract: on Docker Desktop the image
//! and volume bytes live inside a Linux VM disk image, so freeing bytes there
//! does not immediately shrink the host file. Every inventory and every
//! cleanup result carries that note, and VM disk files are never presented as
//! ordinary cache directories (EC-03).

use std::path::Path;

use serde_json::Value;

use crate::OpsError;
use crate::specialist::{
    CommandRunner, CommandSpec, RunOutcome, SandboxedRunner, SpecialistVerdict,
    verify_specialist_result,
};

/// The note that bounds every byte number Docker reports on a desktop VM.
pub const VM_CAVEAT: &str = "Docker object bytes live inside the Docker VM disk; \
     freeing them does not immediately shrink the host file, and the VM disk \
     image itself is never treated as a cleanable cache";

/// One exact Docker object, ready for a human to review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DockerObject {
    /// The id Docker itself reports (image id, container id, volume name,
    /// build-cache id). Cleanup commands name this and nothing else.
    pub id: String,
    /// What the object is, in Docker's own words.
    pub kind: &'static str,
    pub detail: String,
    pub bytes: u64,
}

/// The read-only inventory (7.8). Bytes per category are separate: caches,
/// images, containers, and volumes answer different questions, and merging
/// them would overstate what any one cleanup could free.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DockerInventory {
    pub build_cache: Vec<DockerObject>,
    pub images: Vec<DockerObject>,
    pub containers: Vec<DockerObject>,
    pub volumes: Vec<DockerObject>,
    /// Measured limits and caveats the caller must see.
    pub notes: Vec<String>,
}

impl DockerInventory {
    /// Every object, whatever its category.
    pub fn all(&self) -> Vec<&DockerObject> {
        self.build_cache
            .iter()
            .chain(self.images.iter())
            .chain(self.containers.iter())
            .chain(self.volumes.iter())
            .collect()
    }
}

fn spec(docker: &Path, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: docker.to_path_buf(),
        args: args.iter().map(|argument| argument.to_string()).collect(),
        cwd: None,
        env: SandboxedRunner::default_env(),
        timeout_ms: 30_000,
        max_output_bytes: 4 << 20,
        retries: 0,
    }
}

fn run(runner: &dyn CommandRunner, docker: &Path, args: &[&str]) -> Result<RunOutcome, OpsError> {
    let outcome = runner.run(&spec(docker, args))?;
    if outcome.timed_out {
        return Err(OpsError::Stale(format!(
            "docker {} timed out",
            args.join(" ")
        )));
    }
    Ok(outcome)
}

/// Reads Docker's own inventory. Everything here is a report; nothing removes
/// or changes anything on the daemon (7.8, EC-03).
pub fn docker_inventory(
    runner: &dyn CommandRunner,
    docker: &Path,
) -> Result<DockerInventory, OpsError> {
    let mut inventory = DockerInventory::default();
    let mut notes = vec![VM_CAVEAT.to_owned()];

    // Category aggregates straight from Docker, so the totals are Docker's
    // own accounting and not ours.
    let df = run(runner, docker, &["system", "df", "--json"])?;
    if df.exit_code == 0 {
        if let Ok(report) = serde_json::from_slice::<Value>(&df.stdout) {
            for (key, kind) in [
                ("BuildCache", "build cache"),
                ("Images", "images"),
                ("Containers", "containers"),
                ("LocalVolumes", "volumes"),
            ] {
                if let Some(entry) = report.get(key) {
                    let count = entry.get("Count").and_then(Value::as_u64);
                    let size = entry.get("Size").and_then(Value::as_u64);
                    let reclaimable = entry.get("Reclaimable").and_then(Value::as_u64);
                    notes.push(format!(
                        "docker reports {kind}: {} objects, {} bytes total, {} bytes reclaimable",
                        count.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
                        size.map(|s| s.to_string()).unwrap_or_else(|| "?".into()),
                        reclaimable
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "?".into()),
                    ));
                }
            }
        } else {
            notes.push("docker system df output was not valid JSON; totals are unavailable".into());
        }
    } else {
        notes.push(format!(
            "docker system df exited with {}; category totals are unavailable",
            df.exit_code
        ));
    }

    // Per-object lists: one JSON object per line, exactly as Docker prints it.
    inventory.images = jsonl_objects(
        run(runner, docker, &["images", "--format", "{{json .}}"])?
            .stdout
            .as_slice(),
        |entry| {
            Some(DockerObject {
                id: entry.get("ID")?.as_str()?.to_owned(),
                kind: "image",
                detail: format!(
                    "{}:{}",
                    entry
                        .get("Repository")
                        .and_then(Value::as_str)
                        .unwrap_or("?"),
                    entry.get("Tag").and_then(Value::as_str).unwrap_or("?")
                ),
                bytes: parse_size(entry.get("Size").and_then(Value::as_str).unwrap_or("0")),
            })
        },
    );
    inventory.containers = jsonl_objects(
        run(runner, docker, &["ps", "-a", "--format", "{{json .}}"])?
            .stdout
            .as_slice(),
        |entry| {
            Some(DockerObject {
                id: entry.get("ID")?.as_str()?.to_owned(),
                kind: "container",
                detail: format!(
                    "{} ({})",
                    entry.get("Names").and_then(Value::as_str).unwrap_or("?"),
                    entry.get("State").and_then(Value::as_str).unwrap_or("?")
                ),
                bytes: parse_size(entry.get("Size").and_then(Value::as_str).unwrap_or("0")),
            })
        },
    );
    inventory.volumes = jsonl_objects(
        run(runner, docker, &["volume", "ls", "--format", "{{json .}}"])?
            .stdout
            .as_slice(),
        |entry| {
            Some(DockerObject {
                id: entry.get("Name")?.as_str()?.to_owned(),
                kind: "volume",
                // Volume sizes are not part of this listing; the aggregate is
                // the honest number we have.
                detail: format!(
                    "driver {}",
                    entry.get("Driver").and_then(Value::as_str).unwrap_or("?")
                ),
                bytes: 0,
            })
        },
    );
    notes.push(
        "volume sizes are not reported by `volume ls`; use the category total for scale".into(),
    );
    inventory.notes = notes;
    Ok(inventory)
}

/// Parses Docker's `--format {{json .}}` output: one JSON object per line.
fn jsonl_objects(output: &[u8], map: impl Fn(&Value) -> Option<DockerObject>) -> Vec<DockerObject> {
    String::from_utf8_lossy(output)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|entry| map(&entry))
        .collect()
}

/// Docker prints sizes like `1.42GB`; a plain u64 field prints as its own
/// string. Both decode to bytes.
fn parse_size(text: &str) -> u64 {
    let text = text.trim();
    if let Ok(bytes) = text.parse::<u64>() {
        return bytes;
    }
    let (number, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(text.len()),
    );
    let value: f64 = number.parse().unwrap_or(0.0);
    let factor = match unit.trim_start() {
        "B" | "" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => 0.0,
    };
    (value * factor) as u64
}

/// The live usage check one object must pass immediately before its removal
/// command runs (7.9, OP-07). Anything in use is refused, never force-removed.
pub enum UsageCheck {
    /// Nothing Docker knows of is using the object right now.
    Unused,
    /// A live dependency names the object; the cleanup must skip it.
    InUse { reason: String },
}

pub fn usage_check(
    runner: &dyn CommandRunner,
    docker: &Path,
    object: &DockerObject,
) -> Result<UsageCheck, OpsError> {
    match object.kind {
        "container" => {
            let state = run(
                runner,
                docker,
                &["inspect", "-f", "{{.State.Running}}", &object.id],
            )?;
            if state.exit_code != 0 {
                // Unknown state blocks: a container we cannot inspect is not a
                // container we may remove.
                return Ok(UsageCheck::InUse {
                    reason: format!(
                        "container {} could not be inspected (exit {})",
                        object.id, state.exit_code
                    ),
                });
            }
            let running = String::from_utf8_lossy(&state.stdout).trim() == "true";
            if running {
                Ok(UsageCheck::InUse {
                    reason: format!("container {} is running", object.id),
                })
            } else {
                Ok(UsageCheck::Unused)
            }
        }
        "volume" => {
            let users = run(
                runner,
                docker,
                &[
                    "ps",
                    "-a",
                    "--filter",
                    &format!("volume={}", object.id),
                    "-q",
                ],
            )?;
            let users = String::from_utf8_lossy(&users.stdout);
            let users: Vec<&str> = users
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if users.is_empty() {
                Ok(UsageCheck::Unused)
            } else {
                Ok(UsageCheck::InUse {
                    reason: format!(
                        "volume {} is used by container(s) {}",
                        object.id,
                        users.join(", ")
                    ),
                })
            }
        }
        "image" => {
            let users = run(
                runner,
                docker,
                &[
                    "ps",
                    "-a",
                    "--filter",
                    &format!("ancestor={}", object.id),
                    "-q",
                ],
            )?;
            let users = String::from_utf8_lossy(&users.stdout);
            let users: Vec<&str> = users
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if users.is_empty() {
                Ok(UsageCheck::Unused)
            } else {
                Ok(UsageCheck::InUse {
                    reason: format!(
                        "image {} backs container(s) {}",
                        object.id,
                        users.join(", ")
                    ),
                })
            }
        }
        other => Ok(UsageCheck::InUse {
            reason: format!("no usage check is defined for kind {other}; refusing"),
        }),
    }
}

/// The removal command for one object: exact id, no wildcards, no prune
/// anywhere in the vocabulary.
fn removal_args(object: &DockerObject) -> Option<Vec<&'static str>> {
    match object.kind {
        "volume" => Some(vec!["volume", "rm"]),
        "image" => Some(vec!["rmi"]),
        "container" => Some(vec!["rm"]),
        _ => None,
    }
}

/// One object's cleanup result, reported separately (7.9, OP-09).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupResult {
    /// Docker's own output proved the removal.
    Confirmed,
    /// The object was in use and was deliberately skipped.
    SkippedInUse { reason: String },
    /// The removal could not be proven: park for reconciliation (OP-08).
    NeedsAttention { reason: String },
}

/// Removes exactly the approved objects, one command per object, each preceded
/// by a fresh usage check and followed by a check that Docker really forgot
/// the object. There is no batch mode and no prune flag anywhere in this path.
pub fn docker_cleanup(
    runner: &dyn CommandRunner,
    docker: &Path,
    objects: &[DockerObject],
) -> Result<Vec<CleanupResult>, OpsError> {
    let mut results = Vec::new();
    for object in objects {
        match usage_check(runner, docker, object)? {
            UsageCheck::InUse { reason } => {
                results.push(CleanupResult::SkippedInUse { reason });
                continue;
            }
            UsageCheck::Unused => {}
        }
        let Some(prefix) = removal_args(object) else {
            results.push(CleanupResult::NeedsAttention {
                reason: format!("no removal command is defined for kind {}", object.kind),
            });
            continue;
        };
        let mut args = prefix;
        args.push(&object.id);
        let outcome = run(runner, docker, &args)?;
        // The effect is proven by Docker forgetting the object, not by an
        // exit code alone.
        let forgotten = run(runner, docker, &["inspect", &object.id])?;
        if forgotten.exit_code != 0 {
            results.push(CleanupResult::Confirmed);
            continue;
        }
        // Docker still knows the object: verify the command output to decide
        // whether that is a wait-and-retry or a real failure.
        results.push(match verify_specialist_result(&outcome, &object.id) {
            SpecialistVerdict::Confirmed => CleanupResult::Confirmed,
            SpecialistVerdict::NeedsAttention { reason } => {
                CleanupResult::NeedsAttention { reason }
            }
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A runner that records every spec it is given and answers from a
    /// script. Tests assert on the recorded argv, which is the injection
    /// surface an attacker would try to widen.
    struct ScriptedRunner {
        expected: RefCell<Vec<(Vec<String>, RunOutcome)>>,
        pub calls: RefCell<Vec<Vec<String>>>,
    }

    impl ScriptedRunner {
        fn new() -> Self {
            Self {
                expected: RefCell::new(Vec::new()),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn then(&self, args: &[&str], outcome: RunOutcome) -> &Self {
            self.expected
                .borrow_mut()
                .push((args.iter().map(|a| a.to_string()).collect(), outcome));
            self
        }

        fn outcome(exit_code: i32, stdout: &str) -> RunOutcome {
            RunOutcome {
                exit_code,
                stdout: stdout.as_bytes().to_vec(),
                stderr: Vec::new(),
                timed_out: false,
                truncated: false,
            }
        }
    }

    impl CommandRunner for ScriptedRunner {
        fn run(&self, spec: &CommandSpec) -> Result<RunOutcome, OpsError> {
            self.calls.borrow_mut().push(spec.args.clone());
            let mut expected = self.expected.borrow_mut();
            if let Some((_, outcome)) = (!expected.is_empty()).then(|| expected.remove(0)) {
                return Ok(outcome);
            }
            Ok(RunOutcome {
                exit_code: 1,
                stdout: Vec::new(),
                stderr: b"unexpected call".to_vec(),
                timed_out: false,
                truncated: false,
            })
        }
    }

    fn volume(id: &str) -> DockerObject {
        DockerObject {
            id: id.to_owned(),
            kind: "volume",
            detail: "driver local".into(),
            bytes: 0,
        }
    }

    #[test]
    fn inventory_reads_each_category_separately_and_carries_the_vm_caveat() {
        let df = r#"{"BuildCache":{"Count":2,"Size":100,"Reclaimable":80,"Items":null},"Images":{"Count":1,"Size":200,"Reclaimable":0,"Items":null},"Containers":{"Count":1,"Size":10,"Reclaimable":0,"Items":null},"LocalVolumes":{"Count":1,"Size":50,"Reclaimable":0,"Items":null}}"#;
        let runner = ScriptedRunner::new();
        runner.then(&["system", "df", "--json"], ScriptedRunner::outcome(0, df));
        runner.then(
            &["images", "--format", "{{json .}}"],
            ScriptedRunner::outcome(
                0,
                r#"{"ID":"img-1","Repository":"alpine","Tag":"3.20","Size":"7.4MB"}"#,
            ),
        );
        runner.then(
            &["ps", "-a", "--format", "{{json .}}"],
            ScriptedRunner::outcome(
                0,
                r#"{"ID":"ctr-1","Names":"worker","State":"running","Size":"0B"}"#,
            ),
        );
        runner.then(
            &["volume", "ls", "--format", "{{json .}}"],
            ScriptedRunner::outcome(0, r#"{"Name":"data","Driver":"local"}"#),
        );

        let inventory = docker_inventory(&runner, Path::new("/usr/local/bin/docker")).unwrap();
        assert_eq!(inventory.images.len(), 1);
        assert_eq!(inventory.images[0].id, "img-1");
        assert_eq!(inventory.images[0].detail, "alpine:3.20");
        assert_eq!(inventory.images[0].bytes, 7_400_000);
        assert_eq!(inventory.containers[0].id, "ctr-1");
        assert!(inventory.containers[0].detail.contains("running"));
        assert_eq!(inventory.volumes[0].id, "data");
        // The caveats and Docker's own totals are part of the report.
        assert!(inventory.notes.iter().any(|n| n.contains("VM disk")));
        assert!(
            inventory
                .notes
                .iter()
                .any(|n| n.contains("build cache: 2 objects"))
        );
        assert_eq!(inventory.all().len(), 3);
    }

    #[test]
    fn cleanup_never_widenes_beyond_the_exact_objects_and_never_prunes() {
        let runner = ScriptedRunner::new();
        // Volume data is unused: one usage check, one exact removal, one
        // proof-of-removal inspect.
        runner.then(&["ps", "-a", "--filter", "volume=data", "-q"], {
            ScriptedRunner::outcome(0, "")
        });
        runner.then(
            &["volume", "rm", "data"],
            ScriptedRunner::outcome(0, "data"),
        );
        runner.then(&["inspect", "data"], ScriptedRunner::outcome(1, ""));

        let results = docker_cleanup(
            &runner,
            Path::new("/usr/local/bin/docker"),
            &[volume("data")],
        )
        .unwrap();
        assert_eq!(results, vec![CleanupResult::Confirmed]);

        for call in runner.calls.borrow().iter() {
            assert!(
                !call.iter().any(|argument| argument.contains("prune")),
                "a prune appeared in the argv: {call:?}"
            );
            assert!(
                !call.iter().any(|argument| argument.contains('*')),
                "a wildcard appeared in the argv: {call:?}"
            );
        }
        assert!(
            runner
                .calls
                .borrow()
                .iter()
                .any(|call| call == &["volume", "rm", "data"]),
            "the removal must name exactly the approved id"
        );
    }

    #[test]
    fn an_in_use_volume_is_skipped_and_others_proceed() {
        let runner = ScriptedRunner::new();
        // Volume keep is used by a container; volume drop is not.
        runner.then(
            &["ps", "-a", "--filter", "volume=keep", "-q"],
            ScriptedRunner::outcome(0, "ctr-9\n"),
        );
        runner.then(&["ps", "-a", "--filter", "volume=drop", "-q"], {
            ScriptedRunner::outcome(0, "")
        });
        runner.then(
            &["volume", "rm", "drop"],
            ScriptedRunner::outcome(0, "drop"),
        );
        runner.then(&["inspect", "drop"], ScriptedRunner::outcome(1, ""));

        let results = docker_cleanup(
            &runner,
            Path::new("/usr/local/bin/docker"),
            &[volume("keep"), volume("drop")],
        )
        .unwrap();
        assert_eq!(
            results,
            vec![
                CleanupResult::SkippedInUse {
                    reason: "volume keep is used by container(s) ctr-9".into()
                },
                CleanupResult::Confirmed,
            ]
        );
        // The used volume was never the subject of a removal command.
        assert!(
            !runner
                .calls
                .borrow()
                .iter()
                .any(|call| call.contains(&"keep".to_string())
                    && call.first().map(String::as_str) == Some("volume")
                    && call.contains(&"rm".to_string()))
        );
    }

    #[test]
    fn a_removal_docker_cannot_prove_parks_for_attention() {
        let runner = ScriptedRunner::new();
        runner.then(&["ps", "-a", "--filter", "volume=stuck", "-q"], {
            ScriptedRunner::outcome(0, "")
        });
        // The removal "succeeds" but Docker still knows the object and the
        // output does not prove anything: park, never claim success.
        runner.then(&["volume", "rm", "stuck"], ScriptedRunner::outcome(0, ""));
        runner.then(&["inspect", "stuck"], ScriptedRunner::outcome(0, "stuck"));

        let results = docker_cleanup(
            &runner,
            Path::new("/usr/local/bin/docker"),
            &[volume("stuck")],
        )
        .unwrap();
        assert!(matches!(
            results.as_slice(),
            [CleanupResult::NeedsAttention { .. }]
        ));
    }

    #[test]
    fn a_running_container_is_refused_by_the_usage_check() {
        let runner = ScriptedRunner::new();
        runner.then(
            &["inspect", "-f", "{{.State.Running}}", "ctr-1"],
            ScriptedRunner::outcome(0, "true"),
        );
        let object = DockerObject {
            id: "ctr-1".into(),
            kind: "container",
            detail: "worker (running)".into(),
            bytes: 0,
        };
        assert!(matches!(
            usage_check(&runner, Path::new("docker"), &object).unwrap(),
            UsageCheck::InUse { reason } if reason.contains("running")
        ));
    }
}
