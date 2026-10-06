#!/usr/bin/env python3
"""复用已验收原namespace owner，只开放当前Engine八案例的固定入口。"""
import hashlib
import importlib.util
from pathlib import Path
import sys

SOURCE = Path(__file__).with_name('run-native-pid-namespace.py')
SPEC = importlib.util.spec_from_file_location('engine_namespace_shared', SOURCE)
SHARED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SHARED)
# 独立加载实例的固定表，不改变已绑定历史探针的源码或接收其它资格脚本。
SHARED.PROFILES = {'qualify-linux-scan-worker-engine.py': 8}


class EngineNamespaceSupervisor(SHARED.NativeNamespaceSupervisor):
    """保留原init/pidfd/凭据/实际wait，只额外固定Engine脚本来源。"""

    def __init__(self, args):
        super().__init__(args)
        self.receipt['engine_namespace_entry_sha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()

    @staticmethod
    def command(args, output):
        command = ORIGINAL.command(args, output)
        if not args.self_test and Path(command[1]).resolve() != Path(__file__).with_name('qualify-linux-scan-worker-engine.py').resolve():
            raise ValueError('exact fixed Engine qualifier path is required')
        return command


# main原样使用共享parser和固定self-test；保存旧类供委托，避免覆盖后递归。
ORIGINAL = SHARED.NativeNamespaceSupervisor
SHARED.NativeNamespaceSupervisor = EngineNamespaceSupervisor

if __name__ == '__main__':
    sys.exit(SHARED.main())
