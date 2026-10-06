"""资格编排的原命令失败不能被日志关闭次错覆盖。"""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
import tempfile
from unittest.mock import patch

SCRIPTS = Path(__file__).parents[1]
SPEC = importlib.util.spec_from_file_location('engine_qualifier', SCRIPTS / 'qualify-linux-scan-worker-engine.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class EngineQualifierErrorContracts(unittest.TestCase):
    def test_failed_original_command_survives_close_error(self):
        primary = subprocess.CalledProcessError(101, ['cargo', 'test'])
        receipt = {}
        class FailedLog:
            def __enter__(self):
                return self
            def __exit__(self, kind, value, traceback):
                self.close()
            def close(self):
                raise OSError(5, 'actual close error fixture')
        with patch.object(Path, 'open', return_value=FailedLog()), \
                patch.object(MODULE.subprocess, 'run', side_effect=primary), \
                self.assertRaises(subprocess.CalledProcessError) as raised:
            MODULE.run_step(Path('/unused'), Path('/unused'), {}, receipt, 'recovery', ['cargo', 'test'])
        self.assertIs(raised.exception, primary)
        self.assertEqual(receipt['secondary_errors'], [{'stage': 'recovery-log-close', 'kind': 'OSError', 'errno': 5}])

    def test_close_failure_after_success_still_refuses_qualification(self):
        class FailedLog:
            def __enter__(self):
                return self
            def __exit__(self, kind, value, traceback):
                self.close()
            def close(self):
                raise OSError(5, 'close error')
        with patch.object(Path, 'open', return_value=FailedLog()), \
                patch.object(MODULE.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)), \
                self.assertRaises(OSError):
            MODULE.run_step(Path('/unused'), Path('/unused'), {}, {}, 'build', [])


class CurrentCheckoutSourceContracts(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / 'crates/example/src').mkdir(parents=True)
        (self.root / 'Cargo.toml').write_text('[workspace]\n')
        (self.root / 'crates/example/src/lib.rs').write_text('pub fn actual() -> u32 { 1 }\n')
        self.git('init', '-q')
        self.git('add', '.')
        self.git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                 '-c', 'commit.gpgsign=false', 'commit', '-qm', 'actual fixture sources')

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root)

    def test_clean_current_sources_bind_head_and_real_bytes(self):
        result = MODULE.verify_current_sources(self.root)
        self.assertEqual(result['commit'], self.git('rev-parse', 'HEAD').decode().strip())
        self.assertEqual(len(result['files']), 2)
        self.assertEqual(result, MODULE.verify_current_sources(self.root))

    def test_modified_or_staged_source_cannot_substitute_head(self):
        source = self.root / 'crates/example/src/lib.rs'
        source.write_text('pub fn actual() -> u32 { 2 }\n')
        for staged in (False, True):
            if staged:
                self.git('add', str(source))
            with self.subTest(staged=staged), self.assertRaises(ValueError):
                MODULE.verify_current_sources(self.root)

    def test_untracked_cargo_configuration_is_rejected_even_if_ignored(self):
        (self.root / '.cargo').mkdir()
        (self.root / '.cargo/config.toml').write_text('[build]\nrustflags = ["--cfg", "unreviewed"]\n')
        (self.root / '.git/info/exclude').write_text('.cargo/\n')
        with self.assertRaises(ValueError):
            MODULE.verify_current_sources(self.root)

    def test_actual_parent_symlink_is_not_a_current_source(self):
        original = self.root / 'crates/example/src'
        moved = self.root / 'original_source'
        original.rename(moved)
        original.symlink_to(moved, target_is_directory=True)
        with self.assertRaises(ValueError):
            MODULE.verify_current_sources(self.root)


if __name__ == '__main__':
    unittest.main()
