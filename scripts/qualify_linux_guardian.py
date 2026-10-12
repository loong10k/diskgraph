"""隔离root域验证真实单连接guardian和客户CLI；不安装服务，不把失败处置记为退休。"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile

from qualify_linux_public_broker import prepare, original_dispose


def customer():
    """固定夹具身份；真实辅助组也必须进入内核连接凭据。"""
    os.setgroups([23114, 23115])
    os.setgid(23112)
    os.setuid(23112)


def qualify(args, disconnect=False):
    """返回真实CLI、原槽和原guardian正常wait结果；异常不删除未确认安装树。"""
    if sys.platform != 'linux' or os.geteuid() != 0 or os.environ.get('DG_ISOLATED_ROOT_ACCEPTANCE') != '1':
        raise RuntimeError('isolated Linux root qualification only')
    root = Path(tempfile.mkdtemp(prefix='dg-guardian-', dir='/var/lib'))
    root.chmod(0o755)
    success = False
    guardian = frontend = None
    result = {'scope': 'accepted Unix connection -> customer doctor CLI -> original wait',
              'production_ready': False, 'mcp_qualified': False, 'systemd_qualified': False,
              'scan_qualified': False, 'services_installed': False}
    try:
        state, data, config = prepare(root, args)
        # 与旧service executor布局隔离：客户数据库由真实客户持有，两个代码镜像只能执行。
        os.chown(data, 23112, 23112)
        (root / 'diskgraph').chmod(0o511)
        (root / 'worker').chmod(0o511)
        helper = root / 'wire_client.py'
        shutil.copyfile(Path(__file__).with_name('qualify_linux_broker_wire_client.py'), helper)
        helper.chmod(0o555)
        bootstrap = os.open(state / 'bootstrap.slot', os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            original = os.fstat(bootstrap)
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                # 必须先于客户connect/send启用；不能事后补凭据或转交其他客户连接。
                listener.setsockopt(socket.SOL_SOCKET, socket.SO_PASSCRED, 1)
                listener.bind(config['socket_path'])
                os.chmod(config['socket_path'], 0o666)
                listener.listen(1)
                listener.settimeout(30)
                frontend = subprocess.Popen([sys.executable, str(helper), config['socket_path'],
                                             'disconnect' if disconnect else 'doctor'],
                                            preexec_fn=customer, env={}, cwd=root,
                                            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                stream, _ = listener.accept()
                with stream:
                    guardian = subprocess.Popen([str(root / 'broker'), '--config', str(root / 'config.json'),
                                                 '--connection'], stdin=stream, env={}, cwd=root,
                                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                output, errors = frontend.communicate(timeout=35)
                if frontend.returncode != 0:
                    raise AssertionError(('customer query failed', output, errors))
                row = json.loads(output)
                guardian_output, guardian_errors = guardian.communicate(timeout=35)
                if guardian.returncode not in ((0, 10) if disconnect else (0,)):
                    raise AssertionError(('original guardian failed', guardian_output, guardian_errors))
                if os.pread(bootstrap, 8, 0) != b'DGSL01C\n':
                    raise AssertionError('original bootstrap not CLEAN after original guardian wait')
                current = os.fstat(bootstrap)
                if (original.st_dev, original.st_ino) != (current.st_dev, current.st_ino):
                    raise AssertionError('original bootstrap changed')
                databases = list(data.glob('*.sqlite*'))
                if not databases or any(path.stat().st_uid != 23112 for path in databases):
                    raise AssertionError('databases must be created by real customer executor')
                if (state / 'uid_23111.slot').read_bytes() != b'DGSL01C\n':
                    raise AssertionError('customer executor touched legacy service capacity')
                result.update(passed=True, customer=row, customer_database_uid=23112,
                              original_guardian_waited=True, original_bootstrap_clean=True,
                              legacy_service_slot_untouched=True, public_disconnected=disconnect)
                success = True
                return result
        finally:
            os.close(bootstrap)
    finally:
        # 超时只处置夹具，保留安装树/ACTIVE供诊断；从不将kill/wait当产品恢复证据。
        for child in (frontend, guardian):
            if child is not None:
                original_dispose(child)
        if success:
            shutil.rmtree(root)
        else:
            print('unconfirmed fixture retained: ' + str(root), file=sys.stderr)


def main():
    parser = argparse.ArgumentParser()
    for option in ('cli', 'worker', 'broker', 'receipt'):
        parser.add_argument('--' + option, type=Path, required=True)
    args = parser.parse_args()
    result = {'cases': [qualify(args), qualify(args, disconnect=True)], 'production_ready': False}
    args.receipt.write_text(json.dumps(result, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
