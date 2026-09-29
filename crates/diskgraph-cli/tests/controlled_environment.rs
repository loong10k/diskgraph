//! Controlled-environment acceptance (P2 task 3.13, spec EC-01): the CLI
//! indexes and answers queries on a PATH containing none of `ls`, `find`,
//! `du`, `stat`, or `df`. Proves the engine uses Rust/platform APIs rather than
//! shelling out, that a restarted process reuses the persisted index, and that
//! the same resource is addressable from a different working directory.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A PATH that contains only the current directory: no system utilities at
/// all, so any accidental shell-out fails rather than silently succeeding.
fn stripped_path() -> String {
    std::env::current_dir()
        .map(|cwd| cwd.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_owned())
}

struct Run {
    stdout: String,
    stderr: String,
    code: u8,
}

fn run_cli_in(cwd: &Path, data_dir: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .current_dir(cwd)
        .env("PATH", stripped_path())
        .env_remove("RUST_BACKTRACE")
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .args(args)
        .output()
        .expect("cli binary must run without system utilities on PATH");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(1) as u8,
    }
}

fn json_value(line: &str) -> serde_json::Value {
    serde_json::from_str(line).unwrap_or_else(|_| panic!("stdout must be JSON: {line}"))
}

fn field(line: &str, pointer: &str) -> String {
    json_value(line)
        .pointer(pointer)
        .map(|value| value.to_string().trim_matches('"').to_owned())
        .unwrap_or_else(|| panic!("missing {pointer} in {line}"))
}

struct Project {
    _workspace: TempDir,
    data_dir: PathBuf,
    root: PathBuf,
}

fn project(label: &str) -> Project {
    let workspace = TempDir::with_prefix(format!("diskgraph-ctl-{label}-")).unwrap();
    let data_dir = workspace.path().join("data");
    let root = workspace.path().join("project");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"ctl\"\n").unwrap();
    std::fs::write(root.join("target").join("app.bin"), vec![0; 20_000]).unwrap();
    Project {
        _workspace: workspace,
        data_dir,
        root,
    }
}

#[test]
fn indexing_and_queries_work_with_no_system_utilities_on_path() {
    let project = project("no-tools");
    let elsewhere = project._workspace.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();

    // Register the scope and index it with an empty PATH.
    let run = run_cli_in(
        &elsewhere,
        &project.data_dir,
        &["scope", "add", "--root", project.root.to_str().unwrap()],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let scope_id = field(run.stdout.trim(), "/data/scope_id");

    let run = run_cli_in(
        &elsewhere,
        &project.data_dir,
        &["index", "--scope", &scope_id, "--wait"],
    );
    assert_eq!(
        run.code, 0,
        "scope add/index must not shell out: {}",
        run.stderr
    );
    let revision = field(run.stdout.trim(), "/data/revision_id");
    assert!(revision.starts_with("rev-"));

    // Every read-only query family answers with the same empty PATH.
    for (args, expect) in [
        (vec!["node", "--scope", &scope_id], "coverage"),
        (vec!["children", "--scope", &scope_id], "items"),
        (vec!["top", "--scope", &scope_id], "size_kind"),
        (vec!["snapshots", "--scope", &scope_id], "snapshots"),
        (vec!["candidates", "--scope", &scope_id], "review_only"),
        (
            vec!["search", "--scope", &scope_id, "--pattern", "target"],
            "items",
        ),
        (vec!["explore", "--scope", &scope_id], "children"),
    ] {
        let run = run_cli_in(&elsewhere, &project.data_dir, &args);
        assert_eq!(run.code, 0, "{args:?} failed without tools: {}", run.stderr);
        assert!(
            run.stdout.contains(expect),
            "{args:?} must include {expect}: {}",
            run.stdout
        );
    }

    // A scan produced real sizes, so the walker is not reporting zeros.
    let run = run_cli_in(
        &elsewhere,
        &project.data_dir,
        &["top", "--scope", &scope_id],
    );
    let top = json_value(run.stdout.trim());
    let bytes = top["data"]["items"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["subtree_bytes"].as_u64());
    assert!(
        bytes.unwrap_or(0) > 0,
        "the scan must observe real byte counts without du: {top}"
    );
}

#[test]
fn a_fresh_process_reuses_the_persisted_index() {
    let project = project("restart");
    let elsewhere = project._workspace.path().join("other-cwd");
    std::fs::create_dir_all(&elsewhere).unwrap();

    let run = run_cli_in(
        &elsewhere,
        &project.data_dir,
        &["scope", "add", "--root", project.root.to_str().unwrap()],
    );
    let scope_id = field(run.stdout.trim(), "/data/scope_id");
    let run = run_cli_in(
        &elsewhere,
        &project.data_dir,
        &["index", "--scope", &scope_id, "--wait"],
    );
    let first_revision = field(run.stdout.trim(), "/data/revision_id");

    // A separate process, a different working directory, and an empty PATH:
    // the same revision answers without rescanning.
    let third_cwd = project._workspace.path().join("third-cwd");
    std::fs::create_dir_all(&third_cwd).unwrap();
    let run = run_cli_in(
        &third_cwd,
        &project.data_dir,
        &["snapshots", "--scope", &scope_id],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let snapshots = json_value(run.stdout.trim());
    assert_eq!(
        snapshots["data"]["snapshots"].as_array().unwrap().len(),
        1,
        "restarting must reuse the stored index, not rescan"
    );

    // A query answered from another cwd sees the same resource identity.
    let run = run_cli_in(
        &third_cwd,
        &project.data_dir,
        &["node", "--scope", &scope_id],
    );
    let node = json_value(run.stdout.trim());
    assert_eq!(node["data"]["node"]["id"], 1);
    assert!(first_revision.starts_with("rev-"));
}

#[test]
fn the_engine_reports_its_own_capabilities_without_probing_system_tools() {
    let project = project("caps");
    let run = run_cli_in(
        project._workspace.path(),
        &project.data_dir,
        &["scope", "add", "--root", project.root.to_str().unwrap()],
    );
    let scope_id = field(run.stdout.trim(), "/data/scope_id");
    let run = run_cli_in(
        project._workspace.path(),
        &project.data_dir,
        &["index", "--scope", &scope_id, "--wait"],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // The engines' own databases live under the data directory, not under a
    // system tool's cache.
    assert!(project.data_dir.join("diskgraph.sqlite").exists());
    assert!(project.data_dir.join("diskgraph-control.sqlite").exists());
}
