"""Windows 验收进程树的 Job 所有权；不提供跨用户安全隔离。"""

import ctypes
from ctypes import wintypes
import pathlib
import runpy
import subprocess
import sys
import time


class Accounting(ctypes.Structure):
    """Job 基础进程计数；字段布局对应 Windows SDK。"""
    _fields_ = [(name, ctypes.c_longlong) for name in
                ('user', 'kernel', 'period_user', 'period_kernel')] + [
        (name, wintypes.DWORD) for name in
        ('faults', 'total', 'active', 'terminated')]


class BasicLimits(ctypes.Structure):
    """SDK 的 Job 基础限制布局。"""
    _fields_ = [('process_time', ctypes.c_longlong), ('job_time', ctypes.c_longlong),
                ('flags', wintypes.DWORD), ('minimum', ctypes.c_size_t),
                ('maximum', ctypes.c_size_t), ('processes', wintypes.DWORD),
                ('affinity', ctypes.c_size_t), ('priority', wintypes.DWORD),
                ('scheduling', wintypes.DWORD)]


class ExtendedLimits(ctypes.Structure):
    """SDK 的扩展限制布局，用于关闭最后句柄时终止整个 Job。"""
    _fields_ = [('basic', BasicLimits), ('io', ctypes.c_ulonglong * 6),
                ('process_memory', ctypes.c_size_t), ('job_memory', ctypes.c_size_t),
                ('peak_process', ctypes.c_size_t), ('peak_job', ctypes.c_size_t)]


