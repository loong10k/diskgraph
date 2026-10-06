//! CLI binary acceptance (P2 task 3.12, specs CMD-01 / CMD-02): real process
//! runs over an isolated data directory, JSON on stdout, business exit codes.

use std::process::Command;

use tempfile::TempDir;

struct Run {
    stdout: String,
    stderr: String,
    code: u8,
}

fn run_cli(data_dir: &std::path::Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_diskgraph"))
        .arg("--data-dir")
        .arg(data_dir)
        .arg("--json")
        .args(args)
        .env_remove("RUST_BACKTRACE")
        .output()
        .expect("cli binary must run");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(1) as u8,
    }
}

fn json_field(line: &str, key: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(line).expect("stdout line must be JSON");
    value
        .pointer(key)
        .map(|field| field.to_string().trim_matches('"').to_owned())
        .unwrap_or_else(|| panic!("missing {key} in {line}"))
}

#[test]
fn serve_accepts_remote_authentication_configuration_before_dispatch() {
    let workspace = TempDir::with_prefix("diskgraph-serve-cli-").unwrap();
    let run = run_cli(
        workspace.path(),
        &[
            "serve",
            "--transport",
            "streamable-http",
            "--auth",
            "issuer",
            "audience",
            "fixture-key",
            "--profile",
            "invalid-profile",
        ],
    );
    assert_eq!(run.code, 2);
    assert_eq!(
        json_field(run.stdout.trim(), "/error/code"),
        "invalid_argument",
        "serve should reach business validation rather than reject --auth: {}",
        run.stderr
    );
}

