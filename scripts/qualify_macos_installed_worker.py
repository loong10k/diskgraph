"""冻结候选的真实root发行/普通UID运行资格；不把未运行夹具当PASS。"""
import argparse
import hashlib
import json
import os
import platform
import subprocess
import tarfile
from pathlib import Path

CANDIDATE = Path("crates/diskgraph-engine/integration_candidates/macos_installed")
PROTOCOL_CASE = "macos_installed_worker_fixture_tests::macos_worker_request_roundtrip_preserves_non_utf8_native_path_without_filesystem"
BUDGET_FIXTURE_CASE = "macos_engine_scan_fixture_tests::macos_response_fixture_admits_terminal_but_rejects_actual_tree_encoding"
ROOT_CASE = "macos_installation_root_fixture_tests::root_fresh_install_update_interruption_and_floor_bound_recovery"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


CLI_SCAN_REGRESSION_CASES = (
    "query_terminal_tests::encoded_tree_refuses_terminal_scope_revocation",
    "query_terminal_tests::plan_cannot_escape_as_usable_steps_when_encoding_terminal_authorization_expires",
    "query_terminal_tests::tree_and_history_do_not_complete_after_the_dispatch_deadline",
    "query_terminal_tests::encoded_comparison_refuses_terminal_grant_revocation",
    "query_terminal_tests::encoded_changes_refuses_terminal_scope_revocation",
    "query_terminal_tests::comparison_summary_is_partial_when_encoded_terminal_authorization_expires",
    "scan_terminal_tests::synchronous_wait_reclaims_only_its_expired_job",
    "query_terminal_tests::html_export_does_not_write_after_terminal_scope_revocation",
    "scan_terminal_tests::waiting_for_another_owners_failed_job_is_not_success",
    "query_terminal_tests::encoded_growth_refuses_terminal_grant_revocation",
    "tui::tests::nested_frame_limits_queries_and_keeps_navigation_visible",
)


def check_cli_regression(stdout):
    """要求完整原CLI测试及11项原失败用例实际运行，不接受过滤/忽略或零用例。"""
    if "test result: ok. 67 passed; 0 failed; 0 ignored;" not in stdout:
        raise RuntimeError("full CLI regression did not execute all 67 original tests")
    for case in CLI_SCAN_REGRESSION_CASES:
        if "test " + case + " ... ok" not in stdout:
            raise RuntimeError("required original CLI scan regression missing: " + case)


MCP_SCAN_REGRESSION_CASES = (
    'history_budget_tests::encoded_socket_history_rechecks_both_scope_sides',
    'history_budget_tests::encoded_socket_history_rechecks_both_grant_sides',
    'history_budget_tests::encoded_socket_growth_rechecks_both_scope_sides',
    'children_cursor_tests::directory_cursor_resumes_in_stable_order_with_offset_compatibility',
    'children_cursor_tests::directory_cursor_rejects_changed_parent_filter_and_policy',
    'history_budget_tests::history_and_growth_use_real_authentication_origin_and_revision_ownership',
    'children_cursor_tests::directory_cursor_resumes_after_response_byte_truncation',
    'history_budget_tests::encoded_socket_growth_rechecks_both_grant_sides',
    'history_budget_tests::history_decode_failure_keeps_a_bounded_business_diagnostic_on_the_socket',
    'history_budget_tests::socket_history_initial_authorization_cannot_reset_deadline',
    'history_budget_tests::socket_growth_initial_authorization_cannot_reset_deadline',
    'http::tests::a_client_side_path_is_never_resolved_against_the_server',
    'http::tests::a_real_socket_round_trip_answers_the_same_as_the_direct_call',
    'http::tests::a_disconnected_client_job_completes_and_stays_queryable',
    'http::tests::mcp_search_cursors_expire_with_policy_updates',
    'http::tests::scope_ids_from_another_server_are_not_found',
    'relation_budget_tests::escaped_impact_fits_the_real_mcp_structured_envelope',
    'relation_budget_tests::initial_mcp_authorization_wait_cannot_reset_the_candidate_deadline',
    'tests::authorization_tests::impact_requires_the_revision_owners_grant_even_when_a_different_scope_is_supplied',
    'relation_budget_tests::final_mcp_envelope_cannot_escape_after_scope_revocation',
    'tests::candidate_tests::incomplete_candidates_do_not_decode_the_full_revision',
    'tests::candidate_tests::zero_target_candidates_do_not_decode_the_full_revision',
    'tests::candidate_tests::positive_target_candidates_use_a_narrow_read_and_report_the_target_gap',
    'tests::protocol_tests::tool_calls_reject_unknown_and_mistyped_arguments',
    'tests::query_tests::explicit_revision_and_node_id_select_historical_data',
    'tests::relation_tests::explain_mixed_direction_byte_pages_never_skip_edges',
    'tests::query_tests::queries_answer_with_envelopes_and_unknown_scopes_refuse_honestly',
    'tests::query_tests::two_scopes_stay_isolated_and_reuse_their_own_revisions',
    'tests::relation_tests::explain_checks_entity_bytes_before_decoding',
    'tests::relation_tests::explain_returns_typed_evidence_for_a_cargo_project',
    'tests::relation_tests::impact_uses_entity_edges_without_decoding_unrelated_relations',
    'tests::relation_tests::relation_queries_bound_decoding_and_report_continuation',
    'http::tests::dropping_every_connection_is_not_a_cancellation',
    'http::tests::legacy_disconnect_leaves_the_job_queryable',
    'http_lifecycle_tests::stop_joins_idle_accept_and_releases_original_listener',
    'http_lifecycle_tests::stop_closes_accepted_incomplete_request_before_long_read_timeout',
    'http_lifecycle_tests::stop_joins_modern_sse_with_client_still_connected',
    'http_lifecycle_tests::stop_joins_legacy_sse_with_client_still_connected',
    'http_lifecycle_tests::foreground_panic_raii_joins_original_server_before_directory_cleanup',
    'tests::job_runner_fixture::tests::idle_runner_is_actually_joined_before_source_owner_returns',
)


