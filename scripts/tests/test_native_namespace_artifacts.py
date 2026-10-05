"""真实本地文件竞态回归；不代表 Linux namespace 或 root 身份验收。"""
import argparse
import hashlib
import os
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
SUPERVISOR = runpy.run_path(str(SCRIPTS / 'run-native-pid-namespace.py'))['NativeNamespaceSupervisor']
ARTIFACTS = runpy.run_path(str(SCRIPTS / 'native_namespace_artifacts.py'))['NativeNamespaceArtifacts']
RUN = runpy.run_path(str(SCRIPTS / 'native_pid_namespace_run.py'))['NativeNamespaceRun']


class NamespaceArtifactContracts(unittest.TestCase):
    """使用真实 symlink 和打开句柄，检查不读写替换目标。"""

    def supervisor(self, output):
        return SUPERVISOR(argparse.Namespace(output_dir=output))

    def test_receipt_symlink_never_truncates_target(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            victim = output / 'victim'
            victim.write_bytes(b'preserve original bytes')
            (output / 'namespace-receipt.json').symlink_to(victim)
            owner = self.supervisor(output)
            owner.save()
            self.assertEqual(victim.read_bytes(), b'preserve original bytes')
            self.assertFalse((output / 'namespace-receipt.json').is_symlink())

    def test_log_digest_uses_original_descriptor_after_path_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            owner = self.supervisor(output)
            path = output / 'child.stdout'
            stream = path.open('w+b')
            self.addCleanup(stream.close)
            stream.write(b'original capture')
            stream.flush()
            path.unlink()
            victim = output / 'replacement'
            victim.write_bytes(b'replacement capture')
            path.symlink_to(victim)
            owner.logs['stdout'] = [stream, len(b'original capture')]
            RUN.finalize(owner)
            self.assertEqual(owner.receipt['stdout_sha256'], hashlib.sha256(b'original capture').hexdigest())
            self.assertTrue(stream.closed)

    def test_runner_receipt_symlink_is_rejected_before_read(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            owner = self.supervisor(output)
            owner.args.self_test = False
            owner.setup.extend(b'{"phase":"ready","pid_namespace":"model-private"}\n')
            owner.receipt["outer_pid_namespace"] = "model-outer"
            owner.receipt.update(init_wait_code=1, init_wait_status=0)
            workspace = output / 'runner/qualification'
            workspace.mkdir(parents=True)
            # 旧路径也放同一诱饵，保证失败来自拒绝链接而非路径缺失。
            old = output / 'qualification'
            old.mkdir()
            victim = output / 'victim'
            victim.write_bytes(b'not a trusted receipt')
            (workspace / 'receipt.json').symlink_to(victim)
            (old / 'receipt.json').symlink_to(victim)
            with patch.object(os, "CLD_EXITED", 1, create=True), self.assertRaises(OSError):
                owner.qualify()

    def test_fifo_receipt_is_rejected_without_waiting_for_writer(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / 'receipt'
            os.mkfifo(path)
            with self.assertRaises(ValueError):
                ARTIFACTS.bounded_read(path, 1024)

    def test_parent_directory_link_is_rejected_before_target_read(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory).resolve()
            target = output / 'real'
            target.mkdir()
            (target / 'receipt').write_bytes(b'private target')
            (output / 'alias').symlink_to(target, target_is_directory=True)
            with self.assertRaises(OSError):
                ARTIFACTS.bounded_read(output / 'alias/receipt', 1024)

    def test_second_finalization_preserves_original_capture_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            owner = self.supervisor(output)
            stream = (output / 'child.stdout').open('w+b')
            self.addCleanup(stream.close)
            stream.write(b'actual capture')
            owner.logs['stdout'] = [stream, len(b'actual capture')]
            RUN.finalize(owner)
            expected = owner.receipt['stdout_sha256']
            RUN.finalize(owner)
            self.assertEqual(owner.receipt['stdout_sha256'], expected)
            self.assertIsNone(owner.primary)

    def test_renamed_record_directory_keeps_receipt_on_original_owner_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory).resolve()
            output = parent / 'output'
            owner = self.supervisor(output)
            owner.args.runner_uid, owner.args.runner_gid = os.getuid(), os.getgid()
            ARTIFACTS.initialize(owner)
            original = parent / 'original'
            output.rename(original)
            replacement = parent / 'replacement'
            replacement.mkdir()
            output.symlink_to(replacement, target_is_directory=True)
            owner.save()
            self.assertTrue((original / 'namespace-receipt.json').is_file())
            self.assertFalse((replacement / 'namespace-receipt.json').exists())
            if getattr(owner, 'artifact_dir_fd', None) is not None:
                os.close(owner.artifact_dir_fd)

    def test_root_run_does_not_reload_helpers_after_initial_import(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory).resolve() / 'output'
            owner = self.supervisor(output)
            owner.args.runner_uid, owner.args.runner_gid = os.getuid(), os.getgid()
            owner.args.timeout_seconds, owner.args.self_test = 20, False
            owner.args.command = [os.sys.executable, str(SCRIPTS / 'qualify-linux-atomic-launcher.py'),
                                  '--output-dir', str(output / 'runner/qualification')]
            original = TimeoutError('actual pre-birth model boundary')
            with patch.object(owner, 'validate', return_value=owner.args.command), \
                    patch.object(owner, 'spawn', side_effect=original), \
                    patch.object(runpy, 'run_path', side_effect=AssertionError('root must not reload helpers')):
                with self.assertRaises(TimeoutError) as result:
                    owner.run()
            self.assertIs(result.exception, original)

    def test_secondary_close_failure_preserves_original_error_object(self):
        original = OSError(5, 'original artifact read failure')
        secondary = OSError(9, 'secondary actual modeled close failure')
        with patch.object(os, 'close', side_effect=secondary):
            ARTIFACTS.close_preserving(123, original)
        self.assertIn('artifact close secondary', original.__notes__[0])
        with patch.object(os, 'close', side_effect=secondary), self.assertRaises(OSError) as result:
            ARTIFACTS.close_preserving(123, None)
        self.assertIs(result.exception, secondary)

    def test_receipt_write_error_survives_later_stream_close_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            owner = self.supervisor(Path(directory))
            original = OSError(5, 'original receipt write')
            secondary = OSError(9, 'secondary receipt close')
            fdopen = os.fdopen

            class FailingStream:
                def __init__(self, fd, mode):
                    self.actual = fdopen(fd, mode)
                def write(self, value):
                    raise original
                def flush(self):
                    self.actual.flush()
                def close(self):
                    self.actual.close()
                    raise secondary
                def __enter__(self):
                    return self
                def __exit__(self, *errors):
                    self.close()

            with patch.object(os, 'fdopen', side_effect=FailingStream):
                owner.save()
            self.assertIs(owner.primary, original)
            self.assertIn('receipt stream close secondary', original.__notes__[0])


if __name__ == '__main__':
    unittest.main()
