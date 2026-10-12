"""仅隔离Linux runner启动真正生成的guardian socket/template；不enable，不安装桌面服务。"""
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

from qualify_linux_guardian import customer, install_candidate
from qualify_linux_public_broker import original_dispose
from prepare_linux_supervisor_installation import verify_root_directory, write_at


def systemctl(*arguments):
    """只操作本夹具的明确unit；返回原命令结果，不重试启动或延长工作期限。"""
    return subprocess.run(['systemctl', *arguments], check=True, capture_output=True,
                          text=True, encoding='utf-8', timeout=35).stdout


def qualify_unit(args):
    """实际安装/启动/客户查询/原槽退休/停止；任一失败保留原树与unit以供诊断。"""
    if sys.platform != 'linux' or os.geteuid() != 0 or os.environ.get('DG_ISOLATED_ROOT_ACCEPTANCE') != '1':
        raise RuntimeError('isolated Linux root unit qualification only')
    workspace = Path(tempfile.mkdtemp(prefix='dg-guardian-unit-', dir='/var/lib'))
    workspace.chmod(0o755)
    unit_base = 'dg-guardian-isolated-' + uuid.uuid4().hex
    socket_name, template_name = unit_base + '.socket', unit_base + '@.service'
    directory = os.open('/run/systemd/system', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    installed, original_slot, frontend, completed = {}, None, None, False
    try:
        verify_root_directory(directory)
        prefix = install_candidate(workspace, args)
        for name, source in ((socket_name, 'diskgraph-guardian.socket'),
                             (template_name, 'diskgraph-guardian@.service')):
            # 原生成字节直接进入随机同名前缀的socket/template，不修改其安全或停止策略。
            installed[name] = write_at(directory, name, io.BytesIO((prefix / source).read_bytes()), 4096, 0o644)
        os.fsync(directory)
        subprocess.run(['systemd-analyze', 'verify', '/run/systemd/system/' + socket_name,
                        '/run/systemd/system/' + template_name], check=True, capture_output=True, timeout=20)
        original_slot = os.open(prefix / 'state/bootstrap.slot', os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        original_identity = os.fstat(original_slot)
        systemctl('daemon-reload')
        systemctl('start', socket_name)
        config = json.loads((prefix / 'supervisor-config.json').read_text())
        endpoint = Path(config['socket_path'])
        until = time.monotonic() + 30
        while not endpoint.exists():
            if time.monotonic() >= until:
                raise TimeoutError('original generated guardian socket did not bind')
            time.sleep(0.005)
        info = endpoint.stat()
        if info.st_uid != 23112 or info.st_mode & 0o777 != 0o600:
            raise AssertionError('actual socket must be private to admitted customer')
        frontend = subprocess.Popen([str(prefix / 'images/diskgraph'), '--json', '--supervisor-socket',
                                     str(endpoint), 'doctor'], preexec_fn=customer, env={}, cwd=prefix,
                                    stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        stdout, stderr = frontend.communicate(timeout=35)
        if frontend.returncode != 0 or json.loads(stdout).get('ok') is not True:
            raise AssertionError(('actual customer frontend failed', stdout, stderr))
        until = time.monotonic() + 5
        while os.pread(original_slot, 8, 0) != b'DGSL01C\n':
            if time.monotonic() >= until:
                raise TimeoutError('original guardian did not retire original slot')
            time.sleep(0.005)
        systemctl('stop', socket_name)
        if os.pread(original_slot, 8, 0) != b'DGSL01C\n':
            raise AssertionError('stopping socket changed original slot')
        # 原实例只允许实际MainPID已结束；GC后的实例不在列表中，不伪造它的exit status。
        until = time.monotonic() + 5
        while True:
            instances = systemctl('list-units', '--all', '--plain', '--no-legend', unit_base + '@*.service')
            active = False
            for line in instances.splitlines():
                if not line.strip():
                    continue
                name = line.split()[0]
                state = systemctl('show', name, '--property=MainPID,Result')
                values = dict(item.split('=', 1) for item in state.splitlines() if '=' in item)
                if values.get('Result') != 'success':
                    raise AssertionError(('original instance not normally retired', name, state))
                active |= values.get('MainPID') != '0'
            if not active:
                break
            if time.monotonic() >= until:
                raise TimeoutError('original guardian instance still live after retirement')
            time.sleep(0.005)
        current = os.fstat(original_slot)
        if (current.st_dev, current.st_ino) != (original_identity.st_dev, original_identity.st_ino):
            raise AssertionError('original capacity object changed')
        databases = list((prefix / 'state/data').glob('*.sqlite*'))
        if not databases or any(path.stat().st_uid != 23112 for path in databases):
            raise AssertionError('actual unit did not run customer executor')
        for name, identity in installed.items():
            actual = os.stat(name, dir_fd=directory, follow_symlinks=False)
            if (actual.st_dev, actual.st_ino) != (identity['device'], identity['inode']):
                raise AssertionError('original isolated unit file was replaced')
        for name in installed:
            os.unlink(name, dir_fd=directory)
        os.fsync(directory)
        systemctl('daemon-reload')
        completed = True
        return {'passed': True, 'actual_generated_socket_started': True,
                'actual_generated_template_used': True, 'actual_customer_cli_exit': 0,
                'customer_database_uid': 23112, 'original_slot_clean_before_stop': True,
                'socket_stopped_after_retirement': True, 'boot_enable_performed': False,
                'desktop_service_installed': False, 'pending_stop_qualified': False,
                'mcp_qualified': False, 'scan_qualified': False, 'production_ready': False}
    finally:
        if frontend is not None:
            original_dispose(frontend)
        if original_slot is not None:
            os.close(original_slot)
        os.close(directory)
        if completed:
            shutil.rmtree(workspace)
        else:
            print('unconfirmed guardian unit retained: ' + unit_base + ' at ' + str(workspace), file=sys.stderr)