def check_mcp_regression(stdout):
    """要求完整MCP回归与原34失败案、真实socket生命周期案实际运行。"""
    if "test result: ok. 159 passed; 0 failed; 0 ignored;" not in stdout:
        raise RuntimeError("full MCP regression did not execute all 159 required tests")
    for case in MCP_SCAN_REGRESSION_CASES:
        if "test " + case + " ... ok" not in stdout:
            raise RuntimeError("required MCP regression missing: " + case)


def permitted(name, allow_products=False):
    path = Path(name)
    product = allow_products and (
        name in {"crates/diskgraph-cli/Cargo.toml", "crates/diskgraph-mcp/Cargo.toml", "crates/diskgraph-cli/QUICKSTART.md"}
        or ((name.startswith("crates/diskgraph-cli/src/") or name.startswith("crates/diskgraph-mcp/src/")) and path.suffix == ".rs")
    )
    return (not path.is_absolute() and ".." not in path.parts and name == path.as_posix()
            and (product or name in {"Cargo.lock", "crates/diskgraph-engine/Cargo.toml", "crates/diskgraph-engine/build.rs", "crates/diskgraph-scan-worker/Cargo.toml"}
                 or (name.startswith("crates/diskgraph-engine/src/") and path.suffix in {".rs", ".c", ".h"})
                 or (name.startswith("crates/diskgraph-engine/tests/fixtures/") and path.suffix in {".rs", ".c", ".h"})
                 or (name.startswith("crates/diskgraph-scan-worker/src/") and path.suffix == ".rs")))


def mount(checkout, candidate=CANDIDATE, *, allow_products=False):
    checkout = checkout.resolve(strict=True)
    archive = checkout / candidate / "candidate.tar.gz"
    manifest_path = checkout / candidate / "manifest.json"
    if manifest_path.stat().st_size > 1024 * 1024 or archive.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("candidate exceeds archive metadata budget")
    manifest = json.loads(manifest_path.read_text())
    if manifest["schema_version"] != 1 or digest(archive) != manifest["archive_sha256"]:
        raise ValueError("candidate archive identity mismatch")
    names = manifest["sources"]
    if not names or any(not permitted(name, allow_products) for name in names):
        raise ValueError("candidate path escapes fixed source scope")
    with tarfile.open(archive, "r:gz") as source:
        members = source.getmembers()
        if sum(member.size for member in members) > 16 * 1024 * 1024:
            raise ValueError("candidate exceeds decoded source budget")
        if len(members) != len(names) or len({m.name for m in members}) != len(members):
            raise ValueError("duplicate or missing candidate entries")
        # 校验完整归档再修改checkout，拒绝链接、额外名称、过大文件及目录逃逸。
        contents = {}
        for member in members:
            if not member.isfile() or member.name not in names or member.size > 2 * 1024 * 1024:
                raise ValueError("invalid candidate archive member")
            data = source.extractfile(member).read()
            if hashlib.sha256(data).hexdigest() != names[member.name]:
                raise ValueError("candidate source digest mismatch")
            destination = checkout / member.name
            if destination.is_symlink():
                raise ValueError("checkout contains source symlink")
            for parent in destination.parents:
                if parent == checkout:
                    break
                if parent.is_symlink():
                    raise ValueError("checkout contains source symlink")
            contents[member.name] = data
        for name, data in contents.items():
            destination = checkout / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
    return manifest


