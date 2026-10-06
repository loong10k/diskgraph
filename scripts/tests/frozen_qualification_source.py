"""消费固定历史工具源码；不修改历史清单，也不从当前内容建立预期摘要。"""
import hashlib
from pathlib import Path
import shutil
import zipfile

ARCHIVE = Path(__file__).with_name("fixtures") / "linux_qualification_sources.zip"

def copy_source(root, target, binding):
    """按原清单复制源码；历史固定文件必须仍与原摘要一致，未知漂移直接失败。"""
    name = binding["path"]
    with zipfile.ZipFile(ARCHIVE) as archive:
        if name in archive.namelist():
            data = archive.read(name)
            expected = binding.get("sha256")
            if expected is None or hashlib.sha256(data).hexdigest() != expected:
                raise ValueError(f"frozen qualification source digest mismatch: {name}")
            target.write_bytes(data)
            return
    shutil.copyfile(root / name, target)
