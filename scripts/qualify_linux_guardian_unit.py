"""仅隔离Linux runner启动真正生成的guardian socket/template；不enable，不安装桌面服务。"""
import io
import json
import os
import selectors
from pathlib import Path
import shutil
import sqlite3
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
    try:
        return subprocess.run(['systemctl', *arguments], check=True, capture_output=True,
                              text=True, encoding='utf-8', timeout=35).stdout
    except subprocess.CalledProcessError as error:
        print(('original systemctl failure: ' + (error.stdout or '') + (error.stderr or ''))[-32768:], file=sys.stderr)
        if arguments and arguments[0] in ('start', 'stop'):
            unit = arguments[-1]
            # 只观察同一失败unit，不重试start/stop，不停止或替换原资源。
            for command in (['systemctl', 'status', '--no-pager', '--full', unit],
                            ['journalctl', '--no-pager', '-n', '30', '--unit', unit]):
                try:
                    observed = subprocess.run(command, check=False, capture_output=True,
                                              text=True, encoding='utf-8', timeout=5)
                    print(((observed.stdout or '') + (observed.stderr or ''))[-32768:], file=sys.stderr)
                except (OSError, subprocess.SubprocessError) as diagnostic:
                    print('same-unit diagnostics unavailable: ' + str(diagnostic), file=sys.stderr)
        raise


