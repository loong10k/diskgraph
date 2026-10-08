"""冻结容量实现的真实创建依赖；不为旧容量伪造新恢复能力。"""
import re

PREFIX = "crates/diskgraph-engine/src/live_evidence/"
SUPPORT = [PREFIX + name for name in (
    "git_private_directory.rs", "git_private_allocation.rs",
    "git_private_directory_owner.rs", "mod.rs",
)]


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
    return {SUPPORT[2]: owner, SUPPORT[3]: modules}


def prepare(root, baseline, git_output):
    """返回原源码与回放源码；真实创建及分配来自同一冻结提交，调用方负责 finally 恢复。"""
    current = {name: (root / name).read_bytes() for name in SUPPORT}
    frozen = {name: git_output(["git", "show", f"{baseline}:{name}"], cwd=root)
              for name in SUPPORT[:2]}
    frozen.update(adapt_support(current))
    return current, frozen
