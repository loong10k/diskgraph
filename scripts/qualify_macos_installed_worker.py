"""冻结候选的真实root发行/普通UID运行资格；不把未运行夹具当PASS。"""
import argparse
import hashlib
import json
import os
import platform
import re
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


NATIVE_BIRTH_REGRESSION_CASES = (
    "native_child::native_birth_gate_tests::contended_legacy_birth_keeps_postbirth_checkpoint_after_actual_creation",
    "native_child::native_birth_gate_tests::contended_admission_rejection_does_not_advance_lifecycle_callback",
    "native_child::unix_control_input_tests::explicit_null_preserves_two_checkpoints_and_all_control_methods_are_unsupported",
    "native_child::unix_control_input_tests::worker_control_checkpoint_failure_keeps_original_non_clone_error_and_real_reap",
    "native_child::unix_leader_tests::nonpositive_native_pid_cannot_reap_an_unrelated_child",
    "native_child::unix_normal_exit_tests::external_reap_loses_normal_permission_and_refuses_old_numeric_group_cleanup",
    "native_child::unix_normal_exit_tests::group_termination_failure_retains_original_leader_and_capacity_until_retry",
    "native_child::macos_registry_deadline_tests::expired_recovery_deadline_retains_live_original_and_capacity_without_signaling",
    "native_child::unix_normal_exit_tests::deadline_drain_group_failure_and_late_reap_keep_original_slot_on_every_retry",
    "native_child::unix_leader_tests::native_and_standard_poll_wait_keep_live_owner_and_cache_only_actual_reaping",
)


def check_native_birth_regression(stdout, expected=44):
    """实际并行执行44项；唯一ignored是由真实回归显式调用的隔离夹具。"""
    summaries = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;", stdout, re.M)
    if len(summaries) != 1 or tuple(map(int, summaries[0][:4])) != (expected, 0, 1, 0):
        raise RuntimeError("parallel native child regression differs from the complete required inventory")
    fixture = "test native_child::unix_leader_tests::nonpositive_native_pid_isolated_fixture ... ignored, invoked by the real isolated ownership regression"
    if fixture not in stdout or "DG_NONPOSITIVE_WAIT_FIXTURE_VERIFIED pid=0,-1,-42" not in stdout:
        raise RuntimeError("isolated nonpositive wait fixture was not actually verified")
    for case in NATIVE_BIRTH_REGRESSION_CASES:
        if "test " + case + " ... ok" not in stdout:
            raise RuntimeError("required native birth regression missing: " + case)


