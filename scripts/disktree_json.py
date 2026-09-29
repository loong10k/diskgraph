#!/usr/bin/env python3
"""disktree-json: render a published DiskGraph revision as a disktree-style
nested JSON tree.

Reads the published graph database READ-ONLY (the viewer's contract: it never
writes) and emits the same shape a tree UI wants:

    {"name": "/", "kind": "directory", "size_bytes": <subtree total>,
     "own_bytes": <this node's own block bytes>, "files": N, "dirs": N,
     "children": [ ... sorted by size, largest first ... ]}

Depth is bounded with --depth; cut children are reported as "truncated": true
so the JSON never lies about having shown everything.
"""
import argparse
import json
import sqlite3
import sys


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, help="path to diskgraph.sqlite")
    parser.add_argument("--revision", required=True, help="revision id to render")
    parser.add_argument("--depth", type=int, default=3, help="tree depth to expand")
    parser.add_argument("--min-bytes", type=int, default=0,
                        help="hide children smaller than this at the cut level")
    parser.add_argument("--out", default="-", help="output file (default stdout)")
    args = parser.parse_args()

    # Read-only URI: the viewer can never corrupt the published snapshot.
    connection = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row

    revision = connection.execute(
        "SELECT snapshot_id FROM graph_revisions WHERE revision_id = ?",
        (args.revision,),
    ).fetchone()
    if revision is None:
        print(f"revision not found: {args.revision}", file=sys.stderr)
        return 2
    snapshot_id = revision["snapshot_id"]

    # The store keeps the full node as JSON; the viewer extracts only the
    # fields a tree UI needs (SQLite JSON1 does this inside the database).
    rows = connection.execute(
        "SELECT id AS node_id, parent_id, name, subtree_bytes,"
        " json_extract(node_json, '$.kind') AS kind,"
        " json_extract(node_json, '$.direct_bytes') AS direct_bytes,"
        " json_extract(node_json, '$.files') AS files,"
        " json_extract(node_json, '$.directories') AS directories,"
        " json_extract(node_json, '$.read_error') AS read_error"
        " FROM nodes WHERE snapshot_id = ?",
        (snapshot_id,),
    ).fetchall()
    children_of: dict[int | None, list[sqlite3.Row]] = {}
    for row in rows:
        children_of.setdefault(row["parent_id"], []).append(row)
    root_row = children_of.get(None, [None])[0]
    if root_row is None:
        print("revision has no root node", file=sys.stderr)
        return 2

    def render(row: sqlite3.Row, depth: int) -> dict:
        node = {
            "name": row["name"],
            "kind": row["kind"],
            "size_bytes": row["subtree_bytes"],
            "own_bytes": row["direct_bytes"],
            "files": row["files"],
            "dirs": row["directories"],
        }
        if row["read_error"]:
            node["read_error"] = True
        kids = children_of.get(row["node_id"], [])
        if depth >= args.depth or not kids:
            if kids:
                node["truncated"] = True
                node["children_count"] = len(kids)
            return node
        kept = [kid for kid in kids if kid["subtree_bytes"] >= args.min_bytes]
        kept.sort(key=lambda kid: (-kid["subtree_bytes"], kid["name"]))
        hidden = len(kids) - len(kept)
        node["children"] = [render(kid, depth + 1) for kid in kept]
        if hidden > 0:
            node["hidden_below_min_bytes"] = hidden
        return node

    tree = render(root_row, 1)
    tree["revision_id"] = args.revision
    tree["rendered_depth"] = args.depth
    payload = json.dumps(tree, indent=2, ensure_ascii=False)
    if args.out == "-":
        sys.stdout.write(payload + "\n")
    else:
        with open(args.out, "w", encoding="utf-8") as handle:
            handle.write(payload + "\n")
        print(f"wrote {args.out} ({len(payload)} bytes)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
