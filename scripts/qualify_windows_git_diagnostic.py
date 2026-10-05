"""仅隔离 CI 的 Windows Git 源状态诊断组装，不改变产品拒绝或预算。"""
import hashlib
import json
import pathlib
import subprocess
import sys


def replace_exact(text, old, new, count):
    if text.count(old) != count:
        raise ValueError(f"qualification source shape changed: expected {count} matches")
    return text.replace(old, new)


def assemble(root, output):
    engine = root / "crates/diskgraph-engine/src"
    live = engine / "live_evidence"
    originals = {}
    pending = {}
    output.mkdir(parents=True, exist_ok=True)
    (output / "setup.json").write_text(json.dumps({"stage": "source assembly", "security_acceptance": False}) + "\n", encoding="utf-8")

    def edit(path, transform):
        before = pending.get(path, path.read_text(encoding="utf-8"))
        originals.setdefault(str(path.relative_to(root)), hashlib.sha256(path.read_bytes()).hexdigest())
        pending[path] = transform(before)

    # 仅使现有固定 TLS 诊断在依赖库的 integration 构建中可见。
    for name, count in [("mod.rs", 2), ("git_scope_boundary.rs", 4),
                        ("git_worktree_capture.rs", 3), ("git_source_file.rs", 3)]:
        edit(live / name, lambda s, n=count: replace_exact(
            s, "#[cfg(all(test, windows))]", "#[cfg(windows)]", n))
    edit(live / "git_source_windows.rs", lambda s: replace_exact(
        s, "#[cfg(test)]", "#[cfg(windows)]", 3))
    edit(engine / "windows_file_state.rs", lambda s: replace_exact(
        s, "    #[cfg(test)]\n    pub(crate) fn changed_mask", "    pub(crate) fn changed_mask", 1))
    session = live / "evidence_probe_session.rs"
    # 限定 indexed 方法，不触碰其它公共采样 API。
    def indexed_guard(s):
        begin = s.index("    pub(crate) fn sample_git_indexed(")
        end = s.index("    /// 在任务剩余额度内", begin)
        part = s[begin:end]
        part = replace_exact(part, "        let result = (|| {", "        #[cfg(windows)]\n        let diagnostic = super::git_source_windows_diagnostic::GitSourceWindowsDiagnostic::new();\n        let result = (|| {", 1)
        part = replace_exact(part, "        match result {", "        #[cfg(windows)]\n        diagnostic.report();\n        match result {", 1)
        return s[:begin] + part + s[end:]
    edit(session, indexed_guard)
    # 保留原一次 state 调用及比较，补 drive 分支有限差异位。
    def drive(s):
        old = '    if state(&file, true)? != initial {\n        return Err("scoped Git drive changed".into());\n    }'
        new = '    let reopened = state(&file, true)?;\n    if reopened != initial {\n        GitSourceWindowsDiagnostic::record(&initial, &reopened, true, "drive_reopened");\n        return Err("scoped Git drive changed".into());\n    }'
        return replace_exact(s, old, new, 1)
    edit(live / "git_source_windows.rs", drive)
    for name in ["git_source_windows_diagnostic.rs", "git_source_windows_phase.rs"]:
        path = live / name
        originals[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
    for path, text in pending.items():
        path.write_text(text, encoding="utf-8")
    (output / "diagnostic.patch").write_bytes(subprocess.check_output([
        "git", "diff", "--", *originals], cwd=root))
    record = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
              "scope": "Isolation-only TLS observation, no security qualification",
              "security_acceptance": False,
              "original_source_sha256": originals,
              "instrumented_source_sha256": {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in originals}}
    (output / "provenance.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    assemble(pathlib.Path(sys.argv[1]).resolve(), pathlib.Path(sys.argv[2]).resolve())
