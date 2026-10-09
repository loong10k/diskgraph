"""冻结容量实现的真实创建依赖；不为旧容量伪造新恢复能力。"""
import re

PREFIX = "crates/diskgraph-engine/src/live_evidence/"
SUPPORT = [PREFIX + name for name in (
    "git_private_directory.rs", "git_private_allocation.rs",
    "git_private_directory_owner.rs", "mod.rs",
)]
BUDGET = PREFIX + "probe_budget.rs"
SUMMARY_DECLARATION = b"#[cfg(all(test, windows))]\nmod git_private_write_summary;\n"


def adapt_support(current):
    """仅剔除旧容量不存在的恢复调用及不可达诊断模块；保留现有期限接口。"""
    owner = current[SUPPORT[2]]
    pattern = (rb"(?m)^([ ]+)if let Some\(capacity\) = self\.capacity\.as_mut\(\) \{\r?\n"
               rb"\1    capacity\.recover_created_entry\(\)\?;\r?\n\1}\r?\n")
    owner, count = re.subn(pattern, b"", owner)
    if count != 2:
        raise RuntimeError("frozen capacity creation recovery call boundaries changed")
    modules = current[SUPPORT[3]].replace(b"\r\n", b"\n")
    for declaration in (
        b"#[cfg(windows)]\nmod git_private_created_entry;\n",
        b"#[cfg(all(test, windows))]\nmod git_private_write_profile;\n",
    ):
        if modules.count(declaration) != 1:
            raise RuntimeError("frozen capacity unreachable creation module is not unique")
        modules = modules.replace(declaration, b"", 1)
    adapted = {SUPPORT[2]: owner, SUPPORT[3]: modules}
    if b"mod git_private_write_summary;" in modules:
        # 旧创建路径不产生阶段计时；汇总及其预算槽位必须成对移除，原期限正文逐字保留。
        if modules.count(SUMMARY_DECLARATION) != 1:
            raise RuntimeError("frozen capacity diagnostic summary module is not unique")
        budget = current[BUDGET]
        for declaration in (
            b"    #[cfg(all(test, windows))]\n"
            b"    _write_summary: Option<super::git_private_write_summary::GitPrivateWriteSummary>,\n",
            b"            #[cfg(all(test, windows))]\n"
            b"            _write_summary: super::git_private_write_summary::GitPrivateWriteSummary::new(),\n",
        ):
            if budget.count(declaration) != 1:
                raise RuntimeError("frozen capacity diagnostic budget boundary is not unique")
            budget = budget.replace(declaration, b"", 1)
        adapted[SUPPORT[3]] = modules.replace(SUMMARY_DECLARATION, b"", 1)
        adapted[BUDGET] = budget
    return adapted


def prepare(root, baseline, git_output):
    """返回原源码与回放源码；真实创建及分配来自同一冻结提交，调用方负责 finally 恢复。"""
    current = {name: (root / name).read_bytes() for name in SUPPORT}
    if b"mod git_private_write_summary;" in current[SUPPORT[3]]:
        current[BUDGET] = (root / BUDGET).read_bytes()
    frozen = {name: git_output(["git", "show", f"{baseline}:{name}"], cwd=root)
              for name in SUPPORT[:2]}
    frozen.update(adapt_support(current))
    return current, frozen
