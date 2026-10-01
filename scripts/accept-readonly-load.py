#!/usr/bin/env python3
"""Measure a packaged read-only binary over an isolated 20k-file workload."""

import argparse
import concurrent.futures
import json
import math
import pathlib
import statistics
import subprocess
import sys
import tempfile
import time


SUFFIX = ".exe" if sys.platform == "win32" else ""


def invoke(cli, data, *arguments, timeout=300, expected=0):
    result = subprocess.run(
        [cli, "--data-dir", data, "--json", *arguments],
        capture_output=True, text=True, timeout=timeout,
    )
    if result.returncode != expected:
        raise RuntimeError(
            f"{arguments!r} exited {result.returncode}, expected {expected}:\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    if not lines:
        raise RuntimeError(f"{arguments!r} produced no JSON response")
    return json.loads(lines[-1])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", required=True, type=pathlib.Path)
    parser.add_argument("--files", type=int, default=20_000)
    parser.add_argument("--queries", type=int, default=32)
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()
    if args.files < 101 or args.queries < 4:
        parser.error("need at least 101 files and 4 queries")
    cli = args.bin_dir.resolve() / f"diskgraph{SUFFIX}"
    version = subprocess.run([cli, "--version"], capture_output=True, text=True,
                             check=True, timeout=10).stdout.strip()
    checks = {}
    with tempfile.TemporaryDirectory(prefix="diskgraph-readonly-load-") as temporary:
        work = pathlib.Path(temporary)
        root, data = work / "project", work / "data"
        root.mkdir()
        for index in range(args.files):
            (root / f"file-{index:06}.bin").write_bytes(b"x" * 32)
        scope = invoke(cli, data, "scope", "add", "--root", root)["data"]["scope_id"]
        started = time.perf_counter()
        indexed = invoke(cli, data, "index", "--scope", scope, "--wait")
        scan_seconds = time.perf_counter() - started
        revision = indexed["data"]["revision_id"]
        checks["full_index_published"] = indexed["data"]["state"] == "completed" and bool(revision)

        def query(number):
            arguments = ("top", "--scope", scope) if number % 2 else (
                "children", "--scope", scope, "--limit", "25"
            )
            started = time.perf_counter()
            answer = invoke(cli, data, *arguments)
            return time.perf_counter() - started, answer

        query(0)
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as workers:
            results = list(workers.map(query, range(args.queries)))
        latencies = sorted(duration for duration, _ in results)
        checks["concurrent_reads_bound_to_revision"] = all(
            answer["ok"] and answer["data"]["revision_id"] == revision
            and bool(answer["data"]["items"])
            for _, answer in results
        )
        checks["query_results_bounded"] = all(
            len(answer["data"]["items"]) <= (20 if index % 2 else 25)
            for index, (_, answer) in enumerate(results)
        )

        limited_data = work / "limited-data"
        limited_scope = invoke(cli, limited_data, "scope", "add", "--root", root)["data"]["scope_id"]
        refusal = invoke(cli, limited_data, "--max-nodes-per-scan", "100", "index",
                         "--scope", limited_scope, "--wait", expected=7)
        unindexed = invoke(cli, limited_data, "tree", "--scope", limited_scope, expected=4)
        checks["node_budget_refuses_partial_publish"] = (
            refusal["error"]["code"] == "budget_exceeded"
            and unindexed["error"]["code"] == "not_indexed"
        )
        database_bytes = sum(
            path.stat().st_size for path in data.glob("diskgraph*.sqlite*") if path.is_file()
        )

    report = {
        "version": version,
        "platform": sys.platform,
        "files": args.files,
        "queries": args.queries,
        "concurrent_clients": 4,
        "scan_seconds": round(scan_seconds, 3),
        "query_p50_ms": round(statistics.median(latencies) * 1000, 3),
        "query_p95_ms": round(latencies[math.ceil(0.95 * len(latencies)) - 1] * 1000, 3),
        "database_and_wal_bytes": database_bytes,
        "passed": sum(checks.values()),
        "total": len(checks),
        "checks": checks,
    }
    encoded = json.dumps(report, indent=2)
    print(encoded)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded + "\n")
    return 0 if report["passed"] == report["total"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
