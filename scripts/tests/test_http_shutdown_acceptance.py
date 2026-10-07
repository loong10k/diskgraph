"""Unix 原生验收必须拒绝信号死亡，且保留真实子进程回收。"""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest

SPEC = importlib.util.spec_from_file_location(
    'http_acceptance', Path(__file__).resolve().parents[1] / 'accept-readonly-http.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


@unittest.skipIf(sys.platform == 'win32', 'Windows terminate 不提供 Unix 正常信号协议')
class ShutdownAcceptance(unittest.TestCase):
    def test_actual_unresponsive_child_is_reaped_but_rejected(self):
        child = subprocess.Popen([sys.executable, '-c',
            'import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); '
            'print("ready", flush=True); time.sleep(30)'],
            stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), 'ready')
            with self.assertRaisesRegex(RuntimeError, 'forced cleanup is not acceptance'):
                MODULE.stop_server(child)
            self.assertIsNotNone(child.poll())
            self.assertNotEqual(child.returncode, 0)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            child.stdout.close()

    def test_actual_handled_signal_is_success(self):
        child = subprocess.Popen([sys.executable, '-c',
            'import signal,time,sys; signal.signal(signal.SIGTERM, lambda *_: sys.exit(0)); '
            'print("ready", flush=True); time.sleep(30)'],
            stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), 'ready')
            MODULE.stop_server(child)
            self.assertEqual(child.returncode, 0)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            child.stdout.close()

    def test_actual_signal_death_is_not_success(self):
        child = subprocess.Popen([sys.executable, '-c',
                                  'import time; print("ready", flush=True); time.sleep(30)'],
                                 stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), 'ready')
            with self.assertRaisesRegex(RuntimeError, 'normal shutdown'):
                MODULE.stop_server(child)
            self.assertIsNotNone(child.poll())
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            child.stdout.close()
