#!/usr/bin/env bash
# Correctness and cost baseline for the local evaluation set (P3 task 4.9,
# spec RE-03): a fixed synthetic dataset, verified answers, and cold-index /
# warm-query / sync timings. Numbers are dataset measurements, not product
# promises, and are recorded with their hardware and version.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

OUT_DIR="${1:-$REPO_ROOT/dist/evaluation}"
mkdir -p "$OUT_DIR"
DATASET="$OUT_DIR/dataset"
DATA="$OUT_DIR/data"

rm -rf "$DATASET" "$DATA"
mkdir -p "$DATASET"

echo "==> building the fixed dataset"
# A deterministic tree: known project ownership, known large items, and a
# directory that merely looks like build output.
for index in $(seq 1 12); do
    project="$DATASET/project-$index"
    mkdir -p "$project/target" "$project/src"
    printf '[package]\nname = "p%s"\n' "$index" > "$project/Cargo.toml"
    printf 'fn main() {}\n' > "$project/src/main.rs"
    # Varying but reproducible payload sizes.
    head -c $((index * 4096)) /dev/zero > "$project/target/artifact-$index.bin"
done
# A decoy: a `target` directory with no manifest must never be claimed.
mkdir -p "$DATASET/orphan/target"
head -c 8192 /dev/zero > "$DATASET/orphan/target/stray.bin"
# A hidden directory, because dotfiles are first-class observations.
mkdir -p "$DATASET/.cache"
head -c 16384 /dev/zero > "$DATASET/.cache/blob.bin"

echo "==> indexing (cold)"
COLD_START=$(python3 -c 'import time; print(time.time())')
cargo run --release --quiet -p diskgraph-cli -- \
    --data-dir "$DATA" --json scope add --root "$DATASET" > "$OUT_DIR/scope.json"
SCOPE="$(python3 -c 'import json;print(json.load(open("'"$OUT_DIR"'/scope.json"))["data"]["scope_id"])')"
cargo run --release --quiet -p diskgraph-cli -- \
    --data-dir "$DATA" --json index --scope "$SCOPE" --wait > "$OUT_DIR/index.json"
COLD_END=$(python3 -c 'import time; print(time.time())')
COLD_SECONDS=$(python3 -c "print(f'{$COLD_END - $COLD_START:.3f}')")

echo "==> verifying the expected answers"
cargo run --release --quiet -p diskgraph-cli -- \
    --data-dir "$DATA" --json explain --scope "$SCOPE" --revision "$(python3 -c 'import json;print(json.load(open("'"$OUT_DIR"'/index.json"))["data"]["revision_id"])')" --entity resource-1 > "$OUT_DIR/explain-root.json" 2>/dev/null || true
cargo run --release --quiet -p diskgraph-cli -- \
    --data-dir "$DATA" --json top --scope "$SCOPE" --limit 100 > "$OUT_DIR/top.json"
cargo run --release --quiet -p diskgraph-cli -- \
    --data-dir "$DATA" --json candidates --scope "$SCOPE" --target-bytes 1 > "$OUT_DIR/candidates.json"
cargo run --release --quiet -p diskgraph-cli -- \
    --data-dir "$DATA" --json changes --scope "$SCOPE" --before "$(python3 -c 'import json;print(json.load(open("'"$OUT_DIR"'/index.json"))["data"]["revision_id"])')" --after "$(python3 -c 'import json;print(json.load(open("'"$OUT_DIR"'/index.json"))["data"]["revision_id"])')" > "$OUT_DIR/changes-self.json"

echo "==> measuring warm queries"
for query in "top --scope $SCOPE --limit 20" "children --scope $SCOPE" "search --scope $SCOPE --pattern project"; do
    label=$(echo "$query" | cut -d' ' -f1)
    START=$(python3 -c 'import time; print(time.time())')
    # shellcheck disable=SC2086
    cargo run --release --quiet -p diskgraph-cli -- --data-dir "$DATA" --json $query > /dev/null
    END=$(python3 -c 'import time; print(time.time())')
    echo "$label $(python3 -c "print(f'{$END - $START:.3f}')")" >> "$OUT_DIR/warm-timings.txt"
done

echo "==> measuring a controlled rescan (sync)"
SYNC_START=$(python3 -c 'import time; print(time.time())')
cargo run --release --quiet -p diskgraph-cli -- \
    --data-dir "$DATA" --json sync --scope "$SCOPE" --wait > "$OUT_DIR/sync.json"
SYNC_END=$(python3 -c 'import time; print(time.time())')
SYNC_SECONDS=$(python3 -c "print(f'{$SYNC_END - $SYNC_START:.3f}')")

echo "==> scoring correctness"
python3 - "$OUT_DIR" "$SCOPE" <<'PY'
import json, pathlib, sys

out = pathlib.Path(sys.argv[1])
top = json.loads((out / "top.json").read_text())
candidates = json.loads((out / "candidates.json").read_text())
changes = json.loads((out / "changes-self.json").read_text())
index = json.loads((out / "index.json").read_text())

checks = {}

items = top["data"]["items"]
# The decoy directory is never a project candidate without a manifest.
checks["candidates_are_empty_without_rebuild_evidence"] = (
    candidates["data"]["candidates"] == [] and candidates["data"]["review_only"] is True
)
# A scan of the same revision reports no changes.
checks["self_comparison_is_clean"] = (
    changes["data"]["incompatible"] is None
    and changes["data"]["added"] == 0
    and changes["data"]["removed"] == 0
    and changes["data"]["size_changed"] == 0
)
# The index completed and reported bytes.
checks["index_completed"] = index["data"]["state"] == "completed"
checks["sizes_are_observed"] = bool(items) and items[0]["subtree_bytes"] > 0

passed = sum(1 for value in checks.values() if value)
report = {"passed": passed, "total": len(checks), "checks": checks}
(out / "correctness.json").write_text(json.dumps(report, indent=2))
print(json.dumps(report, indent=2))
if passed != len(checks):
    raise SystemExit("correctness checks failed")
PY

# Assemble the baseline record.
{
    echo "# DiskGraph local evaluation baseline"
    echo
    echo "- generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "- version:   $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
    echo "- platform:  $(uname -s) $(uname -m)"
    echo "- rustc:     $(rustc --version)"
    echo "- dataset:   12 cargo projects + 1 orphan decoy + 1 hidden cache"
    echo
    echo "## Timings (seconds, this machine, this dataset)"
    echo
    echo "| phase | seconds |"
    echo "| --- | --- |"
    echo "| cold index (scope add + full scan) | $COLD_SECONDS |"
    echo "| sync (controlled rescan) | $SYNC_SECONDS |"
    if [ -f "$OUT_DIR/warm-timings.txt" ]; then
        while read -r label seconds; do
            echo "| warm query: $label | $seconds |"
        done < "$OUT_DIR/warm-timings.txt"
    fi
    echo
    echo "## Correctness"
    echo
    echo "See correctness.json. These are dataset measurements, not product"
    echo "promises, and they are not comparable to another tool's numbers without"
    echo "the same dataset, machine, and measurement method."
} > "$OUT_DIR/BASELINE.md"

echo
echo "baseline written to $OUT_DIR/BASELINE.md"
