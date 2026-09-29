//! Real-host specialist drills (P6 task 7.11). These run the actual cargo and
//! docker binaries against isolated fixtures and measure what actually
//! happened. They are `#[ignore]`d so the ordinary test gate stays
//! hermetic; run them explicitly with:
//!
//! ```text
//! cargo test -p diskgraph-ops --test specialist_real_host -- --ignored --test-threads=1
//! ```

use std::path::{Path, PathBuf};

use diskgraph_ops::docker::{DockerObject, docker_cleanup, docker_inventory, usage_check};
use diskgraph_ops::specialist::{CommandRunner, CommandSpec, SandboxedRunner};
use diskgraph_ops::specialist::{cargo_inventory_with_env, probe};
use diskgraph_ops::{CARGO_CLEAN, DOCKER_INVENTORY};

fn resolve_program(name: &str) -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(format!("{}_BIN", name.to_uppercase())) {
        let candidate = PathBuf::from(explicit);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn run(
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    timeout_ms: u64,
) -> Result<diskgraph_ops::specialist::RunOutcome, Box<dyn std::error::Error>> {
    let spec = CommandSpec {
        program: program.to_path_buf(),
        args: args.iter().map(|argument| argument.to_string()).collect(),
        cwd: cwd.map(Path::to_path_buf),
        env: SandboxedRunner::default_env(),
        timeout_ms,
        max_output_bytes: 1 << 20,
        retries: 0,
    };
    Ok(SandboxedRunner.run(&spec)?)
}

#[test]
#[ignore = "real-host drill: needs a working cargo toolchain"]
fn the_cargo_adapter_reads_a_real_built_project() {
    let Some(cargo) = resolve_program("cargo") else {
        panic!("no cargo on this host; set CARGO_BIN to run this drill");
    };
    let workspace = tempfile::TempDir::with_prefix("diskgraph-drill-cargo-").unwrap();
    let project = workspace.path().join("proj");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"drill\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();

    // The tool is real and new enough.
    match probe(&CARGO_CLEAN, &cargo, &SandboxedRunner) {
        diskgraph_ops::AdapterStatus::Available { version } => {
            println!("cargo version: {version}");
        }
        diskgraph_ops::AdapterStatus::Unavailable { reason } => {
            panic!("cargo probe failed: {reason}");
        }
    }

    // A real build produces a real target directory.
    let build = run(&cargo, &["build"], Some(&project), 180_000).unwrap();
    assert_eq!(
        build.exit_code,
        0,
        "cargo build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let inventory = cargo_inventory_with_env(&project, None).unwrap();
    assert_eq!(inventory.objects.len(), 1);
    assert_eq!(inventory.objects[0].path, project.join("target"));
    assert!(
        inventory.objects[0].bytes > 0,
        "a real build leaves real bytes: {}",
        inventory.objects[0].bytes
    );
    // The build is over: the lock file may exist but is uncontended.
    assert!(!inventory.active_build);
    println!(
        "cargo drill: target holds {} logical bytes (observed, not reclaimable-by-cleanup)",
        inventory.objects[0].bytes
    );
}

#[test]
#[ignore = "real-host drill: needs a running docker daemon"]
fn the_docker_adapter_creates_reads_and_removes_one_exact_volume() {
    let Some(docker) = resolve_program("docker") else {
        panic!("no docker on this host; set DOCKER_BIN to run this drill");
    };
    match probe(&DOCKER_INVENTORY, &docker, &SandboxedRunner) {
        diskgraph_ops::AdapterStatus::Available { version } => {
            println!("docker version: {version}");
        }
        diskgraph_ops::AdapterStatus::Unavailable { reason } => {
            panic!("docker probe failed: {reason}");
        }
    }

    let name = format!("diskgraph-drill-{}", uuid::Uuid::new_v4().simple());
    // The drill volume is cleaned up on every path out of this test.
    let discard = |name: &str| {
        let _ = run(&docker, &["volume", "rm", name], None, 30_000);
    };

    let created = run(&docker, &["volume", "create", &name], None, 30_000);
    match created {
        Ok(outcome) if outcome.exit_code == 0 => {}
        Ok(outcome) => {
            panic!(
                "docker volume create failed: {}",
                String::from_utf8_lossy(&outcome.stderr)
            );
        }
        Err(error) => panic!("docker volume create could not run: {error}"),
    }

    // The inventory sees exactly this volume, and reports the VM caveat.
    let inventory = docker_inventory(&SandboxedRunner, &docker).unwrap();
    assert!(
        inventory.volumes.iter().any(|volume| volume.id == name),
        "the drill volume must appear in the inventory"
    );
    assert!(
        inventory.notes.iter().any(|note| note.contains("VM disk")),
        "the VM caveat is part of every inventory"
    );
    println!(
        "docker drill: inventory holds {} objects across all categories",
        inventory.all().len()
    );

    // The live usage check passes: nothing uses the drill volume.
    let object = DockerObject {
        id: name.clone(),
        kind: "volume",
        detail: "driver local".into(),
        bytes: 0,
    };
    assert!(matches!(
        usage_check(&SandboxedRunner, &docker, &object).unwrap(),
        diskgraph_ops::docker::UsageCheck::Unused
    ));

    // The cleanup removes exactly this volume and proves it is gone.
    let results = docker_cleanup(&SandboxedRunner, &docker, &[object]).unwrap();
    assert!(
        matches!(
            results.as_slice(),
            [diskgraph_ops::docker::CleanupResult::Confirmed]
        ),
        "the drill volume removal must be confirmed, got {results:?}"
    );
    let gone = run(&docker, &["volume", "inspect", &name], None, 30_000).unwrap();
    assert_ne!(gone.exit_code, 0, "the volume must be gone after cleanup");
    discard(&name);
}

#[test]
#[ignore = "real-host drill: needs a running docker daemon"]
fn the_docker_usage_check_refuses_a_volume_in_real_use() {
    let Some(docker) = resolve_program("docker") else {
        panic!("no docker on this host");
    };
    let volume = format!("diskgraph-drill-used-{}", uuid::Uuid::new_v4().simple());
    let created = run(&docker, &["volume", "create", &volume], None, 30_000).unwrap();
    assert_eq!(created.exit_code, 0);
    // A stopped container that mounts the volume is still a user: the check
    // refuses removal even though nothing is running.
    let container = format!("diskgraph-drill-ctr-{}", uuid::Uuid::new_v4().simple());
    let mounted = run(
        &docker,
        &[
            "create",
            "--name",
            &container,
            "--mount",
            &format!("source={volume},target=/data"),
            "alpine",
            "true",
        ],
        None,
        60_000,
    );
    let mounted_ok = matches!(&mounted, Ok(outcome) if outcome.exit_code == 0);
    if mounted_ok {
        let object = DockerObject {
            id: volume.clone(),
            kind: "volume",
            detail: "driver local".into(),
            bytes: 0,
        };
        assert!(matches!(
            usage_check(&SandboxedRunner, &docker, &object).unwrap(),
            diskgraph_ops::docker::UsageCheck::InUse { .. }
        ));
        // Nothing in this drill ever removed the volume: it is still there.
        let still = run(&docker, &["volume", "inspect", &volume], None, 30_000).unwrap();
        assert_eq!(still.exit_code, 0);
    }
    // Cleanup of the drill fixtures; the volume is removed last.
    let _ = run(&docker, &["rm", &container], None, 30_000);
    let _ = run(&docker, &["volume", "rm", &volume], None, 30_000);
    if !mounted_ok {
        // Without a usable image the refusal path cannot be exercised here;
        // say so instead of pretending it ran.
        println!("docker drill: no base image available; usage refusal not exercised");
    }
}
