"""区分配对门禁实际失败的一侧，避免候选失败误触发旧基线重扫。"""
import argparse
import json
import re
from pathlib import Path


def failure_side(receipt):
    """返回原回执中的失败侧；不把分类当作性能验收成功。"""
    if not isinstance(receipt, dict):
        return "unknown"
    if receipt.get("status") in ("passed", "running", "aborted"):
        return "none"
    if receipt.get("status") != "failed":
        return "unknown"
    commands = receipt.get("commands")
    if not isinstance(commands, list) or not commands or not isinstance(commands[-1], dict):
        return "unknown"
    label = commands[-1].get("label")
    if not isinstance(label, str):
        return "unknown"
    if label == "baseline-build":
        return "baseline"
    if label in ("candidate-build", "candidate-worker-build"):
        return "candidate"
    phase = re.fullmatch(r"[0-9]+-(?:wide|deep)-round[12]-(baseline|candidate)", label)
    return phase.group(1) if phase else "unknown"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--github-output", type=Path, required=True)
    args = parser.parse_args()
    try:
        side = failure_side(json.loads(args.receipt.read_text()))
    except (OSError, ValueError, TypeError):
        side = "unknown"
    with args.github_output.open("a", encoding="utf-8") as output:
        output.write(f"side={side}\n")
    print(f"paired scan failed side: {side}; original gate result retained")


if __name__ == "__main__":
    main()