class WindowsJob:
    """调用者持有原 Job 句柄，并在删除镜像前确认整个 Job 退出。"""
    def __init__(self):
        if sys.platform != 'win32':
            raise OSError('Windows Job requires Windows')
        self.api = ctypes.WinDLL('kernel32', use_last_error=True)
        signatures = {
            'CreateJobObjectW': ([ctypes.c_void_p, wintypes.LPCWSTR], wintypes.HANDLE),
            'GetCurrentProcess': ([], wintypes.HANDLE),
            'AssignProcessToJobObject': ([wintypes.HANDLE, wintypes.HANDLE], wintypes.BOOL),
            'DuplicateHandle': ([wintypes.HANDLE, wintypes.HANDLE, wintypes.HANDLE,
                                 ctypes.POINTER(wintypes.HANDLE), wintypes.DWORD,
                                 wintypes.BOOL, wintypes.DWORD], wintypes.BOOL),
            'SetInformationJobObject': ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                          wintypes.DWORD], wintypes.BOOL),
            'CloseHandle': ([wintypes.HANDLE], wintypes.BOOL),
            'TerminateJobObject': ([wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            'QueryInformationJobObject': ([wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                           wintypes.DWORD, ctypes.c_void_p], wintypes.BOOL),
        }
        for name, (arguments, result) in signatures.items():
            function = getattr(self.api, name)
            function.argtypes, function.restype = arguments, result
        self.handle = self.api.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = ExtendedLimits()
        limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE；不允许 breakaway。
        try:
            self.check(self.api.SetInformationJobObject(self.handle, 9,
                       ctypes.byref(limits), ctypes.sizeof(limits)))
        except BaseException:
            self.close()
            raise

    def check(self, result):
        """传播原生失败，不将未知状态当成已回收。"""
        if not result:
            raise ctypes.WinError(ctypes.get_last_error())

    def duplicate(self):
        """只复制此 Job 的可继承句柄，不按名称或 PID 重新发现对象。"""
        handle = wintypes.HANDLE()
        process = self.api.GetCurrentProcess()
        self.check(self.api.DuplicateHandle(process, self.handle, process,
                                            ctypes.byref(handle), 0, True, 2))
        return handle.value

    def active(self):
        """返回此原 Job 的活动进程数，查询失败直接拒绝回收。"""
        state = Accounting()
        self.check(self.api.QueryInformationJobObject(self.handle, 1,
                   ctypes.byref(state), ctypes.sizeof(state), None))
        return state.active

    def retire(self, deadline):
        """终止整棵受控 Job；在独立清理期限内确认活动数归零。"""
        self.check(self.api.TerminateJobObject(self.handle, 1))
        while self.active():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError('Windows acceptance Job retirement unconfirmed')
            time.sleep(min(0.01, remaining))

    def close(self):
        """释放当前拥有的句柄；失败不得伪装成功。"""
        if self.handle:
            self.check(self.api.CloseHandle(self.handle))
            self.handle = None


def run_owned(command, **kwargs):
    """保持原执行期限；先确认 Job 退休，再有界收集包装器管道。"""
    timeout = kwargs.pop('timeout')
    deadline = time.monotonic() + timeout
    if kwargs.pop('capture_output', False):
        kwargs['stdout'] = subprocess.PIPE
        kwargs['stderr'] = subprocess.PIPE
    job = WindowsJob()
    inherited = None
    process = None
    drained = False
    try:
        inherited = job.duplicate()
        startup = subprocess.STARTUPINFO()
        startup.lpAttributeList = {'handle_list': [inherited]}
        wrapped = [sys.executable, str(pathlib.Path(__file__).resolve()),
                   str(inherited), *command[1:]]
        try:
            process = subprocess.Popen(wrapped, startupinfo=startup, close_fds=True, **kwargs)
            stdout, stderr = process.communicate(timeout=max(0, deadline - time.monotonic()))
            drained = True
        except BaseException as failure:
            cleanup_deadline = time.monotonic() + 5
            uncertainties = []
            try:
                # 先退休 Job，再等待管道；失败也独立处理尚未入 Job 的原包装器。
                job.retire(cleanup_deadline)
            except BaseException as cleanup:
                uncertainties.append(cleanup)
            if process is not None:
                try:
                    if process.poll() is None:
                        process.kill()
                    output, errors = process.communicate(
                        timeout=max(0, cleanup_deadline - time.monotonic()))
                    drained = True
                    if isinstance(failure, subprocess.TimeoutExpired):
                        failure.stdout, failure.stderr = output, errors
                except BaseException as cleanup:
                    uncertainties.append(cleanup)
            for cleanup in uncertainties:
                note = f'Job retirement or pipe drain unconfirmed: {cleanup}'
                failure.add_note(note)
                print(note, file=sys.stderr, flush=True)
            raise
        else:
            job.retire(time.monotonic() + 5)
            return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
    finally:
        primary = sys.exc_info()[1]
        errors = []
        actions = []
        if inherited is not None:
            actions.append(lambda: job.check(job.api.CloseHandle(inherited)))
        actions.append(job.close)
        if process is not None and drained:
            actions.extend(pipe.close for pipe in (process.stdout, process.stderr)
                           if pipe is not None)
        # 每个原资源均独立尝试清理；清理失败不能替换正在传播的主异常。
        for action in actions:
            try:
                action()
            except BaseException as cleanup:
                errors.append(cleanup)
        if errors:
            if primary is None:
                primary = errors.pop(0)
                for cleanup in errors:
                    primary.add_note(f'owner release unconfirmed: {cleanup}')
                raise primary
            for cleanup in errors:
                note = f'owner release unconfirmed: {cleanup}'
                primary.add_note(note)
                print(note, file=sys.stderr, flush=True)


def main():
    """可信包装器：继承原 Job 句柄，先绑定自身，再执行验收脚本。"""
    job = WindowsJob()
    try:
        inherited = int(sys.argv[1])
        job.check(job.api.AssignProcessToJobObject(inherited, job.api.GetCurrentProcess()))
        job.check(job.api.CloseHandle(inherited))
    finally:
        job.close()
    script = sys.argv[2]
    sys.argv = sys.argv[2:]
    sys.path.insert(0, str(pathlib.Path(script).resolve().parent))
    runpy.run_path(script, run_name='__main__')


if __name__ == '__main__':
    main()
