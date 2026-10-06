"""当前源码的七项精确Origin/监听信任回归；不代替完整平台验收。"""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess

CASES = (
    "http::tests::malformed_http_header_tokens_are_rejected_before_dispatch",
    "http::tests::malformed_origins_are_not_truncated_or_allowed_by_configuration",
    "http::tests::malformed_bind_hosts_never_receive_local_trust",
    "http::tests::ipv6_loopback_origin_without_port_is_valid",
    "http::tests::origins_are_validated_before_anything_else",
    "http::tests::a_hostile_origin_is_refused_even_with_a_valid_token",
    "http_lifecycle_tests::malformed_origins_are_refused_before_auth_on_all_real_transport_routes",
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def invoke(command, root, output, name):
    result = subprocess.run(command, cwd=root, capture_output=True, timeout=600)
    (output / (name + ".stdout")).write_bytes(result.stdout)
    (output / (name + ".stderr")).write_bytes(result.stderr)
    if result.returncode:
        raise RuntimeError(f"{name} exited {result.returncode}; inspect original logs")
    return result.stdout.decode("utf-8")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    files = [root / "Cargo.toml", root / "Cargo.lock"]
    files += sorted((root / "crates").rglob("*.rs"))
    files += sorted((root / "crates").rglob("Cargo.toml"))
    sources = {str(path.relative_to(root)): digest(path) for path in files}
    receipt = {
        "production_acceptance": False, "status": "started", "platform": platform.platform(),
        "driver_sha256": digest(Path(__file__)),
        "rustc": subprocess.check_output(["rustc", "--version"], cwd=root, text=True).strip(),
        "checkout_sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "source_binding": "runtime content SHA256; checkout SHA alone does not prove clean sources",
        "changed_sources": [name for name in subprocess.check_output(
            ["git", "diff", "--name-only", "HEAD", "--", "Cargo.toml", "Cargo.lock", "crates"],
            cwd=root, text=True).splitlines() if name in sources],
        "sources": sources, "cases": [],
    }
    try:
        stdout = invoke(["cargo", "test", "--locked", "-p", "diskgraph-mcp", "--lib",
                         "--no-run", "--message-format=json"], root, output, "build")
        binaries = []
        for line in stdout.splitlines():
            if line.startswith("{"):
                record = json.loads(line)
                if record.get("reason") == "compiler-artifact" and record.get("target", {}).get("name") == "diskgraph_mcp" and record.get("executable"):
                    binaries.append(Path(record["executable"]))
        if len(binaries) != 1:
            raise RuntimeError("expected one actual current-source MCP test executable")
        binary = binaries[0]
        receipt["fixture_sha256"] = digest(binary)
        for index, case in enumerate(CASES):
            stdout = invoke([str(binary), "--exact", case, "--nocapture", "--test-threads=1"],
                            root, output, f"case-{index}")
            if f"test {case} ... " not in stdout or "test result: ok. 1 passed; 0 failed; 0 ignored;" not in stdout:
                raise RuntimeError(f"{case} was not actually executed and passed")
            receipt["cases"].append({"case": case, "passed": True})
        if digest(binary) != receipt["fixture_sha256"] or any(digest(root / name) != expected for name, expected in sources.items()):
            raise RuntimeError("source or executable changed during qualification")
        receipt["status"] = "verified_origin_subset"
    except BaseException as error:
        receipt["status"] = "failed"
        receipt["failure"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
