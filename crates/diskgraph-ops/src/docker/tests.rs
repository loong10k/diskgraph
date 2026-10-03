use crate::OpsError;
use crate::specialist::{CommandRunner, CommandSpec, RunOutcome};
use std::path::Path;

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