#[test]
fn full_readonly_flow_with_business_exit_codes() {
    let workspace = TempDir::with_prefix("diskgraph-cli-").unwrap();
    let data_dir = workspace.path().join("data");
    let root = workspace.path().join("project");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(root.join("target").join("artifact.bin"), vec![0; 256]).unwrap();

    // C01 scope add
    let run = run_cli(
        &data_dir,
        &["scope", "add", "--root", root.to_str().unwrap()],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let scope_id = json_field(run.stdout.trim(), "/data/scope_id");
    assert!(scope_id.starts_with("scope-"));

    // Envelope contract: api_version and ok are always present.
    let first_line = run.stdout.lines().next().unwrap();
    assert_eq!(json_field(first_line, "/api_version"), "2");
    assert_eq!(json_field(first_line, "/ok"), "true");

    // C02 index --wait
    let run = run_cli(&data_dir, &["index", "--scope", &scope_id, "--wait"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let revision = json_field(run.stdout.trim(), "/data/revision_id");
    assert!(revision.starts_with("rev-"));

    // C05 snapshots
    let run = run_cli(&data_dir, &["snapshots", "--scope", &scope_id]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("\"snapshot_id\""));

    // C10 node (root facts)
    let run = run_cli(&data_dir, &["node", "--scope", &scope_id]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("\"coverage\""));

    // C11 children of the root
    let run = run_cli(
        &data_dir,
        &["children", "--scope", &scope_id, "--limit", "10"],
    );
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("Cargo.toml") && run.stdout.contains("target"));

    // C16 candidates: review queue is empty without explicit evidence, and
    // the response never claims safety.
    let run = run_cli(&data_dir, &["candidates", "--scope", &scope_id]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("\"review_only\":true"));

    // C14 explain on the cargo project entity chain: find the target node id
    // through children, then explain its resource entity.
    let children = run_cli(
        &data_dir,
        &["children", "--scope", &scope_id, "--limit", "10"],
    );
    let children_json: serde_json::Value =
        serde_json::from_str(children.stdout.trim()).expect("children must be JSON");
    let target_id = children_json["data"]["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|node| node["name"] == "target")
        .map(|node| node["id"].to_string())
        .expect("children must include the target directory");
    let run = run_cli(
        &data_dir,
        &[
            "explain",
            "--scope",
            &scope_id,
            "--revision",
            &revision,
            "--entity",
            &format!("resource-{target_id}"),
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("owned_by_project"));
    assert!(run.stdout.contains("rebuildable_by"));

    // CMD-02 exit codes: an unindexed scope reports 4.
    let run = run_cli(&data_dir, &["node", "--scope", "scope-does-not-exist"]);
    assert_eq!(run.code, 4, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("not_indexed") || run.stdout.contains("not_found"));

    // Unknown jobs also report 4.
    let run = run_cli(&data_dir, &["status", "--job", "job-none"]);
    assert_eq!(run.code, 4);
}

#[test]
fn synchronous_budget_failure_never_returns_a_success_envelope() {
    let directory = TempDir::new().unwrap();
    let root = directory.path().join("project");
    std::fs::create_dir(&root).unwrap();
    for index in 0..120 {
        std::fs::write(root.join(index.to_string()), "data").unwrap();
    }
    let data = directory.path().join("data");
    let registered = run_cli(&data, &["scope", "add", "--root", root.to_str().unwrap()]);
    assert_eq!(registered.code, 0, "{}", registered.stderr);
    let scope = json_field(registered.stdout.trim(), "/data/scope_id");
    let result = run_cli(
        &data,
        &[
            "--max-nodes-per-scan",
            "100",
            "index",
            "--scope",
            &scope,
            "--wait",
        ],
    );
    assert_eq!(result.code, 7, "{} {}", result.stdout, result.stderr);
    assert_eq!(
        json_field(result.stdout.trim(), "/error/code"),
        "budget_exceeded"
    );
    assert_eq!(json_field(result.stdout.trim(), "/ok"), "false");
    let status = run_cli(&data, &["node", "--scope", &scope]);
    assert_eq!(
        status.code, 4,
        "partial scan must not publish: {}",
        status.stdout
    );
}

#[test]
fn positive_candidate_cli_does_not_decode_an_unrelated_corrupt_node() {
    let workspace = TempDir::with_prefix("diskgraph-cli-candidate-").unwrap();
    let data = workspace.path().join("data");
    let root = workspace.path().join("project");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='candidate'\n").unwrap();
    std::fs::write(root.join("target/bin"), vec![0; 4096]).unwrap();
    let added = run_cli(&data, &["scope", "add", "--root", root.to_str().unwrap()]);
    assert_eq!(added.code, 0);
    let scope = json_field(added.stdout.trim(), "/data/scope_id");
    assert_eq!(
        run_cli(&data, &["index", "--scope", &scope, "--wait"]).code,
        0
    );

    let database = rusqlite::Connection::open(data.join("diskgraph.sqlite")).unwrap();
    let (snapshot, target): (String, i64) = database
        .query_row(
            "SELECT snapshot_id, id FROM nodes WHERE name = 'target'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    database
        .execute(
            "INSERT INTO evidence (snapshot_id, node_id, evidence_json) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                snapshot,
                target,
                serde_json::json!({
                    "node_id": target, "relation": "rebuildable", "subject": "fixture",
                    "source": "test", "observed_at_unix_ms": 1, "confidence": 100,
                })
                .to_string()
            ],
        )
        .unwrap();
    database
        .execute(
            "UPDATE nodes SET kind = 'invalid-kind' WHERE name = 'bin'",
            [],
        )
        .unwrap();
    for args in [
        vec!["node", "--scope", &scope],
        vec!["top", "--scope", &scope],
        vec!["children", "--scope", &scope],
        vec!["explore", "--scope", &scope],
    ] {
        let query = run_cli(&data, &args);
        assert_eq!(query.code, 0, "{args:?}: {}", query.stderr);
    }
    let selected = run_cli(
        &data,
        &[
            "candidates",
            "--scope",
            &scope,
            "--target-bytes",
            "18446744073709551615",
        ],
    );
    assert_eq!(
        selected.code, 0,
        "stdout: {}\nstderr: {}",
        selected.stdout, selected.stderr
    );
    let output: serde_json::Value = serde_json::from_str(selected.stdout.trim()).unwrap();
    assert_eq!(output["data"]["review_only"], true);
    assert_eq!(output["data"]["complete"], true);
    assert_eq!(output["data"]["candidates"][0]["node"]["name"], "target");
    assert!(
        output["data"]["remaining_bytes"]
            .as_str()
            .is_some_and(|value| value != "0")
    );
}

#[test]
fn scope_add_is_idempotent_and_remove_revokes() {
    let workspace = TempDir::with_prefix("diskgraph-cli-scope-").unwrap();
    let data_dir = workspace.path().join("data");
    let root = workspace.path();

    let first = run_cli(
        &data_dir,
        &["scope", "add", "--root", root.to_str().unwrap()],
    );
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let second = run_cli(
        &data_dir,
        &["scope", "add", "--root", root.to_str().unwrap()],
    );
    assert_eq!(second.code, 0);
    assert_eq!(
        json_field(first.stdout.trim(), "/data/scope_id"),
        json_field(second.stdout.trim(), "/data/scope_id"),
        "re-adding the same root must return the same scope"
    );

    let scope_id = json_field(first.stdout.trim(), "/data/scope_id");
    let run = run_cli(&data_dir, &["scope", "remove", "--scope", &scope_id]);
    assert_eq!(run.code, 0);
    let run = run_cli(&data_dir, &["scope", "show", "--scope", &scope_id]);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("\"revoked\":true"));

    // Jobs for revoked scopes are refused with a conflict exit code (9).
    let run = run_cli(&data_dir, &["index", "--scope", &scope_id]);
    assert_eq!(run.code, 9, "stderr: {}", run.stderr);
}

#[test]
fn top_growth_changes_report_bounded_and_honest_results() {
    let workspace = TempDir::with_prefix("diskgraph-cli-cgc-").unwrap();
    let data_dir = workspace.path().join("data");
    let root = workspace.path().join("proj");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    std::fs::write(root.join("target").join("a.bin"), vec![0; 128]).unwrap();

    let run = run_cli(
        &data_dir,
        &["scope", "add", "--root", root.to_str().unwrap()],
    );
    let scope_id = json_field(run.stdout.trim(), "/data/scope_id");
    let run = run_cli(&data_dir, &["index", "--scope", &scope_id, "--wait"]);
    let revision_one = json_field(run.stdout.trim(), "/data/revision_id");

    // C12 top: bounded largest children with an explicit size kind.
    let run = run_cli(&data_dir, &["top", "--scope", &scope_id, "--limit", "5"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("\"size_kind\":\"allocated\""));

    // C06 changes between identical revisions reports zero differences.
    let run = run_cli(
        &data_dir,
        &[
            "changes",
            "--scope",
            &scope_id,
            "--before",
            &revision_one,
            "--after",
            &revision_one,
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("\"added\":0"));
    assert!(run.stdout.contains("\"removed\":0"));

    // C07 growth between the same revision is comparable and zero.
    let run = run_cli(
        &data_dir,
        &[
            "growth",
            "--scope",
            &scope_id,
            "--before",
            &revision_one,
            "--after",
            &revision_one,
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("\"comparable\":true"));

    // A second scan after growing a file reports the change honestly. The
    // file must cross allocation-block boundaries: 128 B and 4 KiB occupy the
    // same allocated blocks, so the allocated size kind would (correctly)
    // report no change.
    std::fs::write(root.join("target").join("a.bin"), vec![0; 1024 * 1024]).unwrap();
    let run = run_cli(&data_dir, &["sync", "--scope", &scope_id, "--wait"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let revision_two = json_field(run.stdout.trim(), "/data/revision_id");
    let run = run_cli(
        &data_dir,
        &[
            "changes",
            "--scope",
            &scope_id,
            "--before",
            &revision_one,
            "--after",
            &revision_two,
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let report: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    let size_changed = report["data"]["size_changed"].as_u64().unwrap();
    assert!(
        size_changed >= 1,
        "the grown file must surface as a size change: {report}"
    );
    assert_eq!(report["data"]["incompatible"], serde_json::Value::Null);
}

#[test]
fn search_explore_and_impact_report_bounded_results() {
    let workspace = TempDir::with_prefix("diskgraph-cli-sei-").unwrap();
    let data_dir = workspace.path().join("data");
    let root = workspace.path().join("proj");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    std::fs::write(root.join("target").join("artifact.bin"), vec![0; 512]).unwrap();

    let run = run_cli(
        &data_dir,
        &["scope", "add", "--root", root.to_str().unwrap()],
    );
    let scope_id = json_field(run.stdout.trim(), "/data/scope_id");
    let run = run_cli(&data_dir, &["index", "--scope", &scope_id, "--wait"]);
    let revision = json_field(run.stdout.trim(), "/data/revision_id");

    // C09 search: bounded matches with a resumable cursor.
    let run = run_cli(
        &data_dir,
        &[
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            "t",
            "--limit",
            "1",
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let page: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(page["data"]["items"].as_array().unwrap().len(), 1);
    let cursor = page["data"]["next_cursor"]
        .as_str()
        .expect("a first page must offer a cursor");
    // The cursor resumes in the same context.
    let run = run_cli(
        &data_dir,
        &[
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            "t",
            "--limit",
            "1",
            "--cursor",
            cursor,
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let second: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_ne!(
        second["data"]["items"][0]["id"], page["data"]["items"][0]["id"],
        "the second page must not repeat the first"
    );

    // A cursor issued for another pattern is refused (exit 2, invalid_argument).
    let run = run_cli(
        &data_dir,
        &[
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            "Cargo",
            "--limit",
            "1",
            "--cursor",
            cursor,
        ],
    );
    assert_eq!(run.code, 2, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("invalid_argument"));

    // A corrupt cursor is refused rather than silently restarting.
    let run = run_cli(
        &data_dir,
        &[
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            "t",
            "--cursor",
            "!!!bogus!!!",
        ],
    );
    assert_eq!(run.code, 2);

    // C08 explore: bounded children plus coverage and truncation flags.
    let run = run_cli(
        &data_dir,
        &["explore", "--scope", &scope_id, "--max-nodes", "1"],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let summary: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(summary["data"]["children"].as_array().unwrap().len(), 1);
    assert_eq!(summary["data"]["truncated"], "node_limit");

    // C15 impact: typed reachability that explicitly grants no execution.
    let run = run_cli(
        &data_dir,
        &[
            "impact",
            "--scope",
            &scope_id,
            "--revision",
            &revision,
            "--entity",
            "resource-1",
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("\"grants_execution\":false"));
}

#[test]
fn unimplemented_families_report_unsupported_not_unknown_command() {
    let workspace = TempDir::with_prefix("diskgraph-cli-unsup-").unwrap();
    let data_dir = workspace.path().join("data");

    // Each family that exists in the catalog but is not enabled in this build
    // must resolve to a real command with the unsupported business result
    // (exit 6), never a clap "unknown command" parse error (exit 2). C27
    // (serve), C28 (install), and C29 (doctor) now ship, so they are absent.
    for args in [
        vec!["duplicates"],
        vec!["read", "--scope", "scope-1"],
        vec!["move", "--scope", "scope-1"],
        vec!["trash", "--scope", "scope-1"],
        vec!["purge", "--scope", "scope-1"],
        vec!["apply", "--scope", "scope-1"],
    ] {
        let run = run_cli(&data_dir, &args);
        assert_eq!(
            run.code, 6,
            "{args:?} must report unsupported, stderr: {}",
            run.stderr
        );
        assert!(
            run.stdout.contains("\"code\":\"unsupported\""),
            "{args:?} must carry the unsupported business code: {}",
            run.stdout
        );
    }
}

#[test]
fn children_reports_unknown_sizes_and_honours_filters() {
    let workspace = TempDir::with_prefix("diskgraph-cli-flt-").unwrap();
    let data_dir = workspace.path().join("data");
    let root = workspace.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("big.bin"), vec![0; 100_000]).unwrap();
    std::fs::write(root.join("small.bin"), vec![0; 10]).unwrap();

    let run = run_cli(
        &data_dir,
        &["scope", "add", "--root", root.to_str().unwrap()],
    );
    let scope_id = json_field(run.stdout.trim(), "/data/scope_id");
    let run = run_cli(&data_dir, &["index", "--scope", &scope_id, "--wait"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // Default listing sorts by size and reports the unknown count.
    let run = run_cli(&data_dir, &["children", "--scope", &scope_id]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let page: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(page["data"]["items"][0]["name"], "big.bin");
    assert_eq!(page["data"]["unknown_size_count"], 0);

    // A size threshold filters the listing. The threshold is above one
    // allocation block, so the small file (one 4 KiB block) is excluded while
    // the 100 KB file is kept.
    let run = run_cli(
        &data_dir,
        &["children", "--scope", &scope_id, "--min-bytes", "50000"],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let filtered: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(filtered["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["data"]["items"][0]["name"], "big.bin");

    // Contradictory filters are an argument error, not an empty success.
    let run = run_cli(
        &data_dir,
        &[
            "children",
            "--scope",
            &scope_id,
            "--min-bytes",
            "1",
            "--unknown-only",
        ],
    );
    assert_eq!(run.code, 2, "stderr: {}", run.stderr);
}

#[test]
fn changes_and_growth_refuse_incomparable_revisions_honestly() {
    let workspace = TempDir::with_prefix("diskgraph-cli-incmp-").unwrap();
    let data_dir = workspace.path().join("data");
    let first_root = workspace.path().join("a");
    let second_root = workspace.path().join("b");
    std::fs::create_dir_all(&first_root).unwrap();
    std::fs::create_dir_all(&second_root).unwrap();
    std::fs::write(first_root.join("f.bin"), vec![0; 64]).unwrap();
    std::fs::write(second_root.join("g.bin"), vec![0; 64]).unwrap();

    let run = run_cli(
        &data_dir,
        &["scope", "add", "--root", first_root.to_str().unwrap()],
    );
    let scope_a = json_field(run.stdout.trim(), "/data/scope_id");
    let run = run_cli(
        &data_dir,
        &["scope", "add", "--root", second_root.to_str().unwrap()],
    );
    let scope_b = json_field(run.stdout.trim(), "/data/scope_id");
    let run = run_cli(&data_dir, &["index", "--scope", &scope_a, "--wait"]);
    let revision_a = json_field(run.stdout.trim(), "/data/revision_id");
    let run = run_cli(&data_dir, &["index", "--scope", &scope_b, "--wait"]);
    let revision_b = json_field(run.stdout.trim(), "/data/revision_id");

    // 单 scope 命令不能用 A 的提示替代 B revision 的实际归属。
    let run = run_cli(
        &data_dir,
        &[
            "changes",
            "--scope",
            &scope_a,
            "--before",
            &revision_a,
            "--after",
            &revision_b,
        ],
    );
    assert_eq!(run.code, 3, "stderr: {}", run.stderr);
    let report: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(report["error"]["code"], "permission_denied");
    assert!(report.get("data").is_none());

    // growth 对双方实际 scope 采用同样的检查。
    let run = run_cli(
        &data_dir,
        &[
            "growth",
            "--scope",
            &scope_a,
            "--before",
            &revision_a,
            "--after",
            &revision_b,
        ],
    );
    assert_eq!(run.code, 3, "stderr: {}", run.stderr);
    let report: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(report["error"]["code"], "permission_denied");
    assert!(report.get("data").is_none());

    // 双 scope 比较分别授权双方，仍可报告真实路径差异。
    let run = run_cli(
        &data_dir,
        &["compare", "--from", &revision_a, "--to", &revision_b],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let report: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(report["data"]["left"]["revision_id"], revision_a);
    assert_eq!(report["data"]["right"]["revision_id"], revision_b);
    assert_eq!(report["data"]["complete"], true);

    // Same revision against itself: comparable and no difference.
    let run = run_cli(
        &data_dir,
        &[
            "changes",
            "--scope",
            &scope_a,
            "--before",
            &revision_a,
            "--after",
            &revision_a,
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("\"incompatible\":null"));
}

#[test]
fn doctor_and_install_are_delivered_and_stay_reversible() {
    let workspace = TempDir::with_prefix("diskgraph-cli-di-").unwrap();
    let data_dir = workspace.path().join("data");
    let config = workspace.path().join("mcp.json");

    // C29 doctor: read-only diagnostics that report health as data.
    let run = run_cli(&data_dir, &["doctor"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let report: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    assert_eq!(report["data"]["healthy"], true);
    assert!(!report["data"]["checks"].as_array().unwrap().is_empty());

    // C28 install: preview does not write; --apply-config writes once.
    let run = run_cli(
        &data_dir,
        &[
            "install",
            "add",
            "--client",
            "json",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        !config.exists(),
        "a preview must not write the configuration"
    );

    let run = run_cli(
        &data_dir,
        &[
            "install",
            "add",
            "--client",
            "json",
            "--config",
            config.to_str().unwrap(),
            "--apply-config",
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(config.exists());
    assert!(
        std::fs::read_to_string(&config)
            .unwrap()
            .contains("diskgraph-mcp")
    );

    // An unknown client is refused before any file access.
    let run = run_cli(
        &data_dir,
        &[
            "install",
            "add",
            "--client",
            "unknown-client",
            "--config",
            config.to_str().unwrap(),
            "--apply-config",
        ],
    );
    assert_ne!(run.code, 0);
}

#[test]
fn cursors_bind_principal_and_policy_epoch() {
    let workspace = TempDir::with_prefix("diskgraph-cli-cursor-").unwrap();
    let data_dir = workspace.path().join("data");
    let root = workspace.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for name in ["alpha.txt", "beta.txt", "gamma.txt"] {
        std::fs::write(root.join(name), vec![0; 64]).unwrap();
    }
    let run = run_cli(
        &data_dir,
        &["scope", "add", "--root", root.to_str().unwrap()],
    );
    let scope_id = json_field(run.stdout.trim(), "/data/scope_id");
    let run = run_cli(&data_dir, &["index", "--scope", &scope_id, "--wait"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // Page one under the default principal; the offered cursor binds that
    // principal, scope, revision, filter, and the current policy epoch.
    let first = run_cli(
        &data_dir,
        &[
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            ".txt",
            "--limit",
            "1",
        ],
    );
    assert_eq!(first.code, 0, "stderr: {}", first.stderr);
    let page: serde_json::Value = serde_json::from_str(first.stdout.trim()).unwrap();
    let cursor = page["data"]["next_cursor"].as_str().unwrap().to_owned();
    assert!(!cursor.is_empty());

    // A different principal cannot ride the cursor: the principal binding is
    // verified before any page content moves, so the refusal is an argument
    // error rather than a silent page from someone else's context (P4-5.9).
    let as_bob = run_cli(
        &data_dir,
        &[
            "--principal",
            "bob",
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            ".txt",
            "--limit",
            "1",
            "--cursor",
            &cursor,
        ],
    );
    assert_eq!(as_bob.code, 2, "stderr: {}", as_bob.stderr);
    assert!(as_bob.stdout.contains("invalid_argument"));

    // A policy bump expires the old epoch: the same principal's old cursor is
    // refused even though its grants were re-issued at the new version.
    let run = run_cli(&data_dir, &["policy", "publish", "--version", "2"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let stale = run_cli(
        &data_dir,
        &[
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            ".txt",
            "--limit",
            "1",
            "--cursor",
            &cursor,
        ],
    );
    assert_eq!(stale.code, 2, "stale cursor must be an argument error");
    assert!(stale.stdout.contains("invalid_argument"));

    // Under the new epoch a fresh page works, and its cursor is valid again.
    let fresh = run_cli(
        &data_dir,
        &[
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            ".txt",
            "--limit",
            "1",
        ],
    );
    assert_eq!(fresh.code, 0, "stderr: {}", fresh.stderr);
    let fresh_page: serde_json::Value = serde_json::from_str(fresh.stdout.trim()).unwrap();
    let fresh_cursor = fresh_page["data"]["next_cursor"].as_str().unwrap();
    let resumed = run_cli(
        &data_dir,
        &[
            "search",
            "--scope",
            &scope_id,
            "--pattern",
            ".txt",
            "--limit",
            "1",
            "--cursor",
            fresh_cursor,
        ],
    );
    assert_eq!(resumed.code, 0, "stderr: {}", resumed.stderr);
}

#[test]
fn policy_revocation_denies_everything_until_republish() {
    let workspace = TempDir::with_prefix("diskgraph-cli-revoke-").unwrap();
    let data_dir = workspace.path().join("data");
    let root = workspace.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("f"), vec![0; 8]).unwrap();
    let run = run_cli(
        &data_dir,
        &["scope", "add", "--root", root.to_str().unwrap()],
    );
    let scope_id = json_field(run.stdout.trim(), "/data/scope_id");
    run_cli(&data_dir, &["index", "--scope", &scope_id, "--wait"]);

    // Queries work, then revocation denies them; the bootstrap on the next
    // invocation re-issues grants but the revoked flag still denies.
    let run = run_cli(&data_dir, &["children", "--scope", &scope_id]);
    assert_eq!(run.code, 0);
    let run = run_cli(&data_dir, &["policy", "revoke"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let denied = run_cli(&data_dir, &["children", "--scope", &scope_id]);
    assert_eq!(denied.code, 3, "stderr: {}", denied.stderr);
    assert!(denied.stdout.contains("permission_denied"));

    // Republishing restores service.
    let run = run_cli(&data_dir, &["policy", "publish", "--version", "3"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let run = run_cli(&data_dir, &["children", "--scope", &scope_id]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
}

#[test]
fn du_summarizes_multiple_paths_like_du_sh() {
    let workspace = TempDir::with_prefix("diskgraph-du-").unwrap();
    let data_dir = workspace.path().join("data");
    let big = workspace.path().join("big");
    let small = workspace.path().join("small");
    std::fs::create_dir_all(big.join("nested")).unwrap();
    std::fs::create_dir_all(&small).unwrap();
    std::fs::write(big.join("nested").join("payload.bin"), vec![0; 4096]).unwrap();
    std::fs::write(small.join("tiny.txt"), vec![0; 64]).unwrap();

    let run = run_cli(
        &data_dir,
        &[
            "du",
            big.to_str().unwrap(),
            small.to_str().unwrap(),
            "--total",
        ],
    );
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    // Under --json the envelope carries one sizes object keyed by path.
    let envelope: serde_json::Value = serde_json::from_str(run.stdout.trim()).unwrap();
    let sizes = envelope
        .pointer("/data/sizes")
        .unwrap()
        .as_object()
        .unwrap();
    assert_eq!(
        sizes.len(),
        2,
        "both paths measured; stderr: {}",
        run.stderr
    );
    let mut observed = sizes
        .values()
        .map(|entry| entry["bytes"].as_u64().unwrap())
        .collect::<Vec<_>>();
    observed.sort_unstable();
    assert_eq!(
        envelope.pointer("/data/total_bytes").unwrap().as_u64(),
        Some(observed.iter().sum()),
        "total covers both indexed paths"
    );
    // The filesystem reports allocation differently by platform; the large
    // payload and the small payload must still be represented honestly.
    assert!(observed[0] >= 64 && observed[1] >= 4096);
}

#[test]
fn du_reports_missing_paths_and_still_measures_the_rest() {
    let workspace = TempDir::with_prefix("diskgraph-du-missing-").unwrap();
    let data_dir = workspace.path().join("data");
    let ghost = workspace.path().join("ghost");
    let real = workspace.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("f.bin"), vec![0; 128]).unwrap();

    let run = run_cli(
        &data_dir,
        &["du", ghost.to_str().unwrap(), real.to_str().unwrap()],
    );
    // du keeps measuring the paths it can and reports the ones it cannot.
    assert!(
        run.stderr.contains("cannot access"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stdout.contains("real"),
        "the surviving path is still measured"
    );
}

#[test]
fn snapshots_prune_previews_then_removes_only_old_history() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let added = run_cli(&data, &["scope", "add", "--root", root.to_str().unwrap()]);
    assert_eq!(added.code, 0, "{}", added.stderr);
    let scope = json_field(&added.stdout, "/data/scope_id");
    for index in 0..3 {
        std::fs::write(root.join(format!("file-{index}")), [0]).unwrap();
        let indexed = run_cli(&data, &["index", "--scope", &scope, "--wait"]);
        assert_eq!(indexed.code, 0, "{}", indexed.stderr);
    }
    let preview = run_cli(
        &data,
        &["snapshots", "prune", "--scope", &scope, "--keep-last", "1"],
    );
    assert_eq!(preview.code, 0, "{}", preview.stderr);
    assert_eq!(json_field(&preview.stdout, "/data/applied"), "false");
    let history = run_cli(&data, &["snapshots", "--scope", &scope]);
    let value: serde_json::Value = serde_json::from_str(&history.stdout).unwrap();
    assert_eq!(value["data"]["snapshots"].as_array().unwrap().len(), 3);
    let applied = run_cli(
        &data,
        &[
            "snapshots",
            "prune",
            "--scope",
            &scope,
            "--keep-last",
            "1",
            "--apply",
        ],
    );
    assert_eq!(applied.code, 0, "{}", applied.stderr);
    let history = run_cli(&data, &["snapshots", "--scope", &scope]);
    let value: serde_json::Value = serde_json::from_str(&history.stdout).unwrap();
    assert_eq!(value["data"]["snapshots"].as_array().unwrap().len(), 1);
}
