#!/usr/bin/env python3
"""Isolated old-binary upgrade and backup-based rollback drill."""

import argparse
import json
import pathlib
import shutil
import sqlite3
import subprocess
import tempfile


def call(binary, data, *arguments):
    result = subprocess.run(
        [binary, "--data-dir", data, "--json", *arguments],
        capture_output=True, text=True, check=True, timeout=120,
    )
    return json.loads(result.stdout.splitlines()[-1])


def schema(database):
    with sqlite3.connect(database) as connection:
        return connection.execute("PRAGMA user_version").fetchone()[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old-cli", required=True, type=pathlib.Path)
    parser.add_argument("--new-cli", required=True, type=pathlib.Path)
    args = parser.parse_args()
    old_cli, new_cli = args.old_cli.resolve(), args.new_cli.resolve()
    checks = {}
    with tempfile.TemporaryDirectory(prefix="diskgraph-upgrade-accept-") as temporary:
        work = pathlib.Path(temporary)
        root = work / "project"
        data = work / "data"
        restored = work / "restored"
        root.mkdir()
        (root / "file.txt").write_text("upgrade fixture\n")
        scope = call(old_cli, data, "scope", "add", "--root", str(root))["data"]["scope_id"]
        old_index = call(old_cli, data, "index", "--scope", scope, "--wait")
        old_tree = call(old_cli, data, "tree", "--scope", scope)
        old_graph_schema = schema(data / "diskgraph.sqlite")
        old_control_schema = schema(data / "diskgraph-control.sqlite")

        new_tree = call(new_cli, data, "tree", "--scope", scope)
        checks["revision_and_tree_survive_upgrade"] = (
            new_tree["data"]["revision_id"] == old_index["data"]["revision_id"]
            and new_tree["data"]["tree"] == old_tree["data"]["tree"]
        )
        checks["graph_schema_advanced"] = schema(data / "diskgraph.sqlite") > old_graph_schema
        checks["control_schema_advanced"] = schema(data / "diskgraph-control.sqlite") > old_control_schema
        incompatible = subprocess.run(
            [old_cli, "--data-dir", data, "--json", "tree", "--scope", scope],
            capture_output=True, text=True, timeout=120,
        )
        checks["old_binary_refuses_upgraded_schema"] = incompatible.returncode != 0

        backups = data / "migration_backups"
        restored.mkdir()
        for name, previous in (("diskgraph.sqlite", old_graph_schema),
                               ("diskgraph-control.sqlite", old_control_schema)):
            matches = list(backups.glob(f"{name}.pre-v*.bak"))
            checks[f"{name}_consistent_backup"] = (
                len(matches) == 1 and schema(matches[0]) == previous
            )
            if len(matches) != 1:
                raise RuntimeError(f"expected one pre-upgrade backup for {name}")
            shutil.copy2(matches[0], restored / name)

        rolled_back = call(old_cli, restored, "tree", "--scope", scope)
        checks["old_binary_reads_restored_pair"] = (
            rolled_back["data"]["revision_id"] == old_index["data"]["revision_id"]
            and rolled_back["data"]["tree"] == old_tree["data"]["tree"]
        )

    passed = sum(checks.values())
    print(json.dumps({"passed": passed, "total": len(checks), "checks": checks,
                      "old_graph_schema": old_graph_schema,
                      "old_control_schema": old_control_schema}, indent=2))
    return 0 if passed == len(checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
