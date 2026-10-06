"""Engine固定入口委托原namespace生命周期，不改历史工具源码身份。"""
import argparse
import importlib.util
from pathlib import Path
import sys
import unittest

SOURCE = Path(__file__).parents[1] / 'run-scan-worker-engine-namespace.py'
SPEC = importlib.util.spec_from_file_location('engine_namespace', SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class EngineNamespaceContracts(unittest.TestCase):
    def args(self, script):
        return argparse.Namespace(output_dir=Path('/tmp/engine-profile-unused'), self_test=False,
                                  command=[sys.executable, str(script.resolve()), '--output-dir', '{qualification_output}'])

    def test_fixed_engine_profile_has_eight_cases(self):
        args = self.args(SOURCE.with_name('qualify-linux-scan-worker-engine.py'))
        command = MODULE.EngineNamespaceSupervisor.command(args, args.output_dir)
        self.assertEqual(command[1], args.command[1])
        self.assertEqual(MODULE.SHARED.PROFILES, {'qualify-linux-scan-worker-engine.py': 8})
        self.assertIs(MODULE.EngineNamespaceSupervisor.cleanup_owner, MODULE.ORIGINAL.cleanup_owner)
        self.assertIs(MODULE.EngineNamespaceSupervisor.wait_once, MODULE.ORIGINAL.wait_once)

    def test_copied_name_and_old_profiles_cannot_substitute_consumer(self):
        for script in [Path('/tmp/qualify-linux-scan-worker-engine.py'), SOURCE.with_name('qualify-linux-atomic-launcher.py')]:
            args = self.args(script)
            with self.subTest(script=script), self.assertRaises(ValueError):
                MODULE.EngineNamespaceSupervisor.command(args, args.output_dir)


if __name__ == '__main__':
    unittest.main()