def customer_query(prefix, endpoint, original_slot, arguments, expected_success=True):
    """执行实际公开CLI，随后沿同一原容量FD确认退休；不伪造扫描或私有完成。"""
    child = subprocess.Popen([str(prefix / 'images/diskgraph'), '--json', '--supervisor-socket',
                              str(endpoint), *arguments], preexec_fn=customer, env={}, cwd=prefix,
                             stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        stdout, stderr = child.communicate(timeout=35)
        reply = json.loads(stdout)
        if (child.returncode == 0) != expected_success or reply.get('ok') is not expected_success:
            raise AssertionError(('actual customer command failed', arguments, child.returncode, stdout, stderr))
        until = time.monotonic() + 5
        while os.pread(original_slot, 8, 0) != b'DGSL01C\n':
            if time.monotonic() >= until:
                raise TimeoutError('same original capacity not retired after customer command')
            time.sleep(0.005)
        return reply
    finally:
        original_dispose(child)


def qualify_scan_mainline(workspace, prefix, endpoint, original_slot):
    """实际无损注册、扫描发布、Unicode查询、重扫与拒绝入队；每步复用真实socket配置。"""
    roots = [workspace / 'scan-root-unicode',
             *[workspace / ('scan-root-' + suffix) for suffix in ('\udcff', '\udcfe')]]
    for root in roots:
        root.mkdir(mode=0o700)
        os.chown(root, 23112, 23112)
    for name in ('hello.txt', 'İSTANBUL.txt', '替换字符-\ufffd.txt'):
        (roots[0] / name).write_bytes(b'original customer scan input\n')
    for root in roots[1:]:
        (root / 'other.txt').write_bytes(b'different raw directory\n')
    scopes = [customer_query(prefix, endpoint, original_slot, ['scope', 'add', '--root', str(root)])['data']['scope_id']
              for root in roots]
    if len(set(scopes)) != len(scopes):
        raise AssertionError('distinct non-UTF-8 roots collapsed to display identity')
    indexed = customer_query(prefix, endpoint, original_slot,
                             ['index', '--scope', scopes[0], '--wait', '--max-nodes-per-scan=100',
                              '--max-staging-bytes=1048576', '--depth=2', '--apparent-size'])['data']
    if indexed['state'] != 'completed' or not indexed['revision_id']:
        raise AssertionError('actual scan did not publish completed revision')
    node = customer_query(prefix, endpoint, original_slot, ['node', '--scope', scopes[0]])['data']
    if node['revision_id'] != indexed['revision_id'] or node['node']['files'] != 3:
        raise AssertionError('query did not read actual selected raw root revision')
    found = customer_query(prefix, endpoint, original_slot,
                           ['search', '--scope', scopes[0], '--pattern', 'İSTANBUL'])['data']
    if found['revision_id'] != indexed['revision_id'] or not any(item['name'] == 'İSTANBUL.txt' for item in found['items']):
        raise AssertionError('actual Unicode search lost original name semantics')
    synced = customer_query(prefix, endpoint, original_slot,
                            ['sync', '--scope', scopes[0], '--wait', '--max-nodes-per-scan=100',
                             '--max-staging-bytes=1048576', '--depth=2', '--apparent-size'])['data']
    latest = customer_query(prefix, endpoint, original_slot, ['node', '--scope', scopes[0]])['data']
    if synced['state'] != 'completed' or synced['revision_id'] == indexed['revision_id'] or latest['revision_id'] != synced['revision_id']:
        raise AssertionError('rescan did not atomically publish new latest revision')
    # 固定上游仅保留显示名称；原始编码注册须无损，无法无损扫描时必须拒绝发布。
    # 普通主线不得靠此拒绝冒充成功，也不能删除原始路径的负向验收。
    for scope in scopes[1:]:
        refused = customer_query(prefix, endpoint, original_slot,
                                 ['index', '--scope', scope, '--wait'], expected_success=False)
        if refused.get('error', {}).get('code') != 'unsupported':
            raise AssertionError('raw root scan did not preserve explicit unsupported classification')
        unpublished = customer_query(prefix, endpoint, original_slot,
                                     ['node', '--scope', scope], expected_success=False)
        if unpublished.get('error', {}).get('code') != 'not_indexed':
            raise AssertionError('unsupported raw root published a queryable revision')
    database = prefix / 'state/data/diskgraph-control.sqlite'
    def queued_count():
        with sqlite3.connect('file:' + str(database) + '?mode=ro', uri=True) as connection:
            return connection.execute('SELECT COUNT(*) FROM jobs').fetchone()[0]
    before = queued_count()
    customer_query(prefix, endpoint, original_slot, ['index', '--scope', 'unknown-scope', '--wait'], expected_success=False)
    if queued_count() != before:
        raise AssertionError('unknown scope refusal still created a job')
    return {'passed': True, 'actual_index_and_sync_completed': True, 'distinct_raw_root_ids': True,
            'scope_id': scopes[0], 'revision_id': synced['revision_id'],
            'selected_root_files': 3, 'unicode_search': True, 'unknown_scope_did_not_enqueue': True,
            'raw_root_scan_refused_without_revision': True,
            'original_slot_retired_after_each_command': True, 'production_ready': False}


def qualify_mcp_mainline(prefix, endpoint, original_slot, scan, frontend='mcp'):
    """产品MCP前端经过原guardian出生；真实RPC与正常EOF，仍检查同一容量FD退休。"""
    command = ([str(prefix / 'images/diskgraph-mcp'), '--supervisor-socket', str(endpoint)] if frontend == 'mcp'
               else [str(prefix / 'images/diskgraph'), '--supervisor-socket', str(endpoint), 'serve'])
    child = subprocess.Popen(command,
                             preexec_fn=customer, env={}, cwd=prefix, stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    selector = selectors.DefaultSelector()
    selector.register(child.stdout, selectors.EVENT_READ, 'stdout')
    selector.register(child.stderr, selectors.EVENT_READ, 'stderr')
    pending, diagnostics = bytearray(), bytearray()
    def exchange(identity, method, parameters):
        child.stdin.write((json.dumps({'jsonrpc': '2.0', 'id': identity, 'method': method,
                                      'params': parameters}) + '\n').encode())
        child.stdin.flush()
        until = time.monotonic() + 30
        while b'\n' not in pending:
            remaining = until - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(('original MCP response observation expired', bytes(diagnostics)))
            for key, _ in selector.select(min(remaining, 0.1)):
                block = os.read(key.fileobj.fileno(), 4096)
                if not block:
                    selector.unregister(key.fileobj)
                    if key.data == 'stdout':
                        raise AssertionError(('MCP stdout ended before response', child.poll(), bytes(diagnostics)))
                    continue
                target = pending if key.data == 'stdout' else diagnostics
                target.extend(block)
                if len(pending) + len(diagnostics) > 1 << 20:
                    raise AssertionError('MCP fixture response/diagnostic budget exceeded')
        line, _, tail = pending.partition(b'\n')
        pending[:] = tail
        reply = json.loads(line)
        if reply.get('id') != identity or 'error' in reply:
            raise AssertionError(('original MCP RPC failed', reply, bytes(diagnostics)))
        return reply['result']
    try:
        initialized = exchange(1, 'initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                                                'clientInfo': {'name': 'actual-guardian-fixture', 'version': '1'}})
        if initialized.get('serverInfo', {}).get('name') != 'diskgraph':
            raise AssertionError('actual MCP server identity missing')
        tools = exchange(2, 'tools/list', {})
        if not any(tool['name'] == 'diskgraph_node' for tool in tools['tools']):
            raise AssertionError('actual MCP tool catalog missing node')
        # 超过旧有限CLI的30秒窗口后，同一原MCP会话仍须可查询；不重连或换owner。
        if frontend == 'mcp':
            time.sleep(31)
        if child.poll() is not None:
            raise AssertionError('original MCP session expired at finite CLI observation window')
        queried = exchange(3, 'tools/call', {'name': 'diskgraph_node', 'arguments':
                                           {'scope': scan['scope_id'], 'revision': scan['revision_id']}})
        data = queried.get('structuredContent', {})
        if queried.get('isError') is not False or data.get('revision_id') != scan['revision_id'] or data.get('node', {}).get('files') != 3:
            raise AssertionError(('MCP did not query actual customer revision', queried))
        child.stdin.close()
        child.stdin = None
        stdout, stderr = child.communicate(timeout=35)
        if child.returncode != 0 or pending or stdout:
            raise AssertionError(('MCP original EOF did not finish normally', child.returncode, stdout, stderr))
        until = time.monotonic() + 5
        while os.pread(original_slot, 8, 0) != b'DGSL01C\n':
            if time.monotonic() >= until:
                raise TimeoutError('same original MCP executor capacity not retired')
            time.sleep(0.005)
        return {'passed': True, 'actual_initialize_and_catalog': True, 'actual_revision_query': True,
                'frontend': frontend, 'idle_seconds_before_query': 31 if frontend == 'mcp' else 0,
                'normal_input_eof': True, 'original_slot_retired': True, 'production_ready': False}
    finally:
        selector.close()
        original_dispose(child)


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
        scan_mainline = qualify_scan_mainline(workspace, prefix, endpoint, original_slot)
        mcp_mainline = qualify_mcp_mainline(prefix, endpoint, original_slot, scan_mainline)
        cli_mcp_mainline = qualify_mcp_mainline(prefix, endpoint, original_slot, scan_mainline, frontend='cli')
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
                'mcp_qualified': False, 'scan_mainline': scan_mainline,
                'guardian_stdio_mainline': mcp_mainline,
                'guardian_cli_serve_mainline': cli_mcp_mainline, 'production_ready': False}
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