def invoke(command, checkout, output, name, environment, timeout=900):
    with (output / (name + ".stdout")).open("wb") as stdout, (output / (name + ".stderr")).open("wb") as stderr:
        result = subprocess.run(command, cwd=checkout, env=environment, stdout=stdout, stderr=stderr, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{name} exited {result.returncode}; inspect preserved original logs")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    checkout = Path(__file__).resolve().parent.parent
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if platform.system() != "Darwin" or os.getuid() == 0 or os.geteuid() == 0:
        raise RuntimeError("qualification parent must be ordinary macOS UID")
    if os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("root fixture is restricted to explicit ephemeral hosted CI")
    receipt = {"schema_version": 1, "production_acceptance": False, "status": "started", "uid": os.getuid(), "architecture": platform.machine()}
    try:
        receipt["checkout_sha"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
        receipt["rustc"] = subprocess.check_output(["rustc", "--version"], text=True).strip()
        receipt["os_version"] = platform.mac_ver()[0]
        manifest = mount(checkout, allow_products=True)
        required_cases = {"macos_installed_worker_fixture_tests::" + name for name in (
            "macos_installed_helper_returns_complete_tree_after_real_normal_wait",
            "macos_installed_helper_original_cancel_after_birth_is_reaped",
            "macos_installed_helper_original_panic_after_birth_keeps_recovery_responsibility")}
        required_cases.update("macos_engine_scan_fixture_tests::" + name for name in (
            "macos_engine_installed_scan_publishes_revision_and_relations",
            "macos_engine_response_exhaustion_has_no_partial_revision",
            "macos_engine_postbirth_panic_preserves_payload_and_recovery"))
        if manifest.get("fixture_features") != ["macos_native_scan_candidate"]:
            raise ValueError("candidate fixture feature inventory differs")
        if set(manifest["ordinary_cases"]) != required_cases or len(manifest["ordinary_cases"]) != 6:
            raise ValueError("candidate ordinary acceptance inventory differs")
        if manifest.get("protocol_cases") != [PROTOCOL_CASE, BUDGET_FIXTURE_CASE]:
            raise ValueError("candidate raw path protocol acceptance inventory differs")
        receipt["candidate"] = manifest
        env = os.environ.copy()
        invoke(["cargo", "build", "--locked", "-p", "diskgraph-scan-worker"], checkout, output, "build-helper", env)
        helper = checkout / "target/debug/diskgraph-scan-worker"
        receipt["helper_sha256"] = digest(helper)
        receipt["helper_bytes"] = helper.stat().st_size
        invoke(["cargo", "test", "--locked", "-p", "diskgraph-engine", "--features", "macos_native_scan_candidate", "--lib", "--no-run", "--message-format=json"], checkout, output, "build-fixtures", env)
        binaries = []
        for line in (output / "build-fixtures.stdout").read_text().splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "diskgraph_engine" and record.get("executable"):
                    binaries.append(Path(record["executable"]))
        if len(binaries) != 1:
            raise RuntimeError("expected one actual Engine test executable")
        binary = binaries[0]
        receipt["fixture_sha256"] = digest(binary)
        for protocol_case in manifest["protocol_cases"]:
            name = "raw-path-protocol" if protocol_case == PROTOCOL_CASE else "response-budget-fixture"
            invoke([str(binary), protocol_case, "--exact", "--nocapture", "--test-threads=1"], checkout, output, name, env)
            if "test result: ok. 1 passed; 0 failed;" not in (output / (name + ".stdout")).read_text():
                raise RuntimeError("protocol fixture did not execute exact required case: " + protocol_case)
        receipt["protocol_cases_passed"] = len(manifest["protocol_cases"])
        env.update(DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE=os.environ["GITHUB_RUN_ID"],
                   DISKGRAPH_MACOS_FIXTURE_HELPER=str(helper),
                   DISKGRAPH_MACOS_FIXTURE_SHA256=receipt["helper_sha256"],
                   DISKGRAPH_MACOS_FIXTURE_BYTES=str(receipt["helper_bytes"]))
        preserved = "GITHUB_ACTIONS,GITHUB_RUN_ID,RUNNER_OS,RUNNER_ENVIRONMENT,DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE,DISKGRAPH_MACOS_FIXTURE_HELPER,DISKGRAPH_MACOS_FIXTURE_SHA256,DISKGRAPH_MACOS_FIXTURE_BYTES"
        invoke(["sudo", "-n", "--preserve-env=" + preserved, str(binary), ROOT_CASE, "--exact", "--ignored", "--nocapture", "--test-threads=1"], checkout, output, "root-issuer", env)
        if "test result: ok. 1 passed; 0 failed;" not in (output / "root-issuer.stdout").read_text():
            raise RuntimeError("root fixture did not execute its exact required case")
        receipt["root_cases_passed"] = 1
        for case in manifest["ordinary_cases"]:
            invoke([str(binary), case, "--exact", "--ignored", "--nocapture", "--test-threads=1"], checkout, output, case.rsplit("::", 1)[-1], env)
            if "test result: ok. 1 passed; 0 failed;" not in (output / (case.rsplit("::", 1)[-1] + ".stdout")).read_text():
                raise RuntimeError("ordinary fixture did not execute exact required case")
        receipt["ordinary_cases_passed"] = len(manifest["ordinary_cases"])
        if manifest.get("product_flows") != ["cli_init_node", "mcp_stdio_index_status_node", "cli_observes_mcp_revision"]:
            raise ValueError("actual product flow inventory differs")
        invoke(["cargo", "build", "--locked", "-p", "diskgraph-cli", "-p", "diskgraph-mcp", "--features", "diskgraph-engine/macos_native_scan_candidate"],
               checkout, output, "build-products", env)
        import qualify_macos_product_flows
        receipt["product_flows"] = qualify_macos_product_flows.run(
            checkout, output / "products", env,
            checkout / "target/debug/diskgraph", checkout / "target/debug/diskgraph-mcp")
        invoke(["cargo", "test", "--locked", "-p", "diskgraph-cli", "--bin", "diskgraph",
                "--features", "diskgraph-engine/macos_native_scan_candidate", "--no-run", "--message-format=json"],
               checkout, output, "build-cli-regression", env)
        cli_fixtures = []
        for line in (output / "build-cli-regression.stdout").read_text().splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if (record.get("reason") == "compiler-artifact"
                        and record.get("target", {}).get("name") == "diskgraph"
                        and record.get("profile", {}).get("test") and record.get("executable")):
                    cli_fixtures.append(Path(record["executable"]))
        if len(cli_fixtures) != 1:
            raise RuntimeError("expected one actual CLI regression executable")
        cli_fixture = cli_fixtures[0]
        receipt["cli_regression_binary_sha256"] = digest(cli_fixture)
        invoke([str(cli_fixture), "--test-threads=1"], checkout, output, "cli-regression", env)
        check_cli_regression((output / "cli-regression.stdout").read_text())
        if digest(cli_fixture) != receipt["cli_regression_binary_sha256"]:
            raise RuntimeError("CLI regression binary identity changed")
        receipt["cli_regression_tests_passed"] = 67
        invoke(["cargo", "test", "--locked", "-p", "diskgraph-mcp", "--lib",
                "--features", "diskgraph-engine/macos_native_scan_candidate", "--no-run", "--message-format=json"],
               checkout, output, "build-mcp-regression", env)
        mcp_fixtures = []
        for line in (output / "build-mcp-regression.stdout").read_text().splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if (record.get("reason") == "compiler-artifact"
                        and record.get("target", {}).get("name") == "diskgraph_mcp"
                        and record.get("profile", {}).get("test") and record.get("executable")):
                    mcp_fixtures.append(Path(record["executable"]))
        if len(mcp_fixtures) != 1:
            raise RuntimeError("expected one actual MCP regression executable")
        mcp_fixture = mcp_fixtures[0]
        receipt["mcp_regression_binary_sha256"] = digest(mcp_fixture)
        invoke([str(mcp_fixture), "--test-threads=1"], checkout, output, "mcp-regression", env)
        check_mcp_regression((output / "mcp-regression.stdout").read_text())
        if digest(mcp_fixture) != receipt["mcp_regression_binary_sha256"]:
            raise RuntimeError("MCP regression binary identity changed")
        receipt["mcp_regression_tests_passed"] = 159
        for name, expected in manifest["sources"].items():
            if digest(checkout / name) != expected:
                raise RuntimeError("qualification modified original candidate source")
        if digest(helper) != receipt["helper_sha256"] or digest(binary) != receipt["fixture_sha256"]:
            raise RuntimeError("qualification binary identity changed")
        receipt["status"] = "passed"
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