MCP_SCAN_REGRESSION_CASES = (
    "http_request_syntax_tests::malformed_request_line_never_becomes_a_dispatchable_request",
    "http_request_syntax_tests::duplicate_mcp_session_and_protocol_headers_are_rejected_before_dispatch",
    "http_request_syntax_tests::supported_http_versions_preserve_request_and_single_value_headers",
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


def check_mcp_regression(stdout, inventory=None):
    """冻结模式保留原162项；当前模式逐项核实二进制清单、必跑安全案和无过滤执行。"""
    if inventory is None:
        if "test result: ok. 162 passed; 0 failed; 0 ignored;" not in stdout:
            raise RuntimeError("full MCP regression did not execute all 162 required tests")
        cases = MCP_SCAN_REGRESSION_CASES
    else:
        if (not inventory or len(set(inventory)) != len(inventory)
                or not set(MCP_SCAN_REGRESSION_CASES).issubset(inventory)):
            raise RuntimeError("current MCP inventory lost or duplicated required cases")
        summaries = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;", stdout, re.M)
        if len(summaries) != 1 or tuple(map(int, summaries[0])) != (len(inventory), 0, 0, 0, 0):
            raise RuntimeError("current MCP regression differs from exact unfiltered binary inventory")
        cases = inventory
    for case in cases:
        if "test " + case + " ... ok" not in stdout:
            raise RuntimeError("required MCP regression missing: " + case)


MOVED_SETTINGS_REGRESSION_CASES = (
    "scan_worker_settings_tests::unified_host_preserves_original_cancel_before_environment_or_paths",
    "scan_worker_settings_tests::unified_host_rejects_original_expired_deadline_before_configuration",
    "scan_worker_settings_tests::unified_host_partial_configuration_stays_invalid_and_never_opens_image",
    "scan_worker_settings_tests::unified_host_absent_configuration_still_rechecks_original_cancel",
    "scan_worker_settings_tests::unified_macos_host_refuses_complete_legacy_environment_without_opening_file",
    "scan_worker_settings_tests::unified_host_invalid_digest_preserves_argument_error",
    "scan_worker_settings_tests::unified_host_never_resets_deadline_after_lookup",
    "scan_worker_settings_tests::unified_macos_absent_environment_uses_fixed_installation_semantics",
    "scan_worker_settings_tests::absent_configuration_is_distinct_from_every_partial_configuration",
    "scan_worker_settings_tests::invalid_material_is_rejected_before_opening_any_image",
    "scan_worker_settings_tests::independent_expected_digest_and_original_open_file_survive_name_replacement",
    "scan_worker_settings_tests::opening_missing_material_preserves_the_actual_io_error",
    "scan_worker_settings_tests::neighboring_correct_values_cannot_override_wrong_but_well_formed_expectations",
    "scan_worker_settings_tests::uppercase_complete_digest_is_preserved_as_the_same_expected_material",
    "scan_worker_settings_tests::native_non_utf8_open_failure_and_non_unicode_material_preserve_their_boundaries",
)


def check_moved_settings_regression(stdout):
    """部署配置迁入Engine后仍须实际通过，不能用MCP计数减少掩盖回归删除。"""
    for case in MOVED_SETTINGS_REGRESSION_CASES:
        if "test " + case + " ... ok" not in stdout:
            raise RuntimeError("moved deployment settings regression missing: " + case)


def check_full_engine_regression(stdout, total, ignored):
    """完整冻结Engine二进制必须执行全部非ignored清单；ignored单列，禁止过滤或少量冒充。"""
    matches = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;", stdout, re.MULTILINE)
    if (total < 559 or ignored < 0 or ignored >= total
            or len(matches) != 1
            or tuple(map(int, matches[0])) != (total - ignored, 0, ignored, 0, 0)):
        raise RuntimeError("full Engine regression differs from exact unfiltered binary inventory")


def permitted(name, allow_products=False):
    path = Path(name)
    product = allow_products and (
        name in {"crates/diskgraph-cli/Cargo.toml", "crates/diskgraph-mcp/Cargo.toml", "crates/diskgraph-cli/QUICKSTART.md"}
        or ((name.startswith("crates/diskgraph-cli/src/") or name.startswith("crates/diskgraph-mcp/src/")) and path.suffix == ".rs")
    )
    return (not path.is_absolute() and ".." not in path.parts and name == path.as_posix()
            and (product or name in {"Cargo.lock", "crates/diskgraph-engine/Cargo.toml", "crates/diskgraph-engine/build.rs", "crates/diskgraph-scan-worker/Cargo.toml", "crates/diskgraph-engine/tests/macos_installer_api.rs"}
                 or (name.startswith("crates/diskgraph-engine/src/") and path.suffix in {".rs", ".c", ".h"})
                 or (name.startswith("crates/diskgraph-engine/tests/fixtures/") and path.suffix in {".rs", ".c", ".h"})
                 or (name.startswith("crates/diskgraph-scan-worker/src/") and path.suffix == ".rs")))


def mount(checkout, candidate=CANDIDATE, *, allow_products=False):
    checkout = checkout.resolve(strict=True)
    archive = checkout / candidate / "candidate.tar.gz"
    manifest_path = checkout / candidate / "manifest.json"
    if manifest_path.stat().st_size > 1024 * 1024 or archive.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("candidate exceeds archive metadata budget")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
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


def current_sources(checkout):
    """只记录当前Git源码与摘要；旧归档仅提供验收清单，绝不挂载其源码。"""
    checkout = checkout.resolve(strict=True)
    manifest = json.loads((checkout / CANDIDATE / "manifest.json").read_text(encoding="utf-8"))
    paths = subprocess.check_output(["git", "ls-files", "-z"], cwd=checkout).decode("utf-8").split("\0")
    sources = {}
    for name in paths:
        path = Path(name)
        if (not name or "integration_candidates" in path.parts
                or not (name in {"Cargo.toml", "Cargo.lock"}
                        or (name.startswith("crates/") and path.suffix in {".rs", ".c", ".h", ".toml"})
                        or (name.startswith("scripts/") and path.suffix == ".py")
                        or (name.startswith(".github/workflows/") and path.suffix in {".yml", ".yaml"}))):
            continue
        if path.is_absolute() or ".." in path.parts or name != path.as_posix():
            raise ValueError("current source path escapes checkout")
        actual = checkout / path
        if actual.is_symlink() or any(parent.is_symlink() for parent in actual.parents if parent != checkout):
            raise ValueError("current source contains symlink")
        if not actual.is_file():
            raise ValueError("current tracked source is missing")
        sources[name] = digest(actual)
    if not sources:
        raise ValueError("current checkout has no tracked source inventory")
    manifest.pop("archive_sha256", None)
    manifest["sources"] = sources
    manifest["source_mode"] = "current_checkout"
    return manifest


def driver_artifact(stdout):
    """只接收本次Cargo输出的唯一样例产物，不按PATH或旧邻接清单猜测。"""
    rows = [json.loads(line) for line in stdout.splitlines() if line.startswith("{")]
    images = [Path(row["executable"]) for row in rows
              if row.get("reason") == "compiler-artifact"
              and row.get("target", {}).get("name") == "scan_worker_driver_fixture"
              and row.get("target", {}).get("kind") == ["example"] and row.get("executable")]
    if len(images) != 1:
        raise RuntimeError("one actual current protocol-driver artifact is required")
    image = images[0]
    if not image.is_absolute() or image.is_symlink() or not image.is_file() or any(c in str(image) for c in "\r\n\0"):
        raise RuntimeError("invalid current protocol-driver artifact path")
    return image


def invoke(command, checkout, output, name, environment, timeout=900):
    with (output / (name + ".stdout")).open("wb") as stdout, (output / (name + ".stderr")).open("wb") as stderr:
        result = subprocess.run(command, cwd=checkout, env=environment, stdout=stdout, stderr=stderr, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{name} exited {result.returncode}; inspect preserved original logs")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--current-checkout", action="store_true", help="qualify current tracked source without mounting a frozen archive")
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
        if args.current_checkout:
            dirty = subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=checkout)
            if dirty:
                raise RuntimeError("current checkout qualification requires unchanged tracked source")
            manifest = current_sources(checkout)
        else:
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
        for line in (output / "build-fixtures.stdout").read_text(encoding="utf-8").splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "diskgraph_engine" and record.get("executable"):
                    binaries.append(Path(record["executable"]))
        if len(binaries) != 1:
            raise RuntimeError("expected one actual Engine test executable")
        binary = binaries[0]
        receipt["fixture_sha256"] = digest(binary)
        if args.current_checkout:
            invoke(["cargo", "build", "--locked", "-p", "diskgraph-engine", "--example", "scan_worker_driver_fixture",
                    "--features", "macos_native_scan_candidate", "--message-format=json"], checkout, output, "build-driver-fixture", env)
            driver = driver_artifact((output / "build-driver-fixture.stdout").read_text(encoding="utf-8"))
            env["DISKGRAPH_SCAN_DRIVER_FIXTURE"] = str(driver)
            receipt["driver_fixture"] = {"path": str(driver), "sha256": digest(driver), "bytes": driver.stat().st_size,
                                          "source_sha256": digest(checkout / "crates/diskgraph-engine/tests/fixtures/scan_worker_driver_fixture.rs"),
                                          "product_scan_image": False}

        for protocol_case in manifest["protocol_cases"]:
            name = "raw-path-protocol" if protocol_case == PROTOCOL_CASE else "response-budget-fixture"
            invoke([str(binary), protocol_case, "--exact", "--nocapture", "--test-threads=1"], checkout, output, name, env)
            if "test result: ok. 1 passed; 0 failed;" not in (output / (name + ".stdout")).read_text(encoding="utf-8"):
                raise RuntimeError("protocol fixture did not execute exact required case: " + protocol_case)
        receipt["protocol_cases_passed"] = len(manifest["protocol_cases"])
        env.update(DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE=os.environ["GITHUB_RUN_ID"],
                   DISKGRAPH_MACOS_FIXTURE_HELPER=str(helper),
                   DISKGRAPH_MACOS_FIXTURE_SHA256=receipt["helper_sha256"],
                   DISKGRAPH_MACOS_FIXTURE_BYTES=str(receipt["helper_bytes"]))
        preserved = "GITHUB_ACTIONS,GITHUB_RUN_ID,RUNNER_OS,RUNNER_ENVIRONMENT,DISKGRAPH_MACOS_EPHEMERAL_ROOT_FIXTURE,DISKGRAPH_MACOS_FIXTURE_HELPER,DISKGRAPH_MACOS_FIXTURE_SHA256,DISKGRAPH_MACOS_FIXTURE_BYTES"
        invoke(["sudo", "-n", "--preserve-env=" + preserved, str(binary), ROOT_CASE, "--exact", "--ignored", "--nocapture", "--test-threads=1"], checkout, output, "root-issuer", env)
        if "test result: ok. 1 passed; 0 failed;" not in (output / "root-issuer.stdout").read_text(encoding="utf-8"):
            raise RuntimeError("root fixture did not execute its exact required case")
        receipt["root_cases_passed"] = 1
        for case in manifest["ordinary_cases"]:
            invoke([str(binary), case, "--exact", "--ignored", "--nocapture", "--test-threads=1"], checkout, output, case.rsplit("::", 1)[-1], env)
            if "test result: ok. 1 passed; 0 failed;" not in (output / (case.rsplit("::", 1)[-1] + ".stdout")).read_text(encoding="utf-8"):
                raise RuntimeError("ordinary fixture did not execute exact required case")
        receipt["ordinary_cases_passed"] = len(manifest["ordinary_cases"])
        expected_native = 44
        if args.current_checkout:
            invoke([str(binary), "native_child::", "--list"], checkout, output, "native-current-inventory", env)
            invoke([str(binary), "native_child::", "--ignored", "--list"], checkout, output, "native-current-ignored", env)
            native = [line[:-6] for line in (output / "native-current-inventory.stdout").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
            ignored_native = [line[:-6] for line in (output / "native-current-ignored.stdout").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
            if len(set(native)) != len(native) or len(ignored_native) != 1 or not set(ignored_native).issubset(native):
                raise RuntimeError("current native binary inventory is inconsistent")
            expected_native = len(native) - len(ignored_native)
            if expected_native < 44:
                raise RuntimeError("current native inventory lost required regressions")
            manifest["native_child_parallel_tests_required"] = expected_native
            receipt["native_current_inventory"] = native
        if manifest.get("native_child_parallel_tests_required") != expected_native:
            raise ValueError("candidate parallel birth regression inventory differs")
        invoke([str(binary), "native_child::", "--nocapture"],
               checkout, output, "native-child-parallel-regression", env)
        check_native_birth_regression((output / "native-child-parallel-regression.stdout").read_text(encoding="utf-8"), expected_native)
        receipt["native_child_parallel_regression_tests_passed"] = expected_native
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
        for line in (output / "build-cli-regression.stdout").read_text(encoding="utf-8").splitlines():
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
        check_cli_regression((output / "cli-regression.stdout").read_text(encoding="utf-8"))
        if digest(cli_fixture) != receipt["cli_regression_binary_sha256"]:
            raise RuntimeError("CLI regression binary identity changed")
        receipt["cli_regression_tests_passed"] = 67
        invoke(["cargo", "test", "--locked", "-p", "diskgraph-mcp", "--lib",
                "--features", "diskgraph-engine/macos_native_scan_candidate", "--no-run", "--message-format=json"],
               checkout, output, "build-mcp-regression", env)
        mcp_fixtures = []
        for line in (output / "build-mcp-regression.stdout").read_text(encoding="utf-8").splitlines():
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
        mcp_inventory = None
        if args.current_checkout:
            invoke([str(mcp_fixture), "--list"], checkout, output, "mcp-current-inventory", env)
            mcp_inventory = [line[:-6] for line in (output / "mcp-current-inventory.stdout").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
            receipt["mcp_current_inventory"] = mcp_inventory
        invoke([str(mcp_fixture), "--test-threads=1"], checkout, output, "mcp-regression", env)
        check_mcp_regression((output / "mcp-regression.stdout").read_text(encoding="utf-8"), inventory=mcp_inventory)
        if digest(mcp_fixture) != receipt["mcp_regression_binary_sha256"]:
            raise RuntimeError("MCP regression binary identity changed")
        receipt["mcp_regression_tests_passed"] = len(mcp_inventory) if mcp_inventory is not None else 162
        # 在同一root发行/普通UID环境执行完整Engine，不能用39项native子集代替。
        invoke([str(binary), "--list"], checkout, output, "engine-full-inventory", env)
        invoke([str(binary), "--ignored", "--list"], checkout, output, "engine-ignored-inventory", env)
        inventory = [line[:-6] for line in (output / "engine-full-inventory.stdout").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
        ignored = [line[:-6] for line in (output / "engine-ignored-inventory.stdout").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
        if len(set(inventory)) != len(inventory) or len(set(ignored)) != len(ignored) or not set(ignored).issubset(inventory):
            raise RuntimeError("Engine binary inventory is duplicated or inconsistent")
        receipt["engine_full_inventory"] = inventory
        receipt["engine_ignored_inventory"] = ignored
        invoke([str(binary), "--nocapture"], checkout, output, "engine-full-parallel-regression", env, timeout=1200)
        check_full_engine_regression((output / "engine-full-parallel-regression.stdout").read_text(encoding="utf-8"), len(inventory), len(ignored))
        if args.current_checkout:
            check_moved_settings_regression((output / "engine-full-parallel-regression.stdout").read_text(encoding="utf-8"))
        receipt["engine_full_regression_tests_passed"] = len(inventory) - len(ignored)
        for name, expected in manifest["sources"].items():
            if digest(checkout / name) != expected:
                raise RuntimeError("qualification modified original candidate source")
        if digest(helper) != receipt["helper_sha256"] or digest(binary) != receipt["fixture_sha256"]:
            raise RuntimeError("qualification binary identity changed")
        if args.current_checkout and digest(driver) != receipt["driver_fixture"]["sha256"]:
            raise RuntimeError("protocol-driver fixture identity changed")
        receipt["status"] = "passed"
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    main()
