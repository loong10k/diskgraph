#!/usr/bin/env python3
"""诊断 Unix 继承 socket 的创建期身份；不证明受信监督出生或 IPC 认证。"""
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys


def peer_pid(stream):
    """读取真实内核凭据；不将创建期 PID 推定为继承后持有者。"""
    if sys.platform == "linux":
        return struct.unpack("3i", stream.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))[0]
    if sys.platform == "darwin":
        # Darwin libc 的 SOL_LOCAL/LOCAL_PEERPID；只用于本诊断，不作为生产认证策略。
        return struct.unpack("i", stream.getsockopt(0, 2, 4))[0]
    raise RuntimeError("unsupported diagnostic platform")


def main():
    """真实子进程仅读取原继承 socket；有界等待失败时显式 kill/wait 原 child。"""
    if len(sys.argv) == 3 and sys.argv[1] == "--credential-child":
        with socket.socket(fileno=int(sys.argv[2])) as stream:
            print(json.dumps({"actual_child_pid": os.getpid(), "inherited_peer_pid": peer_pid(stream)}))
        return
    if len(sys.argv) != 1:
        raise RuntimeError("unsupported diagnostic arguments")
    left, right = socket.socketpair()
    with left, right:
        child = subprocess.Popen(
            [sys.executable, str(Path(__file__).resolve()), "--credential-child", str(right.fileno())],
            pass_fds=(right.fileno(),), stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        try:
            parent_peer = peer_pid(left)
            stdout, stderr = child.communicate(timeout=5)
        except BaseException:
            child.kill()
            child.wait()
            raise
        if child.returncode or len(stdout) > 4096 or len(stderr) > 4096:
            raise RuntimeError("credential diagnostic child failed")
        observation = json.loads(stdout)
        if observation["actual_child_pid"] != child.pid:
            raise RuntimeError("unexpected original child PID")
        report = {
            "platform": sys.platform, "pair_creator_pid": os.getpid(),
            "parent_observed_peer_pid": parent_peer, "child_observation": observation,
            "parent_credential_identifies_inheriting_child": parent_peer == child.pid,
            "qualification": "diagnostic only; no trusted supervisor birth or authentication proof",
        }
        print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
